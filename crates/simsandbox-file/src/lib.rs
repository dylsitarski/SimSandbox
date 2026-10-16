//! # simsandbox-file
//!
//! Model files: plain-text JSON that names each Block by a registered type
//! (ADR 0007, `docs/decisions/0007-json-model-files.md`), plus an optional
//! `controls` section describing how the model is meant to be explored
//! (ADR 0012, `docs/decisions/0012-controls-in-model-files.md`).
//!
//! ```json
//! {
//!   "format": 1,
//!   "run": { "dt": 0.01 },
//!   "blocks": [
//!     { "name": "lorenz", "type": "eq.lorenz", "method": { "name": "rk4" },
//!       "params": { "rho": 28.0 } }
//!   ],
//!   "controls": { "params": { "lorenz.rho": { "slider": true, "range": [0.0, 30.0] } } }
//! }
//! ```
//!
//! - [`load`] parses a file and builds a runnable [`Model`] with a
//!   [`Registry`]. Errors carry a line and column (syntax and structure) or a
//!   path such as `blocks[0].params.rho` (meaning).
//! - `run` holds settings that affect results, such as the step `dt`
//!   (ADR 0013); invalid values are load errors.
//! - Results never depend on `controls`: a malformed or dangling control is
//!   dropped with a warning, never a load failure.
//! - [`ModelFile::capture_params`] copies current parameter values back into
//!   the file before saving; [`ModelFile::to_json`] writes it with a stable
//!   key order.
//!
//! The JSON Schema for the format is `crates/simsandbox-file/model.schema.json`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use simsandbox_core::block::{Causality, Shape};
use simsandbox_core::graph::{BlockId, Model};
use simsandbox_core::registry::{MethodSettings, Registry};
use std::collections::BTreeMap;
use std::fmt;

/// The format version this crate reads and writes.
pub const FORMAT_VERSION: u32 = 1;

/// Conventional file extension.
pub const EXTENSION: &str = ".ssbx.json";

/// The JSON Schema describing the format.
pub const SCHEMA: &str = include_str!("../model.schema.json");

/// A model file as written on disk.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelFile {
    pub format: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub run: RunSettings,
    pub blocks: Vec<BlockEntry>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<ConnectionEntry>,
    #[serde(skip_serializing_if = "Controls::is_empty")]
    pub controls: Controls,
}

/// Default communication step when a file has no `run.dt`.
pub const DEFAULT_DT: f64 = 0.01;

fn default_dt() -> f64 {
    DEFAULT_DT
}

/// How the model is run (ADR 0013). Unlike `controls`, these affect results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSettings {
    /// Simulated seconds per communication step.
    #[serde(default = "default_dt")]
    pub dt: f64,
    /// Where batch runs stop and where the app pauses; none runs on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t_end: Option<f64>,
}

impl Default for RunSettings {
    fn default() -> Self {
        Self {
            dt: DEFAULT_DT,
            t_end: None,
        }
    }
}

/// One Block: a unique name, a registered type, a method and parameter
/// values in each parameter's declared unit. Omitted parameters take their
/// defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockEntry {
    pub name: String,
    #[serde(rename = "type")]
    pub type_name: String,
    pub method: MethodEntry,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, f64>,
}

/// A method name plus numeric options, e.g. `{"name": "rk4", "substeps": 4}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MethodEntry {
    pub name: String,
    #[serde(flatten)]
    pub options: BTreeMap<String, f64>,
}

impl From<&MethodEntry> for MethodSettings {
    fn from(m: &MethodEntry) -> Self {
        MethodSettings {
            name: m.name.clone(),
            options: m.options.clone(),
        }
    }
}

impl From<MethodSettings> for MethodEntry {
    fn from(m: MethodSettings) -> Self {
        MethodEntry {
            name: m.name,
            options: m.options,
        }
    }
}

/// A signal connection, `"block.port"` to `"block.port"`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionEntry {
    pub from: String,
    pub to: String,
}

