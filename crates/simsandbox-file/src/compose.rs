//! Compositions (ADR 0065): a file whose `models` use model files, each
//! under a name and with its own clock, linked by probes into the models'
//! input blocks. Loading reads and builds each used model from its own
//! folder, applies the use's parameter values and clock, and checks the
//! links; running is the session's.

use crate::{invalid, load_in, FileError, Loaded, ModelFile, ModelUse};
use simsandbox_core::block::Shape;
use simsandbox_core::clock::Ticks;
use simsandbox_core::graph::BlockId;
use simsandbox_core::registry::Registry;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The block type a link drives (ADR 0058's `input.value`).
const INPUT_VALUE: &str = "input.value";

/// One used model, built.
pub struct Use {
    pub name: String,
    /// The model file, as found.
    pub path: PathBuf,
    pub loaded: Loaded,
    /// Its communication step.
    pub dt: f64,
}

/// Where a scalar comes from in a use's model.
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    pub block: BlockId,
    pub port: String,
    pub index: usize,
}

/// A link: probe `probe` samples `signal` (`use/block.port`) of use
/// `from` and drives the input block `block` of use `to`.
#[derive(Debug, Clone)]
pub struct Link {
    pub probe: usize,
    pub signal: String,
    pub from: usize,
    pub source: Source,
    pub to: usize,
    /// The input block, by its name in the use (`speed`).
    pub block: String,
    pub id: BlockId,
}

/// A composition, ready to run.
pub struct Composition {
    pub file: ModelFile,
    pub uses: Vec<Use>,
    pub links: Vec<Link>,
    /// The uses' clocks in a common tick.
    pub ticks: Ticks,
    /// The order uses evaluate in at a shared point: a use that feeds
    /// another first; a loop is broken in written order.
    pub order: Vec<usize>,
    /// Each loop between uses, by name, as it is broken.
    pub loops: Vec<String>,
    /// Digests of the used files and their data: part of the hash.
    pub data: BTreeMap<String, String>,
    pub warnings: Vec<String>,
}

impl Composition {
    /// The hash a run file records (ADR 0059), covering every used model
    /// file and the data it reads.
    pub fn hash(&self) -> String {
        crate::runs::model_hash_with(&self.file, &self.data)
    }
}

/// Whether a model file's text is a composition.
pub fn is_composition(text: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| {
            v.get("models")
                .and_then(|m| m.as_array())
                .map(|m| !m.is_empty())
        })
        .unwrap_or(false)
}

/// Uses checked as the file is read: names unique and plain, clocks
/// positive.
pub(crate) fn check_uses(uses: &[ModelUse]) -> Result<(), FileError> {
    for (k, u) in uses.iter().enumerate() {
        let path = format!("models[{k}]");
        let plain = !u.name.is_empty()
            && u.name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-');
        if !plain {
            return Err(invalid(
                format!("{path}.name"),
                format!(
                    "'{}': a model's name is letters, digits, '_' and '-'",
                    u.name
                ),
            ));
        }
        if uses[..k].iter().any(|o| o.name == u.name) {
            return Err(invalid(
                format!("{path}.name"),
                format!("'{}' is used twice", u.name),
            ));
        }
        if let Some(r) = &u.run {
            if !(r.dt.is_finite() && r.dt > 0.0) {
                return Err(invalid(
                    format!("{path}.run.dt"),
                    format!("must be positive, got {}", r.dt),
                ));
            }
        }
    }
    Ok(())
}

