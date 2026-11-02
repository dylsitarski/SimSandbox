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
    /// Its communication step.
    pub dt: f64,
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
    /// The domains' clocks in a common tick.
    pub ticks: Ticks,
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

/// Whether a model file's text uses model files.
pub fn is_composition(text: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| {
            v.get("blocks")?
                .as_array()
                .map(|b| b.iter().any(|e| e.get("file").is_some()))
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
        match &b.file {
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
            None => {}
            Some(_) => {
                let plain = !b.name.is_empty()
                    && b.name
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '-');
                if !plain {
                    return Err(invalid(
                        format!("{path}.name"),
                        format!(
                            "'{}': a used model's name is letters, digits, '_' and '-'",
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
                    ("start", b.start.is_some() || !b.starts.is_empty()),
                ];
                if let Some((key, _)) = block_only.iter().find(|(_, has)| *has) {
                    return Err(invalid(
                        format!("{path}.{key}"),
                        "a use of a model file has a name, file, params, clock and canvas",
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
        if let Some(k) = s.blocks.iter().position(|b| b.file.is_some()) {
            return Err(invalid(
                format!("subsystems.{name}.blocks[{k}]"),
                "model files are used at a model's level, not inside a subsystem",
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
}

impl Level {
    /// The domain `name` (relative to this level) lies in, and the name
    /// inside it.
    fn resolve<'a>(&self, name: &'a str) -> Option<(usize, &'a str)> {
        if let Some((first, rest)) = name.split_once('/') {
            if let Some(l) = self.uses.get(first) {
                return l.resolve(rest);
            }
        }
        self.domain.map(|d| (d, name))
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
}

/// Read and build a model that uses model files, saved in `folder` (or the
/// thread's folder, `in_folder`; without one, only absolute paths).
pub fn load_composition(
    text: &str,
    registry: &Registry,
    folder: Option<&Path>,
) -> Result<Composition, FileError> {
    let (file, warnings) = ModelFile::from_json(text)?;
    if !file.blocks.iter().any(|b| b.file.is_some()) {
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
    };
    l.level(&file, "", folder.as_deref(), None, file.run.dt)?;
    let ticks = Ticks::new(&l.domains.iter().map(|d| d.dt).collect::<Vec<_>>())
        .map_err(|e| invalid("blocks", e))?;
    let (order, loops) = order(&l.domains, &l.links);
    Ok(Composition {
        file,
        domains: l.domains,
        probes: l.probes,
        links: l.links,
        ticks,
        order,
        loops,
        data: l.data,
        warnings: l.warnings,
    })
}

fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(bytes);
    let hex: String = d.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
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

/// Whether an end of a connection (relative to a model) lies in one of
/// its uses: its first segment names one.
fn in_use(end: &str, uses: &[&BlockEntry]) -> bool {
    let name = end
        .strip_prefix("probe:")
        .and_then(|r| r.split_once('.'))
        .map_or(end, |(_, signal)| signal);
    name.split_once('/')
        .is_some_and(|(first, _)| uses.iter().any(|u| u.name == first))
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
    /// Load one model at `path` (its own blocks as a domain, then each
    /// use, then its links) and return what its names resolve to.
    fn level(
        &mut self,
        file: &ModelFile,
        path: &str,
        folder: Option<&Path>,
        source: Option<&Path>,
        dt: f64,
    ) -> Result<Level, FileError> {
        let mut file = file.clone();
        self.inline_shared(&mut file, path, folder)?;
        let file = &file;
        let full = |name: &str| {
            if path.is_empty() {
                name.to_string()
            } else {
                format!("{path}/{name}")
            }
        };
        let uses: Vec<&BlockEntry> = file.blocks.iter().filter(|b| b.file.is_some()).collect();
        // Its own blocks and the connections among them (with probes
        // feeding them) form its domain.
        let own: Vec<usize> = (0..file.blocks.len())
            .filter(|&k| file.blocks[k].file.is_none())
            .collect();
        let inside: Vec<usize> = (0..file.connections.len())
            .filter(|&k| {
                let c = &file.connections[k];
                !in_use(&c.from, &uses) && !in_use(&c.to, &uses)
            })
            .collect();
        let mut level = Level {
            domain: None,
            uses: BTreeMap::new(),
        };
        if !own.is_empty() {
            let mut f = file.clone();
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
            f.controls.probes = probes;
            f.run.dt = dt;
            f.run.t_end = None;
            f.run.inputs.clear();
            if !path.is_empty() {
                f.run.seed = Some(seed_for(self.seed, path));
            }
            let loaded = load_in(&f.to_json(), self.registry, folder)
                .map_err(|e| written_at(e, &own, &inside))?;
            if loaded.plan.is_some() {
                return Err(invalid(
                    "blocks",
                    "a model without states runs on new data, which comes in a later step \
                     (ADR 0066); this build runs models in time only",
                ));
            }
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
            });
        }
        for (k, u) in file
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| b.file.is_some())
        {
            let at = format!("blocks[{k}]");
            let child = self
                .use_file(u, &full(&u.name), folder)
                .map_err(|e| match e {
                    FileError::Invalid { path, message } if path.is_empty() => {
                        invalid(at.clone(), message)
                    }
                    e => invalid(at.clone(), e),
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

    /// Load the model file `u` uses, at `path`.
    /// Find, read and parse the model file `u` uses from `folder`, with
    /// the use's parameter values written in; its digest goes into the
    /// hash under `path`. Returns the model, where it was found, and the
    /// canonical path to push while it loads (a file that uses itself is
    /// refused).
    fn read_use(
        &mut self,
        u: &BlockEntry,
        path: &str,
        folder: Option<&Path>,
    ) -> Result<(ModelFile, PathBuf, PathBuf), FileError> {
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
        let bytes = std::fs::read(&found)
            .map_err(|e| invalid("file", format!("{}: {e}", found.display())))?;
        let text =
            String::from_utf8(bytes.clone()).map_err(|_| invalid("file", "not a text file"))?;
        let (mut model, _) = ModelFile::from_json(&text)
            .map_err(|e| invalid("file", format!("{}: {e}", found.display())))?;
        set_params(&mut model, &u.params)?;
        self.data.insert(format!("{path}.file"), digest(&bytes));
        Ok((model, found, canonical))
    }

    /// Load the model file `u` uses on its own clock, at `path`.
    fn use_file(
        &mut self,
        u: &BlockEntry,
        path: &str,
        folder: Option<&Path>,
    ) -> Result<Level, FileError> {
        let (model, found, canonical) = self.read_use(u, path, folder)?;
        let dt = match u.clock {
            Some(UseClock::Step(dt)) => dt,
            _ => model.run.dt,
        };
        self.stack.push(canonical);
        let child_folder = found.parent().map(Path::to_path_buf);
        let level = self
            .level(&model, path, child_folder.as_deref(), Some(&found), dt)
            .map_err(|e| invalid("", format!("{}: {e}", found.display())));
        self.stack.pop();
        level
    }

    /// Put each use of `file` on a shared clock in place as a subsystem
    /// of `file` made from the used file (ADR 0066 step 2), so it
    /// flattens into `file`'s model: named `use/inner`, fused with its
    /// neighbours where it can be. A wire into `use/input` (an input block
    /// of the used file) becomes an input port feeding what that block
    /// fed, the block dropped; a wire from `use/block.port` becomes an
    /// output port. Uses inside the used file on a shared clock are put
    /// in place first, in the same way.
    fn inline_shared(
        &mut self,
        file: &mut ModelFile,
        path: &str,
        folder: Option<&Path>,
    ) -> Result<(), FileError> {
        let shared: Vec<usize> = (0..file.blocks.len())
            .filter(|&k| {
                file.blocks[k].file.is_some() && file.blocks[k].clock == Some(UseClock::Shared)
            })
            .collect();
        let mut defs: BTreeMap<String, String> = BTreeMap::new();
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
            self.stack.push(canonical);
            let inner = self.inline_shared(&mut child, &full, child_folder.as_deref());
            self.stack.pop();
            inner.map_err(|e| invalid(&at, format!("{shown}: {e}")))?;
            let refuse = |what: &str| {
                invalid(
                    &at,
                    format!("{shown}: a model used on a shared clock {what} yet"),
                )
            };
            if child.blocks.iter().any(|b| b.file.is_some()) {
                return Err(refuse("cannot use models on their own clocks"));
            }
            if child
                .connections
                .iter()
                .any(|c| c.from.starts_with("probe:"))
            {
                return Err(refuse("cannot have probes feeding its own blocks"));
            }
            if !child.groups.is_empty() {
                return Err(refuse("cannot have groups"));
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
            let def = unique(&u.name.replace('-', "_"), &file.subsystems);
            file.subsystems.insert(
                def.clone(),
                Subsystem {
                    description: child.description.clone(),
                    params: child.params.clone(),
                    inputs: Vec::new(),
                    outputs: Vec::new(),
                    blocks: child.blocks,
                    connections: child.connections,
                },
            );
            file.blocks[k] = BlockEntry {
                name: u.name.clone(),
                type_name: def.clone(),
                canvas: u.canvas,
                ..Default::default()
            };
            defs.insert(u.name.clone(), def);
        }
        // Wires into and out of them go through ports. Connections from
        // probes stay: they name flat signals and drive input blocks as
        // they are.
        for (ci, c) in file.connections.iter_mut().enumerate() {
            if c.from.starts_with("probe:") {
                continue;
            }
            let at = format!("connections[{ci}]");
            if let Some((name, rest)) = c.to.split_once('/') {
                if let Some(def_name) = defs.get(name) {
                    let def = file.subsystems.get_mut(def_name).expect("inlined");
                    if def.inputs.iter().any(|p| p.name == rest) {
                        return Err(invalid(
                            format!("{at}.to"),
                            format!("{} is wired twice", c.to),
                        ));
                    }
                    let is_input = def
                        .blocks
                        .iter()
                        .any(|b| b.name == rest && b.type_name == INPUT_VALUE);
                    if !is_input {
                        return Err(invalid(
                            format!("{at}.to"),
                            format!(
                                "{}: a wire into a model on a shared clock goes to one of its input blocks ({INPUT_VALUE})",
                                c.to
                            ),
                        ));
                    }
                    let fed = format!("{rest}.y");
                    let to: Vec<String> = def
                        .connections
                        .iter()
                        .filter(|w| w.from == fed)
                        .map(|w| w.to.clone())
                        .collect();
                    def.connections.retain(|w| w.from != fed);
                    def.blocks.retain(|b| b.name != rest);
                    def.inputs.push(crate::InputPort {
                        name: rest.into(),
                        to,
                        canvas: None,
                    });
                    c.to = format!("{name}.{rest}");
                }
            }
            if let Some((name, rest)) = c.from.split_once('/') {
                if let Some(def_name) = defs.get(name) {
                    let def = file.subsystems.get_mut(def_name).expect("inlined");
                    let own = rest
                        .split_once('.')
                        .is_some_and(|(b, _)| def.blocks.iter().any(|e| e.name == b));
                    if !own {
                        return Err(invalid(
                            format!("{at}.from"),
                            format!(
                                "{}: a wire from a model on a shared clock comes from one of its own blocks; \
                                 models inside it are wired in its own file",
                                c.from
                            ),
                        ));
                    }
                    let port = match def.outputs.iter().find(|p| p.from == rest) {
                        Some(p) => p.name.clone(),
                        None => {
                            let taken: Vec<&str> = def
                                .inputs
                                .iter()
                                .map(|p| p.name.as_str())
                                .chain(def.outputs.iter().map(|p| p.name.as_str()))
                                .collect();
                            let base: String = rest
                                .chars()
                                .map(|ch| if ch.is_alphanumeric() { ch } else { '_' })
                                .collect();
                            let mut port = base.clone();
                            let mut n = 1;
                            while taken.contains(&port.as_str()) {
                                port = format!("{base}_{n}");
                                n += 1;
                            }
                            def.outputs.push(crate::OutputPort {
                                name: port.clone(),
                                from: rest.into(),
                                canvas: None,
                            });
                            port
                        }
                    };
                    c.from = format!("{name}.{port}");
                }
            }
        }
        Ok(())
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

/// A use's parameter values, written into its model before it is built.
fn set_params(
    model: &mut ModelFile,
    params: &BTreeMap<String, crate::ParamValue>,
) -> Result<(), FileError> {
    for (key, value) in params {
        let path = format!("params.{key}");
        let crate::ParamValue::Number(v) = value else {
            return Err(invalid(path, "a use's parameter values are numbers"));
        };
        if let Some(decl) = model.params.get_mut(key) {
            decl.value = *v;
            continue;
        }
        let entry = key.split_once('.').and_then(|(b, p)| {
            Some((
                model
                    .blocks
                    .iter_mut()
                    .find(|e| e.name == b && e.file.is_none())?,
                p,
            ))
        });
        match entry {
            Some((e, p)) => {
                e.params.insert(p.into(), value.clone());
            }
            None => return Err(invalid(
                path,
                "no such parameter in the used file: a declared parameter's name, or block.param",
            )),
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
