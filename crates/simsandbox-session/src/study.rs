//! The study block (ADR 0070): a block whose definition is a model and a
//! `sample`. Each time its inputs change it runs the model at every point
//! (times its replicates) from the model's start, and outputs arrays with
//! one value per run.
//!
//! - **Model:** named as a use names it (ADR 0070): `model`, a definition
//!   in scope (ADR 0068), or `file`, a model file found from this file's
//!   folder and part of the run hash. The study's `sample` replaces the model's own.
//! - **Inputs:** the model's own input blocks (`input.value`), one scalar
//!   each, set before every point.
//! - **Outputs:** one array per swept parameter, then one per result: the
//!   `reduce` formulas, or each scalar output's final value. Names are the
//!   keys with `.` and `/` written `_` (`car.m` → `car_m`).
//! - It has no state: results depend only on the inputs and are kept until
//!   they change. Only the outermost study runs on several cores.
//! - **Points from inputs** (`"design": "inputs"`, ADR 0078): `count`
//!   points arrive through one array input per swept parameter, chosen by
//!   another model (a search strategy). Each point's results are kept by
//!   its values: when the inputs change, only new or changed points run.

use crate::reduce::{Reducer, UnitOf};
use crate::run::{loaded_outputs, run_study, Study};
use simsandbox_core::block::{
    AccuracyClass, Block, BlockSpec, Domain, MethodLabels, MethodSpec, ParamSpec, PortSpec, Shape,
    SpeedClass, StepContext, StepIo, TimeSemantics,
};
use simsandbox_core::registry::{BlockConfig, Registry, TypeInfo};
use simsandbox_core::units::Unit;
use simsandbox_core::{Result, SimError};
use std::path::PathBuf;
use std::sync::Mutex;

pub use simsandbox_file::STUDY;

/// The one method: every run as its model's methods compute it.
pub const RUNS: &str = "runs";