/// Read and build a composition saved in `folder` (its used files are
/// found from there; without a folder, only absolute paths).
pub fn load_composition(
    text: &str,
    registry: &Registry,
    folder: Option<&Path>,
) -> Result<Composition, FileError> {
    let (file, mut warnings) = ModelFile::from_json(text)?;
    if file.models.is_empty() {
        return Err(invalid("models", "not a composition: it uses no models"));
    }
    let folder = folder
        .map(Path::to_path_buf)
        .or_else(simsandbox_core::registry::current_folder);
    let mut uses = Vec::new();
    let mut data = BTreeMap::new();
    for (k, u) in file.models.iter().enumerate() {
        let at = format!("models[{k}]");
        let path = PathBuf::from(&u.file);
        let path = match (&folder, path.is_absolute()) {
            (_, true) => path,
            (Some(f), false) => f.join(path),
            (None, false) => {
                return Err(invalid(
                    format!("{at}.file"),
                    format!(
                        "'{}' is found from the composition's folder: save it first, or give a full path",
                        u.file
                    ),
                ))
            }
        };
        let bytes = std::fs::read(&path)
            .map_err(|e| invalid(format!("{at}.file"), format!("{}: {e}", path.display())))?;
        let own = String::from_utf8(bytes.clone())
            .map_err(|_| invalid(format!("{at}.file"), "not a text file"))?;
        if is_composition(&own) {
            return Err(invalid(
                format!("{at}.file"),
                "a composition cannot use another composition (yet): use model files",
            ));
        }
        let (mut model, _) = ModelFile::from_json(&own)
            .map_err(|e| invalid(format!("{at}.file"), format!("{}: {e}", path.display())))?;
        let model_folder = path.parent().map(Path::to_path_buf);
        let as_written = model.to_json();
        set_params(&mut model, &u.params, &at)?;
        let loaded = load_in(&model.to_json(), registry, model_folder.as_deref()).map_err(|e| {
            // A model that loads without this use's values: they are wrong.
            if !u.params.is_empty()
                && load_in(&as_written, registry, model_folder.as_deref()).is_ok()
            {
                invalid(format!("{at}.params"), e)
            } else {
                invalid(&at, format!("{}: {e}", path.display()))
            }
        })?;
        if loaded.plan.is_some() {
            return Err(invalid(
                &at,
                "a model without states is evaluated on new data, which compositions do from \
                 ADR 0065 step 2; this build runs models in time only",
            ));
        }
        warnings.extend(loaded.warnings.iter().map(|w| format!("{}: {w}", u.name)));
        data.insert(format!("{}.file", u.name), digest(&bytes));
        for (key, d) in &loaded.data {
            data.insert(format!("{}/{key}", u.name), d.clone());
        }
        let dt = u.run.as_ref().map_or(loaded.file.run.dt, |r| r.dt);
        uses.push(Use {
            name: u.name.clone(),
            path,
            loaded,
            dt,
        });
    }
    let ticks = Ticks::new(&uses.iter().map(|u| u.dt).collect::<Vec<_>>())
        .map_err(|e| invalid("models", e))?;
    let links = links(&file, &uses)?;
    let (order, loops) = order(&uses, &links);
    Ok(Composition {
        file,
        uses,
        links,
        ticks,
        order,
        loops,
        data,
        warnings,
    })
}

fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(bytes);
    let hex: String = d.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

/// A use's parameter values, written into its model before it is built.
fn set_params(
    model: &mut ModelFile,
    params: &BTreeMap<String, f64>,
    at: &str,
) -> Result<(), FileError> {
    for (key, &value) in params {
        let path = format!("{at}.params.{key}");
        if !value.is_finite() {
            return Err(invalid(path, "must be a finite number"));
        }
        if let Some(decl) = model.params.get_mut(key) {
            decl.value = value;
            continue;
        }
        let entry = key
            .split_once('.')
            .and_then(|(b, p)| Some((model.blocks.iter_mut().find(|e| e.name == b)?, p)));
        match entry {
            Some((e, p)) => {
                e.params.insert(p.into(), crate::ParamValue::Number(value));
            }
            None => {
                return Err(invalid(
                    path,
                    "no such parameter: a declared parameter's name, or block.param",
                ))
            }
        }
    }
    Ok(())
}

