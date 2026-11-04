//! Models that use model files (ADR 0066): an entry of a model's `blocks`
//! may name a `file` in place of a `type`. Each use runs on its own clock
//! and may itself use files, to any depth; uses are linked by probes into
//! input blocks. Loading walks the tree and gives a flat list of clock
//! domains (each one `Simulator`) and of links; running is the session's.

use crate::{invalid, load_in, BlockEntry, FileError, Loaded, ModelFile, Subsystem, UseClock};
use simsandbox_core::block::Shape;
use simsandbox_core::clock::Ticks;
use simsandbox_core::graph::BlockId;
use simsandbox_core::registry::Registry;
use simsandbox_core::schedule::Every;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The block type a link drives (ADR 0058's `input.value`).
const INPUT_VALUE: &str = "input.value";

/// A model on its own clock: the blocks of one model file (or of the
/// top level), built.
pub struct Domain {
    /// The path of uses leading to it: empty at the top level, `plant`,
    /// `vehicle/plant`.
    pub path: String,
    /// The model file it comes from; `None` at the top level.
    pub file: Option<PathBuf>,
    pub loaded: Loaded,
    /// Its communication step; unused when it runs on new data.
    pub dt: f64,
    /// A model without states used with no clock (ADR 0066 step 3): it is
    /// evaluated at the start and whenever a link into it delivers a
    /// sample, at that time; with a `sample` section, it runs its whole
    /// sweep then.
    pub on_data: bool,
    /// The outputs its file declares (ADR 0066 step 4b).
    pub declared: Vec<crate::OutputPort>,
}

impl Domain {
    /// Its name in results and reports: its path, or `.` at the top level.
    pub fn name(&self) -> &str {
        if self.path.is_empty() {
            "."
        } else {
            &self.path
        }
    }

    /// A name inside it, as seen from the top level (`plant/car.v`).
    pub fn full(&self, inner: &str) -> String {
        if self.path.is_empty() {
            inner.into()
        } else {
            format!("{}/{inner}", self.path)
        }
    }
}

/// Where a scalar comes from in a domain's model.
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    pub block: BlockId,
    pub port: String,
    pub index: usize,
}

/// A probe that feeds links: its full name (`vehicle/speed`), when it
/// samples, and the domain it records from.
#[derive(Debug, Clone)]
pub struct LinkProbe {
    pub name: String,
    pub every: Every,
    pub domain: usize,
}

/// A link: probe `probe` samples `signal` (a full name) in domain `from`
/// and drives the input block `block` (a full name) of domain `to`.
#[derive(Debug, Clone)]
pub struct Link {
    pub probe: usize,
    pub signal: String,
    pub from: usize,
    pub source: Source,
    pub to: usize,
    pub block: String,
    pub id: BlockId,
}

/// A model that uses model files, ready to run.
pub struct Composition {
    /// The top-level file.
    pub file: ModelFile,
    /// Depth first, in written order, the top level's own blocks first.
    pub domains: Vec<Domain>,
    pub probes: Vec<LinkProbe>,
    pub links: Vec<Link>,
    /// The clocks of the domains that run in time, in a common tick.
    pub ticks: Ticks,
    /// Each domain's clock in `ticks` (its index there); `None` for one
    /// that runs on new data.
    pub clocks: Vec<Option<usize>>,
    /// The order domains evaluate in at a shared instant: a domain that
    /// feeds another first; a loop is broken in written order.
    pub order: Vec<usize>,
    /// Each loop between domains, as it is broken.
    pub loops: Vec<String>,
    /// Digests of the used files and the data they read.
    pub data: BTreeMap<String, String>,
    pub warnings: Vec<String>,
}

impl Composition {
    /// The hash a run file records (ADR 0059), covering every used model
    /// file and the data each reads.
    pub fn hash(&self) -> String {
        crate::runs::model_hash_with(&self.file, &self.data)
    }

    /// The domain a full name lies in, and the name inside it.
    pub fn domain_of<'a>(&self, name: &'a str) -> Option<(usize, &'a str)> {
        self.domains
            .iter()
            .enumerate()
            .filter_map(|(k, d)| {
                if d.path.is_empty() {
                    Some((k, name, 0))
                } else {
                    let rest = name.strip_prefix(&d.path)?.strip_prefix('/')?;
                    Some((k, rest, d.path.len()))
                }
            })
            .max_by_key(|(_, _, depth)| *depth)
            .map(|(k, rest, _)| (k, rest))
    }
}

/// A name not yet in `taken`: `base`, else `base_1`, `base_2`, ….
fn unique(base: &str, taken: &BTreeMap<String, Subsystem>) -> String {
    let mut name = base.to_string();
    let mut n = 1;
    while taken.contains_key(&name) {
        name = format!("{base}_{n}");
        n += 1;
    }
    name
}

/// Whether a model file's text uses model files (or local copies).
pub fn is_composition(text: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| {
            v.get("blocks")?.as_array().map(|b| {
                b.iter()
                    .any(|e| e.get("file").is_some() || e.get("model").is_some())
            })
        })
        .unwrap_or(false)
}

/// Block entries checked as the file is read: a block has a type, or a
/// use has a file with nothing a block has.
pub(crate) fn check_uses(
    blocks: &[BlockEntry],
    subsystems: &BTreeMap<String, Subsystem>,
) -> Result<(), FileError> {
    for (k, b) in blocks.iter().enumerate() {
        let path = format!("blocks[{k}]");
        if b.file.is_some() && b.model.is_some() {
            return Err(invalid(
                format!("{path}.model"),
                "a use names a file or a model defined here, not both",
            ));
        }
        if let Some(m) = b
            .model
            .as_deref()
            .filter(|m| !crate::subsystems::valid_name(m))
        {
            return Err(invalid(
                format!("{path}.model"),
                format!("'{m}': a model's name is letters, digits and '_'"),
            ));
        }
        match b.is_use().then_some(()) {
            None if b.type_name.is_empty() => {
                return Err(invalid(
                    path,
                    "a block needs a type, or a file to use (ADR 0066)",
                ))
            }
            None if b.clock.is_some() => {
                return Err(invalid(
                    format!("{path}.clock"),
                    "only a use of a model file has a clock",
                ))
            }
            None if !b.files.is_empty() => {
                return Err(invalid(
                    format!("{path}.files"),
                    "only a use of a model file replaces files inside it",
                ))
            }
            None => {}
            Some(_) => {
                if !crate::subsystems::valid_name(&b.name) {
                    return Err(invalid(
                        format!("{path}.name"),
                        format!(
                            "'{}': a used model's name is letters, digits and '_'",
                            b.name
                        ),
                    ));
                }
                let block_only = [
                    ("type", !b.type_name.is_empty()),
                    ("method", !b.method.is_empty()),
                    ("equations", !b.equations.is_empty()),
                    ("update", b.update.is_some()),
                    ("settings", !b.settings.is_empty()),
                    ("start", b.start.is_some()),
                ];
                if let Some((key, _)) = block_only.iter().find(|(_, has)| *has) {
                    return Err(invalid(
                        format!("{path}.{key}"),
                        "a use of a model has a name, file or model, files, params, starts, clock and canvas",
                    ));
                }
                if let Some(key) = b
                    .files
                    .keys()
                    .find(|k| k.split('/').any(|p| !crate::subsystems::valid_name(p)))
                {
                    return Err(invalid(
                        format!("{path}.files"),
                        format!("'{key}': a path of used models' names, e.g. plant/engine"),
                    ));
                }
                if let Some(UseClock::Step(dt)) = b.clock {
                    if !(dt.is_finite() && dt > 0.0) {
                        return Err(invalid(
                            format!("{path}.clock.dt"),
                            format!("must be positive, got {dt}"),
                        ));
                    }
                }
            }
        }
    }
    for (name, s) in subsystems {
        if let Some(k) = s.blocks.iter().position(BlockEntry::is_use) {
            return Err(invalid(
                format!("subsystems.{name}.blocks[{k}]"),
                "model files are used at a model's level, not inside a subsystem",
            ));
        }
    }
    Ok(())
}

/// Declared outputs checked as the file is read: plain names, unique and
/// apart from its input blocks' names; `from` a block's output or a path
/// into a model it uses (ADR 0066 step 4b).
pub(crate) fn check_outputs(
    outputs: &[crate::OutputPort],
    blocks: &[BlockEntry],
) -> Result<(), FileError> {
    for (k, o) in outputs.iter().enumerate() {
        let path = format!("outputs[{k}]");
        if !crate::subsystems::valid_name(&o.name) || outputs[..k].iter().any(|p| p.name == o.name)
        {
            return Err(invalid(
                format!("{path}.name"),
                format!(
                    "'{}': output names are unique letters, digits and '_'",
                    o.name
                ),
            ));
        }
        if blocks
            .iter()
            .any(|b| b.name == o.name && b.type_name == INPUT_VALUE)
        {
            return Err(invalid(
                format!("{path}.name"),
                format!(
                    "'{}' is an input block's name: name the output apart",
                    o.name
                ),
            ));
        }
        let first = o.from.split(['/', '.']).next().unwrap_or("");
        if !blocks.iter().any(|b| b.name == first)
            || !(o.from.contains('.') || o.from.contains('/'))
        {
            return Err(invalid(
                format!("{path}.from"),
                format!(
                    "'{}': one of its blocks' outputs (block.port) or a path into a model it uses",
                    o.from
                ),
            ));
        }
    }
    Ok(())
}

