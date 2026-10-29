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
use simsandbox_core::registry::{BlockConfig, MethodSettings, Registry};
use simsandbox_core::SimError;
use std::collections::BTreeMap;
use std::fmt;

pub mod layout;
pub mod runs;
pub mod subsystems;
pub use subsystems::{
    Declared, Derived, InputPort, OutputPort, ParamDecl, ParamPlan, ParamValue, Subsystem,
};

/// The format version this crate reads and writes.
pub const FORMAT_VERSION: u32 = 1;

/// Conventional file extension.
pub const EXTENSION: &str = ".ssbx.json";

/// The JSON Schema describing the format.
pub const SCHEMA: &str = include_str!("../model.schema.json");

/// Whether a file gives block or subsystem port places as `at`, the
/// spelling before ADR 0052.
fn places_written_at(v: &Value) -> bool {
    let has_at = |list: Option<&Value>| {
        list.and_then(Value::as_array)
            .is_some_and(|items| items.iter().any(|b| b.get("at").is_some()))
    };
    has_at(v.get("blocks"))
        || v.get("subsystems")
            .and_then(Value::as_object)
            .is_some_and(|defs| {
                defs.values().any(|d| {
                    has_at(d.get("blocks")) || has_at(d.get("inputs")) || has_at(d.get("outputs"))
                })
            })
}

/// A model file as written on disk.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelFile {
    pub format: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub run: RunSettings,
    /// The parameter values a model without states is evaluated at
    /// (ADR 0027, ADR 0028); `None` for one that runs in time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample: Option<Sampling>,
    /// Parameters of the model itself, which block parameters can use in
    /// formulas, e.g. a field size shared by two blocks (ADR 0043).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, ParamDecl>,
    /// Groups of blocks used like block types by name (ADR 0043).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub subsystems: BTreeMap<String, Subsystem>,
    pub blocks: Vec<BlockEntry>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<ConnectionEntry>,
    /// Whether connected equation blocks are solved together
    /// automatically (ADR 0023).
    #[serde(skip_serializing_if = "Fusion::is_auto")]
    pub fusion: Fusion,
    /// Blocks to solve together with a chosen method, whatever `fusion` says.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<GroupEntry>,
    #[serde(skip_serializing_if = "Controls::is_empty")]
    pub controls: Controls,
    /// Older spellings converted as the file was read (ADR 0044); never
    /// written.
    #[serde(skip)]
    pub converted: Vec<String>,
}

/// Automatic fusion of equation blocks (ADR 0023).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fusion {
    /// Connected continuous equation blocks with identical methods are
    /// solved together as one system.
    #[default]
    Auto,
    /// Only explicit `groups` are solved together.
    Off,
}

impl Fusion {
    fn is_auto(&self) -> bool {
        *self == Fusion::Auto
    }
}

/// Blocks solved together as one system with one method (ADR 0023).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupEntry {
    pub blocks: Vec<String>,
    pub method: MethodEntry,
}

/// Blocks the loader fused, for display.
#[derive(Debug, Clone, PartialEq)]
pub struct FusedGroup {
    /// Block names, in file order for automatic groups.
    pub blocks: Vec<String>,
    /// The method's display name, e.g. "RK4 (4 substeps)".
    pub method: String,
    /// From `groups` rather than automatic fusion.
    pub explicit: bool,
}

/// Default communication step when a file has no `run.dt`.
pub const DEFAULT_DT: f64 = 0.01;

fn default_dt() -> f64 {
    DEFAULT_DT
}

/// How a sweep chooses its parameter values (ADR 0028).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Design {
    /// Every combination of evenly spaced values.
    #[default]
    Grid,
    /// The points given in `values`.
    List,
    /// Independent uniform values from a seeded generator.
    Random,
    /// One value in each of `count` slices of every parameter.
    LatinHypercube,
    /// The first `count` points of the Sobol sequence.
    Sobol,
}

impl Design {
    /// Grids keep their order along each parameter; the others are clouds
    /// of points.
    pub fn structured(self) -> bool {
        self == Design::Grid
    }
}

/// A sweep (ADR 0027, ADR 0028): the swept parameters and how their values
/// are chosen. Unlike `controls`, this decides what is computed, so invalid
/// values are load errors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sampling {
    #[serde(default)]
    pub design: Design,
    /// The swept parameters: range (the slider's or the suggested range if
    /// absent) and, for grids, number of values.
    pub params: Vec<SweepControl>,
    /// Random, Latin hypercube and Sobol: how many points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    /// Random and Latin hypercube: the generator's seed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// List: the points, one value per parameter each.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<Vec<f64>>,
}

/// Points in a design that takes a count, unless the file says otherwise.
pub const DEFAULT_SAMPLE_COUNT: usize = 1000;
/// Seed of random designs unless the file says otherwise.
pub const DEFAULT_SEED: u64 = 1;

/// A sweep made concrete: where the model is evaluated.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// `"block.param"` of each swept parameter.
    pub keys: Vec<String>,
    /// One value per key for each sample.
    pub points: Vec<Vec<f64>>,
    /// The range of each swept parameter, as given or defaulted.
    pub ranges: Vec<[f64; 2]>,
    /// For a grid design: values per parameter (first varies fastest).
    pub grid: Option<Vec<usize>>,
    pub design: Design,
}