/// How the model is meant to be explored (ADR 0012). Never affects results.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Controls {
    /// Keyed by `"block.param"`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, ParamControl>,
    /// Default playback speed: simulated seconds per wall-clock second.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<f64>,
    /// Outputs plotted by default, as `"block.port"`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plots: Vec<String>,
    /// The views the model opens with (ADR 0015); the app shows the first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<ViewEntry>,
    /// The single view of ADR 0014. Still read; converted into `views` on
    /// load and never written.
    #[serde(default, skip_serializing)]
    pub view: Option<ViewControl>,
}

/// One view: a registered view type, slot bindings and style values
/// (ADR 0015). The file checks that bindings name real signals; the app
/// checks types, slots and style names against its view registry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewEntry {
    /// E.g. `plot.time`, `plot.phase`, `plot.3d`.
    #[serde(rename = "type")]
    pub type_name: String,
    /// Slot name to signal: `"t"`, `"block.port"` (scalar) or
    /// `"block.port[i]"`; a list for slots that take several.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bind: BTreeMap<String, Binding>,
    /// Style property values, e.g. `trail_length`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub style: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<CameraControl>,
}

/// One signal, or a list for slots that take several.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Binding {
    One(String),
    Many(Vec<String>),
}

impl Binding {
    pub fn signals(&self) -> Vec<String> {
        match self {
            Binding::One(s) => vec![s.clone()],
            Binding::Many(v) => v.clone(),
        }
    }
}

impl From<Vec<String>> for Binding {
    /// A single signal is written as a plain string.
    fn from(mut v: Vec<String>) -> Self {
        if v.len() == 1 {
            Binding::One(v.remove(0))
        } else {
            Binding::Many(v)
        }
    }
}

impl Controls {
    pub fn is_empty(&self) -> bool {
        self.params.is_empty()
            && self.speed.is_none()
            && self.plots.is_empty()
            && self.views.is_empty()
            && self.view.is_none()
    }
}

/// Which view a model opens in, and on which outputs (ADR 0014; superseded
/// in files by [`ViewEntry`], still read).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewControl {
    /// `"time"`, `"phase"` or `"3d"`.
    pub kind: String,
    /// Output elements on the view's axes, as `"block.port"` (scalar) or
    /// `"block.port[i]"`: two for `phase`, three for `3d`, none for `time`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub axes: Vec<String>,
    /// Orientation of the `3d` view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<CameraControl>,
}

/// Orientation of a 3D view, in radians; `zoom` 1 fits the data.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CameraControl {
    pub yaw: f64,
    pub pitch: f64,
    pub zoom: f64,
}

impl ViewControl {
    /// The equivalent `views` entry: `time`, `phase`, `3d` become
    /// `plot.time`, `plot.phase`, `plot.3d`, the axes fill the `x`, `y`, `z`
    /// slots, and the trail fades with time as it did.
    pub fn into_entry(self) -> ViewEntry {
        let type_name = match self.kind.as_str() {
            "phase" => "plot.phase",
            "3d" => "plot.3d",
            _ => "plot.time",
        };
        let mut bind: BTreeMap<String, Binding> = ["x", "y", "z"]
            .into_iter()
            .zip(self.axes)
            .map(|(slot, s)| (slot.to_string(), Binding::One(s)))
            .collect();
        if type_name != "plot.time" {
            bind.insert("colour".into(), Binding::One("t".into()));
        }
        ViewEntry {
            type_name: type_name.into(),
            bind,
            style: BTreeMap::new(),
            camera: self.camera,
        }
    }

    /// Number of axes a view kind takes, or `None` for an unknown kind.
    pub fn axis_count(kind: &str) -> Option<usize> {
        match kind {
            "time" => Some(0),
            "phase" => Some(2),
            "3d" => Some(3),
            _ => None,
        }
    }
}

/// Presentation of one parameter.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ParamControl {
    /// Show a slider in addition to the typed entry field.
    #[serde(default)]
    pub slider: bool,
    /// Slider range `[min, max]`; the parameter's suggested range if absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<[f64; 2]>,
}