thread_local! {
    /// The models whose studies are being built on this thread, outermost
    /// first: each one's folder and text, and a name for messages.
    static CHAIN: std::cell::RefCell<Vec<(Option<PathBuf>, String, String)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// Set while models are built without running their studies.
    static UNRUN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Build models in `f` without running their studies (ADR 0071): each
/// study is built and checked (its model, sample and results), but outputs
/// NaN until it is evaluated outside `f`. The app builds models so on its
/// window's thread, to check an edit or to open one, and its worker runs
/// the studies ([`crate::Session::evaluate`]).
pub fn without_running_studies<R>(f: impl FnOnce() -> R) -> R {
    let was = UNRUN.with(|u| u.replace(true));
    let r = f();
    UNRUN.with(|u| u.set(was));
    r
}

/// Whether studies built now run (not inside [`without_running_studies`]).
pub(crate) fn studies_run() -> bool {
    !UNRUN.with(std::cell::Cell::get)
}

/// Register the study type.
pub fn register(registry: &mut Registry) -> Result<()> {
    registry.register(STUDY, |name, c| {
        Ok(Box::new(StudyBlock::from_config(name, c)?))
    })?;
    registry.describe(
        STUDY,
        TypeInfo::new(
            "Runs a model at each point of its `sample` (the designs, keep, seeds, replicates and \
             reduce of an experiment) and outputs arrays with one value per run: each swept \
             parameter, then each `reduce` result or each scalar output's final value. The \
             model is named as a use names it, `model` (a definition in `models`) or `file`; its input \
             blocks are the study's inputs. Write the sample in the block's `sample` section.",
        )
        .method(RUNS, &[])
        .setting("model", &[])
        .setting("file", &[])
        .file("file")
        .definition(&[
            r#"{"design": "list", "params": [{"param": "f.a"}], "values": [[1], [2]]}"#,
            r#"{"format": 1, "blocks": [{"name": "f", "type": "eq.equations", "equations": ["param a = 1", "y = 2*a"]}]}"#,
        ]),
    )
}

/// Where an output's values come from in a study's results.
#[derive(Debug, Clone)]
enum From {
    /// An index column (a swept parameter).
    Index(String),
    /// An output (a result).
    Output(String),
}

pub struct StudyBlock {
    spec: BlockSpec,
    method: MethodSpec,
    /// The model with the study's sample, and the folder it reads from.
    text: String,
    folder: Option<PathBuf>,
    /// Its model's name for messages: a file's path or a definition's name.
    label: String,
    plan: simsandbox_file::Plan,
    timeless: bool,
    /// The input block keys set from the inputs (`u.y`).
    keys: Vec<String>,
    /// Its parameters: its model's declared parameters (ADR 0074), in
    /// `spec.params` order, set before every point.
    declared: Vec<String>,
    from: Vec<From>,
    /// Values per run of each output: 1, or a vector's length (ADR 0080).
    widths: Vec<usize>,
    runs: usize,
    /// The inputs last run with, by their bits, and the outputs.
    kept: Mutex<Option<(Vec<u64>, Vec<f64>)>>,
    /// Points from inputs (ADR 0078): how many, and each point's results
    /// kept by its values.
    from_inputs: Option<usize>,
    by_point: Mutex<ByPoint>,
}

#[cfg(test)]
thread_local! {
    /// Tests: the points from inputs run on this thread so far.
    static RUNS_MADE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A study's results kept point by point (ADR 0078): the shared inputs and
/// parameters they were run with, by their bits, and for each point its
/// values' bits and its results (each output's replicates, in output
/// order). A point keeps its results while both are unchanged.
#[derive(Default)]
struct ByPoint {
    shared: Option<Vec<u64>>,
    points: Vec<Option<(Vec<u64>, Vec<f64>)>>,
}

fn invalid(reason: impl Into<String>) -> SimError {
    SimError::InvalidDefinition(reason.into())
}

/// A port name for a key: `.` and `/` written `_`.
fn port_name(key: &str) -> String {
    key.replace(['.', '/'], "_")
}

impl StudyBlock {
    pub fn from_config(name: &str, c: &BlockConfig) -> Result<Self> {
        if let Some(k) = c.settings.keys().find(|k| k.as_str() != "file") {
            return Err(SimError::InvalidSetting {
                setting: format!("settings.{k}"),
                reason: "a study has no settings: it names its model as a use does, with \"model\" (a definition here) or \"file\"".into(),
            });
        }
        // No method given is its one method.
        if !matches!(c.method.name.as_str(), "" | RUNS) || !c.method.options.is_empty() {
            return Err(SimError::InvalidMethod(format!(
                "a study's method is '{RUNS}', with no options: its model's blocks keep theirs"
            )));
        }
        if c.update.is_some() {
            return Err(invalid("a study takes no 'update'"));
        }
        let mut lines = c.definition.iter();
        let sample = lines.next().filter(|s| !s.is_empty()).ok_or_else(|| {
            invalid("a study needs a `sample`: its design and parameters, as for an experiment")
        })?;
        let defined = lines.next().filter(|s| !s.is_empty());
        if lines.next().is_some() {
            return Err(invalid("a study takes no equations"));
        }
        // Its model: a definition's text (found by the loader from
        // `model`), or a file's.
        let (model, folder, label) = match (defined, c.file("file")) {
            (Some(_), Some(_)) => {
                return Err(SimError::InvalidSetting {
                    setting: "file".into(),
                    reason: "a study names a model or a file, not both".into(),
                })
            }
            // A definition reads files from its file's folder, or the
            // thread's when it was given none.
            (Some(text), None) => (
                text.clone(),
                c.folder
                    .clone()
                    .or_else(simsandbox_core::registry::current_folder),
                "a model defined in the file".to_string(),
            ),
            (None, Some(path)) => {
                let path = path?;
                let bytes = simsandbox_file::compose::read_model(&path).map_err(|e| {
                    SimError::InvalidSetting {
                        setting: "file".into(),
                        reason: format!("{}: {e}", path.display()),
                    }
                })?;
                let text = String::from_utf8(bytes).map_err(|_| SimError::InvalidSetting {
                    setting: "file".into(),
                    reason: format!("{}: not a text file", path.display()),
                })?;
                let label = path.display().to_string();
                (text, path.parent().map(Into::into), label)
            }
            (None, None) => {
                return Err(SimError::InvalidSetting {
                    setting: "model".into(),
                    reason: "a study names its model as a use does: \"model\" (a definition here) or \"file\"".into(),
                })
            }
        };
        let sampling: simsandbox_file::Sampling =
            serde_json::from_str(sample).map_err(|e| invalid(format!("sample: {e}")))?;
        let text = model;
        // Its exposed inputs are the study's inputs (ADR 0074).
        let text = with_exposed_inputs(&text)?;
        let loaded = load(&text, folder.as_deref(), &label)?;
        // Its points file and its model's data files are in the run hash of
        // the model holding it.
        let mut data = loaded.data.clone();
        if let Some(d) = loaded.points_digest(&sampling) {
            data.insert("sample.file".into(), d);
        }
        simsandbox_file::report_data(name, &data);
        let plan = loaded
            .plan_of(&sampling)
            .map_err(|e| invalid(e.to_string()))?;
        // Nothing in time: each point is one evaluation (ADR 0067).
        // Its parameters are its model's (ADR 0074): the model's values
        // are their defaults, each study sets its own.
        let declared: Vec<String> = loaded.file.params.keys().cloned().collect();
        let params: Vec<ParamSpec> = loaded
            .file
            .params
            .iter()
            .map(|(n, d)| {
                let [lo, hi] = d.range.unwrap_or(if d.value == 0.0 {
                    [-1.0, 1.0]
                } else {
                    [d.value.min(0.0), 2.0 * d.value.max(0.0)]
                });
                ParamSpec {
                    name: n.clone(),
                    unit: Unit::parse(&d.unit).unwrap_or(Unit::DIMENSIONLESS),
                    default: d.value,
                    suggested_min: lo,
                    suggested_max: hi,
                    description: if d.description.is_empty() {
                        format!("{n} of the model it studies")
                    } else {
                        d.description.clone()
                    },
                    initial_condition: false,
                }
            })
            .collect();
        let timeless = loaded.flat.blocks.iter().all(|b| {
            loaded
                .ids
                .get(&b.name)
                .and_then(|id| loaded.model.block(*id).ok())
                .is_some_and(|x| x.spec().states.is_empty() && x.spec().tables.is_empty())
        });
        if plan.keep == simsandbox_file::Keep::Series {
            return Err(invalid(
                "sample.keep: a study keeps one value per run; \"series\" is for an experiment \
                 run on its own (`simsandbox run` of the model)",
            ));
        }
        let runs = plan.points.len() * plan.replicates;
        // A grid over two parameters, run once each: 2D results, the first
        // parameter fastest (ADR 0071).
        let grid2 = match plan.grid.as_deref() {
            Some(&[nx, ny]) if plan.replicates == 1 && plan.keys.len() == 2 => Some((nx, ny)),
            _ => None,
        };
        // Inputs: the points, one array per swept parameter, when they
        // come from inputs (ADR 0078); then the model's own input blocks.
        let mut ports = Vec::new();
        let mut keys = Vec::new();
        let from_inputs =
            (plan.design == simsandbox_file::Design::Inputs).then_some(plan.points.len());
        if let Some(n) = from_inputs {
            for key in &plan.keys {
                ports.push(PortSpec::input(
                    &port_name(key),
                    key_unit(&loaded, key),
                    Shape::Grid { nx: n, ny: 1 },
                ));
            }
        }
        for b in &loaded.flat.blocks {
            // Those it sweeps are set by its points instead.
            let key = format!("{}.y", b.name);
            if b.type_name != simsandbox_eq::input::VALUE
                || b.name.contains('/')
                || plan.keys.contains(&key)
            {
                continue;
            }
            let unit = param_unit(&loaded, &b.name, "y");
            ports.push(PortSpec::input(&b.name, unit, Shape::Scalar));
            keys.push(key);
        }
        let inputs: Vec<String> = ports.iter().map(|p| p.name.clone()).collect();
        let inputs: Vec<&str> = inputs.iter().map(String::as_str).collect();
        let mut from = Vec::new();
        let mut widths = Vec::new();
        let mut vectors = Vec::new();
        let mut out = |name: String, unit: Unit, f: From, width: Option<usize>| -> Result<()> {
            if ports.iter().any(|p| p.name == name) {
                return Err(invalid(format!(
                    "two of its ports would be named '{name}': rename a result or an input block"
                )));
            }
            // A line of cells, one per run, as array formulas read them
            // (`grid k = runs`, `input a[k]`); a vector per run is over its
            // own axis (fastest) and the runs (ADR 0080).
            let shape = match (grid2, width) {
                (Some((nx, ny)), None) => Shape::Grid { nx, ny },
                (None, None) => Shape::Grid { nx: runs, ny: 1 },
                (None, Some(n)) => Shape::Grid { nx: n, ny: runs },
                (Some(_), Some(_)) => {
                    return Err(invalid(format!(
                        "sample.reduce.{name}: a vector per run over a grid of two parameters would have three axes; a port has at most two until ADR 0081 is built"
                    )))
                }
            };
            ports.push(PortSpec::output(&name, unit, shape).depends_on(&inputs));
            from.push(f);
            widths.push(width.unwrap_or(1));
            vectors.push(width.is_some());
            Ok(())
        };
        // Ids given by a points file, as the rows give them.
        if plan.ids.is_some() {
            out(
                "id".into(),
                Unit::DIMENSIONLESS,
                From::Index("id".into()),
                None,
            )?;
        }
        // The points are outputs too, unless they came in as inputs.
        for key in plan.keys.iter().filter(|_| from_inputs.is_none()) {
            out(
                port_name(key),
                key_unit(&loaded, key),
                From::Index(key.clone()),
                None,
            )?;
        }
        let fixed = loaded_outputs(&loaded).map_err(invalid)?;
        // The model's input blocks give the study's inputs (or its keys),
        // not results.
        let given = |full: &str| {
            loaded.flat.blocks.iter().any(|b| {
                b.type_name == simsandbox_eq::input::VALUE
                    && full.strip_prefix(b.name.as_str()) == Some(".y")
            })
        };
        if plan.reduce.is_empty() {
            for (id, port, full, shape) in &fixed {
                if *shape != Shape::Scalar || given(full) {
                    continue;
                }
                let unit = loaded
                    .model
                    .block(*id)?
                    .spec()
                    .outputs()
                    .find(|p| &p.name == port)
                    .map_or(Unit::DIMENSIONLESS, |p| p.unit);
                out(port_name(full), unit, From::Output(full.clone()), None)?;
            }
        } else {
            for (name, formula) in &plan.reduce {
                if !simsandbox_file::subsystems::valid_name(name) {
                    return Err(invalid(format!(
                        "sample.reduce.{name}: a result's name is letters, digits and '_'"
                    )));
                }
                let mut r = Reducer::parse(name, formula)
                    .map_err(|e| invalid(format!("sample.reduce.{e}")))?;
                // `at_samples` reads its data's times within the run.
                let t_end = crate::run::end_time(&loaded.model, loaded.file.run.t_end);
                r.place(&|d| crate::run::sample_times(&loaded, d, t_end))
                    .map_err(|e| invalid(format!("sample.reduce.{e}")))?;
                let unit = match r.unit_of() {
                    UnitOf::Time => Unit::parse("s").expect("seconds parse"),
                    UnitOf::Signal(s) => signal_unit(&loaded, &fixed, &s),
                    UnitOf::Unknown => Unit::DIMENSIONLESS,
                };
                out(name.clone(), unit, From::Output(name.clone()), r.width())?;
            }
        }
        // One parameter on a grid, run once each: its values are the
        // coordinate of every result, as an axis is of a field.
        // On a grid over two, each parameter places the results' cells
        // along its axis.
        let keys_placing = match (plan.keys.len(), grid2) {
            (1, _) if plan.replicates == 1 && plan.design == simsandbox_file::Design::Grid => 1,
            (2, Some(_)) => 2,
            _ => 0,
        };
        if keys_placing > 0 {
            let first_key = ports.len() - from.len() + usize::from(plan.ids.is_some());
            let first_result = first_key + keys_placing;
            // Over the runs: the first axis, or the second of a vector
            // per run.
            let results: Vec<(String, usize)> = ports[first_result..]
                .iter()
                .zip(&vectors[vectors.len() + first_result - ports.len()..])
                .map(|(p, &v)| (p.name.clone(), usize::from(v)))
                .collect();
            for axis in 0..keys_placing {
                let key = &mut ports[first_key + axis];
                for (r, along) in &results {
                    *key = key.clone().coordinate_of(r, axis + along);
                }
            }
        }
        // As slow as running many runs, and as accurate as its model.
        let inner = MethodLabels::combine(loaded.flat.blocks.iter().filter_map(|b| {
            let id = loaded.ids.get(&b.name)?;
            Some(loaded.model.block(*id).ok()?.method().labels)
        }))
        .unwrap_or(MethodLabels {
            speed: SpeedClass::RealTime,
            accuracy: AccuracyClass::Quantitative,
        });
        Ok(Self {
            spec: BlockSpec {
                name: name.into(),
                // Neither fused nor solved with others: a machine of its own.
                domain: Domain::External,
                ports,
                params,
                states: Vec::new(),
                tables: Vec::new(),
                time: TimeSemantics::Stateless,
            },
            method: MethodSpec {
                name: RUNS.into(),
                labels: MethodLabels {
                    speed: inner.speed.max(SpeedClass::Responsive),
                    accuracy: inner.accuracy,
                },
            },
            text,
            folder,
            label,
            plan,
            timeless,
            keys,
            declared,
            from,
            widths,
            runs,
            kept: Mutex::new(None),
            from_inputs,
            by_point: Mutex::new(ByPoint::default()),
        })
    }

    /// Every output's values for these inputs and parameters, run or kept.
    fn evaluate(&self, inputs: &[f64], params: &[f64]) -> Result<Vec<f64>> {
        // Built only to be checked or opened: nothing run, nothing kept.
        if !studies_run() {
            return Ok(vec![
                f64::NAN;
                self.widths.iter().sum::<usize>() * self.runs
            ]);
        }
        if let Some(n) = self.from_inputs {
            return self.evaluate_points(n, inputs, params);
        }
        let bits: Vec<u64> = inputs.iter().chain(params).map(|v| v.to_bits()).collect();
        let mut kept = self.kept.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((b, values)) = kept.as_ref() {
            if *b == bits {
                return Ok(values.clone());
            }
        }
        let values = self.run(self.plan.clone(), inputs, params)?;
        *kept = Some((bits, values.clone()));
        Ok(values)
    }

    /// Run `plan`'s points with the shared `inputs` (the model's input
    /// blocks) and `params`: each output's values, run by run.
    fn run(&self, plan: simsandbox_file::Plan, inputs: &[f64], params: &[f64]) -> Result<Vec<f64>> {
        let runs = plan.points.len() * plan.replicates;
        let study = Study {
            plan,
            timeless: self.timeless,
            set: self
                .declared
                .iter()
                .cloned()
                .zip(params.iter().copied())
                .chain(self.keys.iter().cloned().zip(inputs.iter().copied()))
                .collect(),
            t_end: None,
            dt: None,
        };
        let folder = self.folder.as_deref();
        let loaded = load(&self.text, folder, &self.label)?;
        let results = simsandbox_core::registry::in_folder(folder, || {
            run_study(
                &self.text,
                &crate::examples::registry(),
                &loaded,
                &study,
                None,
            )
        })
        .map_err(|e| SimError::Block(format!("{}: {e}", self.spec.name)))?;
        let mut values = Vec::with_capacity(self.from.len() * runs);
        for f in &self.from {
            let column = match f {
                From::Index(k) => results.index.iter().find(|(n, _)| n == k).map(|(_, v)| v),
                From::Output(n) => results
                    .outputs
                    .iter()
                    .find(|o| &o.name == n)
                    .map(|o| &o.values),
            }
            .ok_or_else(|| SimError::Block(format!("{}: no results for {f:?}", self.spec.name)))?;
            values.extend_from_slice(column);
        }
        Ok(values)
    }

    /// Points from inputs (ADR 0078): the first `n` values of each swept
    /// parameter's input are the points, the rest the shared inputs. Only
    /// points whose values (or the shared inputs) changed since they last
    /// ran run now, under their own ids, so their seeds and results are
    /// those of running every point again.
    fn evaluate_points(&self, n: usize, inputs: &[f64], params: &[f64]) -> Result<Vec<f64>> {
        let d = self.plan.keys.len();
        let reps = self.plan.replicates;
        let (points, shared) = inputs.split_at(d * n);
        let point = |p: usize| -> Vec<f64> { (0..d).map(|k| points[k * n + p]).collect() };
        let bits = |v: &[f64]| -> Vec<u64> { v.iter().map(|x| x.to_bits()).collect() };
        let shared_bits: Vec<u64> = shared.iter().chain(params).map(|v| v.to_bits()).collect();
        let mut memory = self.by_point.lock().unwrap_or_else(|e| e.into_inner());
        if memory.shared.as_ref() != Some(&shared_bits) || memory.points.len() != n {
            memory.shared = Some(shared_bits);
            memory.points = vec![None; n];
        }
        let todo: Vec<usize> = (0..n)
            .filter(|&p| {
                memory.points[p]
                    .as_ref()
                    .is_none_or(|(b, _)| *b != bits(&point(p)))
            })
            .collect();
        #[cfg(test)]
        RUNS_MADE.with(|r| r.set(r.get() + todo.len()));
        if !todo.is_empty() {
            let mut plan = self.plan.clone();
            plan.points = todo.iter().map(|&p| point(p)).collect();
            plan.ids = Some(todo.iter().map(|&p| p as u64).collect());
            let values = self.run(plan, shared, params)?;
            // `values` holds each output's runs for the points run, in
            // turn, each run `width` values.
            let runs = todo.len() * reps;
            for (i, &p) in todo.iter().enumerate() {
                let mut start = 0;
                let mut mine = Vec::new();
                for &w in &self.widths {
                    let at = start + i * reps * w;
                    mine.extend_from_slice(&values[at..at + reps * w]);
                    start += runs * w;
                }
                memory.points[p] = Some((bits(&point(p)), mine));
            }
        }
        let width: usize = self.widths.iter().sum();
        let mut out = Vec::with_capacity(width * n * reps);
        let mut start = 0;
        for &w in &self.widths {
            for p in 0..n {
                let (_, mine) = memory.points[p].as_ref().expect("every point ran");
                out.extend_from_slice(&mine[start * reps..(start + w) * reps]);
            }
            start += w;
        }
        Ok(out)
    }
}

/// A studied model's text with its exposed inputs (ADR 0074) as input
/// blocks, which the study takes as inputs; as written if it has none.
fn with_exposed_inputs(text: &str) -> Result<String> {
    let v: serde_json::Value =
        serde_json::from_str(text).map_err(|e| invalid(format!("its model: {e}")))?;
    if v.get("inputs")
        .and_then(serde_json::Value::as_array)
        .is_none_or(Vec::is_empty)
    {
        return Ok(text.to_string());
    }
    let mut model = simsandbox_file::compose::read_definition("studied", &v)
        .map_err(|e| invalid(format!("its model: {e}")))?;
    simsandbox_file::compose::expose_inputs(&mut model);
    Ok(model.to_json())
}

/// Load a study's model (`label` in messages), refusing a loop. Loading
/// builds the model's own studies, which load theirs, on this thread, so a
/// loop shows as the same text from the same folder twice in the chain;
/// the same text builds the same studies, so a repeat is always a loop.
/// (Definitions cannot loop: a study of one sees every definition in scope
/// but itself.)
fn load(
    text: &str,
    folder: Option<&std::path::Path>,
    label: &str,
) -> Result<simsandbox_file::Loaded> {
    let folder_buf = folder.map(PathBuf::from);
    let looped = CHAIN.with(|c| {
        let c = c.borrow();
        let at = c
            .iter()
            .position(|(f, t, _)| *f == folder_buf && t == text)?;
        let mut names: Vec<&str> = c[at..].iter().map(|(_, _, n)| n.as_str()).collect();
        names.push(label);
        Some(names.join(" → "))
    });
    if let Some(chain) = looped {
        return Err(invalid(format!("a study studies itself: {chain}")));
    }
    CHAIN.with(|c| {
        c.borrow_mut()
            .push((folder_buf, text.to_string(), label.to_string()))
    });
    let r = simsandbox_core::registry::in_folder(folder, || {
        simsandbox_file::load_any(text, &crate::examples::registry(), folder)
    })
    .map_err(|e| invalid(format!("its model: {e}")));
    CHAIN.with(|c| c.borrow_mut().pop());
    r
}

/// The unit of parameter `param` of flat block `block`, dimensionless if
/// there is none.
fn param_unit(loaded: &simsandbox_file::Loaded, block: &str, param: &str) -> Unit {
    loaded
        .ids
        .get(block)
        .and_then(|id| loaded.model.block(*id).ok())
        .and_then(|b| {
            b.spec()
                .params
                .iter()
                .find(|p| p.name == param)
                .map(|p| p.unit)
        })
        .unwrap_or(Unit::DIMENSIONLESS)
}

/// The unit of a swept key: a block's parameter (`car.m`) or a declared
/// parameter (`kp`).
fn key_unit(loaded: &simsandbox_file::Loaded, key: &str) -> Unit {
    match key.rsplit_once('.') {
        Some((block, param)) => param_unit(loaded, block, param),
        None => loaded
            .file
            .params
            .get(key)
            .and_then(|d| Unit::parse(&d.unit).ok())
            .unwrap_or(Unit::DIMENSIONLESS),
    }
}

/// The unit of the output `signal` (`car.v`, `car.v[0]`).
fn signal_unit(
    loaded: &simsandbox_file::Loaded,
    fixed: &crate::run::Outputs,
    signal: &str,
) -> Unit {
    let base = signal
        .strip_suffix(']')
        .and_then(|s| s.rsplit_once('['))
        .map_or(signal, |(b, _)| b);
    fixed
        .iter()
        .find(|(_, _, full, _)| full == base)
        .and_then(|(id, port, _, _)| {
            let b = loaded.model.block(*id).ok()?;
            b.spec().outputs().find(|p| &p.name == port).map(|p| p.unit)
        })
        .unwrap_or(Unit::DIMENSIONLESS)
}

impl Block for StudyBlock {
    fn spec(&self) -> &BlockSpec {
        &self.spec
    }

    fn method(&self) -> &MethodSpec {
        &self.method
    }

    fn init(&mut self, _params: &[f64], _state: &mut [f64]) -> Result<()> {
        Ok(())
    }

    fn do_step(&mut self, ctx: StepContext, io: StepIo<'_>) -> Result<()> {
        self.compute_outputs(ctx.t + ctx.dt, io)
    }

    fn compute_outputs(&self, _t: f64, io: StepIo<'_>) -> Result<()> {
        let values = self.evaluate(io.inputs, io.params)?;
        io.outputs.copy_from_slice(&values);
        Ok(())
    }

    /// Its runs have their own time; its results depend on its inputs and
    /// parameters only (ADR 0083).
    fn reads_time(&self) -> Option<bool> {
        Some(false)
    }
}

#[cfg(test)]
mod tests {
    use crate::examples::registry;
    use crate::run::{run, Results};
    use serde_json::{json, Value};
    use simsandbox_core::block::Shape;

    const CRUISE: &str = include_str!("../../../examples/cruise-control.ssbx.json");

    /// The cruise example as a definition: no format, description or
    /// controls.
    fn cruise() -> Value {
        let mut v: Value = serde_json::from_str(CRUISE).unwrap();
        let o = v.as_object_mut().unwrap();
        for k in ["format", "description", "controls"] {
            o.remove(k);
        }
        v
    }

    fn sample() -> Value {
        json!({"design": "list", "params": [{"param": "cruise.kp"}], "values": [[200.0], [800.0]],
               "reduce": {"v_max": "max(car.v)", "settle": "first_time(abs(cruise.e) < 0.5)",
                          "v_end": "final(car.v)", "over": "max(car.v) - final(car.v)"}})
    }

    fn value(r: &Results, name: &str) -> Vec<f64> {
        r.outputs
            .iter()
            .find(|o| o.name == name)
            .unwrap()
            .values
            .clone()
    }

    /// Points from inputs (ADR 0078): a study runs only the points whose
    /// values changed, and its results are those of a study of the same
    /// points from scratch.
    #[test]
    fn points_from_inputs_run_only_what_changed() {
        let r = registry();
        let model = |t_end: f64| {
            json!({"format": 1, "run": {"dt": 1, "t_end": t_end},
                "models": {"cruise": cruise()},
                "blocks": [
                    {"name": "pick", "type": "eq.equations", "equations": [
                        "grid k = 3",
                        "# point 2 moves at t = 1; points 0 and 1 stay",
                        "param moves [s] = 0.5",
                        "param base [N*s/m] = 200", "param spacing [N*s/m] = 300",
                        "param moved [N*s/m] = 100",
                        "kp = base + spacing*k + moved*(k > 1.5)*(t > moves)"]},
                    {"name": "trials", "type": "study", "model": "cruise",
                     "sample": {"design": "inputs", "count": 3, "params": [{"param": "cruise.kp"}],
                                "seeds": "per point",
                                "reduce": {"v_max": "max(car.v)", "v_end": "final(car.v)"}}}],
                "connections": [{"from": "pick.kp", "to": "trials.cruise_kp"}]})
            .to_string()
        };
        super::RUNS_MADE.with(|c| c.set(0));
        let x = run(&model(2.0), &r, Some(2.0), Some(1.0)).unwrap();
        // Three points at the start, then only the one that moved.
        assert_eq!(super::RUNS_MADE.with(std::cell::Cell::get), 4);
        // The same as a study of those points from scratch.
        let mut alone = cruise();
        alone["format"] = json!(1);
        alone["sample"] = json!({"design": "list", "params": [{"param": "cruise.kp"}],
            "values": [[200.0], [500.0], [900.0]], "seeds": "per point",
            "reduce": {"v_max": "max(car.v)", "v_end": "final(car.v)"}});
        let fresh = crate::run::run_sampled(&alone.to_string(), &r, None, Some(1)).unwrap();
        let bits = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        for name in ["v_max", "v_end"] {
            let all = value(&x, &format!("trials.{name}"));
            // The last row: the study's three cells at t = 2.
            let last = &all[all.len() - 3..];
            assert_eq!(bits(last), bits(&value(&fresh, name)), "{name}");
        }
    }

    /// The pattern-search strategy (ADR 0078) as a model: a map wired to a
    /// study whose points come from its inputs finds a bowl's minimum, and
    /// once its step is below the tolerance nothing runs again.
    #[test]
    fn a_pattern_search_model_finds_a_minimum_and_then_holds() {
        let r = registry();
        let model = |t_end: f64| {
            json!({"format": 1, "run": {"dt": 1, "t_end": t_end},
                "models": {"bowl": {"blocks": [{"name": "f", "type": "eq.equations", "equations": [
                    "param a = 0", "param b = 0", "y = (a - 1.3)^2 + 2*(b + 0.4)^2"]}]}},
                "blocks": [
                    {"name": "trials", "type": "study", "model": "bowl",
                     "sample": {"design": "inputs", "count": 5,
                                "params": [{"param": "f.a"}, {"param": "f.b"}]}},
                    {"name": "search", "type": "eq.equations", "equations": [
                        "grid k = 5", "param tol = 1e-4", "input cost[k]",
                        "da = (k > 0.5)*(k < 1.5) - (k > 1.5)*(k < 2.5)",
                        "db = (k > 2.5)*(k < 3.5) - (k > 3.5)",
                        "ta = a + h*da", "tb = b + h*db",
                        "best = argminof(cost)*(h > tol)",
                        "a[n+1] = at(ta, best)", "b[n+1] = at(tb, best)",
                        "h[n+1] = h*(1 - 0.5*(best < 0.5)*(h > tol))"],
                     "params": {"a0": 0, "b0": 0, "h0": 0.5}}],
                "connections": [
                    {"from": "search.ta", "to": "trials.f_a"},
                    {"from": "search.tb", "to": "trials.f_b"},
                    {"from": "trials.f_y", "to": "search.cost"}]})
            .to_string()
        };
        let last = |r: &Results, name: &str| *value(r, name).last().unwrap();
        super::RUNS_MADE.with(|c| c.set(0));
        let x = run(&model(60.0), &r, None, None).unwrap();
        let ran = super::RUNS_MADE.with(std::cell::Cell::get);
        assert!(
            (last(&x, "search.a") - 1.3).abs() < 1e-4,
            "{}",
            last(&x, "search.a")
        );
        assert!(
            (last(&x, "search.b") + 0.4).abs() < 1e-4,
            "{}",
            last(&x, "search.b")
        );
        assert!(last(&x, "search.h") <= 1e-4);
        // Twenty more rounds after it holds still run nothing more.
        super::RUNS_MADE.with(|c| c.set(0));
        let y = run(&model(80.0), &r, None, None).unwrap();
        assert_eq!(super::RUNS_MADE.with(std::cell::Cell::get), ran);
        assert_eq!(
            last(&y, "search.a").to_bits(),
            last(&x, "search.a").to_bits()
        );
    }

    #[test]
    fn a_study_gives_what_the_experiment_gives() {
        let r = registry();
        // The experiment over the model on its own.
        let mut alone: Value = serde_json::from_str(CRUISE).unwrap();
        alone["sample"] = sample();
        let x = crate::run::run_sampled(&alone.to_string(), &r, None, Some(1)).unwrap();
        // A study of it as a definition, in a model with nothing else.
        let file = json!({"format": 1, "models": {"cruise": cruise()},
            "blocks": [{"name": "s", "type": "study", "model": "cruise", "sample": sample()}],
            "run": {"dt": 0.5, "t_end": 1.0}});
        let y = run(&file.to_string(), &r, None, None).unwrap();
        // Nothing else runs in time: one evaluation, a row per run.
        assert_eq!(y.index[0], ("run".to_string(), vec![0.0, 1.0]));
        assert_eq!(value(&y, "s.cruise_kp"), [200.0, 800.0]);
        assert_eq!(value(&y, "s.v_max"), value(&x, "v_max"));
        assert_eq!(value(&y, "s.settle"), value(&x, "settle"));
        // A result made of reductions (ADR 0070 step 3).
        let over: Vec<f64> = value(&y, "s.v_max")
            .iter()
            .zip(value(&y, "s.v_end"))
            .map(|(a, b)| a - b)
            .collect();
        assert_eq!(value(&y, "s.over"), over);
        // In time, the same at every step: run once, kept while the inputs
        // stay.
        let timed = run(&file.to_string(), &r, Some(1.0), Some(0.5)).unwrap();
        let all = value(&timed, "s.settle");
        assert_eq!(all.len(), 3 * 2);
        assert_eq!(all[0..2], all[4..6]);
        // Units: a speed, and seconds.
        let s = crate::describe::check(&file.to_string(), &r);
        assert_eq!(s["ok"], true, "{s}");
        // A study of the file gives the same.
        let examples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let by_file = json!({"format": 1,
            "blocks": [{"name": "s", "type": "study", "file": "cruise-control.ssbx.json", "sample": sample()}],
            "run": {"dt": 0.5, "t_end": 0.5}});
        let z = simsandbox_core::registry::in_folder(Some(&examples), || {
            run(&by_file.to_string(), &r, None, None)
        })
        .unwrap();
        assert_eq!(value(&z, "s.v_max"), value(&x, "v_max"));
        // The file studied is part of the run hash, as a data file is.
        let loaded = simsandbox_file::load_in(&by_file.to_string(), &r, Some(&examples)).unwrap();
        assert!(loaded.data.contains_key("s.file"), "{:?}", loaded.data);
    }

    /// A model whose state decays at rate `k` from `x0`: an input block
    /// `k`, the time to fall below half.
    fn decay() -> Value {
        json!({"blocks": [
            {"name": "k", "type": "input.value", "params": {"y": 1.0}},
            {"name": "x", "type": "eq.equations", "method": {"name": "rk4"},
             "equations": ["input k", "state x = 1", "dx/dt = -k*x"]}],
          "connections": [{"from": "k.y", "to": "x.k"}],
          "run": {"dt": 0.001, "t_end": 3.0}})
    }

    fn halves() -> Value {
        json!({"design": "list", "params": [{"param": "x.x0"}], "values": [[1.0], [2.0]],
               "reduce": {"half": "first_time(x.x < 0.5)"}})
    }

    #[test]
    fn a_formula_reads_a_studys_arrays() {
        // The frequency where the resonance peaks, from a study of it
        // (ADR 0070 step 3): an index axis over the runs, argmaxof and at.
        let r = registry();
        let model = crate::examples::spring();
        let ws = [0.5, 0.8, 0.99, 1.2, 2.0];
        let file = json!({"format": 1, "models": {"resonance": model},
            "blocks": [
                {"name": "s", "type": "study", "model": "resonance",
                 "sample": {"design": "list", "params": [{"param": "spring.w"}],
                            "values": ws.iter().map(|w| vec![*w]).collect::<Vec<_>>()}},
                {"name": "peak", "type": "eq.equations", "equations": [
                    "grid k = 5", "input amp[k] [m], w[k] [rad/s]",
                    "w_peak = at(w, argmaxof(amp)) [rad/s]", "sharp = maxof(amp) > 10*minof(amp)"]}],
            "connections": [{"from": "s.spring_amplitude", "to": "peak.amp"},
                            {"from": "s.spring_w", "to": "peak.w"}]});
        let check = crate::describe::check(&file.to_string(), &r);
        assert_eq!(check["ok"], true, "{check}");
        let y = run(&file.to_string(), &r, Some(1.0), Some(1.0)).unwrap();
        assert_eq!(value(&y, "peak.w_peak"), [0.99, 0.99]);
        assert_eq!(value(&y, "peak.sharp"), [1.0, 1.0]);
        // A wrong number of cells is refused at the wire.
        let mut wrong = file.clone();
        wrong["blocks"][1]["equations"][0] = json!("grid k = 4");
        let e = crate::describe::check(&wrong.to_string(), &r);
        assert_eq!(e["ok"], false, "{e}");
        // A grid design of one parameter: its values are the results'
        // coordinate.
        let grid = json!({"format": 1, "models": {"resonance": file["models"]["resonance"]},
            "blocks": [{"name": "s", "type": "study", "model": "resonance",
                "sample": {"design": "grid", "params": [{"param": "spring.w", "range": [0.5, 1.5], "points": 3}]}}]});
        let loaded = simsandbox_file::load(&grid.to_string(), &r).unwrap();
        let spec = loaded.model.block(loaded.ids["s"]).unwrap().spec().clone();
        let w = spec.ports.iter().find(|p| p.name == "spring_w").unwrap();
        assert!(
            w.coordinate.iter().any(|c| c.port == "spring_amplitude"),
            "{w:?}"
        );
    }

    /// ADR 0080 stage 3: `at_times` and `at_samples` give a vector per
    /// run, over the result's own axis (fastest) and the runs; read by a
    /// typed-equation block as `res[obs, run]`.
    #[test]
    fn vectors_per_run_at_times_and_at_samples() {
        let r = registry();
        let dir = std::env::temp_dir().join(format!("simsandbox-study-vec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // Observations at 0.5, 1 and 2.5 s; 4 s is past the run's end.
        std::fs::write(dir.join("obs.csv"), "t,x\n0.5,0.6\n1,0.4\n2.5,0.1\n4,0\n").unwrap();
        let mut model = decay();
        model["blocks"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": "data", "type": "input.file", "settings": {"file": "obs.csv"}}));
        let reduce = json!({"xs": "at_times(x.x, 0.5, 1, 9)",
                            "res": "at_samples(x.x - data.x, data)",
                            "half": "first_time(x.x < 0.5)"});
        let file = json!({"format": 1, "models": {"decay": model},
            "blocks": [
                {"name": "rate", "type": "input.value", "params": {"y": 1.0}},
                {"name": "pick", "type": "eq.equations", "equations": ["grid k = 2", "x0 = 1 + k"]},
                {"name": "s", "type": "study", "model": "decay",
                 "sample": {"design": "list", "params": [{"param": "x.x0"}], "values": [[1.0], [2.0]],
                            "reduce": reduce}},
                {"name": "s2", "type": "study", "model": "decay",
                 "sample": {"design": "inputs", "count": 2, "params": [{"param": "x.x0"}],
                            "reduce": reduce}},
                {"name": "fit", "type": "eq.equations", "equations": [
                    "grid obs = 3", "grid run = 2", "input res[obs, run]", "sse = sum(res*res, obs)"]}],
            "connections": [{"from": "rate.y", "to": "s.k"}, {"from": "rate.y", "to": "s2.k"},
                            {"from": "pick.x0", "to": "s2.x_x0"}, {"from": "s.res", "to": "fit.res"}]});
        let text = file.to_string();
        let in_dir = |f: &dyn Fn() -> Results| simsandbox_core::registry::in_folder(Some(&dir), f);
        let check =
            simsandbox_core::registry::in_folder(Some(&dir), || crate::describe::check(&text, &r));
        assert_eq!(check["ok"], true, "{check}");
        // Shapes: a vector per run is over [its own axis, runs]; the
        // swept parameter places the runs, along the second axis.
        let loaded = simsandbox_core::registry::in_folder(Some(&dir), || {
            simsandbox_file::load_in(&text, &r, Some(&dir))
        })
        .unwrap();
        let spec = loaded.model.block(loaded.ids["s"]).unwrap().spec().clone();
        let shape = |n: &str| spec.ports.iter().find(|p| p.name == n).unwrap().shape;
        assert_eq!(shape("xs"), Shape::Grid { nx: 3, ny: 2 });
        assert_eq!(shape("res"), Shape::Grid { nx: 3, ny: 2 });
        assert_eq!(shape("half"), Shape::Grid { nx: 2, ny: 1 });
        // The run: rows by run, vectors per row.
        let y = in_dir(&|| run(&text, &r, None, None).unwrap());
        let xs = value(&y, "s.xs");
        let exact = |x0: f64, t: f64| x0 * (-t).exp();
        for (run, x0) in [1.0, 2.0].iter().enumerate() {
            let row = &xs[3 * run..3 * run + 3];
            assert!((row[0] - exact(*x0, 0.5)).abs() < 1e-9, "{row:?}");
            assert!((row[1] - exact(*x0, 1.0)).abs() < 1e-9, "{row:?}");
            assert!(row[2].is_nan(), "past the end: {row:?}");
            // Each sample with the model at the step where the data shows
            // it: 2500 steps of 0.001 s end a hair short of 2.5 s, so the
            // data shows 0.1 from the step after.
            let res = &value(&y, "s.res")[3 * run..3 * run + 3];
            for (k, (t, obs)) in [(0.5, 0.6), (1.0, 0.4), (2.501, 0.1)].iter().enumerate() {
                assert!((res[k] - (exact(*x0, *t) - obs)).abs() < 1e-9, "{res:?}");
            }
        }
        // Points from inputs keep vectors point by point: the same values.
        for n in ["xs", "res", "half"] {
            let (a, b) = (value(&y, &format!("s.{n}")), value(&y, &format!("s2.{n}")));
            assert_eq!(
                a.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                b.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                "{n}"
            );
        }
        // A formula sums each run's squared residuals along obs.
        let sim = simsandbox_core::sim::Simulator::new(loaded.model, 0.0).unwrap();
        let sse = sim.output(loaded.ids["fit"], "sse").unwrap();
        let res = value(&y, "s.res");
        for run in 0..2 {
            let want: f64 = res[3 * run..3 * run + 3].iter().map(|v| v * v).sum();
            assert!((sse[run] - want).abs() < 1e-15, "{sse:?}");
        }
        // Mistakes.
        let bad = |formula: &str| {
            let mut f = file.clone();
            f["blocks"][2]["sample"]["reduce"] = json!({"z": formula});
            let t = f.to_string();
            simsandbox_core::registry::in_folder(Some(&dir), || crate::describe::check(&t, &r))
                .to_string()
        };
        assert!(bad("at_samples(x.x, k)").contains("not recorded data"));
        assert!(bad("at_samples(x.x, nothing)").contains("no block 'nothing'"));
        assert!(bad("at_times(x.x, 1) - at_samples(x.x, data)").contains("different lengths"));
        assert!(bad("at_times(x.x, k.y)").contains("times as numbers"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_study_gives_the_ids_its_points_file_gives() {
        let r = registry();
        let dir = std::env::temp_dir().join(format!("simsandbox-study-ids-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("starts.csv"), "x.x0,id\n1,30\n2,10\n").unwrap();
        let file = json!({"format": 1, "models": {"decay": decay()},
            "blocks": [{"name": "rate", "type": "input.value", "params": {"y": 1.0}},
                {"name": "s", "type": "study", "model": "decay",
                "sample": {"design": "file", "file": "starts.csv", "reduce": {"half": "first_time(x.x < 0.5)"}}}],
            "connections": [{"from": "rate.y", "to": "s.k"}]});
        let y = simsandbox_core::registry::in_folder(Some(&dir), || {
            run(&file.to_string(), &r, None, None)
        })
        .unwrap();
        // The points file is in the run hash of the model holding the
        // study, as `s:sample.file`.
        let hash = |csv: &str| {
            std::fs::write(dir.join("starts.csv"), csv).unwrap();
            let l = simsandbox_file::load_in(&file.to_string(), &r, Some(&dir)).unwrap();
            assert!(l.data.contains_key("s:sample.file"), "{:?}", l.data);
            simsandbox_file::runs::loaded_hash(&l).unwrap()
        };
        let before = hash("x.x0,id\n1,30\n2,10\n");
        assert_ne!(before, hash("x.x0,id\n1,30\n3,10\n"));
        assert_eq!(before, hash("x.x0,id\n1,30\n2,10\n"));
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(value(&y, "s.id"), [30.0, 10.0]);
        assert_eq!(value(&y, "s.x_x0"), [1.0, 2.0]);
    }

    #[test]
    fn a_studys_model_opens_and_edits_like_a_used_one() {
        // ADR 0070: the tuning example's study opens to the cruise model;
        // editing it there changes the definition the study runs.
        use crate::edit_use::{model_at, prepare, read, write, Edited, How};
        let r = registry();
        let text = crate::examples::EXAMPLES
            .iter()
            .find(|e| e.name.starts_with("Tuning"))
            .unwrap()
            .text;
        let (mut top, _) = simsandbox_file::ModelFile::from_json(text).unwrap();
        let path = ["tuning".to_string()];
        let inside = model_at(&top, None, &path).unwrap();
        let names: Vec<&str> = inside.blocks.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["car", "cruise"]);
        // Edit the original: the definition, so the study runs the change.
        let mut edited = Edited::new();
        let target = prepare(&mut top, None, &mut edited, &path, How::Original).unwrap();
        assert_eq!(target.def, ["cruise"]);
        let mut model = read(&top, &edited, &target).unwrap();
        model.blocks[0]
            .params
            .insert("m".into(), simsandbox_file::ParamValue::Number(2000.0));
        write(&mut top, &mut edited, &target, &model).unwrap();
        let heavier = crate::run::run(&top.to_json(), &r, None, None).unwrap();
        let light = crate::run::run(text, &r, None, None).unwrap();
        let peak = |x: &Results| value(x, "tuning.peak");
        assert_ne!(peak(&heavier), peak(&light));
        // A local copy: the study names a new definition of its own.
        let (mut top, _) = simsandbox_file::ModelFile::from_json(text).unwrap();
        let target = prepare(&mut top, None, &mut Edited::new(), &path, How::LocalCopy).unwrap();
        let study = top.blocks.iter().find(|b| b.name == "tuning").unwrap();
        assert_eq!(study.model.as_deref(), Some(target.def[0].as_str()));
        assert_ne!(target.def[0], "cruise");
        assert!(crate::run::run(&top.to_json(), &r, None, None).is_ok());
    }

    #[test]
    fn a_grid_over_two_parameters_gives_2d_results() {
        // ADR 0071: n1 × n2 cells, the first parameter fastest, both
        // parameters placing the cells along their axes.
        let r = registry();
        let model = crate::examples::spring();
        let file = json!({"format": 1, "models": {"resonance": model},
            "blocks": [{"name": "s", "type": "study", "model": "resonance",
                "sample": {"design": "grid", "params": [
                    {"param": "spring.w", "range": [0.5, 1.5], "points": 5},
                    {"param": "spring.c", "range": [0.1, 0.4], "points": 4}]}}]});
        let loaded = simsandbox_file::load(&file.to_string(), &r).unwrap();
        let spec = loaded.model.block(loaded.ids["s"]).unwrap().spec().clone();
        let port = |n: &str| spec.ports.iter().find(|p| p.name == n).unwrap().clone();
        assert_eq!(port("spring_amplitude").shape, Shape::Grid { nx: 5, ny: 4 });
        let w = port("spring_w");
        let c = port("spring_c");
        assert!(w
            .coordinate
            .iter()
            .any(|k| k.port == "spring_amplitude" && k.axis == 0));
        assert!(c
            .coordinate
            .iter()
            .any(|k| k.port == "spring_amplitude" && k.axis == 1));
        let rows = run(&file.to_string(), &r, None, None).unwrap();
        let (w, c, a) = (
            value(&rows, "s.spring_w"),
            value(&rows, "s.spring_c"),
            value(&rows, "s.spring_amplitude"),
        );
        // Cell (i, j): w_i and c_j; the amplitude of that spring.
        for j in 0..4 {
            for i in 0..5 {
                let k = j * 5 + i;
                assert_eq!(w[k], 0.5 + 0.25 * i as f64);
                assert!((c[k] - (0.1 + 0.1 * j as f64)).abs() < 1e-12);
                let (wi, cj) = (w[k], c[k]);
                let expect = 1.0 / ((1.0 - wi * wi).powi(2) + (cj * wi).powi(2)).sqrt();
                assert!((a[k] - expect).abs() < 1e-12, "{k}");
            }
        }
        // Any other design, or replicates: a line of runs.
        let mut listed = file.clone();
        listed["blocks"][0]["sample"] = json!({"design": "sobol", "count": 8,
            "params": [{"param": "spring.w", "range": [0.5, 1.5]}, {"param": "spring.c", "range": [0.1, 0.4]}]});
        let loaded = simsandbox_file::load(&listed.to_string(), &r).unwrap();
        let spec = loaded.model.block(loaded.ids["s"]).unwrap().spec().clone();
        let a = spec
            .ports
            .iter()
            .find(|p| p.name == "spring_amplitude")
            .unwrap();
        assert_eq!(a.shape, Shape::Grid { nx: 8, ny: 1 });
    }

    #[test]
    fn a_study_runs_again_when_its_inputs_change() {
        let r = registry();
        let file = json!({"format": 1, "models": {"decay": decay()},
            "blocks": [
                {"name": "rate", "type": "input.value", "params": {"y": 1.0}},
                {"name": "s", "type": "study", "model": "decay", "sample": halves()}],
            "connections": [{"from": "rate.y", "to": "s.k"}],
            "run": {"dt": 0.25, "t_end": 1.0, "inputs": [{"t": 0.5, "set": "rate.y", "to": 2.0}]}});
        let y = run(&file.to_string(), &r, None, None).unwrap();
        let half = value(&y, "s.half");
        let ln2 = std::f64::consts::LN_2;
        // ln(2 x0)/k, to the inner step.
        for (k, expect) in [(0, ln2), (1, 2.0 * ln2), (8, ln2 / 2.0), (9, ln2)] {
            assert!(
                (half[k] - expect).abs() <= 0.0011,
                "{k}: {} against {expect}",
                half[k]
            );
        }
    }

    #[test]
    fn a_study_of_a_study() {
        let r = registry();
        // `scan`: the halves of decay at rate k, its input.
        let scan = json!({"blocks": [
                {"name": "k", "type": "input.value", "params": {"y": 1.0}},
                {"name": "inner", "type": "study", "model": "decay", "sample": halves()}],
            "connections": [{"from": "k.y", "to": "inner.k"}]});
        // The outer study sweeps scan's rate and keeps the second half.
        let file = json!({"format": 1, "models": {"decay": decay(), "scan": scan},
            "blocks": [{"name": "outer", "type": "study", "model": "scan",
                "sample": {"design": "list", "params": [{"param": "k.y"}], "values": [[1.0], [2.0], [4.0]],
                           "reduce": {"h2": "final(inner.half[1])"}}}],
            "run": {"dt": 1.0, "t_end": 1.0}});
        let y = run(&file.to_string(), &r, None, None).unwrap();
        let h2 = value(&y, "outer.h2");
        let ln4 = 2.0 * std::f64::consts::LN_2;
        for (h, k) in h2.iter().zip([1.0, 2.0, 4.0]) {
            assert!((h - ln4 / k).abs() <= 0.0011, "{h} against {}", ln4 / k);
        }
    }

    #[test]
    fn files_that_study_each_other_are_refused_and_deep_chains_run() {
        let r = registry();
        let dir =
            std::env::temp_dir().join(format!("simsandbox-study-loop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let studying = |file: &str| {
            json!({"format": 1, "blocks": [
                {"name": "k", "type": "input.value"},
                {"name": "s", "type": "study", "file": file,
                 "sample": {"design": "list", "params": [{"param": "k.y"}], "values": [[1.0]]}}]})
            .to_string()
        };
        std::fs::write(dir.join("me.ssbx.json"), studying("me.ssbx.json")).unwrap();
        std::fs::write(dir.join("a.ssbx.json"), studying("b.ssbx.json")).unwrap();
        std::fs::write(dir.join("b.ssbx.json"), studying("a.ssbx.json")).unwrap();
        for (file, chain) in [("me", "me.ssbx.json → "), ("a", "b.ssbx.json → ")] {
            let text = std::fs::read_to_string(dir.join(format!("{file}.ssbx.json"))).unwrap();
            let e = simsandbox_file::load_in(&text, &r, Some(&dir))
                .err()
                .unwrap()
                .to_string();
            assert!(
                e.contains("a study studies itself") && e.contains(chain),
                "{e}"
            );
        }
        std::fs::remove_dir_all(&dir).ok();
        // Ten definitions, each studying the one before: deep, no loop.
        let mut models = serde_json::Map::new();
        models.insert(
            "l0".into(),
            json!({"blocks": [
            {"name": "k", "type": "input.value"},
            {"name": "f", "type": "eq.equations", "equations": ["input k", "y = 2*k"]}],
            "connections": [{"from": "k.y", "to": "f.k"}]}),
        );
        for n in 1..10 {
            let inner = if n == 1 {
                "final(f.y)"
            } else {
                "final(s.r[0])"
            };
            models.insert(format!("l{n}"), json!({"blocks": [
                {"name": "k", "type": "input.value"},
                {"name": "s", "type": "study", "model": format!("l{}", n - 1),
                 "sample": {"design": "list", "params": [{"param": "k.y"}], "values": [[1.0], [2.0]],
                            "reduce": {"r": inner}}}]}));
        }
        let file = json!({"format": 1, "models": models,
            "blocks": [{"name": "top", "type": "study", "model": "l9",
                "sample": {"design": "list", "params": [{"param": "k.y"}], "values": [[1.0]],
                           "reduce": {"r": "final(s.r[0])"}}}],
            "run": {"dt": 1.0, "t_end": 1.0}});
        let y = run(&file.to_string(), &r, None, None).unwrap();
        // l1's first run doubles k = 1; every level above keeps it.
        assert_eq!(value(&y, "top.r"), [2.0]);
    }

    #[test]
    fn studies_refuse_what_they_cannot_run() {
        let r = registry();
        let err = |blocks: Value| -> String {
            let file = json!({"format": 1, "models": {"decay": decay()}, "blocks": blocks});
            let s = crate::describe::check(&file.to_string(), &r);
            assert_eq!(s["ok"], false, "{s}");
            s["errors"][0].to_string()
        };
        let e = err(json!([{"name": "s", "type": "study", "model": "nope", "sample": halves()}]));
        assert!(e.contains("no model named 'nope'"), "{e}");
        let e = err(json!([{"name": "s", "type": "study", "model": "decay"}]));
        assert!(e.contains("needs a `sample`"), "{e}");
        let e = err(json!([{"name": "u", "type": "input.value", "sample": halves()}]));
        assert!(e.contains("only a study block has a sample"), "{e}");
        let mut series = halves();
        series["keep"] = json!("series");
        let e = err(json!([{"name": "s", "type": "study", "model": "decay", "sample": series}]));
        assert!(e.contains("one value per run"), "{e}");
        // A definition that studies itself.
        let file = json!({"format": 1, "models": {"me": {"blocks": [
            {"name": "s", "type": "study", "model": "me", "sample": halves()}]}},
            "blocks": [{"name": "s", "type": "study", "model": "me", "sample": halves()}]});
        let s = crate::describe::check(&file.to_string(), &r);
        assert_eq!(s["ok"], false, "{s}");
    }
}
