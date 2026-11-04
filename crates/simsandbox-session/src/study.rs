//! The study block (ADR 0070): a block whose definition is a model and a
//! `sample`. Each time its inputs change it runs the model at every point
//! (times its replicates) from the model's start, and outputs arrays with
//! one value per run.
//!
//! - **Model:** `settings.model`, a definition in scope (ADR 0068), or
//!   `settings.file`, a model file found from this file's folder and part
//!   of the run hash. The study's `sample` replaces the model's own.
//! - **Inputs:** the model's own input blocks (`input.value`), one scalar
//!   each, set before every point.
//! - **Outputs:** one array per swept parameter, then one per result: the
//!   `reduce` formulas, or each scalar output's final value. Names are the
//!   keys with `.` and `/` written `_` (`car.m` → `car_m`).
//! - It has no state: results depend only on the inputs and are kept until
//!   they change. Only the outermost study runs on several cores.

use crate::reduce::{Reducer, UnitOf};
use crate::run::{loaded_outputs, run_study, Study};
use simsandbox_core::block::{
    AccuracyClass, Block, BlockSpec, Domain, MethodLabels, MethodSpec, PortSpec, Shape, SpeedClass,
    StepContext, StepIo, TimeSemantics,
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
             model is `settings.model` (a definition in `models`) or `settings.file`; its input \
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
    from: Vec<From>,
    runs: usize,
    /// The inputs last run with, by their bits, and the outputs.
    kept: Mutex<Option<(Vec<u64>, Vec<f64>)>>,
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
        if let Some(k) = c
            .settings
            .keys()
            .find(|k| !matches!(k.as_str(), "model" | "file"))
        {
            return Err(SimError::InvalidSetting {
                setting: format!("settings.{k}"),
                reason: "a study has the settings 'model' and 'file'".into(),
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
        // `settings.model`), or a file's.
        let (model, folder, label) = match (defined, c.file("file")) {
            (Some(_), Some(_)) => {
                return Err(SimError::InvalidSetting {
                    setting: "settings.file".into(),
                    reason: "a study names a model or a file, not both".into(),
                })
            }
            (Some(text), None) => (
                text.clone(),
                c.folder.clone(),
                format!(
                    "model '{}'",
                    c.settings.get("model").map_or("", String::as_str)
                ),
            ),
            (None, Some(path)) => {
                let path = path?;
                let bytes = simsandbox_file::compose::read_model(&path).map_err(|e| {
                    SimError::InvalidSetting {
                        setting: "settings.file".into(),
                        reason: format!("{}: {e}", path.display()),
                    }
                })?;
                let text = String::from_utf8(bytes).map_err(|_| SimError::InvalidSetting {
                    setting: "settings.file".into(),
                    reason: format!("{}: not a text file", path.display()),
                })?;
                let label = path.display().to_string();
                (text, path.parent().map(Into::into), label)
            }
            (None, None) => {
                return Err(SimError::InvalidSetting {
                    setting: "settings.model".into(),
                    reason: match c.settings.get("model") {
                        Some(m) => format!("no model named '{m}' is defined here"),
                        None => "a study names its model: `model` (a definition) or `file`".into(),
                    },
                })
            }
        };
        let mut v: serde_json::Value =
            serde_json::from_str(&model).map_err(|e| invalid(format!("its model: {e}")))?;
        let sample: serde_json::Value =
            serde_json::from_str(sample).map_err(|e| invalid(format!("its sample: {e}")))?;
        v.as_object_mut()
            .ok_or_else(|| invalid("its model is not an object"))?
            .insert("sample".into(), sample);
        let text = v.to_string();
        let loaded = load(&text, folder.as_deref(), &label)?;
        let plan = loaded
            .experiment
            .clone()
            .or_else(|| loaded.plan.clone())
            .ok_or_else(|| invalid("its sample gives no points"))?;
        // Nothing in time: each point is one evaluation (ADR 0067).
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
        // Inputs: the model's own input blocks.
        let mut ports = Vec::new();
        let mut keys = Vec::new();
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
        let mut out = |name: String, unit: Unit, f: From| -> Result<()> {
            if ports.iter().any(|p| p.name == name) {
                return Err(invalid(format!(
                    "two of its ports would be named '{name}': rename a result or an input block"
                )));
            }
            // A line of cells, one per run, as array formulas read them
            // (`grid k = runs`, `input a[k]`).
            let shape = Shape::Grid { nx: runs, ny: 1 };
            ports.push(PortSpec::output(&name, unit, shape).depends_on(&inputs));
            from.push(f);
            Ok(())
        };
        for key in &plan.keys {
            out(
                port_name(key),
                key_unit(&loaded, key),
                From::Index(key.clone()),
            )?;
        }
        let fixed = loaded_outputs(&loaded).map_err(invalid)?;
        if plan.reduce.is_empty() {
            for (id, port, full, shape) in &fixed {
                if *shape != Shape::Scalar {
                    continue;
                }
                let unit = loaded
                    .model
                    .block(*id)?
                    .spec()
                    .outputs()
                    .find(|p| &p.name == port)
                    .map_or(Unit::DIMENSIONLESS, |p| p.unit);
                out(port_name(full), unit, From::Output(full.clone()))?;
            }
        } else {
            for (name, formula) in &plan.reduce {
                if !simsandbox_file::subsystems::valid_name(name) {
                    return Err(invalid(format!(
                        "sample.reduce.{name}: a result's name is letters, digits and '_'"
                    )));
                }
                let r = Reducer::parse(name, formula)
                    .map_err(|e| invalid(format!("sample.reduce.{e}")))?;
                let unit = match r.unit_of() {
                    UnitOf::Time => Unit::parse("s").expect("seconds parse"),
                    UnitOf::Signal(s) => signal_unit(&loaded, &fixed, &s),
                    UnitOf::Unknown => Unit::DIMENSIONLESS,
                };
                out(name.clone(), unit, From::Output(name.clone()))?;
            }
        }
        // One parameter on a grid, run once each: its values are the
        // coordinate of every result, as an axis is of a field.
        if plan.keys.len() == 1
            && plan.replicates == 1
            && plan.design == simsandbox_file::Design::Grid
        {
            let first_result = ports.len() - from.len() + 1;
            let results: Vec<String> = ports[first_result..]
                .iter()
                .map(|p| p.name.clone())
                .collect();
            let key = &mut ports[first_result - 1];
            for r in results {
                *key = key.clone().coordinate_of(&r, 0);
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
                params: Vec::new(),
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
            from,
            runs,
            kept: Mutex::new(None),
        })
    }

    /// Every output's values for these inputs, run or kept.
    fn evaluate(&self, inputs: &[f64]) -> Result<Vec<f64>> {
        let bits: Vec<u64> = inputs.iter().map(|v| v.to_bits()).collect();
        let mut kept = self.kept.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((b, values)) = kept.as_ref() {
            if *b == bits {
                return Ok(values.clone());
            }
        }
        let study = Study {
            plan: self.plan.clone(),
            timeless: self.timeless,
            set: self
                .keys
                .iter()
                .cloned()
                .zip(inputs.iter().copied())
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
        let mut values = Vec::with_capacity(self.from.len() * self.runs);
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
        *kept = Some((bits, values.clone()));
        Ok(values)
    }
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
        let values = self.evaluate(io.inputs)?;
        io.outputs.copy_from_slice(&values);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::examples::registry;
    use crate::run::{run, run_experiment, Results};
    use serde_json::{json, Value};

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

    #[test]
    fn a_study_gives_what_the_experiment_gives() {
        let r = registry();
        // The experiment over the model on its own.
        let mut alone: Value = serde_json::from_str(CRUISE).unwrap();
        alone["sample"] = sample();
        let x = run_experiment(&alone.to_string(), &r, None, None, Some(1)).unwrap();
        // A study of it as a definition, in a model with nothing else.
        let file = json!({"format": 1, "models": {"cruise": cruise()},
            "blocks": [{"name": "s", "type": "study", "settings": {"model": "cruise"}, "sample": sample()}],
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
            "blocks": [{"name": "s", "type": "study", "settings": {"file": "cruise-control.ssbx.json"}, "sample": sample()}],
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
    fn a_files_sample_is_the_study_of_its_model() {
        // One meaning of `sample` (ADR 0070): the swept resonance and the
        // study of it as a definition give the same rows, bit for bit.
        let r = registry();
        let text = include_str!("../../../examples/resonance.ssbx.json");
        let swept = run(text, &r, None, None).unwrap();
        let mut model: Value = serde_json::from_str(text).unwrap();
        let sample = model["sample"].clone();
        let o = model.as_object_mut().unwrap();
        for k in ["format", "description", "controls", "sample"] {
            o.remove(k);
        }
        let file = json!({"format": 1, "models": {"resonance": model},
            "blocks": [{"name": "s", "type": "study", "settings": {"model": "resonance"}, "sample": sample}]});
        let rows = run(&file.to_string(), &r, None, None).unwrap();
        let bits = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        let mut compared = 0;
        for o in swept.outputs.iter().filter(|o| o.shape.len() == 1) {
            let study = format!("s.{}", o.name.replace('.', "_"));
            assert_eq!(bits(&value(&rows, &study)), bits(&o.values), "{}", o.name);
            compared += 1;
        }
        assert!(compared >= 2);
        assert_eq!(value(&rows, "s.spring_w"), swept.index[1].1);
    }

    #[test]
    fn a_wrapper_studies_a_model_that_has_its_own_sample() {
        // The case that showed `sample` meant two things (ADR 0070): the
        // used resonance runs as written, the wrapper's sample over it.
        let r = registry();
        let examples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let wrapper = json!({"format": 1, "blocks": [{"name": "r", "file": "resonance.ssbx.json"}],
            "sample": {"design": "list", "params": [{"param": "r/spring.w"}], "values": [[1.0], [2.0]]}});
        let (rows, report) = simsandbox_core::registry::in_folder(Some(&examples), || {
            (
                run(&wrapper.to_string(), &r, None, None).unwrap(),
                crate::describe::check(&wrapper.to_string(), &r),
            )
        });
        assert_eq!(rows.index[1], ("r/spring.w".to_string(), vec![1.0, 2.0]));
        assert!(
            rows.warnings
                .iter()
                .any(|w| w.contains("r: its model's `sample` is not run")),
            "{:?}",
            rows.warnings
        );
        assert_eq!(report["ok"], true, "{report}");
        assert_eq!(report["experiment"]["runs"], 2);
        assert_eq!(report["experiment"]["each_run"], "one evaluation");
    }

    #[test]
    fn a_formula_reads_a_studys_arrays() {
        // The frequency where the resonance peaks, from a study of it
        // (ADR 0070 step 3): an index axis over the runs, argmaxof and at.
        let r = registry();
        let text = include_str!("../../../examples/resonance.ssbx.json");
        let mut model: Value = serde_json::from_str(text).unwrap();
        let o = model.as_object_mut().unwrap();
        for k in ["format", "description", "controls", "sample"] {
            o.remove(k);
        }
        let ws = [0.5, 0.8, 0.99, 1.2, 2.0];
        let file = json!({"format": 1, "models": {"resonance": model},
            "blocks": [
                {"name": "s", "type": "study", "settings": {"model": "resonance"},
                 "sample": {"design": "list", "params": [{"param": "spring.w"}],
                            "values": ws.iter().map(|w| vec![*w]).collect::<Vec<_>>()}},
                {"name": "peak", "type": "eq.equations", "equations": [
                    "grid k = 5", "input amp[k], w[k]",
                    "w_peak = at(w, argmaxof(amp))", "sharp = maxof(amp) > 10*minof(amp)"]}],
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
            "blocks": [{"name": "s", "type": "study", "settings": {"model": "resonance"},
                "sample": {"design": "grid", "params": [{"param": "spring.w", "range": [0.5, 1.5], "points": 3}]}}]});
        let loaded = simsandbox_file::load(&grid.to_string(), &r).unwrap();
        let spec = loaded.model.block(loaded.ids["s"]).unwrap().spec().clone();
        let w = spec.ports.iter().find(|p| p.name == "spring_w").unwrap();
        assert!(
            w.coordinate.iter().any(|c| c.port == "spring_amplitude"),
            "{w:?}"
        );
    }

    #[test]
    fn a_study_runs_again_when_its_inputs_change() {
        let r = registry();
        let file = json!({"format": 1, "models": {"decay": decay()},
            "blocks": [
                {"name": "rate", "type": "input.value", "params": {"y": 1.0}},
                {"name": "s", "type": "study", "settings": {"model": "decay"}, "sample": halves()}],
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
                {"name": "inner", "type": "study", "settings": {"model": "decay"}, "sample": halves()}],
            "connections": [{"from": "k.y", "to": "inner.k"}]});
        // The outer study sweeps scan's rate and keeps the second half.
        let file = json!({"format": 1, "models": {"decay": decay(), "scan": scan},
            "blocks": [{"name": "outer", "type": "study", "settings": {"model": "scan"},
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
                {"name": "s", "type": "study", "settings": {"file": file},
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
                {"name": "s", "type": "study", "settings": {"model": format!("l{}", n - 1)},
                 "sample": {"design": "list", "params": [{"param": "k.y"}], "values": [[1.0], [2.0]],
                            "reduce": {"r": inner}}}]}));
        }
        let file = json!({"format": 1, "models": models,
            "blocks": [{"name": "top", "type": "study", "settings": {"model": "l9"},
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
        let e = err(
            json!([{"name": "s", "type": "study", "settings": {"model": "nope"}, "sample": halves()}]),
        );
        assert!(e.contains("no model named 'nope'"), "{e}");
        let e = err(json!([{"name": "s", "type": "study", "settings": {"model": "decay"}}]));
        assert!(e.contains("needs a `sample`"), "{e}");
        let e = err(json!([{"name": "u", "type": "input.value", "sample": halves()}]));
        assert!(e.contains("only a study block has a sample"), "{e}");
        let mut series = halves();
        series["keep"] = json!("series");
        let e = err(
            json!([{"name": "s", "type": "study", "settings": {"model": "decay"}, "sample": series}]),
        );
        assert!(e.contains("one value per run"), "{e}");
        // A definition that studies itself.
        let file = json!({"format": 1, "models": {"me": {"blocks": [
            {"name": "s", "type": "study", "settings": {"model": "me"}, "sample": halves()}]}},
            "blocks": [{"name": "s", "type": "study", "settings": {"model": "me"}, "sample": halves()}]});
        let s = crate::describe::check(&file.to_string(), &r);
        assert_eq!(s["ok"], false, "{s}");
    }
}