/// Why a file could not be loaded.
#[derive(Debug, Clone, PartialEq)]
pub enum FileError {
    /// Invalid JSON, or JSON that does not have the file's structure.
    Syntax {
        line: usize,
        column: usize,
        message: String,
    },
    /// The `format` version is not one this build reads.
    UnsupportedFormat(u32),
    /// Well-formed, but meaningless: unknown type, parameter, port, etc.
    Invalid { path: String, message: String },
}

impl fmt::Display for FileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FileError::Syntax {
                line,
                column,
                message,
            } => write!(f, "line {line}, column {column}: {message}"),
            FileError::UnsupportedFormat(v) => write!(
                f,
                "unsupported format version {v} (this build reads version {FORMAT_VERSION})"
            ),
            FileError::Invalid { path, message } => write!(f, "{path}: {message}"),
        }
    }
}

impl std::error::Error for FileError {}

fn invalid(path: impl Into<String>, message: impl fmt::Display) -> FileError {
    FileError::Invalid {
        path: path.into(),
        message: message.to_string(),
    }
}

/// A loaded model, ready to hand to `Simulator::new`.
pub struct Loaded {
    /// The file as read, with invalid controls removed.
    pub file: ModelFile,
    pub model: Model,
    /// Block names from the file mapped to their ids in `model`.
    pub ids: BTreeMap<String, BlockId>,
    /// Problems that did not stop loading, e.g. dropped controls.
    pub warnings: Vec<String>,
}

/// Parse a model file and build its model.
pub fn load(text: &str, registry: &Registry) -> Result<Loaded, FileError> {
    let (mut file, mut warnings) = ModelFile::from_json(text)?;
    let (model, ids) = file.build(registry)?;
    warnings.extend(file.check_controls(&model, &ids));
    Ok(Loaded {
        file,
        model,
        ids,
        warnings,
    })
}

/// What is read from disk: like [`ModelFile`], but `controls` stays raw JSON
/// so a malformed section can be dropped instead of failing the load.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    format: u32,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    run: RunSettings,
    blocks: Vec<BlockEntry>,
    #[serde(default)]
    connections: Vec<ConnectionEntry>,
    #[serde(default)]
    controls: Option<Value>,
}

impl ModelFile {
    /// An empty model file of the current format.
    pub fn new() -> Self {
        Self {
            format: FORMAT_VERSION,
            description: None,
            run: RunSettings::default(),
            blocks: Vec::new(),
            connections: Vec::new(),
            controls: Controls::default(),
        }
    }

    /// Parse without building. Returns warnings for a malformed `controls`
    /// section, which is replaced by empty controls.
    pub fn from_json(text: &str) -> Result<(Self, Vec<String>), FileError> {
        let raw: RawFile = serde_json::from_str(text).map_err(|e| FileError::Syntax {
            line: e.line(),
            column: e.column(),
            message: e.to_string(),
        })?;
        if raw.format != FORMAT_VERSION {
            return Err(FileError::UnsupportedFormat(raw.format));
        }
        let positive = |v: f64| v.is_finite() && v > 0.0;
        if !positive(raw.run.dt) {
            return Err(invalid(
                "run.dt",
                format!("must be positive, got {}", raw.run.dt),
            ));
        }
        if let Some(t) = raw.run.t_end.filter(|&t| !positive(t)) {
            return Err(invalid("run.t_end", format!("must be positive, got {t}")));
        }
        let mut warnings = Vec::new();
        let controls = match raw.controls {
            None => Controls::default(),
            Some(v) => serde_json::from_value(v).unwrap_or_else(|e| {
                warnings.push(format!("controls: ignored, {e}"));
                Controls::default()
            }),
        };
        Ok((
            Self {
                format: raw.format,
                description: raw.description,
                run: raw.run,
                blocks: raw.blocks,
                connections: raw.connections,
                controls,
            },
            warnings,
        ))
    }

    /// Pretty-printed JSON with a stable key order and a trailing newline.
    pub fn to_json(&self) -> String {
        let mut s = serde_json::to_string_pretty(self).expect("model files always serialize");
        s.push('\n');
        s
    }