impl Sampling {
    /// Check against the model and make the points; errors carry a path
    /// such as `sample.params[1].range`.
    pub fn plan(
        &self,
        file: &ModelFile,
        model: &Model,
        ids: &BTreeMap<String, BlockId>,
    ) -> Result<Plan, FileError> {
        use simsandbox_core::sampling as design;
        if self.params.is_empty() {
            return Err(invalid("sample.params", "sweep at least one parameter"));
        }
        let mut keys: Vec<String> = Vec::new();
        let mut ranges = Vec::new();
        for (i, p) in self.params.iter().enumerate() {
            let path = format!("sample.params[{i}]");
            let spec = port_ref(&p.param, ids).ok().and_then(|(id, name)| {
                model
                    .block(id)
                    .ok()?
                    .spec()
                    .params
                    .iter()
                    .find(|q| q.name == name)
                    .cloned()
            });
            let Some(spec) = spec else {
                return Err(invalid(
                    format!("{path}.param"),
                    format!("no parameter '{}'", p.param),
                ));
            };
            if keys.contains(&p.param) {
                return Err(invalid(
                    format!("{path}.param"),
                    format!("'{}' is swept twice", p.param),
                ));
            }
            let [lo, hi] = p
                .range
                .or_else(|| file.controls.params.get(&p.param).and_then(|c| c.range))
                .unwrap_or([spec.suggested_min, spec.suggested_max]);
            if !(lo.is_finite() && hi.is_finite() && lo < hi) {
                return Err(invalid(
                    format!("{path}.range"),
                    format!("[{lo}, {hi}] is not an increasing finite range"),
                ));
            }
            keys.push(p.param.clone());
            ranges.push([lo, hi]);
        }
        let count = |default: usize| -> Result<usize, FileError> {
            let n = self.count.unwrap_or(default);
            if (1..=MAX_SAMPLES).contains(&n) {
                Ok(n)
            } else {
                Err(invalid(
                    "sample.count",
                    format!("{n} is outside 1 to {MAX_SAMPLES}"),
                ))
            }
        };
        let seed = self.seed.unwrap_or(DEFAULT_SEED);
        let (points, grid) = match self.design {
            Design::Grid => {
                let default = if keys.len() == 1 {
                    DEFAULT_SWEEP_POINTS
                } else {
                    DEFAULT_GRID_POINTS
                };
                let mut axes = Vec::new();
                for (i, (p, range)) in self.params.iter().zip(&ranges).enumerate() {
                    let n = p.points.unwrap_or(default);
                    if !(2..=100_000).contains(&n) {
                        return Err(invalid(
                            format!("sample.params[{i}].points"),
                            format!("{n} is outside 2 to 100000"),
                        ));
                    }
                    axes.push(design::Axis {
                        range: *range,
                        points: n,
                    });
                }
                let total = axes.iter().try_fold(1usize, |t, a| t.checked_mul(a.points));
                if total.is_none_or(|t| t > MAX_SAMPLES) {
                    return Err(invalid(
                        "sample.params",
                        format!("the grid has more than {MAX_SAMPLES} points"),
                    ));
                }
                let shape = axes.iter().map(|a| a.points).collect();
                (design::grid(&axes), Some(shape))
            }
            Design::Random => (
                design::random(&ranges, count(DEFAULT_SAMPLE_COUNT)?, seed),
                None,
            ),
            Design::LatinHypercube => (
                design::latin_hypercube(&ranges, count(DEFAULT_SAMPLE_COUNT)?, seed),
                None,
            ),
            Design::Sobol => (
                design::sobol(&ranges, count(DEFAULT_SAMPLE_COUNT)?)
                    .map_err(|e| invalid("sample.params", e))?,
                None,
            ),
            Design::List => {
                if self.values.is_empty() || self.values.len() > MAX_SAMPLES {
                    return Err(invalid(
                        "sample.values",
                        format!("give 1 to {MAX_SAMPLES} points"),
                    ));
                }
                for (k, row) in self.values.iter().enumerate() {
                    if row.len() != keys.len() || !row.iter().all(|v| v.is_finite()) {
                        return Err(invalid(
                            format!("sample.values[{k}]"),
                            format!("give {} finite values, one per parameter", keys.len()),
                        ));
                    }
                }
                (self.values.clone(), None)
            }
        };
        Ok(Plan {
            keys,
            ranges,
            points,
            grid,
            design: self.design,
        })
    }
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
    /// The seed every random draw derives from (ADR 0034); 1 if absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    /// How algebraic loops are solved (ADR 0040): `newton` (the default if
    /// absent), `fixed_point` or `refuse`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loops: Option<String>,
    /// The model's default schedule (ADR 0058, ADR 0059): values changed at
    /// set times in every run. Recordings live in run files, never here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<ScheduledChange>,
}

impl Default for RunSettings {
    fn default() -> Self {
        Self {
            dt: DEFAULT_DT,
            t_end: None,
            seed: None,
            loops: None,
            inputs: Vec::new(),
        }
    }
}

/// A value changed at a set time (ADR 0058): `{"t": 10, "set": "car.F",
/// "to": 500}`. It applies at the first step boundary at or after `t`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduledChange {
    pub t: f64,
    /// A parameter key: `"block.param"`, or a declared parameter.
    pub set: String,
    pub to: f64,
}

/// Whether `s` is a name: a letter or `_`, then letters, digits or `_`.
fn is_identifier(s: &str) -> bool {
    let mut c = s.chars();
    c.next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && c.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A probe (ADR 0062): what it records, when, how long, and the file it
/// writes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeEntry {
    pub name: String,
    /// Signals and fields, by the names views bind (`car.v`, `rod.T`);
    /// empty for a probe that records nothing yet.
    #[serde(default)]
    pub record: Vec<String>,
    /// When it records; every step if absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every: Option<ProbeEvery>,
    /// How long it keeps samples; thinned when long if absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep: Option<ProbeKeep>,
    /// The file it writes, beside the model: a CSV of its signals, and
    /// a `.npy` file per grid or point field beside it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// Where its node sits on the canvas's data layer (ADR 0050).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas: Option<[f64; 2]>,
}

impl ProbeEntry {
    /// Check the settings the file can check alone (what it records is
    /// checked against the model by the session); an error says what is
    /// wrong.
    pub fn check(&self) -> Result<(), String> {
        let positive = |v: f64| v.is_finite() && v > 0.0;
        if !is_identifier(&self.name) {
            return Err(format!(
                "'{}' is not a name (letters, digits, _)",
                self.name
            ));
        }
        if let Some(e) = &self.every {
            let set = [e.steps.is_some(), e.seconds.is_some(), e.times.is_some()];
            if set.iter().filter(|s| **s).count() != 1 {
                return Err("'every' gives one of steps, seconds or times".into());
            }
            let ok = e.steps.is_none_or(|n| n >= 1)
                && e.seconds.is_none_or(positive)
                && e.times.as_ref().is_none_or(|ts| {
                    !ts.is_empty() && ts.iter().all(|t| t.is_finite() && *t >= 0.0)
                });
            if !ok {
                return Err(
                    "'every': steps is at least 1, seconds positive, times 0 or more".into(),
                );
            }
        }
        if let Some(kp) = &self.keep {
            let set = [kp.mb.is_some(), kp.last.is_some(), kp.all];
            if set.iter().filter(|s| **s).count() > 1 {
                return Err("'keep' gives at most one of mb, last or all".into());
            }
            if !(kp.mb.is_none_or(positive) && kp.last.is_none_or(|n| n >= 1)) {
                return Err("'keep': mb is positive and last at least 1".into());
            }
        }
        if let Some(to) = &self.to {
            let plain = !to.contains(['/', '\\']) && to.len() > 4 && to.ends_with(".csv");
            if !plain {
                return Err(format!(
                    "'to': '{to}' is a file name ending in .csv, written beside the model"
                ));
            }
        }
        Ok(())
    }
}

/// When a probe records: one of the three.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeEvery {
    /// Every this many steps, from t = 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<u64>,
    /// Every this much simulated time, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
    /// At these times, in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub times: Option<Vec<f64>>,
}

/// How long a probe keeps samples: at most one of these; thinned when
/// long if none.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeKeep {
    /// Thinned (every other sample dropped) past this many megabytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mb: Option<f64>,
    /// Only the newest this many samples.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<usize>,
    /// Every sample, never thinned.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub all: bool,
}

/// How keys and views drive an input block (ADR 0058), in
/// `controls.inputs` by the block's name. Never affects results: a run
/// records the values, not the keys.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputBinding {
    /// Two keys moving an `input.value` down and up, e.g. `["Left",
    /// "Right"]` (egui key names).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<String>,
    /// With keys: how fast the value moves while a key is held, per
    /// second; it stays where it is let go. Without a rate the value is
    /// its range's end while a key is held and 0 when none is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate: Option<f64>,
    /// An `input.pointer` fed by this view (its place in `controls.views`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<usize>,
}

