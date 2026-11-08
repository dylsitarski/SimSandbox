//! Running a model file headless and collecting every output (ADR 0039),
//! as `simsandbox run` and the Python API do: every communication step of a
//! model, or a row per run of a model whose only runs are its studies
//! (ADR 0070, ADR 0071).

use crate::probe::ProbeSpec;
use crate::session::{set_key, InputEvent, SharedHistory, Track};
use crate::Session;
use serde_json::{json, Value};
use simsandbox_core::block::{MethodLabels, Shape};
use simsandbox_core::registry::Registry;
use simsandbox_core::sim::Simulator;
/// The SimSandbox that makes a run, as its output and run files record it.
pub use simsandbox_file::runs::MADE_WITH;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Used when neither the caller nor the model file gives an end time.
pub const DEFAULT_T_END: f64 = 20.0;

/// What a headless run keeps (ADR 0076): by default every output at
/// every step. Keeping less saves memory and time; what is kept is the
/// same values, bit for bit.
#[derive(Clone, Debug, Default)]
pub struct Keep {
    /// The outputs to keep, each named as `block.port`, as `block` (all
    /// its ports), or, in a model using models, as a model's name (all
    /// its outputs: `plant`, or `.` for the top level's own blocks) or a
    /// path inside one (`plant/car`, `plant/car.v`). `None`: all.
    pub signals: Option<Vec<String>>,
    /// Keep every `every`-th step from the first, and always the last
    /// (0 or 1: every step).
    pub every: usize,
}

impl Keep {
    /// Whether output `name` of model `model` (`.` for a single model's or
    /// the top level's own) is kept.
    pub fn wants(&self, model: &str, name: &str) -> bool {
        let Some(signals) = &self.signals else {
            return true;
        };
        signals.iter().any(|s| {
            s == model
                || name == s
                || name
                    .strip_prefix(s.as_str())
                    .is_some_and(|rest| rest.starts_with('.') || rest.starts_with('/'))
        })
    }

    /// An error naming each signal that keeps nothing of `outputs`, as
    /// (model, name).
    pub fn check(&self, outputs: &[(String, String)]) -> Result<(), String> {
        let Some(signals) = &self.signals else {
            return Ok(());
        };
        let one = |s: &String| Keep {
            signals: Some(vec![s.clone()]),
            every: 1,
        };
        let missing: Vec<&str> = signals
            .iter()
            .filter(|s| !outputs.iter().any(|(m, n)| one(s).wants(m, n)))
            .map(String::as_str)
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        let mut names: Vec<&str> = outputs.iter().map(|(_, n)| n.as_str()).collect();
        let more = names.len().saturating_sub(12);
        names.truncate(12);
        Err(format!(
            "no output named {}; outputs: {}{}",
            missing.join(", "),
            names.join(", "),
            if more > 0 {
                format!(" and {more} more")
            } else {
                String::new()
            }
        ))
    }

    /// The steps to keep, counted as they come.
    pub(crate) fn thin(&self) -> Thin {
        Thin {
            every: self.every.max(1),
            seen: 0,
            behind: false,
        }
    }
}

/// Which steps a run keeps: every `every`-th, and the last (`behind`
/// says the latest was left out, so it is kept at the end).
#[derive(Clone, Debug)]
pub(crate) struct Thin {
    every: usize,
    seen: usize,
    pub behind: bool,
}

impl Thin {
    /// Whether to keep the step just reached.
    pub fn keep(&mut self) -> bool {
        let keep = self.seen.is_multiple_of(self.every);
        self.seen += 1;
        self.behind = !keep;
        keep
    }

    /// Forget the latest step; whether it was kept.
    pub fn undo(&mut self) -> bool {
        self.seen -= 1;
        self.behind = false;
        self.seen.is_multiple_of(self.every)
    }
}

/// One fixed-size output, `block.port`, at every row.
#[derive(Clone, Debug)]
pub struct Output {
    pub name: String,
    pub shape: Shape,
    /// Its axes' names, as its block names them (ADR 0081); may be empty.
    pub axes: Vec<String>,
    /// Row by row, `shape.len()` values each.
    pub values: Vec<f64>,
}