/// What one model's names resolve to: its own domain (if it has blocks of
/// its own) and each use's level.
struct Level {
    domain: Option<usize>,
    uses: BTreeMap<String, Level>,
    /// Declared outputs (ADR 0066 step 4b), its own and those of uses on
    /// a shared clock in it (`plant/speed`), by name: what each stands
    /// for, relative to this level.
    aliases: BTreeMap<String, String>,
}

impl Level {
    /// The domain `name` (relative to this level) lies in, and the name
    /// inside it.
    fn resolve<'a>(&'a self, name: &'a str) -> Option<(usize, &'a str)> {
        self.resolve_within(name, 0)
    }

    fn resolve_within<'a>(&'a self, name: &'a str, depth: usize) -> Option<(usize, &'a str)> {
        if let Some(to) = self.aliases.get(name).filter(|_| depth < 32) {
            return self.resolve_within(to, depth + 1);
        }
        // A use moved up from a model on a shared clock has a path for a
        // name (`plant/engine`): the longest that fits.
        let inside = self
            .uses
            .iter()
            .filter(|(k, _)| below(name, k))
            .max_by_key(|(k, _)| k.len());
        match inside {
            Some((k, l)) => l.resolve_within(&name[k.len() + 1..], depth),
            None => self.domain.map(|d| (d, name)),
        }
    }
}

struct Loader<'a> {
    registry: &'a Registry,
    seed: u64,
    domains: Vec<Domain>,
    probes: Vec<LinkProbe>,
    links: Vec<Link>,
    data: BTreeMap<String, String>,
    warnings: Vec<String>,
    /// The files being loaded, outermost first, to find a file that uses
    /// itself.
    stack: Vec<PathBuf>,
    /// Put every use in place, its own clock kept apart by the kernel
    /// ([`load_merged`]), instead of loading uses on their own clocks as
    /// models apart.
    merge: bool,
    /// The subsystems made for uses on a shared clock, by name: where the
    /// model is written and its blocks' names as written, to report
    /// problems inside it where they are written (ADR 0068).
    inlined: BTreeMap<String, Written>,
}

/// Where a model put in place as a subsystem is written.
struct Written {
    /// `models.NAME` for a definition; else the use and its file.
    at: WrittenAt,
    blocks: Vec<String>,
}

enum WrittenAt {
    Definition(String),
    File { at: String, shown: String },
}

/// Read and build a model that uses model files, saved in `folder` (or the
/// thread's folder, `in_folder`; without one, only absolute paths).
pub fn load_composition(
    text: &str,
    registry: &Registry,
    folder: Option<&Path>,
) -> Result<Composition, FileError> {
    let (file, warnings) = ModelFile::from_json(text)?;
    if !file.blocks.iter().any(BlockEntry::is_use) {
        return Err(invalid("blocks", "this model uses no model files"));
    }
    let folder = folder
        .map(Path::to_path_buf)
        .or_else(simsandbox_core::registry::current_folder);
    let mut l = Loader {
        registry,
        seed: file
            .run
            .seed
            .unwrap_or(simsandbox_core::graph::DEFAULT_SEED),
        domains: Vec::new(),
        probes: Vec::new(),
        links: Vec::new(),
        data: BTreeMap::new(),
        warnings,
        stack: Vec::new(),
        merge: false,
        inlined: BTreeMap::new(),
    };
    l.level(&file, "", folder.as_deref(), None, file.run.dt, false)?;
    let mut clocks = Vec::new();
    let mut dts = Vec::new();
    for d in &l.domains {
        clocks.push((!d.on_data).then_some(dts.len()));
        if !d.on_data {
            dts.push(d.dt);
        }
    }
    // With nothing in time, the run is one evaluation of every model at
    // t = 0, in link order (ADR 0067).
    let ticks = Ticks::new(&dts).map_err(|e| invalid("blocks", e))?;
    let (order, loops) = order(&l.domains, &l.links);
    Ok(Composition {
        file,
        domains: l.domains,
        probes: l.probes,
        links: l.links,
        ticks,
        clocks,
        order,
        loops,
        data: l.data,
        warnings: l.warnings,
    })
}

/// A model that uses model files as one model (ADR 0066 step 4b): every
/// use put in place, and each block on its model's clock
/// ([`simsandbox_core::graph::Model::set_clocks`]), drawing as it would in
/// that model alone. A link is a probe feeding the model; an input block
/// it drives becomes a pass-through of the same name. What the session
/// runs, so views, sliders, probes and rewind work as for any model; it
/// keeps the file as written for saving.
pub fn load_merged(
    text: &str,
    registry: &Registry,
    folder: Option<&Path>,
) -> Result<Loaded, FileError> {
    let c = load_composition(text, registry, folder)?;
    let (file, _) = ModelFile::from_json(text)?;
    let folder = folder
        .map(Path::to_path_buf)
        .or_else(simsandbox_core::registry::current_folder);
    let top_seed = file
        .run
        .seed
        .unwrap_or(simsandbox_core::graph::DEFAULT_SEED);
    let mut l = Loader {
        registry,
        seed: top_seed,
        domains: Vec::new(),
        probes: Vec::new(),
        links: Vec::new(),
        data: BTreeMap::new(),
        warnings: Vec::new(),
        stack: Vec::new(),
        merge: true,
        inlined: BTreeMap::new(),
    };
    let mut merged = file.clone();
    let (_, _, mut aliases) = l.inline_shared(&mut merged, "", folder.as_deref())?;
    for o in &file.outputs {
        aliases.insert(o.name.clone(), o.from.clone());
    }
    // Declared names by the flat outputs they stand for.
    let flat = |name: &str| {
        let mut name = name.to_string();
        for _ in 0..32 {
            match aliases.get(&name) {
                Some(to) => name = to.clone(),
                None => break,
            }
        }
        name
    };
    for c in &mut merged.connections {
        if let Some((p, signal)) = c
            .from
            .strip_prefix("probe:")
            .and_then(|r| r.split_once('.'))
        {
            c.from = format!("probe:{p}.{}", flat(signal));
        }
    }
    for p in &mut merged.controls.probes {
        for r in &mut p.record {
            *r = flat(r);
        }
    }
    let shortest = c
        .domains
        .iter()
        .filter(|d| !d.on_data)
        .map(|d| d.dt)
        .fold(f64::INFINITY, f64::min);
    // With nothing in time, the file's own step paces it.
    if shortest.is_finite() {
        merged.run.dt = shortest;
    }
    let mut loaded = with_subsystems(|| load_in(&merged.to_json(), registry, folder.as_deref()))?;
    let mut clocks: Vec<simsandbox_core::graph::Clock> = c
        .domains
        .iter()
        .enumerate()
        .map(|(k, d)| simsandbox_core::graph::Clock {
            dt: d.dt,
            every: c.clocks[k].map_or(0, |i| c.ticks.steps[i]),
            on_data: d.on_data,
        })
        .collect();
    // With nothing in time (ADR 0067), the app evaluates every model at
    // each step of the file's own: its values do not change.
    if clocks.iter().all(|k| k.on_data) {
        clocks.fill(simsandbox_core::graph::Clock {
            dt: merged.run.dt,
            every: 1,
            on_data: false,
        });
    }
    let mut clock_of = Vec::new();
    for id in loaded.model.block_ids() {
        let name = loaded
            .model
            .block(id)
            .map_err(|e| invalid("blocks", e))?
            .spec()
            .name
            .clone();
        // A probe's hold samples on its signal's clock.
        let signal = name
            .strip_prefix("probe:")
            .and_then(|r| r.split_once('/'))
            .map_or(name.as_str(), |(_, s)| s);
        let (d, local) = c
            .domain_of(signal)
            .ok_or_else(|| invalid("blocks", format!("{name} is in no model")))?;
        clock_of.push(d);
        let path = &c.domains[d].path;
        if !path.is_empty() && !name.starts_with("probe:") {
            let stream = simsandbox_core::random::stream(seed_for(top_seed, path), local);
            loaded
                .model
                .set_stream(id, stream)
                .map_err(|e| invalid("blocks", e))?;
        }
    }
    loaded
        .model
        .set_clocks(clocks, clock_of)
        .map_err(|e| invalid("blocks", e))?;
    // The uses' data by path, and what studies reported (`block:key`).
    let reported: BTreeMap<String, String> = std::mem::take(&mut loaded.data)
        .into_iter()
        .filter(|(k, _)| k.contains(':'))
        .collect();
    loaded.data = c.data.clone();
    loaded.data.extend(reported);
    loaded.aliases = aliases.keys().map(|k| (k.clone(), flat(k))).collect();
    // Saving writes the file as written, not the merged one.
    let mut written = file;
    written.folder = folder;
    loaded.file = written;
    // What the uses' own loading noticed, such as a used model's sample
    // not run (ADR 0070).
    for w in &c.warnings {
        if !loaded.warnings.contains(w) {
            loaded.warnings.push(w.clone());
        }
    }
    Ok(loaded)
}

fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(bytes);
    let hex: String = d.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