    /// Copy every parameter's current value from `model` into the file, so a
    /// save records exactly what is running. `ids` comes from [`load`].
    pub fn capture_params(
        &mut self,
        model: &Model,
        ids: &BTreeMap<String, BlockId>,
    ) -> Result<(), FileError> {
        for (bi, entry) in self.blocks.iter_mut().enumerate() {
            let path = format!("blocks[{bi}]");
            let id = *ids
                .get(&entry.name)
                .ok_or_else(|| invalid(&path, format!("no block '{}' in model", entry.name)))?;
            let block = model.block(id).map_err(|e| invalid(&path, e))?;
            let values = model.params(id).map_err(|e| invalid(&path, e))?;
            entry.params = block
                .spec()
                .params
                .iter()
                .zip(values)
                .map(|(p, v)| (p.name.clone(), *v))
                .collect();
        }
        Ok(())
    }

    fn build(&self, registry: &Registry) -> Result<(Model, BTreeMap<String, BlockId>), FileError> {
        let mut model = Model::new();
        let mut ids = BTreeMap::new();
        for (bi, entry) in self.blocks.iter().enumerate() {
            let path = format!("blocks[{bi}]");
            if entry.name.is_empty() || entry.name.contains('.') {
                return Err(invalid(
                    format!("{path}.name"),
                    format!(
                        "'{}': names must be non-empty and contain no '.'",
                        entry.name
                    ),
                ));
            }
            if ids.contains_key(&entry.name) {
                return Err(invalid(
                    format!("{path}.name"),
                    format!("duplicate block name '{}'", entry.name),
                ));
            }
            if !registry.contains(&entry.type_name) {
                let known: Vec<_> = registry.type_names().collect();
                return Err(invalid(
                    format!("{path}.type"),
                    format!(
                        "unknown block type '{}' (known: {})",
                        entry.type_name,
                        known.join(", ")
                    ),
                ));
            }
            let block = registry
                .build(&entry.type_name, &entry.name, &(&entry.method).into())
                .map_err(|e| invalid(format!("{path}.method"), e))?;
            let id = model.add(block);
            for (name, &value) in &entry.params {
                model
                    .set_param(id, name, value)
                    .map_err(|e| invalid(format!("{path}.params.{name}"), e))?;
            }
            ids.insert(entry.name.clone(), id);
        }
        for (ci, c) in self.connections.iter().enumerate() {
            let path = format!("connections[{ci}]");
            let from = port_ref(&c.from, &ids).map_err(|m| invalid(format!("{path}.from"), m))?;
            let to = port_ref(&c.to, &ids).map_err(|m| invalid(format!("{path}.to"), m))?;
            model.connect(from, to).map_err(|e| invalid(&path, e))?;
        }
        Ok((model, ids))
    }