/// Find `block.port` or `block.port[i]`, a scalar, in a use.
fn scalar(u: &Use, signal: &str) -> Option<(Source, simsandbox_core::units::Unit)> {
    let (block, port) = signal.split_once('.')?;
    let (port, index) = match port.strip_suffix(']').and_then(|p| p.rsplit_once('[')) {
        Some((p, i)) => (p, i.parse().ok()?),
        None => (port, 0),
    };
    let id = *u.loaded.ids.get(block)?;
    let spec = u.loaded.model.block(id).ok()?.spec();
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

fn links(file: &ModelFile, uses: &[Use]) -> Result<Vec<Link>, FileError> {
    let find = |name: &str| uses.iter().position(|u| u.name == name);
    let mut out: Vec<Link> = Vec::new();
    for (k, c) in file.connections.iter().enumerate() {
        let at = format!("connections[{k}]");
        let Some(rest) = c.from.strip_prefix("probe:") else {
            return Err(invalid(
                format!("{at}.from"),
                "models are linked only through probes: record the output with a probe and \
                 link the probe's output (ADR 0065)",
            ));
        };
        let (probe_name, signal) = rest
            .split_once('.')
            .ok_or_else(|| invalid(format!("{at}.from"), "write probe:NAME.model/block.port"))?;
        let probe = file
            .controls
            .probes
            .iter()
            .position(|p| p.name == probe_name)
            .ok_or_else(|| invalid(format!("{at}.from"), format!("no probe '{probe_name}'")))?;
        if !file.controls.probes[probe]
            .record
            .iter()
            .any(|r| r == signal)
        {
            return Err(invalid(
                format!("{at}.from"),
                format!("probe '{probe_name}' does not record {signal}"),
            ));
        }
        let (from_use, inner) = signal.split_once('/').ok_or_else(|| {
            invalid(
                format!("{at}.from"),
                format!("'{signal}': name the model, model/block.port"),
            )
        })?;
        let from = find(from_use)
            .ok_or_else(|| invalid(format!("{at}.from"), format!("no model '{from_use}'")))?;
        let (source, unit) = scalar(&uses[from], inner).ok_or_else(|| {
            invalid(
                format!("{at}.from"),
                format!("{signal} is not a scalar output of model '{from_use}'"),
            )
        })?;
        let (to_use, block) = c.to.split_once('/').ok_or_else(|| {
            invalid(
                format!("{at}.to"),
                "write model/input, an input block of a model",
            )
        })?;
        let to = find(to_use)
            .ok_or_else(|| invalid(format!("{at}.to"), format!("no model '{to_use}'")))?;
        if to == from {
            return Err(invalid(
                format!("{at}.to"),
                "a probe feeds its own model inside the model file (ADR 0063)",
            ));
        }
        let u = &uses[to];
        let entry = u
            .loaded
            .flat
            .blocks
            .iter()
            .find(|b| b.name == block)
            .filter(|b| b.type_name == INPUT_VALUE)
            .ok_or_else(|| {
                invalid(
                    format!("{at}.to"),
                    format!("model '{to_use}' has no input block '{block}' ({INPUT_VALUE})"),
                )
            })?;
        let id = u.loaded.ids[&entry.name];
        let want = u
            .loaded
            .model
            .block(id)
            .map_err(|e| invalid(format!("{at}.to"), e))?
            .spec()
            .outputs()
            .next()
            .map(|p| p.unit)
            .unwrap_or(simsandbox_core::units::Unit::DIMENSIONLESS);
        if want != unit {
            return Err(invalid(
                at,
                format!(
                    "{signal} is in {unit}, {} is in {want}: units must match",
                    c.to
                ),
            ));
        }
        if out.iter().any(|l| l.to == to && l.block == block) {
            return Err(invalid(
                format!("{at}.to"),
                format!("{} is linked twice", c.to),
            ));
        }
        out.push(Link {
            probe,
            signal: signal.into(),
            from,
            source,
            to,
            block: block.into(),
            id,
        });
    }
    // A probe records from one model.
    for (k, p) in file.controls.probes.iter().enumerate() {
        let models: Vec<&str> = p
            .record
            .iter()
            .map(|r| r.split_once('/').map_or("", |(m, _)| m))
            .collect();
        if models.iter().any(|m| find(m).is_none()) || models.windows(2).any(|w| w[0] != w[1]) {
            return Err(invalid(
                format!("controls.probes[{k}].record"),
                "a probe records from one model, by model/block.port",
            ));
        }
    }
    Ok(out)
}

/// Uses in the order they evaluate at a shared point: each after the uses
/// that feed it, in written order; where uses feed each other, the first
/// written goes first. Returns the loops so broken.
fn order(uses: &[Use], links: &[Link]) -> (Vec<usize>, Vec<String>) {
    let mut placed: Vec<usize> = Vec::new();
    let mut loops = Vec::new();
    while placed.len() < uses.len() {
        let waiting = |u: usize| {
            links
                .iter()
                .any(|l| l.to == u && !placed.contains(&l.from) && l.from != u)
        };
        let next = (0..uses.len()).find(|u| !placed.contains(u) && !waiting(*u));
        let u = next.unwrap_or_else(|| {
            let u = (0..uses.len())
                .find(|u| !placed.contains(u))
                .expect("some use is left");
            let late: Vec<&str> = links
                .iter()
                .filter(|l| l.to == u && !placed.contains(&l.from))
                .map(|l| uses[l.from].name.as_str())
                .collect();
            loops.push(format!(
                "{} evaluates before {}, which it reads: at a shared point it takes their \
                 samples after its own outputs, so an output of {} depending on them at the \
                 same instant sees them one step late",
                uses[u].name,
                late.join(", "),
                uses[u].name
            ));
            u
        });
        placed.push(u);
    }
    (placed, loops)
}