/// One Block: a unique name, a registered type, a method and parameter
/// values in each parameter's declared unit. Omitted parameters take their
/// defaults.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockEntry {
    pub name: String,
    /// A registered block type, or a subsystem of this file (ADR 0043).
    #[serde(rename = "type")]
    pub type_name: String,
    /// Absent for a subsystem, whose blocks have their own; for a block,
    /// absent means the type's default method.
    #[serde(default, skip_serializing_if = "MethodEntry::is_empty")]
    pub method: MethodEntry,
    /// Agent blocks: the order agents act in within a step (ADR 0034),
    /// e.g. `random_order`; the type's default if absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<String>,
    /// Text settings of the type, e.g. `{"from": "km/h", "to": "m/s"}` for
    /// `units.convert` (ADR 0045); see `simsandbox describe TYPE`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub settings: BTreeMap<String, String>,
    /// The equations of an `eq.equations` block, one per line (ADR 0017),
    /// e.g. `"dx/dt = sigma*(y - x)"`. Absent for types with built-in
    /// equations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub equations: Vec<String>,
    /// Numbers, or formulas of the declared parameters in scope
    /// (ADR 0043). For a subsystem: its declared parameters, and inner
    /// values for this instance only as `"block.param"`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, ParamValue>,
    /// Where the block sits on the node canvas, `[x, y]` in canvas units,
    /// y downwards (ADR 0042). Blocks without one are laid out
    /// automatically. Written `canvas`, like every node's place (ADR 0052);
    /// the older `at` is read and converted.
    #[serde(default, alias = "at", skip_serializing_if = "Option::is_none")]
    pub canvas: Option<[f64; 2]>,
    /// The state the block starts from instead of the one its parameters
    /// give, e.g. a steady state reached before (ADR 0042).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<StartEntry>,
    /// A subsystem's start states, by inner block (`"grass"`, nested
    /// `"field/grass"`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub starts: BTreeMap<String, StartEntry>,
}

/// A saved start (ADR 0042): every state, in the block's state order, and
/// the rows of its tables by table name.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StartEntry {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub state: Vec<f64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tables: BTreeMap<String, Vec<Vec<f64>>>,
}

impl StartEntry {
    /// As the kernel takes it, with tables in the block's order.
    pub fn to_start(
        &self,
        spec: &simsandbox_core::block::BlockSpec,
    ) -> Result<simsandbox_core::graph::Start, String> {
        if let Some(name) = self
            .tables
            .keys()
            .find(|n| !spec.tables.iter().any(|t| &t.name == *n))
        {
            return Err(format!("{} has no table '{name}'", spec.name));
        }
        Ok(simsandbox_core::graph::Start {
            state: self.state.clone(),
            tables: spec
                .tables
                .iter()
                .map(|t| self.tables.get(&t.name).cloned().unwrap_or_default())
                .collect(),
        })
    }

    /// From the kernel's form.
    pub fn from_start(
        start: &simsandbox_core::graph::Start,
        spec: &simsandbox_core::block::BlockSpec,
    ) -> Self {
        Self {
            state: start.state.clone(),
            tables: spec
                .tables
                .iter()
                .zip(&start.tables)
                .map(|(t, rows)| (t.name.clone(), rows.clone()))
                .collect(),
        }
    }
}

/// A method name plus numeric options, e.g. `{"name": "rk4", "substeps": 4}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MethodEntry {
    pub name: String,
    #[serde(flatten)]
    pub options: BTreeMap<String, f64>,
}

impl MethodEntry {
    pub fn is_empty(&self) -> bool {
        self.name.is_empty() && self.options.is_empty()
    }
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
    /// How the app arranges its panes: inspector, diagram and views as
    /// tabs, split and stacked (ADR 0053).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<layout::Layout>,
    /// Diagram nodes whose values are shown (ADR 0055): model-level names,
    /// and `subsystem::block` inside a subsystem. Unknown names are ignored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open: Vec<String>,
    /// Keys and views driving input blocks, by block name (ADR 0058).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<String, InputBinding>,
    /// Filters turning recorded data into new signals and fields
    /// (ADR 0030, ADR 0032): derived expressions, cuts. Checked by the app;
    /// invalid ones are dropped with a warning.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filters: Vec<FilterEntry>,
    /// Probes recording chosen signals and fields at chosen times, and
    /// the files they write (ADR 0062). Checked by the session; one that
    /// records nothing the model has is dropped with a warning.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub probes: Vec<ProbeEntry>,
    /// Derived signals as written before filters (name to expression).
    /// Still read; converted into `expression` filters on load and never
    /// written.
    #[serde(default, skip_serializing)]
    pub signals: BTreeMap<String, String>,
    /// The sweep as written before the `sample` section (ADR 0021,
    /// ADR 0022): one object, or a list of two. Still read; converted into
    /// a grid `sample` on load (ADR 0027) and never written.
    #[serde(
        default,
        skip_serializing,
        deserialize_with = "one_or_many::deserialize"
    )]
    pub sweep: Vec<SweepControl>,
    /// The single view of ADR 0014. Still read; converted into `views` on
    /// load and never written.
    #[serde(default, skip_serializing)]
    pub view: Option<ViewControl>,
}

/// A filter (ADR 0030, ADR 0032): a named transformation of recorded data
/// into new signals or fields. It names its inputs; its type decides its
/// settings:
///
/// - `expression`: `{"expr": "0.5*a.v^2"}`, one signal named after the
///   filter, from recorded signals, other expressions, parameters and `t`;
/// - `cut` (ADR 0031): through the 2D grid named in `inputs`, `{"mode":
///   "along_x" | "along_y" | "free", "at": [...] | "lines": [...], "whole":
///   bool, "range": [a, b], "interpolation": "bilinear" | "nearest"}`, one
///   line field per cut (`name[k]`) and their coordinate (`name.x`,
///   `name.y` or `name.fraction`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilterEntry {
    pub name: String,
    #[serde(rename = "type")]
    pub type_name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<String>,
    /// Where its node sits on the canvas's data layer (ADR 0050); not
    /// `at`, which cut filters use for their cuts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas: Option<[f64; 2]>,
    /// The probe its inputs come from (ADR 0063), unless `from` says
    /// otherwise; the model at every step if absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe: Option<String>,
    /// The probe each of these inputs comes from (ADR 0063).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub from: BTreeMap<String, String>,
    /// When a filter with inputs from probes runs: `paired` (the default)
    /// at the times all its sources recorded, or `any` whenever one does,
    /// with each other input's latest value (ADR 0063).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    /// Settings of the filter's type.
    #[serde(flatten)]
    pub settings: BTreeMap<String, Value>,
}

impl FilterEntry {
    pub fn expression(name: &str, expr: &str, inputs: Vec<String>) -> Self {
        Self {
            name: name.into(),
            type_name: "expression".into(),
            inputs,
            canvas: None,
            probe: None,
            from: BTreeMap::new(),
            when: None,
            settings: [("expr".to_string(), Value::String(expr.into()))].into(),
        }
    }

    /// The probe an input comes from (ADR 0063); `None` for the model.
    pub fn source_of(&self, input: &str) -> Option<&str> {
        self.from
            .get(input)
            .map(String::as_str)
            .or(self.probe.as_deref())
    }

    /// Whether any input comes from a probe, so it has a clock of its own.
    pub fn on_probes(&self) -> bool {
        self.probe.is_some() || !self.from.is_empty()
    }

    /// Whether it runs whenever one of its sources records (`when: any`).
    pub fn on_any(&self) -> bool {
        self.when.as_deref() == Some("any")
    }

    /// An expression filter's text.
    pub fn expr(&self) -> Option<&str> {
        self.settings.get("expr").and_then(Value::as_str)
    }