thread_local! {
    /// Whether files read on this thread may hold `subsystems`: only the
    /// loader's own, which flattens shared uses through them.
    static SUBSYSTEMS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run `f` reading the loader's own files, which hold subsystems.
fn with_subsystems<R>(f: impl FnOnce() -> R) -> R {
    let before = SUBSYSTEMS.with(|s| s.replace(true));
    let r = f();
    SUBSYSTEMS.with(|s| s.set(before));
    r
}

pub(crate) fn subsystems_allowed() -> bool {
    SUBSYSTEMS.with(std::cell::Cell::get)
}

thread_local! {
    /// Model files edited in the app and not yet saved, by canonical path
    /// (ADR 0066 step 4c): read in place of what is on disk.
    static EDITED: std::cell::RefCell<BTreeMap<PathBuf, String>> =
        const { std::cell::RefCell::new(BTreeMap::new()) };
}

/// Read model files in `edited` (by path) as written there rather than
/// from disk, on this thread, until set again.
pub fn set_edited(edited: BTreeMap<PathBuf, String>) {
    let edited = edited
        .into_iter()
        .map(|(p, t)| (canonical(&p), t))
        .collect();
    EDITED.with(|e| *e.borrow_mut() = edited);
}

/// `path` made canonical, also for a file not written yet (by its folder).
pub fn canonical(path: &Path) -> PathBuf {
    if let Ok(p) = path.canonicalize() {
        return p;
    }
    match (path.parent().map(Path::canonicalize), path.file_name()) {
        (Some(Ok(folder)), Some(name)) => folder.join(name),
        _ => path.to_path_buf(),
    }
}

/// A used model file's bytes: as edited (see [`set_edited`]), else from
/// disk.
pub fn read_model(path: &Path) -> std::io::Result<Vec<u8>> {
    let at = canonical(path);
    if let Some(text) = EDITED.with(|e| e.borrow().get(&at).cloned()) {
        return Ok(text.into_bytes());
    }
    std::fs::read(path)
}

/// The model a use `u` in a file saved in `folder` reads, as the run
/// reads it (edited files, its replacements applied), with the file it
/// came from (`None` for a local copy) and the folder its own files are
/// found from.
pub fn used_model(
    u: &BlockEntry,
    folder: Option<&Path>,
) -> Result<(ModelFile, Option<PathBuf>, Option<PathBuf>), String> {
    let (mut model, at) = match (&u.resolved, &u.file) {
        (Some(def), _) => ((**def).clone(), None),
        (None, Some(name)) => {
            let at = PathBuf::from(name);
            let at = match folder {
                Some(f) if at.is_relative() => f.join(at),
                _ => at,
            };
            let bytes = read_model(&at).map_err(|e| format!("{}: {e}", at.display()))?;
            let text = String::from_utf8(bytes)
                .map_err(|_| format!("{}: not a text file", at.display()))?;
            let (m, _) =
                ModelFile::from_json(&text).map_err(|e| format!("{}: {e}", at.display()))?;
            (m, Some(at))
        }
        (None, None) => return Err(format!("'{}' uses no model", u.name)),
    };
    replace_files(&mut model, &u.files, folder)?;
    set_starts(&mut model, &u.starts)?;
    let inner = match &at {
        Some(p) => p.parent().map(Path::to_path_buf),
        None => folder.map(Path::to_path_buf),
    };
    model.folder.clone_from(&inner);
    Ok((model, at, inner))
}

/// Whether the use `u` (in a file saved in `folder`) runs on its user's
/// clock: `"shared"`, or no clock, a model that writes no `run.dt`, and a
/// wire reaching it (`wired`), since wires never cross clocks (ADR 0068).
pub fn runs_shared(u: &BlockEntry, folder: Option<&Path>, wired: bool) -> bool {
    match u.clock {
        Some(UseClock::Shared) => true,
        Some(UseClock::Step(_)) => false,
        None if !wired => false,
        None => match (&u.resolved, &u.file) {
            (Some(def), _) => !def.run.dt_written,
            (None, Some(name)) => {
                let at = PathBuf::from(name);
                let at = match folder {
                    Some(f) if at.is_relative() => f.join(at),
                    _ => at,
                };
                read_model(&at)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .is_some_and(|v| v.get("run").and_then(|r| r.get("dt")).is_none())
            }
            (None, None) => false,
        },
    }
}

/// Find the definitions the uses in `file` name (ADR 0068): in its own
/// `models`, then in `outer` (those of the models holding it). Each is
/// read and resolved in turn; one that uses itself is refused.
pub(crate) fn resolve_models(
    file: &mut ModelFile,
    outer: &BTreeMap<String, serde_json::Value>,
    stack: &mut Vec<String>,
) -> Result<(), FileError> {
    let studies = |b: &BlockEntry| b.type_name == crate::STUDY && b.settings.contains_key("model");
    if file.models.is_empty() && file.blocks.iter().all(|b| b.model.is_none() && !studies(b)) {
        return Ok(());
    }
    let mut scope = outer.clone();
    scope.extend(file.models.iter().map(|(k, v)| (k.clone(), v.clone())));
    // Every definition is read, used or not, so a mistake shows.
    for (name, v) in &file.models {
        if !crate::subsystems::valid_name(name) {
            return Err(invalid(
                format!("models.{name}"),
                "a model's name is letters, digits and '_'",
            ));
        }
        definition(name, v)?;
    }
    let folder = file.folder.clone();
    for (k, b) in file.blocks.iter_mut().enumerate() {
        // A study of a definition (ADR 0070): its text, with the
        // definitions in scope but itself, which it may use.
        if studies(b) {
            let name = b.settings["model"].clone();
            let at = format!("blocks[{k}].settings.model");
            let v = scope.get(&name).ok_or_else(|| {
                invalid(
                    &at,
                    format!("no model named '{name}' is defined here (`models`)"),
                )
            })?;
            if stack.contains(&name) {
                return Err(invalid(
                    &at,
                    format!("a model studies itself: {} → {name}", stack.join(" → ")),
                ));
            }
            definition(&name, v)?;
            let mut text = v.clone();
            if let Some(o) = text.as_object_mut() {
                o.entry("format")
                    .or_insert(serde_json::Value::from(crate::FORMAT_VERSION));
                let models = o
                    .entry("models")
                    .or_insert_with(|| serde_json::Value::Object(Default::default()));
                if let Some(m) = models.as_object_mut() {
                    for (n, d) in scope.iter().filter(|(n, _)| **n != name) {
                        m.entry(n.clone()).or_insert_with(|| d.clone());
                    }
                }
            }
            b.study_model = Some(text.to_string());
            continue;
        }
        let Some(name) = b.model.clone() else {
            continue;
        };
        let at = format!("blocks[{k}].model");
        let v = scope.get(&name).ok_or_else(|| {
            invalid(
                &at,
                format!("no model named '{name}' is defined here (`models`)"),
            )
        })?;
        if stack.contains(&name) {
            return Err(invalid(
                &at,
                format!("a model uses itself: {} → {name}", stack.join(" → ")),
            ));
        }
        let mut m = definition(&name, v)?;
        m.folder.clone_from(&folder);
        stack.push(name.clone());
        let inner = resolve_models(&mut m, &scope, stack);
        stack.pop();
        inner.map_err(|e| nest(&format!("models.{name}"), e))?;
        b.resolved = Some(Box::new(m));
    }
    Ok(())
}

/// Write `file`'s subsystems as models defined in it (ADR 0068 point 10):
/// each input port an input block wired to what the port fed, each output
/// port a declared output, each use of one a use of the model, and wires
/// to and from their ports paths (`car1/lead`, `car1/v`). Returns whether
/// anything changed. The model built is the same.
pub fn convert_subsystems(file: &mut ModelFile) -> bool {
    let names: Vec<String> = file.subsystems.keys().cloned().collect();
    convert_some_subsystems(file, &names)
}

/// [`convert_subsystems`] for the subsystems `names` only; the others stay
/// subsystems, and a definition made holds those it uses.
pub fn convert_some_subsystems(file: &mut ModelFile, names: &[String]) -> bool {
    let names: Vec<String> = names
        .iter()
        .filter(|n| file.subsystems.contains_key(*n))
        .cloned()
        .collect();
    if names.is_empty() {
        return false;
    }
    let uses = |blocks: &mut Vec<BlockEntry>, connections: &mut Vec<crate::ConnectionEntry>| {
        let instances: Vec<String> = blocks
            .iter()
            .filter(|b| names.contains(&b.type_name))
            .map(|b| b.name.clone())
            .collect();
        for b in blocks.iter_mut().filter(|b| names.contains(&b.type_name)) {
            b.model = Some(std::mem::take(&mut b.type_name));
            b.method = Default::default();
        }
        // `car1.lead` → `car1/lead`: a port of a use is reached by path.
        let path = |end: &mut String| {
            if let Some((node, rest)) = end.split_once('.') {
                if instances.iter().any(|i| i == node) {
                    *end = format!("{node}/{rest}");
                }
            }
        };
        for c in connections.iter_mut() {
            path(&mut c.from);
            path(&mut c.to);
        }
    };
    let converted: Vec<(String, Subsystem)> = names
        .iter()
        .map(|n| (n.clone(), file.subsystems.remove(n).expect("filtered")))
        .collect();
    for sub in file.subsystems.values_mut() {
        uses(&mut sub.blocks, &mut sub.connections);
    }
    for (name, sub) in converted {
        let mut def = ModelFile::new();
        def.description = sub.description;
        def.params = sub.params;
        def.blocks = sub.blocks;
        def.connections = sub.connections;
        uses(&mut def.blocks, &mut def.connections);
        for port in &sub.inputs {
            def.blocks.push(BlockEntry {
                name: port.name.clone(),
                type_name: "input.value".into(),
                canvas: port.canvas,
                ..Default::default()
            });
            for to in &port.to {
                def.connections.push(crate::ConnectionEntry {
                    from: format!("{}.y", port.name),
                    to: to.clone(),
                });
            }
        }
        def.outputs = sub.outputs;
        // The subsystems it still uses go with it.
        let mut need: Vec<String> = def.blocks.iter().map(|b| b.type_name.clone()).collect();
        while let Some(t) = need.pop() {
            if let (Some(sd), false) = (file.subsystems.get(&t), def.subsystems.contains_key(&t)) {
                need.extend(sd.blocks.iter().map(|b| b.type_name.clone()));
                def.subsystems.insert(t, sd.clone());
            }
        }
        file.models.insert(name, as_definition(&def));
    }
    uses(&mut file.blocks, &mut file.connections);
    let _ = file.resolve_models();
    true
}

/// A definition of a `models` section read as a model, its own uses not
/// yet resolved (ADR 0068).
pub fn read_definition(name: &str, v: &serde_json::Value) -> Result<ModelFile, String> {
    definition(name, v).map_err(|e| e.to_string())
}

/// `model` as a definition of a `models` section: written like a model
/// file, without `format`.
pub fn as_definition(model: &ModelFile) -> serde_json::Value {
    let mut v: serde_json::Value =
        serde_json::from_str(&model.to_json()).expect("model files always serialize");
    if let Some(o) = v.as_object_mut() {
        o.remove("format");
    }
    v
}

/// The definition `name` of a `models` section, read.
fn definition(name: &str, v: &serde_json::Value) -> Result<ModelFile, FileError> {
    let at = format!("models.{name}");
    let mut v = v.clone();
    let obj = v
        .as_object_mut()
        .ok_or_else(|| invalid(&at, "a model is written like a model file, as an object"))?;
    obj.entry("format")
        .or_insert(serde_json::Value::from(crate::FORMAT_VERSION));
    if let Some(c) = obj.get("controls").and_then(|c| c.as_object()) {
        if let Some(k) = c.keys().find(|k| *k != "probes") {
            return Err(invalid(
                format!("{at}.controls.{k}"),
                "a model defined in a file holds only probes in its controls",
            ));
        }
    }
    ModelFile::from_json_unresolved(&v.to_string())
        .map(|(m, _)| m)
        .map_err(|e| nest(&at, e))
}

/// `e`, found inside what is written at `prefix`, at its full path.
fn nest(prefix: &str, e: FileError) -> FileError {
    match e {
        FileError::Invalid { path, message } if path.is_empty() => invalid(prefix, message),
        FileError::Invalid { path, message } => invalid(format!("{prefix}.{path}"), message),
        e => invalid(prefix, e),
    }
}

/// Put a use's start states (`starts`, by the path of the inner block;
/// ADR 0068) into `model`, the model it reads: a block's own name sets its
/// `start`; a deeper path goes into that use's `starts`.
pub fn set_starts(
    model: &mut ModelFile,
    starts: &BTreeMap<String, crate::StartEntry>,
) -> Result<(), String> {
    for (key, s) in starts {
        match key.split_once('/') {
            None => {
                let b = model
                    .blocks
                    .iter_mut()
                    .find(|b| &b.name == key && !b.is_use())
                    .ok_or_else(|| format!("'{key}': no block of that name"))?;
                b.start = Some(s.clone());
            }
            Some((first, rest)) => {
                let b = model
                    .blocks
                    .iter_mut()
                    .find(|b| b.name == first)
                    .ok_or_else(|| format!("'{key}': no use or subsystem '{first}'"))?;
                b.starts.insert(rest.to_string(), s.clone());
            }
        }
    }
    Ok(())
}

/// Put `files` (replacements written in a use, by path of uses inside
/// the model it uses; ADR 0066 step 4c) into `model`, the file that use
/// reads: a replacement of one of its own uses becomes that use's `file`,
/// found from `folder` (the folder of the file the replacements are
/// written in); a deeper one goes into that use's own `files`, where it
/// wins over one of the same path written there.
pub fn replace_files(
    model: &mut ModelFile,
    files: &BTreeMap<String, String>,
    folder: Option<&Path>,
) -> Result<(), String> {
    // Shallow first, so a deeper replacement lands in the replaced use.
    let mut keys: Vec<&String> = files.keys().collect();
    keys.sort_by_key(|k| k.matches('/').count());
    for key in keys {
        let (first, rest) = match key.split_once('/') {
            Some((f, r)) => (f, Some(r)),
            None => (key.as_str(), None),
        };
        let entry = model
            .blocks
            .iter_mut()
            .find(|b| b.name == first && b.is_use())
            .ok_or_else(|| format!("'{key}': '{first}' is not a use of a model file"))?;
        let with = PathBuf::from(&files[key]);
        // Found from the folder of the file it is written in, wherever it
        // is passed down to.
        let at = match (folder, with.is_absolute()) {
            (_, true) => with,
            (Some(f), false) => f.join(with),
            (None, false) => {
                return Err(format!(
                    "'{}' is found from the model's folder: save it first, or give a full path",
                    files[key]
                ))
            }
        };
        let at = at.to_string_lossy().into_owned();
        match rest {
            None => {
                entry.file = Some(at);
                entry.model = None;
                entry.resolved = None;
            }
            Some(rest) => {
                entry.files.insert(rest.to_string(), at);
            }
        }
    }
    Ok(())
}

/// A seed for the domain at `path`, mixed from the top level's: two uses
/// of one file draw differently (SplitMix64 over FNV-1a of the path).
fn seed_for(seed: u64, path: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in path.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
    }
    let mut z = (seed ^ h).wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Whether `name` lies inside `path`: `path/…`.
fn below(name: &str, path: &str) -> bool {
    name.len() > path.len() && name.starts_with(path) && name.as_bytes()[path.len()] == b'/'
}

/// Whether an end of a connection (relative to a model) lies in one of
/// its uses on their own clocks, named by their paths.
fn in_use(end: &str, uses: &[&str]) -> bool {
    let name = end
        .strip_prefix("probe:")
        .and_then(|r| r.split_once('.'))
        .map_or(end, |(_, signal)| signal);
    uses.iter().any(|u| below(name, u))
}

/// Whether a wire (not a probe link) among `connections` reaches into or
/// out of the use `name`.
pub fn wired(connections: &[crate::ConnectionEntry], name: &str) -> bool {
    connections
        .iter()
        .any(|c| !c.from.starts_with("probe:") && (below(&c.from, name) || below(&c.to, name)))
}

/// A use on its own clock, to load at a model's level: written there, or
/// moved up from a model on a shared clock (ADR 0066 step 2b).
struct OwnClock {
    /// Its path from the model: `controller`, or `plant/engine`.
    name: String,
    entry: BlockEntry,
    /// The folder its file is found from.
    folder: Option<PathBuf>,
    /// The declared parameters formulas in its `params` are of: those of
    /// the file each was written in.
    scope: BTreeMap<String, crate::ParamDecl>,
    /// Where it is written, for errors.
    at: String,
}

/// What putting uses on a shared clock in place gives: the uses on their
/// own clocks moved up, the uses put in place (to pass wires through),
/// and the declared outputs reached through them, by path.
type Inlining = (
    Vec<OwnClock>,
    BTreeMap<String, Inlined>,
    BTreeMap<String, String>,
);

/// A use on a shared clock put in place as a subsystem, and those inside
/// it, to pass wires through at any depth.
struct Inlined {
    def: String,
    inner: BTreeMap<String, Inlined>,
}

impl Inlined {
    fn renamed(mut self, names: &BTreeMap<String, String>) -> Self {
        if let Some(n) = names.get(&self.def) {
            self.def = n.clone();
        }
        self.inner = self
            .inner
            .into_iter()
            .map(|(k, v)| (k, v.renamed(names)))
            .collect();
        self
    }
}

/// A port name not yet used by `def`, from `base` made plain.
fn port_name(def: &Subsystem, base: &str) -> String {
    let base: String = base
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let taken =
        |n: &str| def.inputs.iter().any(|p| p.name == n) || def.outputs.iter().any(|p| p.name == n);
    let mut name = base.clone();
    let mut n = 1;
    while taken(&name) {
        name = format!("{base}_{n}");
        n += 1;
    }
    name
}

/// Pass a wire into `inner` (an input block, at any depth of models on a
/// shared clock) through ports: the port of `inl`'s subsystem to wire to.
fn expose_input(
    subs: &mut BTreeMap<String, Subsystem>,
    inl: &Inlined,
    inner: &str,
) -> Result<String, String> {
    if let Some((first, rest)) = inner.split_once('/') {
        let child = inl.inner.get(first).ok_or_else(|| {
            format!(
                "'{first}' is not a model on a shared clock here: wire into input blocks of \
                 models on a shared clock; a model on its own clock takes a probe link"
            )
        })?;
        let port = expose_input(subs, child, rest)?;
        let target = format!("{first}.{port}");
        let def = subs.get_mut(&inl.def).expect("inlined");
        if def.inputs.iter().any(|p| p.to == [target.clone()]) {
            return Err("wired twice".into());
        }
        let name = port_name(def, inner);
        def.inputs.push(crate::InputPort {
            name: name.clone(),
            to: vec![target],
            canvas: None,
        });
        return Ok(name);
    }
    let def = subs.get_mut(&inl.def).expect("inlined");
    if def.inputs.iter().any(|p| p.name == inner) {
        return Err("wired twice".into());
    }
    if !def
        .blocks
        .iter()
        .any(|b| b.name == inner && b.type_name == INPUT_VALUE)
    {
        return Err(format!(
            "a wire into a model on a shared clock goes to one of its input blocks ({INPUT_VALUE})"
        ));
    }
    let fed = format!("{inner}.y");
    let to: Vec<String> = def
        .connections
        .iter()
        .filter(|w| w.from == fed)
        .map(|w| w.to.clone())
        .collect();
    def.connections.retain(|w| w.from != fed);
    def.blocks.retain(|b| b.name != inner);
    def.inputs.push(crate::InputPort {
        name: inner.into(),
        to,
        canvas: None,
    });
    Ok(inner.into())
}

/// Pass a wire from `inner` (an output, at any depth of models on a
/// shared clock) through ports: the port of `inl`'s subsystem to wire
/// from.
fn expose_output(
    subs: &mut BTreeMap<String, Subsystem>,
    inl: &Inlined,
    inner: &str,
) -> Result<String, String> {
    let from = match inner.split_once('/') {
        Some((first, rest)) if inl.inner.contains_key(first) => {
            format!("{first}.{}", expose_output(subs, &inl.inner[first], rest)?)
        }
        Some((first, _)) => {
            return Err(format!(
                "'{first}' is not a model on a shared clock here: a model on its own clock \
                 is read through a probe link"
            ))
        }
        None if !inner.contains('.') && subs[&inl.def].outputs.iter().any(|p| p.name == inner) => {
            return Ok(inner.to_string());
        }
        None => {
            let def = &subs[&inl.def];
            let own = inner
                .split_once('.')
                .is_some_and(|(b, _)| def.blocks.iter().any(|e| e.name == b));
            if !own {
                return Err(format!("no output '{inner}'"));
            }
            inner.to_string()
        }
    };
    let def = subs.get_mut(&inl.def).expect("inlined");
    if let Some(p) = def.outputs.iter().find(|p| p.from == from) {
        return Ok(p.name.clone());
    }
    let name = port_name(def, inner);
    def.outputs.push(crate::OutputPort {
        name: name.clone(),
        from,
        canvas: None,
    });
    Ok(name)
}

/// Make an input block a pass-through, `y = u`, under the same name and
/// unit: a link feeds its input when the model is merged (ADR 0066 step
/// 4b), and what views read (`speed.y`) stays.
fn pass_through(b: &mut BlockEntry) {
    let unit = b
        .settings
        .get("unit")
        .filter(|u| !u.trim().is_empty() && u.trim() != "1");
    let input = match unit {
        Some(u) => format!("input u [{u}]"),
        None => "input u".to_string(),
    };
    b.type_name = "eq.equations".into();
    b.equations = vec![input, "y = u".into()];
    b.settings.clear();
    b.params.clear();
    b.method = Default::default();
}

/// Pass a link through ports to `inner`: an input block (made a
/// pass-through) or a block's input, at any depth of models put in place.
fn expose_link(
    subs: &mut BTreeMap<String, Subsystem>,
    inl: &Inlined,
    inner: &str,
) -> Result<String, String> {
    let target = match inner.split_once('/') {
        Some((first, rest)) => {
            let child = inl
                .inner
                .get(first)
                .ok_or_else(|| format!("'{first}' is not a model used here"))?;
            format!("{first}.{}", expose_link(subs, child, rest)?)
        }
        None if inner.contains('.') => inner.to_string(),
        None => {
            let def = subs.get_mut(&inl.def).expect("inlined");
            let b = def
                .blocks
                .iter_mut()
                .find(|b| b.name == inner && b.type_name == INPUT_VALUE)
                .ok_or_else(|| format!("no input block '{inner}'"))?;
            pass_through(b);
            format!("{inner}.u")
        }
    };
    let def = subs.get_mut(&inl.def).expect("inlined");
    if let Some(p) = def.inputs.iter().find(|p| p.to == [target.clone()]) {
        return Ok(p.name.clone());
    }
    let name = port_name(def, inner);
    def.inputs.push(crate::InputPort {
        name: name.clone(),
        to: vec![target],
        canvas: None,
    });
    Ok(name)
}

/// Copy the declared parameters a formula names from `scope` (where it is
/// written) into `into` (where it will be compiled), so a formula given to
/// a model one level down keeps its meaning.
fn carry(
    value: &crate::ParamValue,
    scope: &BTreeMap<String, crate::ParamDecl>,
    into: &mut BTreeMap<String, crate::ParamDecl>,
    path: &str,
) -> Result<(), FileError> {
    let crate::ParamValue::Formula(text) = value else {
        return Ok(());
    };
    let ast = simsandbox_expr::parse(text).map_err(|e| invalid(path, format!("'{text}': {e}")))?;
    for (name, _) in ast.names() {
        let name = name.to_string();
        let Some(d) = scope.get(&name) else {
            return Err(invalid(
                path,
                format!("'{name}' is not a parameter declared where this formula is written"),
            ));
        };
        match into.get(&name) {
            Some(there) if there != d => {
                return Err(invalid(
                    path,
                    format!("the used model declares '{name}' too: rename one of them"),
                ))
            }
            _ => {
                into.insert(name, d.clone());
            }
        }
    }
    Ok(())
}

/// An error in a model's own blocks, at the place it is written: the
/// own-blocks file leaves uses and links out, so indices move back.
fn written_at(e: FileError, blocks: &[usize], connections: &[usize]) -> FileError {
    let FileError::Invalid { path, message } = e else {
        return e;
    };
    let remap = |path: &str, key: &str, map: &[usize]| -> Option<String> {
        let rest = path.strip_prefix(key)?.strip_prefix('[')?;
        let (i, tail) = rest.split_once(']')?;
        let i: usize = i.parse().ok()?;
        Some(format!("{key}[{}]{tail}", map.get(i)?))
    };
    let path = remap(&path, "blocks", blocks)
        .or_else(|| remap(&path, "connections", connections))
        .unwrap_or(path);
    FileError::Invalid { path, message }
}

impl Loader<'_> {
    /// A problem inside a model put in place on a shared clock, reported
    /// where it is written: `models.follower.blocks[0].equations`, or the
    /// use of a file with the file's path (ADR 0068).
    fn inside_error(&self, e: FileError, built: &ModelFile) -> FileError {
        let FileError::Invalid { path, message } = &e else {
            return e;
        };
        let Some(rest) = path.strip_prefix("subsystems.") else {
            return e;
        };
        let (def, rest) = rest.split_once('.').unwrap_or((rest, ""));
        let Some(w) = self.inlined.get(def) else {
            return e;
        };
        // `blocks[i]` of the subsystem: the block of that name as written.
        let rest = (|| {
            let tail = rest.strip_prefix("blocks[")?;
            let (i, tail) = tail.split_once(']')?;
            let name = &built
                .subsystems
                .get(def)?
                .blocks
                .get(i.parse::<usize>().ok()?)?
                .name;
            let j = w.blocks.iter().position(|b| b == name)?;
            Some(format!("blocks[{j}]{tail}"))
        })()
        .unwrap_or_else(|| rest.to_string());
        match &w.at {
            WrittenAt::Definition(m) => invalid(format!("models.{m}.{rest}"), message),
            WrittenAt::File { at, shown } => {
                invalid(at.clone(), format!("{shown}: {rest}: {message}"))
            }
        }
    }

    /// Load one model at `path` (its own blocks as a domain, then each
    /// use, then its links) and return what its names resolve to.
    fn level(
        &mut self,
        file: &ModelFile,
        path: &str,
        folder: Option<&Path>,
        source: Option<&Path>,
        dt: f64,
        clocked: bool,
    ) -> Result<Level, FileError> {
        let mut file = file.clone();
        let (lifted, _, aliases) = self.inline_shared(&mut file, path, folder)?;
        let file = &file;
        let full = |name: &str| {
            if path.is_empty() {
                name.to_string()
            } else {
                format!("{path}/{name}")
            }
        };
        // Its uses on their own clocks: written here, then those moved up
        // from models on a shared clock.
        let mut uses: Vec<OwnClock> = file
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| b.is_use())
            .map(|(k, b)| OwnClock {
                name: b.name.clone(),
                entry: b.clone(),
                folder: folder.map(Path::to_path_buf),
                scope: file.params.clone(),
                at: format!("blocks[{k}]"),
            })
            .collect();
        uses.extend(lifted);
        let names: Vec<&str> = uses.iter().map(|u| u.name.as_str()).collect();
        // Its own blocks and the connections among them (with probes
        // feeding them) form its domain.
        let own: Vec<usize> = (0..file.blocks.len())
            .filter(|&k| !file.blocks[k].is_use())
            .collect();
        let inside: Vec<usize> = (0..file.connections.len())
            .filter(|&k| {
                let c = &file.connections[k];
                !in_use(&c.from, &names) && !in_use(&c.to, &names)
            })
            .collect();
        let mut level = Level {
            domain: None,
            uses: BTreeMap::new(),
            aliases,
        };
        for o in &file.outputs {
            level.aliases.insert(o.name.clone(), o.from.clone());
        }
        // A use runs its model as written (ADR 0070): a `sample` of its own
        // is not run. The top level's is its experiment, run apart.
        if file.sample.is_some() && !path.is_empty() {
            self.warnings.push(format!(
                "{path}: its model's `sample` is not run: a use runs its model as written; \
                 put a study block around it to study it (ADR 0070)"
            ));
        }
        if !own.is_empty() {
            let mut f = file.clone();
            f.sample = None;
            f.blocks = own.iter().map(|&k| file.blocks[k].clone()).collect();
            f.connections = inside
                .iter()
                .map(|&k| file.connections[k].clone())
                .collect();
            let holds: Vec<&str> = f
                .connections
                .iter()
                .filter_map(|c| {
                    c.from
                        .strip_prefix("probe:")?
                        .split_once('.')
                        .map(|(p, _)| p)
                })
                .collect();
            let probes = file
                .controls
                .probes
                .iter()
                .filter(|p| holds.contains(&p.name.as_str()))
                .cloned()
                .collect();
            f.controls = Default::default();
            // Declared outputs resolve at this level, not in the domain.
            f.outputs.clear();
            f.controls.probes = probes;
            f.run.dt = dt;
            f.run.t_end = None;
            f.run.inputs.clear();
            if !path.is_empty() {
                f.run.seed = Some(seed_for(self.seed, path));
            }
            let loaded = match with_subsystems(|| load_in(&f.to_json(), self.registry, folder)) {
                Ok(l) => l,
                Err(e) => return Err(self.inside_error(written_at(e, &own, &inside), &f)),
            };
            // A model without states runs on new data, unless its use
            // gives it a clock: then it is evaluated at each step.
            let on_data = loaded.plan.is_some() && !clocked;
            for w in &loaded.warnings {
                self.warnings.push(if path.is_empty() {
                    w.clone()
                } else {
                    format!("{path}: {w}")
                });
            }
            for (key, d) in &loaded.data {
                self.data.insert(full(key), d.clone());
            }
            level.domain = Some(self.domains.len());
            self.domains.push(Domain {
                path: path.into(),
                file: source.map(Path::to_path_buf),
                loaded,
                dt,
                on_data,
                declared: file.outputs.clone(),
            });
        }
        for u in &uses {
            let child = self
                .use_file(&u.entry, &full(&u.name), u.folder.as_deref(), &u.scope)
                .map_err(|e| match e {
                    FileError::Invalid { path, message } if path.is_empty() => {
                        invalid(u.at.clone(), message)
                    }
                    e => invalid(u.at.clone(), e),
                })?;
            level.uses.insert(u.name.clone(), child);
        }
        // Links: connections reaching into a use.
        for (k, c) in file.connections.iter().enumerate() {
            if inside.contains(&k) {
                continue;
            }
            self.link(file, &level, path, &format!("connections[{k}]"), c)?;
        }
        Ok(level)
    }

    /// Find, read and parse the model file `u` uses from `folder`; its
    /// digest goes into the hash under `path`. Returns the model, where it
    /// was found, and the canonical path to push while it loads (a file
    /// that uses itself is refused).
    fn read_use(
        &mut self,
        u: &BlockEntry,
        path: &str,
        folder: Option<&Path>,
    ) -> Result<(ModelFile, PathBuf, PathBuf), FileError> {
        // A model defined in the file (ADR 0068): its files are found
        // from here.
        if let Some(def) = &u.resolved {
            let mut model = (**def).clone();
            replace_files(&mut model, &u.files, folder).map_err(|e| invalid("files", e))?;
            set_starts(&mut model, &u.starts).map_err(|e| invalid("starts", e))?;
            self.data
                .insert(format!("{path}.file"), digest(model.to_json().as_bytes()));
            let here = folder.map_or_else(PathBuf::new, Path::to_path_buf);
            let name = u.model.as_deref().unwrap_or_default();
            let shown = here.join(format!("{} (model {name})", u.name));
            return Ok((model, shown.clone(), shown));
        }
        let name = u.file.as_deref().expect("a use names a file");
        let found = PathBuf::from(name);
        let found = match (folder, found.is_absolute()) {
            (_, true) => found,
            (Some(f), false) => f.join(found),
            (None, false) => {
                return Err(invalid(
                    "file",
                    format!(
                    "'{name}' is found from the model's folder: save it first, or give a full path"
                ),
                ))
            }
        };
        let canonical = found.canonicalize().unwrap_or_else(|_| found.clone());
        if self.stack.contains(&canonical) {
            let chain: Vec<String> = self
                .stack
                .iter()
                .chain([&canonical])
                .map(|p| p.display().to_string())
                .collect();
            return Err(invalid(
                "file",
                format!("a model file uses itself: {}", chain.join(" → ")),
            ));
        }
        let bytes =
            read_model(&found).map_err(|e| invalid("file", format!("{}: {e}", found.display())))?;
        let text =
            String::from_utf8(bytes.clone()).map_err(|_| invalid("file", "not a text file"))?;
        let (mut model, _) = ModelFile::from_json(&text)
            .map_err(|e| invalid("file", format!("{}: {e}", found.display())))?;
        replace_files(&mut model, &u.files, folder).map_err(|e| invalid("files", e))?;
        set_starts(&mut model, &u.starts).map_err(|e| invalid("starts", e))?;
        self.data.insert(format!("{path}.file"), digest(&bytes));
        Ok((model, found, canonical))
    }

    /// Load the model file `u` uses on its own clock, at `path`; formulas
    /// in its `params` are of `scope`.
    fn use_file(
        &mut self,
        u: &BlockEntry,
        path: &str,
        folder: Option<&Path>,
        scope: &BTreeMap<String, crate::ParamDecl>,
    ) -> Result<Level, FileError> {
        let (mut model, found, canonical) = self.read_use(u, path, folder)?;
        set_params(&mut model, &u.params, scope)?;
        let (dt, clocked) = match u.clock {
            Some(UseClock::Step(dt)) => (dt, true),
            _ => (model.run.dt, false),
        };
        self.stack.push(canonical);
        let child_folder = found.parent().map(Path::to_path_buf);
        let level = self
            .level(
                &model,
                path,
                child_folder.as_deref(),
                Some(&found),
                dt,
                clocked,
            )
            .map_err(|e| invalid("", format!("{}: {e}", found.display())));
        self.stack.pop();
        level
    }

    /// Put each use of `file` on a shared clock in place as a subsystem
    /// of `file` made from the used file (ADR 0066 steps 2 and 2b), so it
    /// flattens into `file`'s model: named `use/inner`, fused with its
    /// neighbours where it can be. Uses inside it on a shared clock are
    /// put in place first. What a subsystem cannot hold moves up into
    /// `file` under the use's name: its uses on their own clocks (returned,
    /// to load at `file`'s level), its probes and the connections from
    /// them (holds and links), and its groups. Wires at any depth pass
    /// through ports: into an input block (the block dropped), from an
    /// output.
    fn inline_shared(
        &mut self,
        file: &mut ModelFile,
        path: &str,
        folder: Option<&Path>,
    ) -> Result<Inlining, FileError> {
        let shared: Vec<usize> = (0..file.blocks.len())
            .filter(|&k| {
                file.blocks[k].is_use()
                    && (self.merge
                        || runs_shared(
                            &file.blocks[k],
                            folder,
                            wired(&file.connections, &file.blocks[k].name),
                        ))
            })
            .collect();
        let mut tree: BTreeMap<String, Inlined> = BTreeMap::new();
        let mut lifted: Vec<OwnClock> = Vec::new();
        let mut aliases: BTreeMap<String, String> = BTreeMap::new();
        for k in shared {
            let u = file.blocks[k].clone();
            let at = format!("blocks[{k}]");
            let full = if path.is_empty() {
                u.name.clone()
            } else {
                format!("{path}/{}", u.name)
            };
            let (mut child, found, canonical) = self
                .read_use(&u, &full, folder)
                .map_err(|e| invalid(&at, e))?;
            let child_folder = found.parent().map(Path::to_path_buf);
            let shown = found.display().to_string();
            let fail = |e: FileError| invalid(&at, format!("{shown}: {e}"));
            self.stack.push(canonical);
            let inner = self.inline_shared(&mut child, &full, child_folder.as_deref());
            self.stack.pop();
            let (inner_lifted, child_tree, child_aliases) = inner.map_err(fail)?;
            // Its declared outputs, and those of its shared uses, by path
            // from here.
            for (k2, v) in child_aliases {
                aliases.insert(format!("{}/{k2}", u.name), format!("{}/{v}", u.name));
            }
            for o in &child.outputs {
                aliases.insert(
                    format!("{}/{}", u.name, o.name),
                    format!("{}/{}", u.name, o.from),
                );
            }
            // Its uses on their own clocks move up here.
            let mut moving: Vec<OwnClock> = child
                .blocks
                .iter()
                .enumerate()
                .filter(|(_, b)| b.is_use())
                .map(|(j, b)| OwnClock {
                    name: b.name.clone(),
                    entry: b.clone(),
                    folder: child_folder.clone(),
                    scope: child.params.clone(),
                    at: format!("{at}: {shown}: blocks[{j}]"),
                })
                .collect();
            moving.extend(inner_lifted);
            let written_blocks: Vec<String> = child.blocks.iter().map(|b| b.name.clone()).collect();
            child.blocks.retain(|b| !b.is_use());
            let own_clock: Vec<String> = moving.iter().map(|m| m.name.clone()).collect();
            let own_clock: Vec<&str> = own_clock.iter().map(String::as_str).collect();
            // This use's values: those reaching a model on its own clock
            // inside go to it; the rest to the subsystem's use.
            let mut values = BTreeMap::new();
            for (key, v) in &u.params {
                let kpath = format!("{at}.params.{key}");
                match moving
                    .iter_mut()
                    .filter(|m| below(key, &m.name))
                    .max_by_key(|m| m.name.len())
                {
                    Some(m) => {
                        carry(v, &file.params, &mut m.scope, &kpath)?;
                        m.entry
                            .params
                            .insert(key[m.name.len() + 1..].into(), v.clone());
                    }
                    None => {
                        values.insert(key.clone(), v.clone());
                    }
                }
            }
            // Connections from its probes, and reaching its models on their
            // own clocks, move up, its probes renamed.
            let (moved, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut child.connections)
                .into_iter()
                .partition(|c| {
                    c.from.starts_with("probe:")
                        || in_use(&c.from, &own_clock)
                        || in_use(&c.to, &own_clock)
                });
            child.connections = kept;
            let mut probe_names: BTreeMap<String, String> = BTreeMap::new();
            let mut hold_ports: Vec<crate::InputPort> = Vec::new();
            let mut up: Vec<crate::ConnectionEntry> = Vec::new();
            for c in moved {
                let Some(rest) = c.from.strip_prefix("probe:") else {
                    return Err(fail(invalid(
                        "connections",
                        format!(
                            "{} → {}: models on their own clocks are linked only through probes",
                            c.from, c.to
                        ),
                    )));
                };
                let (p, signal) = rest.split_once('.').ok_or_else(|| {
                    fail(invalid(
                        "connections",
                        format!("'{}' is not probe:NAME.signal", c.from),
                    ))
                })?;
                let entry = child
                    .controls
                    .probes
                    .iter()
                    .find(|e| e.name == p)
                    .ok_or_else(|| fail(invalid("connections", format!("no probe '{p}'"))))?;
                let renamed = match probe_names.get(p) {
                    Some(n) => n.clone(),
                    None => {
                        let base = format!("{}_{p}", u.name);
                        let mut name = base.clone();
                        let mut n = 1;
                        while file.controls.probes.iter().any(|e| e.name == name) {
                            name = format!("{base}_{n}");
                            n += 1;
                        }
                        let mut e = entry.clone();
                        e.name = name.clone();
                        e.record = e.record.iter().map(|r| format!("{}/{r}", u.name)).collect();
                        e.to = None;
                        e.stream = false;
                        e.canvas = None;
                        file.controls.probes.push(e);
                        probe_names.insert(p.into(), name.clone());
                        name
                    }
                };
                let to = if self.merge || in_use(&c.to, &own_clock) || in_use(signal, &own_clock) {
                    // A link between its models on their own clocks and
                    // its own blocks.
                    format!("{}/{}", u.name, c.to)
                } else {
                    // A probe feeding its own blocks: through a port.
                    let port = format!("hold_{}", hold_ports.len());
                    hold_ports.push(crate::InputPort {
                        name: port.clone(),
                        to: vec![c.to.clone()],
                        canvas: None,
                    });
                    format!("{}.{port}", u.name)
                };
                up.push(crate::ConnectionEntry {
                    from: format!("probe:{renamed}.{}/{signal}", u.name),
                    to,
                });
            }
            file.connections.extend(up);
            for g in &child.groups {
                let mut g = g.clone();
                g.blocks = g.blocks.iter().map(|b| format!("{}/{b}", u.name)).collect();
                file.groups.push(g);
            }
            // Data files are read from the used file's folder.
            self.absolute_files(&mut child.blocks, child_folder.as_deref());
            for d in child.subsystems.values_mut() {
                self.absolute_files(&mut d.blocks, child_folder.as_deref());
            }
            // Its subsystems join file's, under names of their own.
            let renamed: BTreeMap<String, String> = child
                .subsystems
                .keys()
                .map(|n| {
                    (
                        n.clone(),
                        unique(&format!("{}_{n}", u.name), &file.subsystems),
                    )
                })
                .collect();
            let rename = |blocks: &mut Vec<BlockEntry>| {
                for b in blocks {
                    if let Some(n) = renamed.get(&b.type_name) {
                        b.type_name = n.clone();
                    }
                }
            };
            for (n, mut d) in std::mem::take(&mut child.subsystems) {
                rename(&mut d.blocks);
                file.subsystems.insert(renamed[&n].clone(), d);
            }
            rename(&mut child.blocks);
            let def = unique(&u.name, &file.subsystems);
            self.inlined.insert(
                def.clone(),
                Written {
                    at: match &u.model {
                        Some(m) => WrittenAt::Definition(m.clone()),
                        None => WrittenAt::File {
                            at: at.clone(),
                            shown: shown.clone(),
                        },
                    },
                    blocks: written_blocks,
                },
            );
            let declared = std::mem::take(&mut child.outputs);
            file.subsystems.insert(
                def.clone(),
                Subsystem {
                    description: child.description.clone(),
                    params: child.params.clone(),
                    inputs: hold_ports,
                    outputs: Vec::new(),
                    blocks: child.blocks,
                    connections: child.connections,
                },
            );
            let inner_tree: BTreeMap<String, Inlined> = child_tree
                .into_iter()
                .map(|(n, t)| (n, t.renamed(&renamed)))
                .collect();
            // Its declared outputs are ports of its subsystem, so wires
            // reach them by name (`plant/speed`).
            for o in declared {
                let from = match o.from.split_once('/') {
                    None => o.from.clone(),
                    Some((first, rest)) => match inner_tree.get(first) {
                        Some(inl) => {
                            let port =
                                expose_output(&mut file.subsystems, inl, rest).map_err(|m| {
                                    fail(invalid("outputs", format!("{}: {m}", o.from)))
                                })?;
                            format!("{first}.{port}")
                        }
                        // From a model on its own clock inside: reached by
                        // links, through the alias.
                        None => continue,
                    },
                };
                file.subsystems
                    .get_mut(&def)
                    .expect("inserted")
                    .outputs
                    .push(crate::OutputPort {
                        name: o.name,
                        from,
                        canvas: None,
                    });
            }
            file.blocks[k] = BlockEntry {
                name: u.name.clone(),
                type_name: def.clone(),
                params: values,
                canvas: u.canvas,
                ..Default::default()
            };
            tree.insert(
                u.name.clone(),
                Inlined {
                    def,
                    inner: inner_tree,
                },
            );
            lifted.extend(moving.into_iter().map(|mut m| {
                m.name = format!("{}/{}", u.name, m.name);
                m
            }));
        }
        // Wires into and out of them, at any depth, go through ports.
        // Connections from probes stay: they name flat signals and drive
        // input blocks as they are.
        for ci in 0..file.connections.len() {
            let c = file.connections[ci].clone();
            let at = format!("connections[{ci}]");
            if c.from.starts_with("probe:") {
                // Merged, a link is a probe feeding the model: into an
                // input block made a pass-through, at any depth.
                if self.merge {
                    let to = match c.to.split_once('/') {
                        Some((name, rest)) if tree.contains_key(name) => {
                            let port = expose_link(&mut file.subsystems, &tree[name], rest)
                                .map_err(|m| {
                                    invalid(format!("{at}.to"), format!("{}: {m}", c.to))
                                })?;
                            format!("{name}.{port}")
                        }
                        None if !c.to.contains('.') => {
                            match file.blocks.iter_mut().find(|b| b.name == c.to) {
                                Some(b) if b.type_name == INPUT_VALUE => pass_through(b),
                                _ => {
                                    return Err(invalid(
                                        format!("{at}.to"),
                                        format!("{} is not an input block", c.to),
                                    ))
                                }
                            }
                            format!("{}.u", c.to)
                        }
                        _ => c.to.clone(),
                    };
                    file.connections[ci].to = to;
                }
                continue;
            }
            if let Some((name, rest)) = c.to.split_once('/') {
                if let Some(inl) = tree.get(name) {
                    let port = expose_input(&mut file.subsystems, inl, rest)
                        .map_err(|m| invalid(format!("{at}.to"), format!("{}: {m}", c.to)))?;
                    file.connections[ci].to = format!("{name}.{port}");
                }
            }
            if let Some((name, rest)) = c.from.split_once('/') {
                if let Some(inl) = tree.get(name) {
                    let port = expose_output(&mut file.subsystems, inl, rest)
                        .map_err(|m| invalid(format!("{at}.from"), format!("{}: {m}", c.from)))?;
                    file.connections[ci].from = format!("{name}.{port}");
                }
            }
        }
        Ok((lifted, tree, aliases))
    }

    /// Make the files blocks' settings name absolute, from `folder`.
    fn absolute_files(&self, blocks: &mut [BlockEntry], folder: Option<&Path>) {
        let Some(folder) = folder else { return };
        for b in blocks {
            let Some(info) = self.registry.info(&b.type_name) else {
                continue;
            };
            for key in &info.files {
                if let Some(v) = b.settings.get_mut(key) {
                    if !Path::new(v.as_str()).is_absolute() {
                        *v = folder.join(v.as_str()).to_string_lossy().into_owned();
                    }
                }
            }
        }
    }

    /// Check one connection reaching into a use, and add it as a link.
    fn link(
        &mut self,
        file: &ModelFile,
        level: &Level,
        path: &str,
        at: &str,
        c: &crate::ConnectionEntry,
    ) -> Result<(), FileError> {
        let Some(rest) = c.from.strip_prefix("probe:") else {
            return Err(invalid(
                format!("{at}.from"),
                "models on their own clocks are linked only through probes: record the \
                 output with a probe and link the probe's output (ADR 0066)",
            ));
        };
        let (probe_name, signal) = rest
            .split_once('.')
            .ok_or_else(|| invalid(format!("{at}.from"), "write probe:NAME.model/block.port"))?;
        let entry = file
            .controls
            .probes
            .iter()
            .find(|p| p.name == probe_name)
            .ok_or_else(|| invalid(format!("{at}.from"), format!("no probe '{probe_name}'")))?;
        if !entry.record.iter().any(|r| r == signal) {
            return Err(invalid(
                format!("{at}.from"),
                format!("probe '{probe_name}' does not record {signal}"),
            ));
        }
        let (from, inner) = level
            .resolve(signal)
            .ok_or_else(|| invalid(format!("{at}.from"), format!("{signal} is in no model")))?;
        if entry
            .record
            .iter()
            .any(|r| level.resolve(r).map(|(d, _)| d) != Some(from))
        {
            return Err(invalid(
                format!("{at}.from"),
                format!(
                    "probe '{probe_name}' records from several models; a probe records from one"
                ),
            ));
        }
        let (source, unit) = scalar(&self.domains[from].loaded, inner).ok_or_else(|| {
            invalid(
                format!("{at}.from"),
                format!("{signal} is not a scalar output"),
            )
        })?;
        let (to, block) = level
            .resolve(&c.to)
            .ok_or_else(|| invalid(format!("{at}.to"), format!("{} is in no model", c.to)))?;
        if to == from {
            return Err(invalid(
                format!("{at}.to"),
                "a probe feeding its own model is wired inside that model's file (ADR 0063)",
            ));
        }
        let loaded = &self.domains[to].loaded;
        let entry_to = loaded
            .flat
            .blocks
            .iter()
            .find(|b| b.name == block && b.type_name == INPUT_VALUE)
            .ok_or_else(|| {
                invalid(
                    format!("{at}.to"),
                    format!("{} is not an input block ({INPUT_VALUE})", c.to),
                )
            })?;
        let id = loaded.ids[&entry_to.name];
        let want = loaded
            .model
            .block(id)
            .map_err(|e| invalid(format!("{at}.to"), e))?
            .spec()
            .outputs()
            .next()
            .map_or(simsandbox_core::units::Unit::DIMENSIONLESS, |p| p.unit);
        if want != unit {
            return Err(invalid(
                at,
                format!(
                    "{signal} is in {unit}, {} is in {want}: units must match",
                    c.to
                ),
            ));
        }
        if self.links.iter().any(|l| l.to == to && l.id == id) {
            return Err(invalid(
                format!("{at}.to"),
                format!("{} is linked twice", c.to),
            ));
        }
        let full_probe = if path.is_empty() {
            probe_name.to_string()
        } else {
            format!("{path}/{probe_name}")
        };
        let probe = match self.probes.iter().position(|p| p.name == full_probe) {
            Some(p) => p,
            None => {
                self.probes.push(LinkProbe {
                    name: full_probe,
                    every: entry.schedule(),
                    domain: from,
                });
                self.probes.len() - 1
            }
        };
        let signal = self.domains[from].full(inner);
        let block = self.domains[to].full(block);
        self.links.push(Link {
            probe,
            signal,
            from,
            source,
            to,
            block,
            id,
        });
        Ok(())
    }
}