/// The names of a port's axes (ADR 0081), as its block gives them.
pub(crate) fn port_axes(
    model: &simsandbox_core::graph::Model,
    id: simsandbox_core::graph::BlockId,
    port: &str,
) -> Vec<String> {
    model
        .block(id)
        .ok()
        .and_then(|b| b.spec().ports.iter().find(|p| p.name == port))
        .map_or_else(Vec::new, |p| p.axis_names.clone())
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
    /// What indexes the rows: `t`, or `run`; a study's own runs by
    /// `sample` and the swept parameters.
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
            match o.shape {
                Shape::Scalar => header.push(o.name.clone()),
                // Over three or more axes, a cell by its places, the first
                // fastest (ADR 0081): `T[3,2,0]`.
                Shape::Tensor { rank, dims } => {
                    let mut at = vec![0; rank];
                    for _ in 0..o.shape.len() {
                        let places: Vec<String> = at.iter().map(|p| p.to_string()).collect();
                        header.push(format!("{}[{}]", o.name, places.join(",")));
                        for (k, p) in at.iter_mut().enumerate() {
                            *p += 1;
                            if *p < dims[k] {
                                break;
                            }
                            *p = 0;
                        }
                    }
                }
                _ => header.extend((0..o.shape.len()).map(|i| format!("{}[{i}]", o.name))),
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
                let shape = crate::describe::shape_with_axes(&o.shape, &o.axes);
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
            "made_with": simsandbox_file::runs::MADE_WITH,
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
    let loaded = simsandbox_file::load_any(text, &crate::examples::registry(), None)
        .map_err(|e| e.to_string())?;
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
    run_keeping(text, registry, t_end, dt, track, &Keep::default())
}

/// [`run_with`], keeping only what `keep` asks for (ADR 0076).
pub fn run_keeping(
    text: &str,
    registry: &Registry,
    t_end: Option<f64>,
    dt: Option<f64>,
    track: Vec<InputEvent>,
    keep: &Keep,
) -> Result<Results, String> {
    for v in [t_end, dt].into_iter().flatten() {
        if !(v.is_finite() && v > 0.0) {
            return Err("the end time and step must be positive".into());
        }
    }
    let loaded = simsandbox_file::load_any(text, registry, None).map_err(|e| e.to_string())?;
    // Nothing in time but studies: one evaluation, a row per run.
    let untimed = dt.is_none()
        && t_end.is_none()
        && track.is_empty()
        && loaded.file.run.inputs.is_empty()
        && loaded.model.clocks().0.is_empty();
    if untimed && only_studies_in_time(&loaded) {
        // A row per run: kept whole, only the outputs chosen.
        let mut r = study_rows(loaded)?;
        let all: Vec<(String, String)> = r
            .outputs
            .iter()
            .map(|o| (".".to_string(), o.name.clone()))
            .chain(
                r.populations
                    .iter()
                    .map(|p| (".".to_string(), p.name.clone())),
            )
            .collect();
        keep.check(&all)?;
        r.outputs.retain(|o| keep.wants(".", &o.name));
        r.populations.retain(|p| keep.wants(".", &p.name));
        return Ok(r);
    }
    let dt = dt.unwrap_or(loaded.file.run.dt);
    let t_end = end_time(&loaded.model, t_end.or(loaded.file.run.t_end));
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
    let mut events = Track::new(schedule, track);
    let mut plan = loaded.params.clone();

    let mut fixed = Vec::new();
    let mut variable = Vec::new();
    let mut all = Vec::new();
    for entry in &loaded.flat.blocks {
        let id = loaded.ids[&entry.name];
        let spec = loaded.model.block(id).map_err(|e| e.to_string())?.spec();
        for port in spec.outputs() {
            let name = format!("{}.{}", entry.name, port.name);
            all.push((".".to_string(), name.clone()));
            if !keep.wants(".", &name) {
                continue;
            }
            match port.shape {
                Shape::Population { dim } => variable.push((id, port.name.clone(), name, dim)),
                // Rows by id come with ADR 0079 stage 1c.
                Shape::Rows { .. } => {}
                shape => fixed.push((id, port.name.clone(), name, shape)),
            }
        }
    }
    keep.check(&all)?;
    // A model using models steps from instant to instant of its clocks
    // (ADR 0066 step 5), not by `dt`.
    let clocked = !loaded.model.clocks().0.is_empty();
    let mut sim = Simulator::new(loaded.model, 0.0).map_err(|e| e.to_string())?;
    // The rows recorded, when known: room for them from the start, not
    // grown (and copied) as they come.
    let steps = (t_end / dt).round() as usize;
    let rows = if clocked {
        0
    } else {
        steps / keep.every.max(1) + 2
    };
    let mut r = Results {
        index: Vec::new(),
        outputs: fixed
            .iter()
            .map(|(id, port, name, shape)| Output {
                name: name.clone(),
                shape: *shape,
                axes: port_axes(sim.model(), *id, port),
                values: Vec::with_capacity(rows * shape.len()),
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
    let mut t = Vec::with_capacity(rows);
    let mut thin = keep.thin();
    thin.keep();
    t.push(sim.time());
    record(&mut r, &sim);
    advance(&mut sim, (dt, t_end, clocked), &mut apply, |sim| {
        if thin.keep() {
            t.push(sim.time());
            record(&mut r, sim);
        }
    })?;
    // The last step, kept whatever `every` says.
    if thin.behind {
        t.push(sim.time());
        record(&mut r, &sim);
    }
    r.index = vec![("t".to_string(), t)];
    Ok(r)
}

/// Where a run that only recorded data drives ends (ADR 0067 step 4): at
/// its last listed instant, when no clock steps every `dt`.
pub fn data_end(model: &simsandbox_core::graph::Model) -> Option<f64> {
    let clocks = model.clocks().0;
    let data_only = !clocks.is_empty() && clocks.iter().all(|c| c.on_data || c.every == 0);
    data_only.then(|| model.last_listed()).flatten()
}

/// A run's end: as given, else where its data ends, else the default.
pub(crate) fn end_time(model: &simsandbox_core::graph::Model, given: Option<f64>) -> f64 {
    given.or_else(|| data_end(model)).unwrap_or(DEFAULT_T_END)
}

/// Whether a model has studies and nothing else that runs in time (no
/// states or tables anywhere): a separate experiment file, say.
fn only_studies_in_time(loaded: &simsandbox_file::Loaded) -> bool {
    let mut studies = 0;
    for b in &loaded.flat.blocks {
        let Some(block) = loaded
            .ids
            .get(&b.name)
            .and_then(|id| loaded.model.block(*id).ok())
        else {
            return false;
        };
        if !(block.spec().states.is_empty() && block.spec().tables.is_empty()) {
            return false;
        }
        studies += usize::from(b.type_name == crate::study::STUDY);
    }
    studies > 0
}

/// Evaluate a model whose only runs are its studies' (ADR 0070 step 2)
/// once, and lay out their arrays as rows: indexed by `run`, a column per
/// study output (`kp.v_max`). Every study must make the same number of
/// runs. Other outputs, evaluated once, are notes.
fn study_rows(loaded: simsandbox_file::Loaded) -> Result<Results, String> {
    // Each study output: block, port, name, runs and values per run.
    let mut columns = Vec::new();
    let mut others = Vec::new();
    for entry in &loaded.flat.blocks {
        let id = loaded.ids[&entry.name];
        let spec = loaded.model.block(id).map_err(|e| e.to_string())?.spec();
        let study = entry.type_name == crate::study::STUDY;
        for port in spec.outputs() {
            let name = format!("{}.{}", entry.name, port.name);
            if study {
                // A vector per run has its own axis first, named for its
                // reduction (ADR 0080, ADR 0081); the rest are the runs.
                let vector = matches!(
                    port.axis_names.first().map(String::as_str),
                    Some("sample" | "time")
                );
                let dims = port.shape.dims().unwrap_or_default();
                let (runs, width) = match dims.split_first() {
                    Some((&w, rest)) if vector => (rest.iter().product(), w),
                    _ => (port.shape.len(), 1),
                };
                columns.push((id, port.name.clone(), name, runs, width));
            } else if !port.shape.is_variable() {
                others.push((id, port.name.clone(), name));
            }
        }
    }
    let runs = columns[0].3;
    if let Some((_, _, name, n, _)) = columns.iter().find(|c| c.3 != runs) {
        return Err(format!(
            "{name} has {n} runs and {} has {runs}: studies written as rows make as many runs \
             each; run them from separate files",
            columns[0].2
        ));
    }
    let warnings = loaded.warnings.clone();
    let sim = Simulator::new(loaded.model, 0.0).map_err(|e| e.to_string())?;
    let mut notes = crate::session::loop_notes(&sim);
    for (id, port, name) in &others {
        let v = sim.output(*id, port).unwrap_or(&[]);
        let v = match v {
            [one] => one.to_string(),
            many => format!("{many:?}"),
        };
        notes.push(format!("{name} = {v} (evaluated once)"));
    }
    Ok(Results {
        index: vec![("run".to_string(), (0..runs).map(|k| k as f64).collect())],
        outputs: columns
            .iter()
            .map(|(id, port, name, _, width)| Output {
                name: name.clone(),
                // A vector per run (ADR 0080): its values, run by run.
                shape: if *width > 1 {
                    Shape::Vector(*width)
                } else {
                    Shape::Scalar
                },
                axes: if *width > 1 {
                    port_axes(sim.model(), *id, port)
                        .into_iter()
                        .take(1)
                        .collect()
                } else {
                    Vec::new()
                },
                values: sim.output(*id, port).unwrap_or(&[]).to_vec(),
            })
            .collect(),
        populations: Vec::new(),
        labels: sim.labels(),
        notes,
        warnings,
    })
}

/// Step `sim` to `t_end`: by `dt`, or from instant to instant of its
/// clocks (`clocked`, the last at or before the end), applying the inputs
/// due before each step and calling `each` after it.
fn advance(
    sim: &mut Simulator,
    (dt, t_end, clocked): (f64, f64, bool),
    apply: &mut dyn FnMut(&mut Simulator) -> Result<bool, String>,
    mut each: impl FnMut(&Simulator),
) -> Result<(), String> {
    if clocked {
        while sim
            .next_instant()
            .is_some_and(|next| next <= t_end * (1.0 + 1e-12))
        {
            apply(sim)?;
            sim.step(dt).map_err(|e| e.to_string())?;
            each(sim);
        }
    } else {
        for _ in 0..(t_end / dt).round() as usize {
            apply(sim)?;
            sim.step(dt).map_err(|e| e.to_string())?;
            each(sim);
        }
    }
    Ok(())
}

/// For tests of the study runner: run `text`'s top-level `sample` (which
/// files no longer have, ADR 0071) as a study of the rest, each point a
/// whole run to `t_end`.
#[cfg(test)]
pub(crate) fn run_sampled(
    text: &str,
    registry: &Registry,
    t_end: Option<f64>,
    threads: Option<usize>,
) -> Result<Results, String> {
    let mut v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let sample = v
        .as_object_mut()
        .and_then(|o| o.remove("sample"))
        .ok_or("no sample")?;
    let sampling: simsandbox_file::Sampling =
        serde_json::from_value(sample).map_err(|e| e.to_string())?;
    let text = v.to_string();
    let loaded = simsandbox_file::load_any(&text, registry, None).map_err(|e| e.to_string())?;
    let plan = loaded.plan_of(&sampling).map_err(|e| e.to_string())?;
    let study = Study {
        plan,
        timeless: false,
        set: Vec::new(),
        t_end,
        dt: None,
    };
    run_study(&text, registry, &loaded, &study, threads)
}

/// How a study runs its model (ADR 0070): the plan, whether the model has
/// nothing in time (each point one evaluation), values set before every
/// point (its input blocks, `block.param`), and the end and step.
pub(crate) struct Study {
    pub plan: simsandbox_file::Plan,
    pub timeless: bool,
    pub set: Vec<(String, f64)>,
    pub t_end: Option<f64>,
    pub dt: Option<f64>,
}

thread_local! {
    /// Set on the threads running a study's points: a study inside runs
    /// on its caller's thread (ADR 0070).
    static IN_STUDY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether this thread runs a study's points.
pub(crate) fn in_study() -> bool {
    IN_STUDY.with(std::cell::Cell::get)
}

/// How far the outermost studies run on a thread have got, and a request
/// to give them up (ADR 0071 step 2): the app's worker watches its steps'
/// studies with one, and the window asks it to give up when something they
/// read changes.
#[derive(Debug, Default)]
pub struct Progress {
    done: std::sync::atomic::AtomicUsize,
    total: std::sync::atomic::AtomicUsize,
    give_up: std::sync::atomic::AtomicBool,
}

impl Progress {
    /// Runs done and runs in all, since the last [`Progress::reset`];
    /// `None` before any study started.
    pub fn runs(&self) -> Option<(usize, usize)> {
        use std::sync::atomic::Ordering::Relaxed;
        let total = self.total.load(Relaxed);
        (total > 0).then(|| (self.done.load(Relaxed).min(total), total))
    }

    /// Ask the studies running to stop between points: each returns an
    /// error that [`given_up`] recognises.
    pub fn give_up(&self) {
        self.give_up
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Start counting afresh, nothing given up.
    pub fn reset(&self) {
        use std::sync::atomic::Ordering::Relaxed;
        self.done.store(0, Relaxed);
        self.total.store(0, Relaxed);
        self.give_up.store(false, Relaxed);
    }

    fn given_up(&self) -> bool {
        self.give_up.load(std::sync::atomic::Ordering::Relaxed)
    }
}

thread_local! {
    /// What watches the studies this thread runs (ADR 0071 step 2).
    static PROGRESS: std::cell::RefCell<Option<std::sync::Arc<Progress>>> =
        const { std::cell::RefCell::new(None) };
}

/// Watch the studies this thread runs from now on with `progress` (or stop
/// watching with `None`).
pub fn watch_studies(progress: Option<std::sync::Arc<Progress>>) {
    PROGRESS.with(|p| *p.borrow_mut() = progress);
}

/// What a study given up says: its step failed only to start again from
/// newer values.
pub const GIVEN_UP: &str = "study given up for a newer change";

/// Whether `error` is a study given up (ADR 0071 step 2).
pub fn given_up(error: &impl std::fmt::Display) -> bool {
    error.to_string().contains(GIVEN_UP)
}

/// Run `study` over `text` (loaded as `loaded`) on `threads` threads (all
/// the machine's if `None`; one inside another study).
pub(crate) fn run_study(
    text: &str,
    registry: &Registry,
    loaded: &simsandbox_file::Loaded,
    study: &Study,
    threads: Option<usize>,
) -> Result<Results, String> {
    let plan = &study.plan;
    let rows = plan.points.len() * plan.replicates;
    let threads = if in_study() {
        1
    } else {
        threads.unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()))
    }
    .clamp(1, rows.max(1));
    let per = rows.div_ceil(threads);
    let folder = simsandbox_core::registry::current_folder();
    // Only the outermost study counts its runs and can be given up.
    let progress = if in_study() {
        None
    } else {
        PROGRESS.with(|p| p.borrow().clone())
    };
    if let Some(p) = &progress {
        if p.given_up() {
            return Err(GIVEN_UP.into());
        }
        p.total
            .fetch_add(rows, std::sync::atomic::Ordering::Relaxed);
    }
    let progress = progress.as_deref();
    let chunks: Vec<Result<Vec<Row>, String>> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|k| {
                let folder = folder.clone();
                let range = k * per..((k + 1) * per).min(rows);
                s.spawn(move || {
                    let was = IN_STUDY.with(|c| c.replace(true));
                    let r = simsandbox_core::registry::in_folder(folder.as_deref(), || {
                        run_rows(text, registry, study, range, progress)
                    });
                    IN_STUDY.with(|c| c.set(was));
                    r
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_else(|_| Err("a run panicked".into())))
            .collect()
    });
    let mut all: Vec<Row> = Vec::with_capacity(rows);
    for c in chunks {
        all.extend(c?);
    }
    let first = loaded_outputs(loaded)?;
    let t_end = end_time(&loaded.model, study.t_end.or(loaded.file.run.t_end));
    let reduced = reducers(plan, loaded, &first, t_end)?;
    let mut index = vec![("sample".to_string(), Vec::new())];
    if plan.replicates > 1 {
        index.push(("replicate".to_string(), Vec::new()));
    }
    // Ids given by a points file (ADR 0070 step 4).
    let ids = plan.ids.is_some();
    if ids {
        index.push(("id".to_string(), Vec::new()));
    }
    index.extend(plan.keys.iter().map(|k| (k.clone(), Vec::new())));
    // The rows hold the reductions if there are any, else final values.
    let mut outputs: Vec<Output> = if reduced.is_empty() {
        first
            .iter()
            .map(|(id, port, name, shape)| Output {
                name: name.clone(),
                shape: *shape,
                axes: port_axes(&loaded.model, *id, port),
                values: Vec::new(),
            })
            .collect()
    } else {
        reduced
            .iter()
            .map(|(r, _)| Output {
                name: r.name.clone(),
                // A vector per run (ADR 0080).
                shape: r.width().map_or(Shape::Scalar, Shape::Vector),
                axes: r
                    .width()
                    .map_or_else(Vec::new, |_| vec![r.axis_name().into()]),
                values: Vec::new(),
            })
            .collect()
    };
    for (row, r) in all.into_iter().enumerate() {
        let (p, j) = (row / plan.replicates, row % plan.replicates);
        index[0].1.push(p as f64);
        let mut at = if plan.replicates > 1 {
            index[1].1.push(j as f64);
            2
        } else {
            1
        };
        if ids {
            index[at].1.push(plan.id(p) as f64);
            at += 1;
        }
        for (column, &v) in index[at..].iter_mut().zip(&plan.points[p]) {
            column.1.push(v);
        }
        for (o, v) in outputs.iter_mut().zip(&r.finals) {
            o.values.extend_from_slice(v);
        }
    }
    // As fast and accurate as the model's methods.
    let labels = MethodLabels::combine(loaded.flat.blocks.iter().filter_map(|b| {
        let id = loaded.ids.get(&b.name)?;
        Some(loaded.model.block(*id).ok()?.method().labels)
    }));
    Ok(Results {
        index,
        outputs,
        populations: Vec::new(),
        labels,
        notes: Vec::new(),
        warnings: loaded.warnings.clone(),
    })
}

/// One run of a study: each output's final value (or each reduction's).
struct Row {
    finals: Vec<Vec<f64>>,
}

/// The `reduce` formulas of `plan` parsed, with where each name they read
/// is among `fixed` (output and element), `None` for `t`.
type Reducers = Vec<(crate::reduce::Reducer, Vec<Option<(usize, usize)>>)>;

fn reducers(
    plan: &simsandbox_file::Plan,
    loaded: &simsandbox_file::Loaded,
    fixed: &Outputs,
    t_end: f64,
) -> Result<Reducers, String> {
    let mut out = Vec::new();
    for (name, text) in &plan.reduce {
        let mut r =
            crate::reduce::Reducer::parse(name, text).map_err(|e| format!("sample.reduce.{e}"))?;
        r.place(&|d| sample_times(loaded, d, t_end))
            .map_err(|e| format!("sample.reduce.{e}"))?;
        let mut at = Vec::new();
        for n in &r.names {
            if n == "t" {
                at.push(None);
                continue;
            }
            // `block.port`, or one element `block.port[k]`.
            let (base, k) = match n.strip_suffix(']').and_then(|s| s.rsplit_once('[')) {
                Some((b, k)) => (
                    b,
                    k.parse::<usize>()
                        .map_err(|_| format!("sample.reduce.{name}: '{n}'"))?,
                ),
                None => (n.as_str(), 0),
            };
            let o = fixed
                .iter()
                .position(|(_, _, full, shape)| full == base && k < shape.len().max(1))
                .ok_or_else(|| format!("sample.reduce.{name}: no output '{n}'"))?;
            at.push(Some((o, k)));
        }
        out.push((r, at));
    }
    Ok(out)
}

/// The sample times of the recorded-data block `name` within a run to
/// `t_end`, for `at_samples` (ADR 0080).
pub(crate) fn sample_times(
    loaded: &simsandbox_file::Loaded,
    name: &str,
    t_end: f64,
) -> Result<Vec<f64>, String> {
    let block = loaded
        .ids
        .get(name)
        .and_then(|id| loaded.model.block(*id).ok())
        .ok_or_else(|| format!("at_samples: the model has no block '{name}'"))?;
    let times = block.instants().ok_or_else(|| {
        format!("at_samples: '{name}' is not recorded data (an input.file block)")
    })?;
    let end = t_end + 1e-9 * t_end.abs().max(1.0);
    Ok(times
        .iter()
        .copied()
        .filter(|&t| t >= 0.0 && t <= end)
        .collect())
}

/// The fixed-shape outputs of a loaded model: block, port, full name and
/// shape.
pub(crate) type Outputs = Vec<(simsandbox_core::graph::BlockId, String, String, Shape)>;

pub(crate) fn loaded_outputs(loaded: &simsandbox_file::Loaded) -> Result<Outputs, String> {
    let mut fixed = Vec::new();
    for entry in &loaded.flat.blocks {
        let id = loaded.ids[&entry.name];
        let spec = loaded.model.block(id).map_err(|e| e.to_string())?.spec();
        for port in spec.outputs() {
            if !port.shape.is_variable() {
                fixed.push((
                    id,
                    port.name.clone(),
                    format!("{}.{}", entry.name, port.name),
                    port.shape,
                ));
            }
        }
    }
    Ok(fixed)
}

/// The study's rows in `range`, on a model built for them.
fn run_rows(
    text: &str,
    registry: &Registry,
    study: &Study,
    range: std::ops::Range<usize>,
    progress: Option<&Progress>,
) -> Result<Vec<Row>, String> {
    let (plan, t_end, dt) = (&study.plan, study.t_end, study.dt);
    if range.is_empty() {
        return Ok(Vec::new());
    }
    let loaded = simsandbox_file::load_any(text, registry, None).map_err(|e| e.to_string())?;
    let fixed = loaded_outputs(&loaded)?;
    let dt = dt.unwrap_or(loaded.file.run.dt);
    let t_end = end_time(&loaded.model, t_end.or(loaded.file.run.t_end));
    let clocked = !loaded.model.clocks().0.is_empty();
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
    let ids = loaded.ids.clone();
    let mut params = loaded.params.clone();
    let reduced = reducers(plan, &loaded, &fixed, t_end)?;
    let mut sim = Simulator::new(loaded.model, 0.0).map_err(|e| e.to_string())?;
    let values = |sim: &Simulator| -> Vec<Vec<f64>> {
        fixed
            .iter()
            .map(|(id, port, _, _)| sim.output(*id, port).unwrap_or(&[]).to_vec())
            .collect()
    };
    let mut out = Vec::new();
    for row in range {
        if progress.is_some_and(Progress::given_up) {
            return Err(GIVEN_UP.into());
        }
        let (p, j) = (row / plan.replicates, row % plan.replicates);
        for (key, v) in &study.set {
            set_key(&mut sim, &mut params, &ids, key, *v).map_err(|e| format!("{key}: {e}"))?;
        }
        for (key, &v) in plan.keys.iter().zip(&plan.points[p]) {
            set_key(&mut sim, &mut params, &ids, key, v).map_err(|e| format!("{key}: {e}"))?;
        }
        // Common random numbers, or a seed mixed from the point's id and
        // the replicate (ADR 0067, ADR 0070 step 4): the run's place in
        // the plan never enters, so a point keeps its results wherever
        // it is. With ids from places this is the row, as before.
        let salt = match plan.seeds {
            simsandbox_file::Seeds::PerPoint => Some(
                plan.id(p)
                    .wrapping_mul(plan.replicates as u64)
                    .wrapping_add(j as u64),
            ),
            simsandbox_file::Seeds::Common if plan.replicates > 1 => Some(j as u64),
            simsandbox_file::Seeds::Common => None,
        };
        sim.reseed(salt);
        sim.restart(0.0).map_err(|e| e.to_string())?;
        let mut events = Track::new(schedule.clone(), Vec::new());
        let mut apply = |sim: &mut Simulator| -> Result<bool, String> {
            let now = sim.time();
            let mut any = false;
            while let Some(e) = events.take_due(now, 1e-9 * dt.max(now.abs())) {
                set_key(sim, &mut params, &ids, &e.key, e.value)
                    .map_err(|err| format!("input at t = {}: {err}", e.t))?;
                any = true;
            }
            Ok(any)
        };
        if apply(&mut sim)? {
            sim.restart(0.0).map_err(|e| e.to_string())?;
        }
        // Reductions need the whole run.
        let record = !reduced.is_empty();
        let mut t = vec![sim.time()];
        let mut kept: Vec<Vec<f64>> = if record { values(&sim) } else { Vec::new() };
        // A model with nothing in time is one evaluation (ADR 0067).
        let t_end = if study.timeless { 0.0 } else { t_end };
        advance(&mut sim, (dt, t_end, clocked), &mut apply, |sim| {
            if record {
                t.push(sim.time());
                for (k, v) in kept.iter_mut().zip(values(sim)) {
                    k.extend(v);
                }
            }
        })?;
        let finals = if reduced.is_empty() {
            values(&sim)
        } else {
            reduced
                .iter()
                .map(|(r, at)| {
                    let series: Vec<Vec<f64>> = at
                        .iter()
                        .map(|a| match a {
                            None => t.clone(),
                            Some((o, k)) => {
                                let n = fixed[*o].3.len().max(1);
                                kept[*o].iter().skip(*k).step_by(n).copied().collect()
                            }
                        })
                        .collect();
                    r.eval(&t, &series)
                })
                .collect()
        };
        out.push(Row { finals });
        if let Some(p) = progress {
            p.done.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
    Ok(out)
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
            // Those written as they recorded are there already.
            if let Some(to) = spec.to.as_ref().filter(|_| !spec.stream) {
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
/// empty). Probes that write as they record (ADR 0063) write into `dir`
/// during the run, if given; [`ProbeRun::write_files`] writes the others.
pub fn run_probes(
    text: &str,
    registry: &Registry,
    t_end: Option<f64>,
    dt: Option<f64>,
    track: Vec<InputEvent>,
    names: &[String],
    dir: Option<&Path>,
) -> Result<ProbeRun, String> {
    for v in [t_end, dt].into_iter().flatten() {
        if !(v.is_finite() && v > 0.0) {
            return Err("the end time and step must be positive".into());
        }
    }
    let mut s = Session::load("probes", text, registry)?;
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
    // Every probe kept, for the results; streaming ones into `dir` from
    // t = 0 on.
    s.record_all_probes();
    s.set_probe_dir(dir.map(Path::to_path_buf));
    if track.is_empty() {
        s.restart().map_err(|e| e.to_string())?;
    } else {
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

    /// Keeping less (ADR 0076) gives the chosen rows and outputs of the
    /// whole run, bit for bit, and the last step whatever `every` says.
    #[test]
    fn keeping_less_gives_the_same_values() {
        let text = example("cruise");
        let all = run(text, &registry(), Some(10.0), Some(0.1)).unwrap();
        let rows = all.rows();
        let names: Vec<&str> = all.outputs.iter().map(|o| o.name.as_str()).collect();
        let block = names[0].split('.').next().unwrap().to_string();
        let keep = Keep {
            signals: Some(vec![names[1].to_string(), block.clone()]),
            every: 7,
        };
        let some =
            run_keeping(text, &registry(), Some(10.0), Some(0.1), Vec::new(), &keep).unwrap();
        let mut want: Vec<usize> = (0..rows).step_by(7).collect();
        if want.last() != Some(&(rows - 1)) {
            want.push(rows - 1);
        }
        assert_eq!(some.rows(), want.len());
        let t = &all.index[0].1;
        let bits = |v: &[f64]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        let picked: Vec<f64> = want.iter().map(|&r| t[r]).collect();
        assert_eq!(bits(&some.index[0].1), bits(&picked));
        let kept: Vec<&str> = some.outputs.iter().map(|o| o.name.as_str()).collect();
        let expected: Vec<&str> = names
            .iter()
            .copied()
            .filter(|n| *n == names[1] || n.starts_with(&format!("{block}.")))
            .collect();
        assert_eq!(kept, expected);
        for o in &some.outputs {
            let full = all.outputs.iter().find(|f| f.name == o.name).unwrap();
            let n = o.shape.len();
            let rows: Vec<f64> = want
                .iter()
                .flat_map(|&r| full.values[r * n..(r + 1) * n].iter().copied())
                .collect();
            assert_eq!(bits(&o.values), bits(&rows), "{}", o.name);
        }
        // A name that keeps nothing is an error naming it.
        let wrong = Keep {
            signals: Some(vec!["nothing.here".into()]),
            every: 1,
        };
        let e = run_keeping(text, &registry(), Some(1.0), None, Vec::new(), &wrong).unwrap_err();
        assert!(e.contains("no output named nothing.here"), "{e}");
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
    fn a_model_whose_only_runs_are_its_studies_writes_a_row_per_run() {
        // The resonance curve (ADR 0071): its study's 600 runs as rows.
        let r = run(example("`curve`"), &registry(), None, None).unwrap();
        assert_eq!(r.index[0].0, "run");
        assert_eq!(r.rows(), 600);
        let j = r.to_json();
        assert_eq!(j["rows"], 600);
        assert!(j["outputs"]["curve.spring_amplitude"]["values"][0].is_number());
        assert!(j["outputs"].get("curve.m").is_none());
        // In time, every step holds every run.
        let timed = run(example("`curve`"), &registry(), Some(1.0), Some(0.5)).unwrap();
        assert_eq!(timed.rows(), 3);
        assert!(run(example("lorenz"), &registry(), Some(-1.0), None).is_err());
    }
}