    /// The names of what this filter makes, as views and other filters
    /// refer to them.
    pub fn outputs(&self) -> Vec<String> {
        match self.type_name.as_str() {
            "expression" => vec![self.name.clone()],
            "cut" => {
                let mode = self.settings.get("mode").and_then(Value::as_str);
                let list = if mode == Some("free") { "lines" } else { "at" };
                let count = self
                    .settings
                    .get(list)
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len);
                let coordinate = match mode {
                    Some("along_y") => "y",
                    Some("free") => "fraction",
                    _ => "x",
                };
                (0..count)
                    .map(|k| format!("{}[{k}]", self.name))
                    .chain([format!("{}.{coordinate}", self.name)])
                    .collect()
            }
            _ => Vec::new(),
        }
    }
}

/// A profile selection of ADR 0029 (end points per cut) as a cut filter
/// (ADR 0031): all horizontal cuts become `along_x`, all vertical ones
/// `along_y`, anything else free lines.
fn cut_filter_from_selection(name: &str, grid: &str, selection: &Value) -> Option<FilterEntry> {
    let cuts = selection.get("cuts")?.as_array()?;
    let ends: Vec<[[f64; 2]; 2]> = cuts
        .iter()
        .map(|c| {
            let p = |k: &str| -> Option<[f64; 2]> {
                let a = c.get(k)?.as_array()?;
                Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?])
            };
            Some([p("from")?, p("to")?])
        })
        .collect::<Option<_>>()?;
    let mut settings = BTreeMap::new();
    let along = |axis: usize| ends.iter().all(|[a, b]| a[axis] == b[axis]);
    if along(1) {
        settings.insert("mode".into(), Value::from("along_x"));
        settings.insert("at".into(), ends.iter().map(|[a, _]| a[1]).collect());
    } else if along(0) {
        settings.insert("mode".into(), Value::from("along_y"));
        settings.insert("at".into(), ends.iter().map(|[a, _]| a[0]).collect());
    } else {
        settings.insert("mode".into(), Value::from("free"));
        settings.insert("whole".into(), Value::from(false));
        let lines: Vec<Value> = ends
            .iter()
            .map(|[a, b]| serde_json::json!([[a[0], a[1]], [b[0], b[1]]]))
            .collect();
        settings.insert("lines".into(), Value::from(lines));
    }
    if let Some(i) = selection.get("interpolation") {
        settings.insert("interpolation".into(), i.clone());
    }
    Some(FilterEntry {
        name: name.into(),
        type_name: "cut".into(),
        inputs: vec![grid.into()],
        canvas: None,
        probe: None,
        from: BTreeMap::new(),
        when: None,
        settings,
    })
}

/// One view: a registered view type, slot bindings and style values
/// (ADR 0015). The file checks that bindings name real signals; the app
/// checks types, slots and style names against its view registry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewEntry {
    /// E.g. `plot.xy`, `plot.3d`.
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
    /// What the user picked inside the view, e.g. profile cuts (ADR 0029);
    /// checked by the view type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<Value>,
    /// Where its node sits on the canvas's data layer (ADR 0050); not
    /// `at`, which cut filters use for their cuts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canvas: Option<[f64; 2]>,
    /// The probe it showed, as written on 2026-10-29 (ADR 0062). Still
    /// read; converted into bindings from the probe (ADR 0063) and never
    /// written.
    #[serde(default, skip_serializing)]
    pub probe: Option<String>,
}

impl ViewEntry {
    /// Convert older forms: the view types that `plot.xy` replaced
    /// (ADR 0018) — `plot.time` becomes `plot.xy` against `t`, `plot.phase`
    /// becomes `plot.xy` with its bindings and style kept — and the heat
    /// map's `position` and `time_axis` (ADR 0024).
    /// Returns whether anything changed.
    pub fn upgrade(&mut self) -> bool {
        let before = self.clone();
        match self.type_name.as_str() {
            "plot.time" => {
                self.type_name = "plot.xy".into();
                self.bind
                    .entry("x".into())
                    .or_insert_with(|| Binding::One("t".into()));
            }
            "plot.phase" => self.type_name = "plot.xy".into(),
            // ADR 0024: `position` and `time_axis` became bindings of the
            // `x` and `y` axes.
            "plot.heatmap" => {
                let upwards = self.style.remove("time_axis").is_some_and(|v| v != 0.0);
                let position = self.bind.remove("position");
                let time = Binding::One("t".into());
                if position.is_some() || upwards {
                    let (along, up) = if upwards {
                        (position, Some(time))
                    } else {
                        (Some(time), position)
                    };
                    for (slot, b) in [("x", along), ("y", up)] {
                        if let Some(b) = b {
                            self.bind.entry(slot.into()).or_insert(b);
                        }
                    }
                }
            }
            _ => {}
        }
        // ADR 0063: a view showing a probe reads each signal from it.
        if let Some(probe) = self.probe.take() {
            for b in self.bind.values_mut() {
                let from: Vec<String> = b
                    .signals()
                    .into_iter()
                    .map(|s| {
                        if s == "t" || s.starts_with("probe:") {
                            s
                        } else {
                            format!("probe:{probe}/{s}")
                        }
                    })
                    .collect();
                *b = Binding::from(from);
            }
        }
        *self != before
    }
}

/// A swept parameter: its range and, in a grid, how many values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SweepControl {
    /// `"block.param"`.
    pub param: String,
    /// The parameter's slider range or suggested range if absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<[f64; 2]>,
    /// [`DEFAULT_SWEEP_POINTS`] for one parameter or
    /// [`DEFAULT_GRID_POINTS`] each for two if absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<usize>,
}

/// Values of a one-parameter grid unless a file says otherwise.
pub const DEFAULT_SWEEP_POINTS: usize = 400;
/// Values along each parameter of a grid over several parameters unless a
/// file says otherwise (100 × 100 evaluations for two).
pub const DEFAULT_GRID_POINTS: usize = 100;
/// The most samples a sweep may ask for.
pub const MAX_SAMPLES: usize = 250_000;