    /// Drop controls that refer to nothing or carry bad values; return a
    /// warning for each.
    fn check_controls(&mut self, model: &Model, ids: &BTreeMap<String, BlockId>) -> Vec<String> {
        let mut warnings = Vec::new();
        let c = &mut self.controls;
        c.params.retain(|key, pc| {
            let known = port_ref(key, ids).ok().is_some_and(|(id, name)| {
                model
                    .block(id)
                    .is_ok_and(|b| b.spec().params.iter().any(|p| p.name == name))
            });
            if !known {
                warnings.push(format!("controls.params.{key}: no such parameter; ignored"));
                return false;
            }
            if let Some([lo, hi]) = pc.range {
                if !(lo.is_finite() && hi.is_finite() && lo < hi) {
                    warnings.push(format!(
                        "controls.params.{key}.range: [{lo}, {hi}] is not an increasing finite range; ignored"
                    ));
                    pc.range = None;
                }
            }
            true
        });
        if let Some(s) = c.speed {
            if !(s.is_finite() && s > 0.0) {
                warnings.push(format!("controls.speed: {s} is not positive; ignored"));
                c.speed = None;
            }
        }
        c.plots.retain(|key| {
            let known = port_ref(key, ids).ok().is_some_and(|(id, name)| {
                model.block(id).is_ok_and(|b| {
                    b.spec()
                        .ports
                        .iter()
                        .any(|p| p.name == name && p.causality == Causality::Output)
                })
            });
            if !known {
                warnings.push(format!("controls.plots: no output '{key}'; ignored"));
            }
            known
        });
        if let Some(v) = c.view.take() {
            match view_problem(&v, model, ids) {
                Some(problem) => warnings.push(format!("controls.view: {problem}; ignored")),
                // ADR 0014's single view becomes the first of `views`.
                None if c.views.is_empty() => c.views.push(v.into_entry()),
                None => {}
            }
        }
        let mut kept = Vec::new();
        for (i, mut v) in std::mem::take(&mut c.views).into_iter().enumerate() {
            let unknown = v
                .bind
                .values()
                .flat_map(Binding::signals)
                .find(|s| s != "t" && !output_element_exists(s, model, ids));
            if let Some(s) = unknown {
                warnings.push(format!("controls.views[{i}]: no signal '{s}'; ignored"));
                continue;
            }
            v.style.retain(|name, value| {
                let ok = value.is_finite();
                if !ok {
                    warnings.push(format!(
                        "controls.views[{i}].style.{name}: not finite; ignored"
                    ));
                }
                ok
            });
            let bad_camera = v.camera.is_some_and(|cam| {
                ![cam.yaw, cam.pitch, cam.zoom].iter().all(|x| x.is_finite()) || cam.zoom <= 0.0
            });
            if bad_camera {
                warnings.push(format!(
                    "controls.views[{i}].camera: needs finite angles and a positive zoom; ignored"
                ));
                v.camera = None;
            }
            kept.push(v);
        }
        c.views = kept;
        warnings
    }
}

/// Why a view cannot be used, if it cannot.
fn view_problem(v: &ViewControl, model: &Model, ids: &BTreeMap<String, BlockId>) -> Option<String> {
    let Some(n) = ViewControl::axis_count(&v.kind) else {
        return Some(format!(
            "unknown kind '{}' (expected time, phase or 3d)",
            v.kind
        ));
    };
    if v.axes.len() != n {
        return Some(format!("'{}' takes {n} axes, got {}", v.kind, v.axes.len()));
    }
    v.axes
        .iter()
        .find(|a| !output_element_exists(a, model, ids))
        .map(|a| format!("no output element '{a}'"))
}

/// `"block.port"` for a scalar output, `"block.port[i]"` for an element of
/// a vector output.
fn output_element_exists(key: &str, model: &Model, ids: &BTreeMap<String, BlockId>) -> bool {
    let Ok((id, rest)) = port_ref(key, ids) else {
        return false;
    };
    let (port, index) = match rest.strip_suffix(']').and_then(|r| r.rsplit_once('[')) {
        Some((port, i)) => match i.parse::<usize>() {
            Ok(i) => (port, Some(i)),
            Err(_) => return false,
        },
        None => (rest, None),
    };
    model.block(id).is_ok_and(|b| {
        b.spec().outputs().any(|p| {
            p.name == port
                && match (p.shape, index) {
                    (Shape::Scalar, None) => true,
                    (Shape::Vector(n), Some(i)) => i < n,
                    _ => false,
                }
        })
    })
}

impl Default for ModelFile {
    fn default() -> Self {
        Self::new()
    }
}

/// Resolve `"block.rest"`: the block name ends at the first '.', so the rest
/// may itself contain dots (as fused-group ports do).
fn port_ref<'a>(s: &'a str, ids: &BTreeMap<String, BlockId>) -> Result<(BlockId, &'a str), String> {
    let (block, rest) = s
        .split_once('.')
        .ok_or_else(|| format!("'{s}' must have the form 'block.name'"))?;
    let id = ids
        .get(block)
        .ok_or_else(|| format!("no block named '{block}'"))?;
    Ok((*id, rest))
}
