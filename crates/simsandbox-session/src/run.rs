//! Running a model file headless and collecting every output (ADR 0039),
//! as `simsandbox run` and the Python API do: every communication step of a
//! timed model, or every sample of a swept one (ADR 0028).

use crate::probe::ProbeSpec;
use crate::session::{set_key, InputEvent, SharedHistory, Track};
use crate::Session;
use serde_json::{json, Value};
use simsandbox_core::block::{MethodLabels, Shape};
use simsandbox_core::registry::Registry;
use simsandbox_core::sim::Simulator;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Used when neither the caller nor the model file gives an end time.
pub const DEFAULT_T_END: f64 = 20.0;

/// One fixed-size output, `block.port`, at every row.
#[derive(Clone, Debug)]
pub struct Output {
    pub name: String,
    pub shape: Shape,
    /// Row by row, `shape.len()` values each.
    pub values: Vec<f64>,
}

/// One population output at every row: its points, `dim` values each.
#[derive(Clone, Debug)]
pub struct Population {
    pub name: String,
    pub dim: usize,
    pub frames: Vec<Vec<f64>>,
}

/// Everything a run produced.
#[derive(Clone, Debug)]
pub struct Results {
    /// What indexes the rows: `t`, or `sample` and the swept parameters.
    pub index: Vec<(String, Vec<f64>)>,
    /// In file block order, then port order.
    pub outputs: Vec<Output>,
    pub populations: Vec<Population>,
    pub labels: Option<MethodLabels>,
    /// Algebraic loops and how they are solved (ADR 0040).
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

impl Results {
    pub fn rows(&self) -> usize {
        self.index.first().map_or(0, |(_, v)| v.len())
    }

    /// CSV: the index, then one column per output element
    /// (`block.port[i]` for vectors, grids and point sets). Populations,
    /// whose length varies, are left out.
    pub fn write_csv(&self, out: &mut impl Write) -> io::Result<()> {
        let mut header: Vec<String> = self.index.iter().map(|(n, _)| n.clone()).collect();
        for o in &self.outputs {
            if o.shape == Shape::Scalar {
                header.push(o.name.clone());
            } else {
                header.extend((0..o.shape.len()).map(|i| format!("{}[{i}]", o.name)));
            }
        }
        writeln!(out, "{}", header.join(","))?;
        for r in 0..self.rows() {
            let mut cells: Vec<String> = self.index.iter().map(|(_, v)| v[r].to_string()).collect();
            for o in &self.outputs {
                let n = o.shape.len();
                cells.extend(o.values[r * n..(r + 1) * n].iter().map(f64::to_string));
            }
            writeln!(out, "{}", cells.join(","))?;
        }
        Ok(())
    }