/// A use's parameter values, written into the model it uses on its own
/// clock before it is built. A key is a declared parameter `k`, a
/// `block.param`, or a path into a use inside (`plant/car.m`,
/// `plant/k`), passed on to it. A value is a number or a formula of
/// `scope` (the declared parameters where it is written): a declared
/// parameter takes the formula's value now, since the model runs in a
/// simulator of its own; elsewhere the parameters the formula names are
/// copied into the model so it keeps its meaning (ADR 0066 step 2b).
fn set_params(
    model: &mut ModelFile,
    params: &BTreeMap<String, crate::ParamValue>,
    scope: &BTreeMap<String, crate::ParamDecl>,
) -> Result<(), FileError> {
    for (key, value) in params {
        let path = format!("params.{key}");
        if let crate::ParamValue::Number(v) = value {
            if !v.is_finite() {
                return Err(invalid(path, "must be a finite number"));
            }
        }
        if let Some(decl) = model.params.get_mut(key) {
            decl.value = match value {
                crate::ParamValue::Number(v) => *v,
                crate::ParamValue::Formula(text) => {
                    crate::subsystems::evaluate_formula(text, scope, &decl.unit, &path)?
                }
            };
            continue;
        }
        // A use inside (`plant/…`) or a block (`car.m`) of the model.
        let (target, rest) = match key.split_once('/') {
            Some((first, rest)) => (first, rest.to_string()),
            None => match key.split_once('.') {
                Some((b, _)) => (b, String::new()),
                None => {
                    return Err(invalid(
                        path,
                        "no such parameter in the used file: a declared parameter's name, \
                         block.param, or a path into a model it uses",
                    ))
                }
            },
        };
        if !model.blocks.iter().any(|e| e.name == target) {
            return Err(invalid(
                path,
                format!("the used file has no block or use '{target}'"),
            ));
        }
        let mut copied = model.params.clone();
        carry(value, scope, &mut copied, &path)?;
        model.params = copied;
        let e = model
            .blocks
            .iter_mut()
            .find(|e| e.name == target)
            .expect("found above");
        if rest.is_empty() {
            let (_, p) = key.split_once('.').expect("block.param");
            e.params.insert(p.into(), value.clone());
        } else {
            e.params.insert(rest, value.clone());
        }
    }
    Ok(())
}