/// `controls.sweep`: one object, or a list of them.
mod one_or_many {
    use super::SweepControl;
    use serde::{Deserialize, Deserializer};

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(SweepControl),
        Many(Vec<SweepControl>),
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<SweepControl>, D::Error> {
        Ok(match OneOrMany::deserialize(d)? {
            OneOrMany::One(one) => vec![one],
            OneOrMany::Many(many) => many,
        })
    }
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
            && self.sweep.is_empty()
            && self.signals.is_empty()
            && self.filters.is_empty()
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
    /// The equivalent `views` entry: `phase` and `3d` become `plot.xy` and
    /// `plot.3d` with the axes in the `x`, `y`, `z` slots and the trail
    /// fading with time as it did; `time` becomes `plot.xy` against `t`.
    pub fn into_entry(self) -> ViewEntry {
        let type_name = if self.kind == "3d" {
            "plot.3d"
        } else {
            "plot.xy"
        };
        let mut bind: BTreeMap<String, Binding> = ["x", "y", "z"]
            .into_iter()
            .zip(self.axes)
            .map(|(slot, s)| (slot.to_string(), Binding::One(s)))
            .collect();
        if self.kind == "time" {
            bind.insert("x".into(), Binding::One("t".into()));
        } else {
            bind.insert("colour".into(), Binding::One("t".into()));
        }
        ViewEntry {
            type_name: type_name.into(),
            bind,
            style: BTreeMap::new(),
            camera: self.camera,
            selection: None,
            canvas: None,
            probe: None,
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
    /// Show a slider in addition to the typed entry field, among the
    /// Controls. A slider in older files pins the parameter too.
    #[serde(default)]
    pub slider: bool,
    /// Slider range `[min, max]`; the parameter's suggested range if absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<[f64; 2]>,
    /// Shown among the Controls (ADR 0053).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pin: bool,
}

impl ParamControl {
    /// Shown among the Controls. A slider in a file read before pins
    /// existed pins its value on reading.
    pub fn pinned(&self) -> bool {
        self.pin || self.slider
    }

    /// Pin or unpin; unpinning removes the slider too, keeping its range.
    pub fn set_pinned(&mut self, pinned: bool) {
        self.pin = pinned;
        if !pinned {
            self.slider = false;
        }
    }
}

/// Whether a parameter control has a slider and no pin: written before
/// pins decided the Controls (ADR 0054).
fn slider_without_pin(v: &Value) -> bool {
    v.pointer("/controls/params")
        .and_then(Value::as_object)
        .is_some_and(|params| {
            params.values().any(|c| {
                c.get("slider").and_then(Value::as_bool) == Some(true)
                    && c.get("pin").and_then(Value::as_bool) != Some(true)
            })
        })
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
            FileError::UnsupportedFormat(v) if *v > FORMAT_VERSION => write!(
                f,
                "format {v} comes from a newer SimSandbox; this build reads formats up to \
                 {FORMAT_VERSION}: update SimSandbox to open it"
            ),
            FileError::UnsupportedFormat(v) => write!(
                f,
                "unsupported format version {v} (this build reads formats 1 to {FORMAT_VERSION})"
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
    /// The same with every subsystem expanded into its blocks (named
    /// `instance/inner`) and formulas evaluated: one entry per Block in
    /// `model` (ADR 0043). Equal to `file` for a file without subsystems
    /// or declared parameters.
    pub flat: ModelFile,
    /// Declared parameters and the formulas they feed.
    pub params: ParamPlan,
    pub model: Model,
    /// Flat block names mapped to their ids in `model`.
    pub ids: BTreeMap<String, BlockId>,
    /// Problems that did not stop loading, e.g. dropped controls.
    pub warnings: Vec<String>,
    /// Older forms the file was written in, converted as it was read;
    /// saving writes the current form (ADR 0044).
    pub conversions: Vec<String>,
    /// Blocks solved together (ADR 0023).
    pub groups: Vec<FusedGroup>,
    /// Where a model without states is evaluated (ADR 0027); `None` for one
    /// that runs in time.
    pub plan: Option<Plan>,
}

/// Parse a model file and build its model.
pub fn load(text: &str, registry: &Registry) -> Result<Loaded, FileError> {
    let (mut file, mut warnings) = ModelFile::from_json(text)?;
    let subsystems::Flat {
        file: mut flat,
        plan: mut params,
        block_origins,
        connection_origins,
        checks,
    } = file.flatten()?;
    for c in &checks {
        c.verify(registry)?;
    }
    // Errors name the place in the file as written.
    let origin = |e: FileError| origin(e, &block_origins, &connection_origins);
    let (mut model, ids) = flat.build(registry).map_err(origin)?;
    // Formulas give each block parameter in its unit (ADR 0045).
    params.fit_blocks(&mut model, &ids)?;
    for (block, param, value) in params.evaluate() {
        if let Some(b) = flat.blocks.iter_mut().find(|b| b.name == block) {
            b.params.insert(param, ParamValue::Number(value));
        }
    }
    // Every parameter of a block inside an instance has a base value (the
    // subsystem's, or the type's default), so saving writes only changes.
    for b in flat.blocks.iter().filter(|b| b.name.contains('/')) {
        let block = model.block(ids[&b.name]).map_err(|e| invalid(&b.name, e))?;
        for p in &block.spec().params {
            let known = params
                .inner
                .iter()
                .any(|n| n.block == b.name && n.param == p.name)
                || params.formula_of(&b.name, &p.name).is_some();
            if !known {
                params.inner.push(subsystems::Inner {
                    block: b.name.clone(),
                    param: p.name.clone(),
                    base: p.default,
                });
            }
        }
    }
    let groups = flat.fuse(&mut model, &ids, registry)?;
    flat.controls = file.controls.clone();
    let mut conversions = file.converted.clone();
    warnings.extend(flat.check_controls(&model, &ids, &params, &mut conversions));
    file.controls = flat.controls.clone();
    let plan = flat.plan(&model, &ids)?;
    // Older sweep controls become a `sample` section as the file is read.
    file.sample = flat.sample.clone();
    file.controls = flat.controls.clone();
    Ok(Loaded {
        conversions,
        file,
        flat,
        params,
        model,
        ids,
        warnings,
        groups,
        plan,
    })
}

/// Map an error at a flat path (`blocks[3].method`) to where it is
/// written (`subsystems.lag.blocks[0].method`).
pub(crate) fn origin(e: FileError, blocks: &[String], connections: &[String]) -> FileError {
    match e {
        FileError::Invalid { path, message } => {
            let mapped = [("blocks[", blocks), ("connections[", connections)]
                .iter()
                .find_map(|(head, origins)| {
                    let rest = path.strip_prefix(head)?;
                    let (i, tail) = rest.split_once(']')?;
                    Some(format!("{}{tail}", origins.get(i.parse::<usize>().ok()?)?))
                });
            FileError::Invalid {
                path: mapped.unwrap_or(path),
                message,
            }
        }
        e => e,
    }
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
    #[serde(default)]
    sample: Option<Sampling>,
    #[serde(default)]
    params: BTreeMap<String, ParamDecl>,
    #[serde(default)]
    subsystems: BTreeMap<String, Subsystem>,
    blocks: Vec<BlockEntry>,
    #[serde(default)]
    connections: Vec<ConnectionEntry>,
    #[serde(default)]
    fusion: Fusion,
    #[serde(default)]
    groups: Vec<GroupEntry>,
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
            sample: None,
            params: BTreeMap::new(),
            subsystems: BTreeMap::new(),
            blocks: Vec::new(),
            connections: Vec::new(),
            fusion: Fusion::Auto,
            groups: Vec::new(),
            controls: Controls::default(),
            converted: Vec::new(),
        }
    }

    /// Parse without building. Returns warnings for a malformed `controls`
    /// section, which is replaced by empty controls.
    pub fn from_json(text: &str) -> Result<(Self, Vec<String>), FileError> {
        // The format number first: a newer file may not parse as this one.
        let mut converted = Vec::new();
        if let Ok(v) = serde_json::from_str::<Value>(text) {
            if let Some(n) = v.get("format").and_then(Value::as_u64) {
                if n > FORMAT_VERSION as u64 {
                    return Err(FileError::UnsupportedFormat(n as u32));
                }
            }
            if slider_without_pin(&v) {
                converted.push(
                    "a slider now pins its value to the Controls, where the slider can be \
                     turned off without unpinning (ADR 0054)"
                        .to_string(),
                );
            }
            if places_written_at(&v) {
                converted.push(
                    "block and subsystem port places 'at' became 'canvas', like every node's \
                     (ADR 0052)"
                        .to_string(),
                );
            }
        }
        let raw: RawFile = serde_json::from_str(text).map_err(|e| {
            let mut message = e.to_string();
            if message.contains("unknown field") {
                // ADR 0044: within a format, newer versions only add.
                message.push_str(
                    " (a field this build does not know: check its spelling against \
                     `simsandbox schema`, or update SimSandbox if the file comes from a newer \
                     version)",
                );
            }
            FileError::Syntax {
                line: e.line(),
                column: e.column(),
                message,
            }
        })?;
        if raw.format != FORMAT_VERSION {
            return Err(FileError::UnsupportedFormat(raw.format));
        }
        let positive = |v: f64| v.is_finite() && v > 0.0;
        for (k, c) in raw.run.inputs.iter().enumerate() {
            if !(c.t.is_finite() && c.t >= 0.0) {
                return Err(invalid(
                    format!("run.inputs[{k}].t"),
                    format!("a time is 0 or more, got {}", c.t),
                ));
            }
            if !c.to.is_finite() {
                return Err(invalid(
                    format!("run.inputs[{k}].to"),
                    format!("must be a finite number, got {}", c.to),
                ));
            }
        }
        if !positive(raw.run.dt) {
            return Err(invalid(
                "run.dt",
                format!("must be positive, got {}", raw.run.dt),
            ));
        }
        if let Some(name) = raw.run.loops.as_deref() {
            if simsandbox_core::loops::LoopMethod::from_name(name).is_none() {
                return Err(invalid(
                    "run.loops",
                    format!("unknown method '{name}' (expected newton, fixed_point or refuse)"),
                ));
            }
        }
        if let Some(t) = raw.run.t_end.filter(|&t| !positive(t)) {
            return Err(invalid("run.t_end", format!("must be positive, got {t}")));
        }
        let mut warnings = Vec::new();
        let mut controls: Controls = match raw.controls {
            None => Controls::default(),
            Some(v) => serde_json::from_value(v).unwrap_or_else(|e| {
                warnings.push(format!("controls: ignored, {e}"));
                Controls::default()
            }),
        };
        // A slider written before pins (ADR 0054) pins its value, so the
        // two can then be changed apart.
        for c in controls.params.values_mut() {
            c.pin |= c.slider;
        }
        // Probes never change results: an invalid one is dropped, not the file.
        let mut names = std::collections::BTreeSet::new();
        let mut k = 0;
        controls.probes.retain(|pr| {
            let checked = pr.check().and_then(|()| {
                names
                    .insert(pr.name.clone())
                    .then_some(())
                    .ok_or_else(|| format!("'{}' is used twice", pr.name))
            });
            if let Err(e) = &checked {
                warnings.push(format!("controls.probes[{k}]: {e}; ignored"));
            }
            k += 1;
            checked.is_ok()
        });
        Ok((
            Self {
                format: raw.format,
                description: raw.description,
                run: raw.run,
                sample: raw.sample,
                params: raw.params,
                subsystems: raw.subsystems,
                blocks: raw.blocks,
                connections: raw.connections,
                fusion: raw.fusion,
                groups: raw.groups,
                controls,
                converted,
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
                .map(|(p, v)| (p.name.clone(), ParamValue::Number(*v)))
                .collect();
        }
        Ok(())
    }

    pub(crate) fn build(
        &self,
        registry: &Registry,
    ) -> Result<(Model, BTreeMap<String, BlockId>), FileError> {
        let mut model = Model::new();
        model.set_seed(
            self.run
                .seed
                .unwrap_or(simsandbox_core::graph::DEFAULT_SEED),
        );
        let loops = self.run.loops.as_deref().unwrap_or("newton");
        model.set_loop_method(
            simsandbox_core::loops::LoopMethod::from_name(loops).unwrap_or_default(),
        );
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
            if entry
                .canvas
                .is_some_and(|p| !p.iter().all(|v| v.is_finite()))
            {
                return Err(invalid(
                    format!("{path}.canvas"),
                    "positions must be finite numbers",
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
            let config = BlockConfig {
                method: (&entry.method).into(),
                definition: entry.equations.clone(),
                update: entry.update.clone(),
                settings: entry.settings.clone(),
            };
            let block = registry
                .build(&entry.type_name, &entry.name, &config)
                .map_err(|e| match e {
                    SimError::InvalidDefinition(_) => invalid(format!("{path}.equations"), e),
                    SimError::InvalidSetting { ref setting, .. } => {
                        invalid(format!("{path}.{setting}"), e)
                    }
                    _ => invalid(format!("{path}.method"), e),
                })?;
            let start = entry
                .start
                .as_ref()
                .map(|s| s.to_start(block.spec()))
                .transpose()
                .map_err(|e| invalid(format!("{path}.start"), e))?;
            let id = model.add(block);
            for (name, value) in &entry.params {
                // Formulas were evaluated when the file was flattened.
                let value = value
                    .number()
                    .ok_or_else(|| invalid(format!("{path}.params.{name}"), "not evaluated"))?;
                model
                    .set_param(id, name, value)
                    .map_err(|e| invalid(format!("{path}.params.{name}"), e))?;
            }
            model
                .set_start(id, start)
                .map_err(|e| invalid(format!("{path}.start"), e))?;
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

    /// Fuse the explicit groups, then (unless `fusion` is off) every set of
    /// connected continuous equation blocks with identical methods
    /// (ADR 0023).
    fn fuse(
        &self,
        model: &mut Model,
        ids: &BTreeMap<String, BlockId>,
        registry: &Registry,
    ) -> Result<Vec<FusedGroup>, FileError> {
        let mut fused = Vec::new();
        let mut taken: Vec<&str> = Vec::new();
        let make = |model: &mut Model,
                    members: Vec<BlockId>,
                    method: &MethodEntry,
                    path: &str|
         -> Result<String, FileError> {
            let domain = model
                .block(members[0])
                .map_err(|e| invalid(path, e))?
                .spec()
                .domain;
            let solver = registry
                .group_solver(domain, &method.into())
                .ok_or_else(|| invalid(path, format!("{domain:?} blocks cannot be fused")))?
                .map_err(|e| invalid(format!("{path}.method"), e))?;
            let name = solver.method().name.clone();
            model
                .fuse(&members, solver)
                .map_err(|e| invalid(format!("{path}.blocks"), e))?;
            Ok(name)
        };
        for (gi, g) in self.groups.iter().enumerate() {
            let path = format!("groups[{gi}]");
            if g.blocks.is_empty() {
                return Err(invalid(
                    format!("{path}.blocks"),
                    "a group needs at least one block",
                ));
            }
            let mut members = Vec::new();
            for (k, name) in g.blocks.iter().enumerate() {
                let id = *ids.get(name).ok_or_else(|| {
                    invalid(format!("{path}.blocks[{k}]"), format!("no block '{name}'"))
                })?;
                if taken.contains(&name.as_str()) {
                    return Err(invalid(
                        format!("{path}.blocks[{k}]"),
                        format!("'{name}' is already in a group"),
                    ));
                }
                let domain = |id| model.block(id).map(|b| b.spec().domain).ok();
                if domain(id) != domain(members.first().copied().unwrap_or(id)) {
                    return Err(invalid(
                        format!("{path}.blocks[{k}]"),
                        "all blocks in a group must belong to one domain",
                    ));
                }
                taken.push(name);
                members.push(id);
            }
            let method = make(model, members, &g.method, &path)?;
            fused.push(FusedGroup {
                blocks: g.blocks.clone(),
                method,
                explicit: true,
            });
        }
        if self.fusion == Fusion::Off {
            return Ok(fused);
        }
        // Automatic: union connected candidates, then fuse each set with
        // two or more blocks with states. Stateless equation blocks
        // (formulas, conversions) join the set they connect, whatever its
        // method, but never merge two sets with different methods
        // (ADR 0051).
        let candidate = |i: usize| {
            let e = &self.blocks[i];
            !taken.contains(&e.name.as_str()) && model.is_fusable(ids[&e.name])
        };
        let stateless: Vec<bool> = self
            .blocks
            .iter()
            .map(|b| model.is_algebraic(ids[&b.name]))
            .collect();
        let algebraic = |i: usize| stateless[i];
        let index = |name: &str| self.blocks.iter().position(|b| b.name == name);
        let mut parent: Vec<usize> = (0..self.blocks.len()).collect();
        // Each set's method (by root): that of its blocks with states, as
        // built, so a block whose method is left out (its type's default)
        // matches one that names it, options included (`RK4 (1 substeps)`).
        let resolved: Vec<String> = self
            .blocks
            .iter()
            .map(|b| {
                model
                    .block(ids[&b.name])
                    .map(|x| x.method().name.clone())
                    .unwrap_or_default()
            })
            .collect();
        let mut method: Vec<Option<&String>> = (0..self.blocks.len())
            .map(|i| (!algebraic(i)).then_some(&resolved[i]))
            .collect();
        fn root(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        for c in &self.connections {
            let block_of = |s: &str| s.split_once('.').and_then(|(b, _)| index(b));
            let (Some(a), Some(b)) = (block_of(&c.from), block_of(&c.to)) else {
                continue;
            };
            let domain = |k: usize| {
                model
                    .block(ids[&self.blocks[k].name])
                    .map(|x| x.spec().domain)
                    .ok()
            };
            if a == b || !candidate(a) || !candidate(b) || domain(a) != domain(b) {
                continue;
            }
            let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
            if ra == rb {
                continue;
            }
            let joined = match (method[ra], method[rb]) {
                (Some(x), Some(y)) if x != y => continue,
                (x, y) => x.or(y),
            };
            let (keep, gone) = (ra.min(rb), ra.max(rb));
            parent[gone] = keep;
            method[keep] = joined;
        }
        let mut sets: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for i in (0..self.blocks.len()).filter(|&i| candidate(i)) {
            let r = root(&mut parent, i);
            sets.entry(r).or_default().push(i);
        }
        for set in sets
            .into_values()
            .filter(|s| s.iter().filter(|&&i| !algebraic(i)).count() > 1)
        {
            let first = *set.iter().find(|&&i| !algebraic(i)).expect("filtered");
            let members = set.iter().map(|&i| ids[&self.blocks[i].name]).collect();
            let path = format!("blocks[{first}]");
            // The method as written by a member that names it, else the
            // type's default (its first), which those left out run.
            let written = set
                .iter()
                .filter(|&&i| !algebraic(i))
                .map(|&i| &self.blocks[i])
                .find(|b| !b.method.name.is_empty())
                .map(|b| b.method.clone());
            let entry = written.unwrap_or_else(|| MethodEntry {
                name: registry
                    .info(&self.blocks[first].type_name)
                    .and_then(|t| t.methods.first().map(|(m, _)| m.clone()))
                    .unwrap_or_default(),
                options: BTreeMap::new(),
            });
            let method = make(model, members, &entry, &path)?;
            fused.push(FusedGroup {
                blocks: set.iter().map(|&i| self.blocks[i].name.clone()).collect(),
                method,
                explicit: false,
            });
        }
        Ok(fused)
    }

    /// The sweep made concrete for `model`: the file's `sample`, or for a
    /// model without states a grid over its first parameter; `None` for a
    /// model that runs in time.
    pub fn plan(
        &self,
        model: &Model,
        ids: &BTreeMap<String, BlockId>,
    ) -> Result<Option<Plan>, FileError> {
        match self.sampling(model, ids)? {
            Some(s) => s.plan(self, model, ids).map(Some),
            None => Ok(None),
        }
    }

    /// How `model` is sampled: the file's `sample`, or for a model without
    /// states a grid over its first parameter; `None` for a model that runs
    /// in time.
    pub fn sampling(
        &self,
        model: &Model,
        ids: &BTreeMap<String, BlockId>,
    ) -> Result<Option<Sampling>, FileError> {
        let stateless = self.blocks.iter().all(|b| {
            model
                .block(ids[&b.name])
                .is_ok_and(|x| x.spec().states.is_empty())
        });
        match (&self.sample, stateless) {
            (Some(_), false) => Err(invalid(
                "sample",
                "sampling models with states is not supported yet (ADR 0027); remove \
                 'sample' to run in time",
            )),
            (Some(s), true) => Ok(Some(s.clone())),
            (None, false) => Ok(None),
            (None, true) => {
                let first = self.blocks.iter().find_map(|b| {
                    let spec = model.block(ids[&b.name]).ok()?.spec().clone();
                    spec.params
                        .first()
                        .map(|p| format!("{}.{}", b.name, p.name))
                });
                Ok(first.map(|param| Sampling {
                    design: Design::Grid,
                    params: vec![SweepControl {
                        param,
                        range: None,
                        points: None,
                    }],
                    count: None,
                    seed: None,
                    values: Vec::new(),
                }))
            }
        }
    }

    /// Drop controls that refer to nothing or carry bad values; return a
    /// warning for each.
    fn check_controls(
        &mut self,
        model: &Model,
        ids: &BTreeMap<String, BlockId>,
        declared: &ParamPlan,
        conversions: &mut Vec<String>,
    ) -> Vec<String> {
        let mut warnings = Vec::new();
        let c = &mut self.controls;
        c.params.retain(|key, pc| {
            let known = declared.declared(key).is_some() || port_ref(key, ids).ok().is_some_and(|(id, name)| {
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
            conversions.push("controls.view became the first of controls.views (ADR 0016)".into());
            match view_problem(&v, model, ids) {
                Some(problem) => warnings.push(format!("controls.view: {problem}; ignored")),
                // ADR 0014's single view becomes the first of `views`.
                None if c.views.is_empty() => c.views.push(v.into_entry()),
                None => {}
            }
        }
        let c = &mut self.controls;
        // Derived signals as written before filters become expression
        // filters (ADR 0030); their inputs are filled in by the app.
        if !c.signals.is_empty() {
            conversions.push(
                "controls.signals became expression filters in controls.filters (ADR 0030)".into(),
            );
        }
        for (name, expr) in std::mem::take(&mut c.signals) {
            if c.filters.iter().any(|f| f.name == name) {
                warnings.push(format!(
                    "controls.signals.{name}: a filter has that name; ignored"
                ));
                continue;
            }
            c.filters
                .push(FilterEntry::expression(&name, &expr, Vec::new()));
        }
        // Filters need a unique, plain name and a known type.
        let mut names: Vec<String> = Vec::new();
        c.filters.retain(|f| {
            let plain = f
                .name
                .chars()
                .next()
                .is_some_and(|ch| ch.is_alphabetic() || ch == '_')
                && f.name.chars().all(|ch| ch.is_alphanumeric() || ch == '_');
            let problem = if !plain {
                Some("names must be letters, digits and _".to_string())
            } else if names.contains(&f.name) {
                Some("defined twice".to_string())
            } else if !["expression", "cut"].contains(&f.type_name.as_str()) {
                Some(format!("unknown filter type '{}'", f.type_name))
            } else {
                None
            };
            if let Some(p) = problem {
                warnings.push(format!("controls.filters.{}: {p}; ignored", f.name));
                return false;
            }
            names.push(f.name.clone());
            true
        });
        // A profile with ADR 0029 cuts becomes a profile of a cut filter
        // (ADR 0031).
        for v in &mut c.views {
            let Some(sel) = v.selection.as_ref().filter(|s| s.get("cuts").is_some()) else {
                continue;
            };
            conversions.push("a profile's cuts became a cut filter (ADR 0031)".into());
            let grid = v.bind.get("y").and_then(|b| b.signals().into_iter().next());
            let mut k = 1;
            while c.filters.iter().any(|f| f.name == format!("cuts{k}")) {
                k += 1;
            }
            let filter = grid.and_then(|g| cut_filter_from_selection(&format!("cuts{k}"), &g, sel));
            if let Some(f) = filter {
                let outputs = f.outputs();
                let (lines, coordinate) = outputs.split_at(outputs.len() - 1);
                v.bind.insert("y".into(), Binding::from(lines.to_vec()));
                v.bind
                    .insert("x".into(), Binding::One(coordinate[0].clone()));
                c.filters.push(f);
            }
            v.selection = None;
        }
        // The sweep as written before `sample` (ADR 0027): a grid over the
        // parameters it names, read leniently as controls were, with views
        // that bound `t` (the swept parameter) bound to the first
        // parameter, and two-parameter results read from their grids
        // (ADR 0028).
        let old_sweep = std::mem::take(&mut c.sweep);
        if !old_sweep.is_empty() {
            conversions
                .push("controls.sweep became the sample section (ADR 0027, ADR 0028)".into());
        }
        let stateless = self.blocks.iter().all(|b| {
            model
                .block(ids[&b.name])
                .is_ok_and(|x| x.spec().states.is_empty())
        });
        if !old_sweep.is_empty() && !stateless {
            warnings
                .push("controls.sweep: the model has states, so it runs in time; ignored".into());
        }
        let mut params: Vec<SweepControl> = Vec::new();
        for (i, mut sw) in old_sweep.into_iter().enumerate() {
            if self.sample.is_some() || !stateless {
                break;
            }
            let path = format!("controls.sweep[{i}]");
            let known = port_ref(&sw.param, ids).ok().is_some_and(|(id, name)| {
                model
                    .block(id)
                    .is_ok_and(|b| b.spec().params.iter().any(|p| p.name == name))
            });
            let problem = if !known {
                Some(format!("no parameter '{}'", sw.param))
            } else if params.iter().any(|p| p.param == sw.param) {
                Some(format!("'{}' is swept twice", sw.param))
            } else if params.len() == 2 {
                Some("at most two parameters".into())
            } else {
                None
            };
            if let Some(p) = problem {
                warnings.push(format!("{path}: {p}; ignored"));
                continue;
            }
            if let Some([lo, hi]) = sw.range {
                if !(lo.is_finite() && hi.is_finite() && lo < hi) {
                    warnings.push(format!(
                        "{path}.range: [{lo}, {hi}] is not an increasing range; using the default"
                    ));
                    sw.range = None;
                }
            }
            if let Some(n) = sw.points.filter(|n| !(2..=100_000).contains(n)) {
                warnings.push(format!(
                    "{path}.points: {n} is outside 2 to 100000; using the default"
                ));
                sw.points = None;
            }
            params.push(sw);
        }
        if let [a, b] = params.as_mut_slice() {
            let n = |p: &SweepControl| p.points.unwrap_or(DEFAULT_GRID_POINTS);
            if n(a).saturating_mul(n(b)) > MAX_SAMPLES {
                warnings.push(format!(
                    "controls.sweep: more than {MAX_SAMPLES} points; using the default"
                ));
                a.points = None;
                b.points = None;
            }
        }
        if !params.is_empty() {
            let keys: Vec<String> = params.iter().map(|s| s.param.clone()).collect();
            for v in &mut c.views {
                for b in v.bind.values_mut() {
                    let names: Vec<String> = b
                        .signals()
                        .into_iter()
                        .map(|n| match (n.as_str(), keys.len(), v.type_name.as_str()) {
                            ("t", _, _) => keys[0].clone(),
                            (_, 2, "plot.heatmap") => format!("grid.{n}"),
                            _ => n,
                        })
                        .collect();
                    *b = Binding::from(names);
                }
            }
            if keys.len() == 2 {
                for f in c.filters.iter_mut().filter(|f| f.type_name == "cut") {
                    for i in &mut f.inputs {
                        *i = format!("grid.{i}");
                    }
                }
            }
            self.sample = Some(Sampling {
                design: Design::Grid,
                params,
                count: None,
                seed: None,
                values: Vec::new(),
            });
        }
        let c = &mut self.controls;
        // Views may bind filter outputs as well as outputs; in a sweep the
        // swept parameters, and in a two-parameter grid every scalar's grid
        // (`grid.<name>`, ADR 0028).
        let mut derived: Vec<String> = c.filters.iter().flat_map(FilterEntry::outputs).collect();
        let (keys, grid2) = match &self.sample {
            Some(sm) => (
                sm.params
                    .iter()
                    .map(|p| p.param.clone())
                    .collect::<Vec<_>>(),
                sm.design == Design::Grid && sm.params.len() == 2,
            ),
            None => (Vec::new(), false),
        };
        derived.extend(keys.iter().cloned());
        let mut kept = Vec::new();
        // Each view's new place, for the layout.
        let mut moved: Vec<Option<usize>> = vec![None; c.views.len()];
        let filter_outputs: Vec<String> = c.filters.iter().flat_map(FilterEntry::outputs).collect();
        for (i, mut v) in std::mem::take(&mut c.views).into_iter().enumerate() {
            let before = v.type_name.clone();
            if v.upgrade() {
                conversions.push(format!("controls.views[{i}]: the older {before} became today's form (ADR 0018, ADR 0024, ADR 0063)"));
            }
            // A filter's outputs are bound by name, wherever it runs
            // (ADR 0063); `probe:NAME/output` was written on 2026-10-29.
            let mut renamed = false;
            for b in v.bind.values_mut() {
                let plain: Vec<String> = b
                    .signals()
                    .into_iter()
                    .map(|s| {
                        let inner = s
                            .strip_prefix("probe:")
                            .and_then(|r| r.split_once('/'))
                            .map(|(_, o)| o.to_string());
                        match inner {
                            Some(o) if filter_outputs.contains(&o) => {
                                renamed = true;
                                o
                            }
                            _ => s,
                        }
                    })
                    .collect();
                *b = Binding::from(plain);
            }
            if renamed {
                conversions.push(format!(
                    "controls.views[{i}]: filter outputs are bound by name (ADR 0063)"
                ));
            }
            let scalar =
                |s: &str| derived.iter().any(|d| d == s) || output_element_exists(s, model, ids);
            let unknown = v.bind.values().flat_map(Binding::signals).find(|s| {
                // From a probe (ADR 0063): the model's signal it records.
                let s = s
                    .strip_prefix("probe:")
                    .and_then(|r| r.split_once('/'))
                    .map_or(s.as_str(), |(_, s)| s);
                let grid = grid2 && s.strip_prefix("grid.").is_some_and(scalar);
                s != "t" && !scalar(s) && !grid
            });
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
            moved[i] = Some(kept.len());
            kept.push(v);
        }
        c.views = kept;
        if let Some(l) = c.layout.take() {
            let map = |k: usize| moved.get(k).copied().flatten();
            c.layout = l.checked(c.views.len(), &map, &mut warnings);
        }
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
                    // Fields and point sets are bound as a whole.
                    (
                        Shape::Grid { .. } | Shape::Points { .. } | Shape::Population { .. },
                        None,
                    ) => true,
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