    /// JSON: the index as lists; each output as a list over rows of its
    /// value (a number for a scalar, else a list in the port's element
    /// order); populations as a list over rows of points. JSON has no
    /// infinities or NaN: they become null.
    pub fn to_json(&self) -> Value {
        let list = |v: &[f64]| Value::Array(v.iter().map(|&x| number(x)).collect());
        let index: serde_json::Map<String, Value> = self
            .index
            .iter()
            .map(|(n, v)| (n.clone(), list(v)))
            .collect();
        let outputs: serde_json::Map<String, Value> = self
            .outputs
            .iter()
            .map(|o| {
                let values = if o.shape == Shape::Scalar {
                    list(&o.values)
                } else {
                    Value::Array(o.values.chunks(o.shape.len().max(1)).map(list).collect())
                };
                let shape = crate::describe::shape(&o.shape);
                (o.name.clone(), json!({"shape": shape, "values": values}))
            })
            .collect();
        let populations: serde_json::Map<String, Value> = self
            .populations
            .iter()
            .map(|p| {
                let frames: Vec<Value> = p
                    .frames
                    .iter()
                    .map(|f| Value::Array(f.chunks(p.dim).map(list).collect()))
                    .collect();
                (p.name.clone(), json!({"dim": p.dim, "frames": frames}))
            })
            .collect();
        json!({
            "rows": self.rows(),
            "index": index,
            "outputs": outputs,
            "populations": populations,
            "labels": self.labels.map(crate::describe::labels),
            "notes": self.notes,
            "warnings": self.warnings,
        })
    }
}

fn number(v: f64) -> Value {
    serde_json::Number::from_f64(v).map_or(Value::Null, Value::Number)
}

/// Run a model file: to `t_end` in steps of `dt` (each defaulting to the
/// file's `run` section), or through its sweep if it has no states.
pub fn run(
    text: &str,
    registry: &Registry,
    t_end: Option<f64>,
    dt: Option<f64>,
) -> Result<Results, String> {
    run_with(text, registry, t_end, dt, Vec::new())
}

/// The hash a run file records of a model file (ADR 0059), with the
/// built-in types.
pub fn model_hash(text: &str) -> Result<String, String> {
    let loaded =
        simsandbox_file::load(text, &crate::examples::registry()).map_err(|e| e.to_string())?;
    simsandbox_file::runs::loaded_hash(&loaded).map_err(|e| e.to_string())
}

/// The run file's JSON Schema.
pub const RUN_SCHEMA: &str = simsandbox_file::runs::RUN_SCHEMA;

/// [`run`] with recorded input (a run file's track, ADR 0059) applied
/// along with the file's schedule (`run.inputs`, ADR 0058), each at the
/// first step boundary at or after its time.
pub fn run_with(
    text: &str,
    registry: &Registry,
    t_end: Option<f64>,
    dt: Option<f64>,
    track: Vec<InputEvent>,
) -> Result<Results, String> {
    for v in [t_end, dt].into_iter().flatten() {
        if !(v.is_finite() && v > 0.0) {
            return Err("the end time and step must be positive".into());
        }
    }
    let loaded = simsandbox_file::load(text, registry).map_err(|e| e.to_string())?;
    if loaded.plan.is_some() && (dt.is_some() || t_end.is_some()) {
        return Err("the end time and step do not apply to a swept model".into());
    }
    let dt = dt.unwrap_or(loaded.file.run.dt);
    let t_end = t_end.or(loaded.file.run.t_end).unwrap_or(DEFAULT_T_END);
    let schedule: Vec<InputEvent> = loaded
        .file
        .run
        .inputs
        .iter()
        .map(|c| InputEvent {
            t: c.t,
            key: c.set.clone(),
            value: c.to,
        })
        .collect();
    if loaded.plan.is_some() && !(schedule.is_empty() && track.is_empty()) {
        return Err("a swept model is not run in time, so it takes no inputs".into());
    }
    let mut events = Track::new(schedule, track);
    let mut plan = loaded.params.clone();

    let mut fixed = Vec::new();
    let mut variable = Vec::new();
    for entry in &loaded.flat.blocks {
        let id = loaded.ids[&entry.name];
        let spec = loaded.model.block(id).map_err(|e| e.to_string())?.spec();
        for port in spec.outputs() {
            let name = format!("{}.{}", entry.name, port.name);
            if let Shape::Population { dim } = port.shape {
                variable.push((id, port.name.clone(), name, dim));
            } else {
                fixed.push((id, port.name.clone(), name, port.shape));
            }
        }
    }
    let mut sim = Simulator::new(loaded.model, 0.0).map_err(|e| e.to_string())?;
    let mut r = Results {
        index: Vec::new(),
        outputs: fixed
            .iter()
            .map(|(_, _, name, shape)| Output {
                name: name.clone(),
                shape: *shape,
                values: Vec::new(),
            })
            .collect(),
        populations: variable
            .iter()
            .map(|(_, _, name, dim)| Population {
                name: name.clone(),
                dim: *dim,
                frames: Vec::new(),
            })
            .collect(),
        labels: sim.labels(),
        notes: crate::session::loop_notes(&sim),
        warnings: loaded.warnings.clone(),
    };
    let record = |r: &mut Results, sim: &Simulator| {
        for (o, (id, port, _, _)) in r.outputs.iter_mut().zip(&fixed) {
            o.values
                .extend_from_slice(sim.output(*id, port).unwrap_or(&[]));
        }
        for (p, (id, port, _, _)) in r.populations.iter_mut().zip(&variable) {
            p.frames.push(sim.output(*id, port).unwrap_or(&[]).to_vec());
        }
    };
    if let Some(plan) = &loaded.plan {
        let targets: Vec<_> = plan
            .keys
            .iter()
            .map(|k| {
                let (block, param) = k.split_once('.').expect("plan keys are block.param");
                (loaded.ids[block], param)
            })
            .collect();
        let mut index = vec![("sample".to_string(), Vec::new())];
        index.extend(plan.keys.iter().map(|k| (k.clone(), Vec::new())));
        for (k, point) in plan.points.iter().enumerate() {
            for (&(id, param), &v) in targets.iter().zip(point) {
                sim.set_param(id, param, v).map_err(|e| e.to_string())?;
            }
            sim.restart(0.0).map_err(|e| e.to_string())?;
            index[0].1.push(k as f64);
            for (column, &v) in index[1..].iter_mut().zip(point) {
                column.1.push(v);
            }
            record(&mut r, &sim);
        }
        r.index = index;
    } else {
        let ids = &loaded.ids;
        let mut apply = |sim: &mut Simulator| -> Result<bool, String> {
            let now = sim.time();
            let mut any = false;
            while let Some(e) = events.take_due(now, 1e-9 * dt.max(now.abs())) {
                set_key(sim, &mut plan, ids, &e.key, e.value)
                    .map_err(|err| format!("input at t = {}: {err}", e.t))?;
                any = true;
            }
            Ok(any)
        };
        // Input at t = 0 sets where the run starts.
        if apply(&mut sim)? {
            sim.restart(0.0).map_err(|e| e.to_string())?;
        }
        let mut t = vec![sim.time()];
        record(&mut r, &sim);
        for _ in 0..(t_end / dt).round() as usize {
            apply(&mut sim)?;
            sim.step(dt).map_err(|e| e.to_string())?;
            t.push(sim.time());
            record(&mut r, &sim);
        }
        r.index = vec![("t".to_string(), t)];
    }
    Ok(r)
}

/// What a run's probes recorded (ADR 0062).
pub struct ProbeRun {
    /// Each probe as set, with what it recorded, in file order.
    pub probes: Vec<(ProbeSpec, SharedHistory)>,
    pub labels: Option<MethodLabels>,
    /// The model's warnings and notes on its algebraic loops.
    pub warnings: Vec<String>,
}

impl ProbeRun {
    /// Write the files of the probes that have one (`to`) into `dir`:
    /// the files written, and notes on what was left out.
    pub fn write_files(&self, dir: &Path) -> io::Result<(Vec<PathBuf>, Vec<String>)> {
        let mut written = Vec::new();
        let mut notes = Vec::new();
        for (spec, h) in &self.probes {
            if let Some(to) = &spec.to {
                let (w, n) = crate::probe::write_files(&h.read(), dir, to)?;
                written.extend(w);
                notes.extend(n.into_iter().map(|n| format!("probe {}: {n}", spec.name)));
            }
        }
        Ok((written, notes))
    }
}

/// Run a model file with its probes, as the app does: to `t_end` in
/// steps of `dt` (each defaulting to the file's `run` section), with
/// `track` replayed as recorded input. `names` picks probes (all if
/// empty). A swept model is not run in time and records no probes.
pub fn run_probes(
    text: &str,
    registry: &Registry,
    t_end: Option<f64>,
    dt: Option<f64>,
    track: Vec<InputEvent>,
    names: &[String],
) -> Result<ProbeRun, String> {
    for v in [t_end, dt].into_iter().flatten() {
        if !(v.is_finite() && v > 0.0) {
            return Err("the end time and step must be positive".into());
        }
    }
    let mut s = Session::load("probes", text, registry)?;
    if s.sweep().is_some() {
        return Err("a swept model is not run in time, so it records no probes".into());
    }
    if let Some(unknown) = names
        .iter()
        .find(|n| !s.probe_names().contains(&n.as_str()))
    {
        return Err(format!(
            "no probe '{unknown}'; the model has: {}",
            s.probe_names().join(", ")
        ));
    }
    if let Some(dt) = dt {
        s.dt = dt;
    }
    let t_end = t_end.or(s.file.run.t_end).unwrap_or(DEFAULT_T_END);
    let steps = (t_end / s.dt).round() as usize;
    if !track.is_empty() {
        s.replay(track)?;
    }
    for _ in 0..steps {
        s.step().map_err(|e| e.to_string())?;
    }
    s.flush(true);
    let probes = s
        .probe_specs()
        .into_iter()
        .filter(|p| names.is_empty() || names.contains(&p.name))
        .map(|p| {
            (
                p.clone(),
                s.probe(&p.name).expect("a probe the session has"),
            )
        })
        .collect();
    Ok(ProbeRun {
        probes,
        labels: s.labels(),
        warnings: s.warnings.clone(),
    })
}

/// Run a model file to `t` and make that state its start (ADR 0042):
/// the model file that starts there, and a note per block. See
/// [`crate::Session::start_from_now`].
pub fn start_from(
    text: &str,
    registry: &Registry,
    t: f64,
) -> Result<(String, Vec<String>), String> {
    if !(t.is_finite() && t >= 0.0) {
        return Err("the time must be zero or positive".into());
    }
    let mut s = crate::Session::load("model", text, registry)?;
    s.run_until(t).map_err(|e| e.to_string())?;
    let notes = s.start_from_now()?;
    let speed = s.file.controls.speed.unwrap_or(1.0);
    let mut text = s.save_json(speed, None)?;
    if s.file.controls.speed.is_none() {
        // Keep the file as it was where nothing changed.
        let (mut f, _) = simsandbox_file::ModelFile::from_json(&text).map_err(|e| e.to_string())?;
        f.controls.speed = None;
        text = f.to_json();
    }
    Ok((text, notes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::examples::{registry, EXAMPLES};

    fn example(part: &str) -> &'static str {
        EXAMPLES
            .iter()
            .find(|e| e.text.contains(part))
            .unwrap()
            .text
    }

    #[test]
    fn timed_runs_record_every_step_and_populations() {
        let r = run(example("agents.grazers"), &registry(), Some(1.0), None).unwrap();
        let rows = r.rows();
        assert_eq!(rows, 11);
        let count = r.outputs.iter().find(|o| o.name == "sheep.count").unwrap();
        let positions = &r
            .populations
            .iter()
            .find(|p| p.name == "sheep.position")
            .unwrap();
        assert_eq!(positions.dim, 2);
        for k in 0..rows {
            assert_eq!(positions.frames[k].len() as f64, 2.0 * count.values[k]);
        }
        let grass = r.outputs.iter().find(|o| o.name == "grass.grass").unwrap();
        assert_eq!(grass.values.len(), rows * 1600);
        let mut csv = Vec::new();
        r.write_csv(&mut csv).unwrap();
        let csv = String::from_utf8(csv).unwrap();
        assert_eq!(csv.lines().count(), rows + 1);
        assert!(csv.starts_with("t,grass.grass[0],"));
    }

    #[test]
    fn start_from_gives_a_file_that_starts_there() {
        let (text, notes) = start_from(example("cruise"), &registry(), 30.0).unwrap();
        assert!(notes.iter().any(|n| n.contains("v0 = ")), "{notes:?}");
        let r = run(&text, &registry(), Some(0.1), None).unwrap();
        let v = r.outputs.iter().find(|o| o.name == "car.v").unwrap();
        let before = run(example("cruise"), &registry(), Some(30.0), None).unwrap();
        let v30 = before.outputs.iter().find(|o| o.name == "car.v").unwrap();
        assert_eq!(v.values[0], *v30.values.last().unwrap());
        assert!(start_from(example("cruise"), &registry(), -1.0).is_err());
    }

    #[test]
    fn swept_runs_are_indexed_by_sample_and_parameters() {
        let r = run(example("\"points\": 600"), &registry(), None, None).unwrap();
        assert_eq!(r.index[0].0, "sample");
        assert_eq!(r.index[1].0, "spring.w");
        assert_eq!(r.rows(), 600);
        let j = r.to_json();
        assert_eq!(j["rows"], 600);
        assert!(j["outputs"]["spring.amplitude"]["values"][0].is_number());
        assert!(run(example("\"points\": 600"), &registry(), Some(1.0), None).is_err());
        assert!(run(example("lorenz"), &registry(), Some(-1.0), None).is_err());
    }
}