/// Find `block.port` or `block.port[i]`, a scalar, in a built model.
fn scalar(loaded: &Loaded, signal: &str) -> Option<(Source, simsandbox_core::units::Unit)> {
    let (block, port) = signal.split_once('.')?;
    let (port, index) = match port.strip_suffix(']').and_then(|p| p.rsplit_once('[')) {
        Some((p, i)) => (p, i.parse().ok()?),
        None => (port, 0),
    };
    let id = *loaded.ids.get(block)?;
    let spec = loaded.model.block(id).ok()?.spec();
    let p = spec.outputs().find(|p| p.name == port)?;
    let fits = match p.shape {
        Shape::Scalar => index == 0 && !signal.ends_with(']'),
        Shape::Vector(n) => index < n && signal.ends_with(']'),
        _ => false,
    };
    fits.then(|| {
        (
            Source {
                block: id,
                port: port.into(),
                index,
            },
            p.unit,
        )
    })
}

/// Domains in the order they evaluate at a shared instant: each after the
/// domains that feed it, in written order; where domains feed each other,
/// the first written goes first. Returns the loops so broken.
fn order(domains: &[Domain], links: &[Link]) -> (Vec<usize>, Vec<String>) {
    let mut placed: Vec<usize> = Vec::new();
    let mut loops = Vec::new();
    while placed.len() < domains.len() {
        let waiting = |u: usize| {
            links
                .iter()
                .any(|l| l.to == u && !placed.contains(&l.from) && l.from != u)
        };
        let next = (0..domains.len()).find(|u| !placed.contains(u) && !waiting(*u));
        let u = next.unwrap_or_else(|| {
            let u = (0..domains.len())
                .find(|u| !placed.contains(u))
                .expect("some domain is left");
            let mut late: Vec<&str> = links
                .iter()
                .filter(|l| l.to == u && !placed.contains(&l.from))
                .map(|l| domains[l.from].name())
                .collect();
            late.dedup();
            loops.push(format!(
                "{} evaluates before {}, which it reads: at a shared instant it takes their \
                 samples after its own outputs, so an output of {} depending on them at the \
                 same instant sees them one step late",
                domains[u].name(),
                late.join(", "),
                domains[u].name()
            ));
            u
        });
        placed.push(u);
    }
    (placed, loops)
}
