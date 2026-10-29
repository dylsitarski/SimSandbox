//! One open model: the running simulator, its snapshots for rewind, the
//! recorded signals the views draw from, and its parameters.

use crate::filters::Filters;
use crate::probe::{ProbeSpec, Trigger};
use simsandbox_core::block::{MethodLabels, Shape};
use simsandbox_core::graph::{BlockId, Model};
use simsandbox_core::registry::Registry;
use simsandbox_core::sim::{Simulator, Snapshot};
use simsandbox_core::units::Unit;
use simsandbox_core::SimError;
use simsandbox_file::runs::RunFile;
use simsandbox_file::{
    Design, FilterEntry, FusedGroup, ModelFile, ParamPlan, Plan, ProbeEntry, Sampling, ViewEntry,
};
use simsandbox_views::cut::Geometry;
use simsandbox_views::history::thin;
use simsandbox_views::{Available, Coordinate, FieldShape, History, SignalRef};
use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::{Duration, Instant};

/// The recorded history, shared with the window (ADR 0035): the session
/// writes it, the window reads it to draw. Cloning shares it.
#[derive(Clone)]
pub struct SharedHistory(Arc<RwLock<History>>);

impl SharedHistory {
    fn new(h: History) -> Self {
        Self(Arc::new(RwLock::new(h)))
    }

    /// Read access; waits only while the session replaces or appends.
    pub fn read(&self) -> RwLockReadGuard<'_, History> {
        self.0.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, History> {
        self.0.write().unwrap_or_else(PoisonError::into_inner)
    }

    fn try_write(&self) -> Option<RwLockWriteGuard<'_, History>> {
        match self.0.try_write() {
            Ok(g) => Some(g),
            Err(std::sync::TryLockError::Poisoned(e)) => Some(e.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => None,
        }
    }
}

/// A probe besides the implicit one (ADR 0062): a subset of what the
/// session records, on its own schedule, kept its own way.
struct Extra {
    spec: ProbeSpec,
    trigger: Trigger,
    history: SharedHistory,
    /// Where its signals and fields are in a whole sample.
    signals: Vec<usize>,
    fields: Vec<usize>,
    pending: Vec<Pending>,
}

/// Where an input of a filter with a clock of its own comes from
/// (ADR 0063).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Src {
    /// The model, at every step.
    Model,
    /// A probe, by place.
    Probe(usize),
    /// Another such filter, by place (always an earlier one).
    Derived(usize),
}

/// One input: its name, source, and where its value is: a place in the
/// whole sample (model, probe), or among that filter's outputs.
#[derive(Clone, Debug)]
struct Input {
    name: String,
    src: Src,
    at: usize,
    field: bool,
}

/// A filter with an input from a probe (ADR 0063): it runs when its
/// sources record (all of them, or any with `when: any`) and keeps its
/// own recording, bound by its outputs' names.
struct Derived {
    entry: FilterEntry,
    /// Its probes as set, so it is kept while they do not change.
    key: Vec<ProbeSpec>,
    filters: Filters,
    inputs: Vec<Input>,
    sources: Vec<Src>,
    /// Each input's latest value (one number for a signal).
    held: Vec<Option<Vec<f64>>>,
    history: SharedHistory,
    pending: Vec<Pending>,
    /// What it gave at the current step, for filters reading it.
    now: Option<(Vec<f64>, Vec<Vec<f64>>)>,
}

impl Derived {
    /// Its outputs' names: signals, then fields.
    fn outputs(&self) -> (Vec<String>, Vec<String>) {
        (
            self.filters.exprs.iter().map(|f| f.name.clone()).collect(),
            self.filters
                .cuts
                .iter()
                .flat_map(|c| c.fields().into_iter().map(|(n, _)| n))
                .collect(),
        )
    }

    fn flush(&mut self, wait: bool) -> bool {
        if self.pending.is_empty() {
            return true;
        }
        let guard = if wait {
            Some(self.history.write())
        } else {
            self.history.try_write()
        };
        let Some(mut h) = guard else {
            return false;
        };
        for p in self.pending.drain(..) {
            let frames: Vec<&[f64]> = p.frames.iter().map(Vec::as_slice).collect();
            h.push_sample(p.index, &p.values, &frames);
        }
        true
    }
}

impl Extra {
    /// Append any pending samples, waiting for readers if `wait`.
    /// Returns whether none are left.
    fn flush(&mut self, wait: bool) -> bool {
        if self.pending.is_empty() {
            return true;
        }
        let guard = if wait {
            Some(self.history.write())
        } else {
            self.history.try_write()
        };
        let Some(mut h) = guard else {
            return false;
        };
        for p in self.pending.drain(..) {
            let frames: Vec<&[f64]> = p.frames.iter().map(Vec::as_slice).collect();
            h.push_sample(p.index, &p.values, &frames);
        }
        true
    }
}

/// A sample that found the history being read, kept to append later.
struct Pending {
    index: f64,
    values: Vec<f64>,
    frames: Vec<Vec<f64>>,
}

/// A sweep being evaluated, a slice at a time (ADR 0035): the next sample,
/// and for a two-parameter grid the columns collected so far.
struct ResampleJob {
    next: usize,
    columns: Vec<Vec<f64>>,
    started: Instant,
}

/// A sweep counts as real-time (ADR 0037) when it recomputes within this.
const REAL_TIME_SWEEP: Duration = Duration::from_millis(33);

/// Take a snapshot for rewind every this many steps.
const SNAPSHOT_EVERY: usize = 10;
/// When the snapshots grow past this many, or past [`SNAPSHOT_VALUES`]
/// numbers in all, every other one is dropped: the whole run stays
/// reachable, at coarser resolution.
const MAX_SNAPSHOTS: usize = 10_000;
/// The memory snapshots may take, in numbers: 256 MB. A 128 × 128 grid
/// model takes about 1 MB a snapshot, so it keeps about 250.
const SNAPSHOT_VALUES: usize = 32_000_000;

/// Where a recorded signal comes from: an element of an output port.
struct Source {
    block: BlockId,
    port: String,
    index: usize,
}

/// A whole output port recorded as a field (a grid or a point set).
struct FieldSource {
    block: BlockId,
    port: String,
    /// Values per frame; `None` for a population, whose frames vary.
    len: Option<usize>,
}

/// One parameter as the panel shows it.
#[derive(Clone, Debug)]
pub struct ParamRow {
    /// `"block.param"`, the key used in `controls.params`; for a declared
    /// parameter (ADR 0043) `"name"` (the model's) or `"instance.name"`.
    pub key: String,
    /// The Block it belongs to; `None` for a declared parameter.
    pub block: Option<BlockId>,
    /// The flat block name (`"pasture/grass"`), or for a declared
    /// parameter the instance (`""` for the model's own).
    pub owner: String,
    pub name: String,
    /// Set when a formula computes it: it is shown, not edited.
    pub formula: Option<String>,
    pub unit: String,
    pub description: String,
    pub suggested: [f64; 2],
    pub initial_condition: bool,
    pub value: f64,
}

/// A sweep (ADR 0027, ADR 0028), for models with no states: the outputs
/// are evaluated at each sample of the swept parameters instead of running
/// in time. Every sample is recorded, with the swept parameters as signals;
/// a grid over two parameters also gives every signal reshaped as a grid
/// named `grid.<signal>`, the parameters' grids as its coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct Sweep {
    /// As written in the file's `sample` section.
    pub sampling: Sampling,
    /// The samples it gives.
    pub plan: Plan,
    /// The block and parameter of each swept key.
    targets: Vec<(BlockId, String)>,
}

impl Sweep {
    /// The swept parameters' keys, `"block.param"`.
    pub fn keys(&self) -> &[String] {
        &self.plan.keys
    }

    /// The grid's size when the design is a grid over two parameters.
    pub fn grid(&self) -> Option<(usize, usize)> {
        match self.plan.grid.as_deref() {
            Some(&[nx, ny]) => Some((nx, ny)),
            _ => None,
        }
    }

    /// Two parameters on a grid: results are also offered as grids.
    pub fn is_grid(&self) -> bool {
        self.grid().is_some()
    }

    /// The samples have no order to draw lines along: anything but a grid
    /// over one parameter.
    pub fn unordered(&self) -> bool {
        !(self.plan.design == Design::Grid && self.plan.keys.len() == 1)
    }
}

/// The name of signal `name` reshaped as a grid over a two-parameter sweep.
pub fn grid_name(name: &str) -> String {
    format!("grid.{name}")
}

pub struct Session {
    /// Display name, e.g. the file name.
    pub title: String,
    pub description: Option<String>,
    /// The file as loaded; the source for saving.
    pub file: ModelFile,
    /// The file flattened: one entry per Block, subsystems expanded
    /// (ADR 0043).
    pub flat: ModelFile,
    /// Declared parameters and the formulas they feed.
    plan: ParamPlan,
    pub ids: BTreeMap<String, BlockId>,
    pub warnings: Vec<String>,
    /// Older forms the file was written in, converted on reading
    /// (ADR 0044).
    pub conversions: Vec<String>,
    /// Blocks solved together as one system (ADR 0023).
    pub groups: Vec<FusedGroup>,
    pub dt: f64,
    /// Recorded signals, one per plotted output element (ADR 0015), and in
    /// a sweep the swept parameters (ADR 0028); shared with the window.
    history: SharedHistory,
    /// Samples waiting for the window to stop reading.
    pending: Vec<Pending>,
    /// Probes besides the implicit one, which is `history` (ADR 0062).
    probes: Vec<Extra>,
    /// What views read from the model directly (ADR 0063): the history
    /// records these and what filters read, nothing else. `None` until
    /// known (a model without saved views, before the app shows any):
    /// everything.
    view_reads: Option<Vec<String>>,
    /// Every scalar and per-sample field a step yields, in sample order:
    /// what probes take from and views can bind.
    layout_scalars: Vec<String>,
    layout_fields: Vec<(String, FieldShape)>,
    /// Which fields place which others' cells, among everything a step
    /// yields.
    all_coordinates: Vec<Coordinate>,
    /// Filters with an input from a probe, each with its own clock and
    /// recording (ADR 0063), in dependency order.
    derived: Vec<Derived>,
    /// Their outputs' names, as views bind them.
    derived_names: Vec<String>,
    /// What the history keeps of each sample: places in the layout.
    keep_signals: Vec<usize>,
    keep_fields: Vec<usize>,
    sources: Vec<Source>,
    /// Names of the scalar output signals, in `sources` order.
    output_names: Vec<String>,
    /// Grid and point-set outputs and their shapes.
    base_fields: Vec<(String, FieldShape)>,
    /// Coordinates among the grid outputs.
    base_coordinates: Vec<Coordinate>,
    /// Grids with one row, which views can draw over time.
    lines: Vec<String>,
    /// Grid and point-set outputs, recorded whole (ADR 0016).
    field_sources: Vec<FieldSource>,
    /// Names of the recorded grids and point sets, and the model's block
    /// types: what views can bind besides scalars.
    grids: Vec<String>,
    points: Vec<String>,
    block_types: Vec<String>,
    /// Where the app pauses (the file's `run.t_end`); cleared once reached so
    /// Run continues past it (ADR 0013).
    stop_at: Option<f64>,
    sim: Simulator,
    snapshots: Vec<Snapshot>,
    steps_since_snapshot: usize,
    /// Set when the model has no states: it is swept, not run.
    sweep: Option<Arc<Sweep>>,
    /// The sweep being evaluated, if not finished.
    job: Option<ResampleJob>,
    /// Sweeps are evaluated by [`Session::resume_resample`] (on the worker
    /// thread) instead of at once.
    deferred: bool,
    /// How long the last complete sweep took.
    last_sweep: Option<Duration>,
    /// Filters as written, for saving and editing (ADR 0030, ADR 0032).
    filter_entries: Vec<FilterEntry>,
    /// Filters compiled: expressions recorded after the output signals,
    /// cut fields after the recorded fields.
    filters: Filters,
    /// The schedule and the recorded live input (ADR 0058).
    track: Track,
    /// The file as it was when the current run started (its parameters at
    /// t = 0), which a run file is made with (ADR 0059).
    start_file: ModelFile,
}

/// Set a value by its key on a running model: a declared parameter (the
/// formulas it feeds follow, ADR 0043) or `"block.param"`, unless a formula
/// sets it.
pub(crate) fn set_key(
    sim: &mut Simulator,
    plan: &mut ParamPlan,
    ids: &BTreeMap<String, BlockId>,
    key: &str,
    value: f64,
) -> Result<(), String> {
    if plan.declared(key).is_some() {
        for (block, param, v) in plan.set(key, value)? {
            sim.set_param(ids[&block], &param, v)
                .map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    let (block, name) = key
        .rsplit_once('.')
        .ok_or_else(|| format!("'{key}' is not 'block.param'"))?;
    let id = *ids
        .get(block)
        .ok_or_else(|| format!("no block '{block}'"))?;
    if let Some(f) = plan.formula_of(block, name) {
        return Err(format!("{key} is computed from '{}'", f.text));
    }
    sim.set_param(id, name, value).map_err(|e| e.to_string())
}

/// One change of a value during a run (ADR 0058): `key` is a parameter
/// key (`"block.param"` or a declared parameter), applied at the first step
/// boundary at or after `t`.
#[derive(Debug, Clone, PartialEq)]
pub struct InputEvent {
    pub t: f64,
    pub key: String,
    pub value: f64,
}

/// The input track: the schedule (kept by restarts) and the recorded live
/// input (dropped by restarts, kept by rewinds and replayed), each with
/// how much of it has been applied.
#[derive(Debug, Clone, Default)]
pub(crate) struct Track {
    schedule: Vec<InputEvent>,
    /// The model's own value of each key the schedule sets, which every
    /// run starts from.
    base: Vec<(String, f64)>,
    recorded: Vec<InputEvent>,
    next_schedule: usize,
    next_recorded: usize,
}

impl Track {
    /// A schedule and recorded input, each put in time order.
    pub(crate) fn new(mut schedule: Vec<InputEvent>, mut recorded: Vec<InputEvent>) -> Self {
        schedule.sort_by(|a, b| a.t.total_cmp(&b.t));
        recorded.sort_by(|a, b| a.t.total_cmp(&b.t));
        Self {
            schedule,
            recorded,
            ..Self::default()
        }
    }

    /// The next event due by `now`: the earlier of the two lists, the
    /// schedule first at equal times (as it was applied live).
    pub(crate) fn take_due(&mut self, now: f64, eps: f64) -> Option<InputEvent> {
        let s = self
            .schedule
            .get(self.next_schedule)
            .filter(|e| e.t <= now + eps);
        let r = self
            .recorded
            .get(self.next_recorded)
            .filter(|e| e.t <= now + eps);
        match (s, r) {
            (Some(a), Some(b)) if b.t < a.t - eps => {
                self.next_recorded += 1;
                Some(b.clone())
            }
            (Some(a), _) => {
                self.next_schedule += 1;
                Some(a.clone())
            }
            (None, Some(b)) => {
                self.next_recorded += 1;
                Some(b.clone())
            }
            (None, None) => None,
        }
    }

    /// Start applying again from time `t`: events at `t` or later are
    /// still ahead.
    fn seek(&mut self, t: f64, eps: f64) {
        self.next_schedule = self.schedule.partition_point(|e| e.t < t - eps);
        self.next_recorded = self.recorded.partition_point(|e| e.t < t - eps);
    }
}

impl Session {
    /// Load a model file, evaluating any sweep at once. Errors are
    /// formatted for display.
    pub fn load(title: &str, text: &str, registry: &Registry) -> Result<Self, String> {
        Self::load_with(title, text, registry, false)
    }

    /// Load a model file, leaving its sweep (if any) to be evaluated by
    /// [`Session::resume_resample`].
    pub fn load_deferred(title: &str, text: &str, registry: &Registry) -> Result<Self, String> {
        Self::load_with(title, text, registry, true)
    }

    fn load_with(
        title: &str,
        text: &str,
        registry: &Registry,
        deferred: bool,
    ) -> Result<Self, String> {
        let loaded = simsandbox_file::load(text, registry).map_err(|e| e.to_string())?;
        let mut names = Vec::new();
        let mut sources = Vec::new();
        let mut fields = Vec::new();
        let mut field_sources = Vec::new();
        let mut coordinates: Vec<Coordinate> = Vec::new();
        let mut block_types = Vec::new();
        for entry in &loaded.flat.blocks {
            let id = loaded.ids[&entry.name];
            let block = loaded.model.block(id).map_err(|e| e.to_string())?;
            if !block_types.contains(&entry.type_name) {
                block_types.push(entry.type_name.clone());
            }
            let spec = block.spec();
            for port in spec.outputs() {
                // Every output can be recorded; what is, wiring decides
                // (ADR 0063), so `controls.plots` no longer limits it.
                let key = format!("{}.{}", entry.name, port.name);
                let field = match port.shape {
                    Shape::Grid { nx, ny } => Some(FieldShape::Grid { nx, ny }),
                    Shape::Points { count, dim } => Some(FieldShape::Points { count, dim }),
                    Shape::Population { dim } => Some(FieldShape::Population { dim }),
                    Shape::Scalar | Shape::Vector(_) => None,
                };
                if let Some(shape) = field {
                    for c in &port.coordinate {
                        coordinates.push(Coordinate {
                            name: key.clone(),
                            of: format!("{}.{}", entry.name, c.port),
                            axis: c.axis,
                        });
                    }
                    fields.push((key, shape));
                    field_sources.push(FieldSource {
                        block: id,
                        port: port.name.clone(),
                        len: (!port.shape.is_variable()).then(|| port.shape.len()),
                    });
                    continue;
                }
                for index in 0..port.shape.len() {
                    names.push(match port.shape {
                        Shape::Vector(_) => format!("{key}[{index}]"),
                        _ => key.clone(),
                    });
                    sources.push(Source {
                        block: id,
                        port: port.name.clone(),
                        index,
                    });
                }
            }
        }
        // A coordinate needs the field it places, recorded, of the same shape.
        coordinates.retain(|c| {
            let shape = |n: &str| fields.iter().find(|(f, _)| f == n).map(|(_, s)| *s);
            shape(&c.name).is_some() && shape(&c.name) == shape(&c.of)
        });
        let sweep = match loaded.plan {
            Some(plan) => {
                let sampling = loaded
                    .file
                    .sampling(&loaded.model, &loaded.ids)
                    .map_err(|e| e.to_string())?
                    .ok_or("a sweep with nothing to sample")?;
                Some(Arc::new(Sweep::new(sampling, plan, &loaded.ids)?))
            }
            None => None,
        };
        let sim = Simulator::new(loaded.model, 0.0).map_err(|e| e.to_string())?;
        let mut schedule: Vec<InputEvent> = loaded
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
        schedule.sort_by(|a, b| a.t.total_cmp(&b.t));
        let start_file = loaded.file.clone();
        let mut warnings = loaded.warnings;
        warnings.extend(loop_notes(&sim));
        let entries = loaded.file.controls.filters.clone();
        let mut session = Self {
            title: title.into(),
            description: loaded.file.description.clone(),
            dt: loaded.file.run.dt,
            stop_at: loaded.file.run.t_end,
            conversions: loaded.conversions,
            file: loaded.file,
            flat: loaded.flat,
            plan: loaded.params,
            ids: loaded.ids,
            groups: loaded.groups,
            warnings,
            history: SharedHistory::new(History::new(Vec::new())),
            pending: Vec::new(),
            probes: Vec::new(),
            view_reads: (!start_file.controls.views.is_empty())
                .then(|| view_reads(&start_file.controls.views)),
            layout_scalars: Vec::new(),
            layout_fields: Vec::new(),
            all_coordinates: Vec::new(),
            keep_signals: Vec::new(),
            keep_fields: Vec::new(),
            derived: Vec::new(),
            derived_names: Vec::new(),
            sources,
            output_names: names,
            base_fields: fields,
            base_coordinates: coordinates,
            lines: Vec::new(),
            field_sources,
            grids: Vec::new(),
            points: Vec::new(),
            block_types,
            snapshots: vec![sim.snapshot()],
            sim,
            steps_since_snapshot: 0,
            sweep,
            job: None,
            deferred,
            last_sweep: None,
            filter_entries: Vec::new(),
            filters: Filters::default(),
            track: Track {
                schedule,
                ..Track::default()
            },
            start_file,
        };
        session.check_events(
            &session
                .track
                .schedule
                .iter()
                .map(|e| (e.key.clone(), e.value))
                .collect::<Vec<_>>(),
            "run.inputs",
        )?;
        let rows = session.params();
        let mut base: Vec<(String, f64)> = Vec::new();
        for e in &session.track.schedule {
            if !base.iter().any(|(k, _)| *k == e.key) {
                if let Some(r) = rows.iter().find(|r| r.key == e.key) {
                    base.push((e.key.clone(), r.value));
                }
            }
        }
        session.track.base = base;
        let (filters, problems) = session.compile_filters(&entries);
        session.filter_entries = kept_entries(&filters, &entries);
        session.filters = filters;
        for p in problems {
            session
                .warnings
                .push(format!("controls.filters.{p}; ignored"));
        }
        if session.sweep.is_some() {
            session.resample();
        } else {
            // No probes yet: they are loaded next, and the filters that
            // read them with them.
            session.build_history();
            session.load_probes();
            // Scheduled changes at t = 0 set where the run starts.
            session.restart().map_err(|e| e.to_string())?;
        }
        Ok(session)
    }

    /// Events must name parameters a person sets (not ones a formula
    /// computes); values outside a parameter's suggested range are noted.
    fn check_events(&mut self, events: &[(String, f64)], what: &str) -> Result<(), String> {
        let rows = self.params();
        for (k, (key, value)) in events.iter().enumerate() {
            let Some(row) = rows.iter().find(|r| r.key == *key) else {
                return Err(format!("{what}[{k}]: no parameter '{key}'"));
            };
            if row.formula.is_some() {
                return Err(format!(
                    "{what}[{k}]: {key} is set by a formula, so it cannot be changed"
                ));
            }
            let [lo, hi] = row.suggested;
            if *value < lo || *value > hi {
                self.warnings.push(format!(
                    "{what}[{k}]: {key} = {value} is outside its suggested range {lo} to {hi}"
                ));
            }
        }
        Ok(())
    }

    /// How close two times must be to count as the same step boundary.
    fn eps(&self) -> f64 {
        1e-9 * self.dt.max(self.time().abs())
    }

    /// Apply every event due by now (ADR 0058). An event that no longer
    /// applies is noted, not fatal.
    fn apply_due(&mut self) {
        let (now, eps) = (self.time(), self.eps());
        while let Some(e) = self.track.take_due(now, eps) {
            if let Err(err) = self.set_value(&e.key, e.value) {
                self.warnings
                    .push(format!("input at t = {}: {err}", short(e.t)));
            }
        }
    }

    /// The scheduled changes in effect (ADR 0058).
    pub fn schedule(&self) -> &[InputEvent] {
        &self.track.schedule
    }

    /// The live input recorded in this run, including any ahead of now
    /// after a rewind (ADR 0058).
    pub fn recorded(&self) -> &[InputEvent] {
        &self.track.recorded
    }

    /// Recorded input lies ahead of now: running on replays it.
    pub fn replaying(&self) -> bool {
        self.track.next_recorded < self.track.recorded.len()
    }

    /// Restart and replay `track` as recorded live input (a run file's,
    /// ADR 0059). Errors name the event that does not apply.
    pub fn replay(&mut self, mut track: Vec<InputEvent>) -> Result<(), String> {
        let events: Vec<(String, f64)> = track.iter().map(|e| (e.key.clone(), e.value)).collect();
        self.check_events(&events, "track")?;
        track.sort_by(|a, b| a.t.total_cmp(&b.t));
        self.restart().map_err(|e| e.to_string())?;
        self.track.recorded = track;
        self.track.next_recorded = 0;
        // Recorded input at t = 0 sets where the run starts, as live.
        if self
            .track
            .recorded
            .first()
            .is_some_and(|e| e.t <= self.eps())
        {
            self.restart_keeping_recorded().map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// The file as it was when the current run started.
    pub fn start_file(&self) -> &ModelFile {
        &self.start_file
    }

    /// This run as a run file (ADR 0059): the model hash of the file it
    /// started from, its seed and settings, the schedule and the recorded
    /// input. `model` is the model file's path relative to where the run
    /// file goes.
    pub fn run_file(&self, model: &str, name: &str, note: &str, created: &str) -> RunFile {
        RunFile {
            format: simsandbox_file::runs::RUN_FORMAT,
            model: model.into(),
            model_hash: simsandbox_file::runs::model_hash(&self.start_file),
            name: name.into(),
            note: note.into(),
            created: created.into(),
            seed: Some(self.file.run.seed.unwrap_or(1)),
            run: Some(simsandbox_file::runs::RunOverrides {
                dt: Some(self.dt),
                t_end: self.file.run.t_end,
            }),
            inputs: self
                .track
                .schedule
                .iter()
                .map(|e| simsandbox_file::ScheduledChange {
                    t: e.t,
                    set: e.key.clone(),
                    to: e.value,
                })
                .collect(),
            track: self
                .track
                .recorded
                .iter()
                .map(|e| (e.t, e.key.clone(), e.value))
                .collect(),
        }
    }

    /// Names of the recorded grids, point sets, one-row grids and the
    /// model's block types: what views can bind besides scalars.
    /// Every scalar a step yields, which views can bind (ADR 0063).
    pub fn bindable_scalars(&self) -> &[String] {
        &self.layout_scalars
    }

    /// The 2D grids a cut filter can take, recorded or not (ADR 0063).
    pub fn cut_inputs(&self) -> Vec<String> {
        if self.sweep.is_some() {
            // A sweep records everything, its whole-sweep grids included.
            return cut_inputs(&self.signals());
        }
        self.layout_fields
            .iter()
            .filter(|(_, s)| matches!(s, FieldShape::Grid { nx, ny } if *nx >= 2 && *ny >= 2))
            .filter(|(n, _)| !self.all_coordinates.iter().any(|c| &c.name == n))
            .map(|(n, _)| n.clone())
            .collect()
    }

    /// Which fields place which others' cells, among all a step yields.
    pub fn all_coordinates(&self) -> &[Coordinate] {
        &self.all_coordinates
    }

    pub fn bindable(&self) -> [&[String]; 4] {
        [&self.grids, &self.points, &self.lines, &self.block_types]
    }

    /// The sweep, shared.
    pub fn sweep_shared(&self) -> Option<Arc<Sweep>> {
        self.sweep.clone()
    }

    /// The recorded signals, for reading.
    pub fn signals(&self) -> RwLockReadGuard<'_, History> {
        self.history.read()
    }

    /// The recorded signals, shared, for the window to read.
    pub fn shared_history(&self) -> SharedHistory {
        self.history.clone()
    }

    /// Write access to the history, with any pending samples appended
    /// first.
    fn history_mut(&mut self) -> RwLockWriteGuard<'_, History> {
        let mut h = self.history.write();
        for p in self.pending.drain(..) {
            let frames: Vec<&[f64]> = p.frames.iter().map(Vec::as_slice).collect();
            h.push_sample(p.index, &p.values, &frames);
        }
        h
    }

    /// Replace the history's contents (the window's handle stays valid).
    fn replace_history(&mut self, h: History) -> History {
        self.pending.clear();
        std::mem::replace(&mut *self.history.write(), h)
    }

    /// Append pending samples: waiting for the window if `wait`, else only
    /// if it is not reading. Returns whether none are left.
    pub fn flush(&mut self, wait: bool) -> bool {
        let mut probes_done = true;
        for p in &mut self.probes {
            probes_done &= p.flush(wait);
        }
        for d in &mut self.derived {
            probes_done &= d.flush(wait);
        }
        if self.pending.is_empty() {
            return probes_done;
        }
        if wait {
            drop(self.history_mut());
            return true;
        }
        let Some(mut h) = self.history.try_write() else {
            return false;
        };
        for p in self.pending.drain(..) {
            let frames: Vec<&[f64]> = p.frames.iter().map(Vec::as_slice).collect();
            h.push_sample(p.index, &p.values, &frames);
        }
        true
    }

    /// Record one sample: scalar values, the frames of the fields before
    /// the cuts, then the cuts' frames. It never waits for the window: a
    /// sample that finds the history being read is appended later.
    fn push(&mut self, index: f64, values: &[f64], before: &[&[f64]], cuts: &[Vec<f64>]) {
        let frames: Vec<&[f64]> = before
            .iter()
            .copied()
            .chain(cuts.iter().map(Vec::as_slice))
            .collect();
        if self.sweep.is_none() && (!self.probes.is_empty() || !self.derived.is_empty()) {
            let (step, eps) = (self.sim.step_count(), self.eps());
            let mut fired = Vec::with_capacity(self.probes.len());
            for p in &mut self.probes {
                let due = p.trigger.due(index, step, eps);
                fired.push(due);
                if !due {
                    continue;
                }
                p.pending.push(Pending {
                    index,
                    values: p.signals.iter().map(|&i| values[i]).collect(),
                    frames: p.fields.iter().map(|&i| frames[i].to_vec()).collect(),
                });
                p.flush(false);
            }
            self.run_derived(index, values, &frames, &fired);
        }
        self.push_kept(index, values, &frames);
    }

    /// Record in the history what it keeps of a whole sample (ADR 0063).
    fn push_kept(&mut self, index: f64, values: &[f64], frames: &[&[f64]]) {
        let values: Vec<f64> = self.keep_signals.iter().map(|&i| values[i]).collect();
        let frames: Vec<&[f64]> = self
            .keep_fields
            .iter()
            .map(|&i| frames.get(i).copied().unwrap_or(&[]))
            .collect();
        if let Some(mut h) = self.history.try_write() {
            for p in self.pending.drain(..) {
                let frames: Vec<&[f64]> = p.frames.iter().map(Vec::as_slice).collect();
                h.push_sample(p.index, &p.values, &frames);
            }
            h.push_sample(index, &values, &frames);
            return;
        }
        self.pending.push(Pending {
            index,
            values,
            frames: frames.iter().map(|f| f.to_vec()).collect(),
        });
    }

    /// Tell the session what the views read (their bindings as written):
    /// the history records what they read from the model directly and
    /// what filters read, from now on (ADR 0063). What it recorded of
    /// those before is kept; what it no longer records is dropped.
    pub fn set_view_reads(&mut self, reads: Vec<String>) {
        let reads = Some(model_reads(reads));
        if self.view_reads == reads {
            return;
        }
        self.view_reads = reads;
        if self.sweep.is_some() {
            return;
        }
        let (signals, fields) = self.kept();
        if signals == self.keep_signals && fields == self.keep_fields {
            return;
        }
        drop(self.history_mut());
        let old = self.replace_history(History::new(Vec::new()));
        self.build_history();
        self.replay_history(&old, false);
    }

    /// The places in the layout the history keeps: everything in a sweep
    /// or when no views are known, else what views read from the model
    /// and what filters read, with the coordinates cuts need.
    fn kept(&self) -> (Vec<usize>, Vec<usize>) {
        let (Some(reads), None) = (&self.view_reads, &self.sweep) else {
            return (
                (0..self.layout_scalars.len()).collect(),
                (0..self.layout_fields.len()).collect(),
            );
        };
        let mut want: std::collections::BTreeSet<String> = reads.iter().cloned().collect();
        // Filters with clocks of their own read probes and each other.
        let derived = crate::observers::clocked(&self.filter_entries);
        let model = |f: &&FilterEntry| !derived.iter().any(|d| d.name == f.name);
        for f in self.filter_entries.iter().filter(model) {
            want.extend(crate::observers::filter_reads(f));
        }
        for c in &self.filters.cuts {
            want.extend(
                self.all_coordinates
                    .iter()
                    .filter(|k| k.of == c.input)
                    .map(|k| k.name.clone()),
            );
        }
        let signals = (0..self.layout_scalars.len())
            .filter(|&i| want.contains(&self.layout_scalars[i]))
            .collect();
        let fields = (0..self.layout_fields.len())
            .filter(|&i| want.contains(&self.layout_fields[i].0))
            .collect();
        (signals, fields)
    }

    /// Append the samples of `old` to the (new, empty) history, by name:
    /// what it did not record is NaN for signals and absent for fields.
    /// With `recompute`, the filters are computed again from the outputs.
    fn replay_history(&mut self, old: &History, recompute: bool) {
        let signal_at: Vec<Option<usize>> = self
            .layout_scalars
            .iter()
            .map(|n| old.names().iter().position(|m| m == n))
            .collect();
        let field_at: Vec<Option<usize>> = self
            .layout_fields
            .iter()
            .map(|(n, _)| old.fields().iter().position(|f| &f.name == n && !f.whole))
            .collect();
        let outputs = self.output_names.len();
        let base = self.base_fields.len();
        for k in 0..old.len() {
            let t = old.times()[k];
            let mut values: Vec<f64> = signal_at
                .iter()
                .map(|i| i.map_or(f64::NAN, |i| old.value(SignalRef::Signal(i), k)))
                .collect();
            let mut frames: Vec<&[f64]> = field_at
                .iter()
                .map(|i| i.map_or(&[][..], |i| old.fields()[i].frame(k)))
                .collect();
            let cuts;
            if recompute {
                values.truncate(outputs);
                self.append_derived(t, &mut values);
                frames.truncate(base);
                cuts = self.sample_cuts(&frames);
                frames.extend(cuts.iter().map(Vec::as_slice));
            }
            self.push_kept(t, &values, &frames);
        }
    }

    /// What views can bind: everything a step yields (ADR 0063), whether
    /// or not the history `h` records it now; and the model's block types.
    pub fn available<'a>(&'a self, h: &'a History) -> Available<'a> {
        let _ = h;
        Available {
            scalars: &self.layout_scalars,
            grids: &self.grids,
            points: &self.points,
            block_types: &self.block_types,
            lines: &self.lines,
            coordinates: &self.all_coordinates,
            swept: self.sweep.as_ref().map_or(&[], |sw| sw.keys()),
            derived: &self.derived_names,
        }
    }

    /// The names of the recorded scalar signals: outputs, then
    /// expression filters.
    fn scalar_names(&self) -> Vec<String> {
        self.output_names
            .iter()
            .cloned()
            .chain(self.filters.exprs.iter().map(|f| f.name.clone()))
            .collect()
    }

    /// The grids recorded before any cut filter, which cuts may take as
    /// input: the grid outputs, and in a two-parameter grid sweep every
    /// scalar (named in `scalars`) and swept parameter as a grid.
    fn source_grids(&self, scalars: &[String]) -> Vec<(String, FieldShape)> {
        let mut grids = self.base_fields.clone();
        if let Some(sw) = &self.sweep {
            if let Some((nx, ny)) = sw.grid() {
                let shape = FieldShape::Grid { nx, ny };
                grids.extend(
                    scalars
                        .iter()
                        .chain(sw.keys())
                        .map(|n| (grid_name(n), shape)),
                );
            }
        }
        grids
    }

    /// Whether a cut filter reads a field made from the whole sweep, so it
    /// is computed once rather than per sample.
    fn whole_cut(&self, input: &str) -> bool {
        !self.base_fields.iter().any(|(n, _)| n == input)
    }

    /// Compile filters for this model; problems are `name: reason`.
    /// Filters on probes are compiled with their probes (ADR 0063).
    fn compile_filters(&self, entries: &[FilterEntry]) -> (Filters, Vec<String>) {
        let entries: Vec<FilterEntry> = entries
            .iter()
            .filter(|e| {
                !crate::observers::clocked(entries)
                    .iter()
                    .any(|d| d.name == e.name)
            })
            .cloned()
            .collect();
        let entries = &entries[..];
        let params = param_keys(&self.flat, &self.ids, self.sim.model());
        // Cuts may take any grid an expression makes in a 2D sweep.
        let mut scalars = self.output_names.clone();
        scalars.extend(
            entries
                .iter()
                .filter(|e| e.type_name == "expression")
                .map(|e| e.name.clone()),
        );
        let grids = self.source_grids(&scalars);
        Filters::compile(entries, &self.output_names, &params, &grids)
    }

    /// An empty history for the current mode: signals against time, or
    /// samples of a sweep with the swept parameters as signals and, for a
    /// two-parameter grid, every signal reshaped as a grid; then the cut
    /// filters' fields.
    fn rebuild_history(&mut self) {
        self.build_history();
        self.reset_probes();
    }

    /// The layout of a sample and an empty history keeping what is wired.
    fn build_history(&mut self) {
        let names = self.scalar_names();
        let mut fields = self.base_fields.clone();
        let mut whole = Vec::new();
        let mut coordinates = self.base_coordinates.clone();
        let mut scalars = names.clone();
        if let Some(sw) = &self.sweep {
            scalars.extend(sw.keys().iter().cloned());
            if let Some((nx, ny)) = sw.grid() {
                let shape = FieldShape::Grid { nx, ny };
                whole.extend(scalars.iter().map(|n| (grid_name(n), shape)));
                // The swept parameters place every result's cells.
                for (axis, key) in sw.keys().iter().enumerate() {
                    coordinates.extend(names.iter().map(|n| Coordinate {
                        name: grid_name(key),
                        of: grid_name(n),
                        axis,
                    }));
                }
            }
        }
        let mut labels = Vec::new();
        for c in &self.filters.cuts {
            if self.whole_cut(&c.input) {
                whole.extend(c.fields());
            } else {
                fields.extend(c.fields());
            }
            let coordinate = c.coordinate();
            // For labels only the coordinates' names matter.
            let mut g = Geometry::cells(c.nx, c.ny);
            for k in coordinates.iter().filter(|k| k.of == c.input && k.axis < 2) {
                // A swept parameter's grid is labelled as the parameter.
                let swept = self.sweep.as_ref().and_then(|sw| {
                    sw.keys()
                        .iter()
                        .find(|key| grid_name(key) == k.name)
                        .cloned()
                });
                g.names[k.axis] = Some(swept.unwrap_or_else(|| k.name.clone()));
            }
            labels.extend(c.labels(&g));
            coordinates.extend(c.lines().into_iter().map(|l| Coordinate {
                name: coordinate.clone(),
                of: l,
                axis: 0,
            }));
        }
        let index = if self.sweep.is_some() { "sample" } else { "t" };
        let all: Vec<&(String, FieldShape)> = fields.iter().chain(&whole).collect();
        let of_kind = |grid: bool| -> Vec<String> {
            all.iter()
                .filter(|(_, s)| matches!(s, FieldShape::Grid { .. }) == grid)
                .map(|(n, _)| n.clone())
                .collect()
        };
        self.grids = of_kind(true);
        self.points = of_kind(false);
        self.lines = all
            .iter()
            .filter(|(_, s)| matches!(s, FieldShape::Grid { ny: 1, .. }))
            .map(|(n, _)| n.clone())
            .collect();
        self.layout_scalars = scalars;
        self.layout_fields = fields;
        self.all_coordinates = coordinates;
        (self.keep_signals, self.keep_fields) = self.kept();
        let kept_fields: Vec<(String, FieldShape)> = self
            .keep_fields
            .iter()
            .map(|&i| self.layout_fields[i].clone())
            .collect();
        let mut h = History::with_fields(
            index,
            self.keep_signals
                .iter()
                .map(|&i| self.layout_scalars[i].clone())
                .collect(),
            kept_fields,
        );
        for (name, shape) in whole {
            h.add_whole(&name, shape);
        }
        if let Some(sw) = &self.sweep {
            h.keep_all();
            h.set_unordered(sw.unordered());
            // A grid of a signal is labelled as the signal.
            if sw.is_grid() {
                for name in self.scalar_names().iter().chain(sw.keys()) {
                    h.set_label(&grid_name(name), name);
                }
            }
        }
        // Coordinates among what it records.
        let recorded = |n: &str| h.fields().iter().any(|f| f.name == n);
        let coordinates = self
            .all_coordinates
            .iter()
            .filter(|c| recorded(&c.name) && recorded(&c.of))
            .cloned()
            .collect();
        h.set_coordinates(coordinates);
        for (name, label) in labels {
            h.set_label(&name, &label);
        }
        self.replace_history(h);
    }

    /// What the probes record may have moved or gone: they start afresh,
    /// without what a step no longer yields.
    fn reset_probes(&mut self) {
        let mut specs: Vec<ProbeSpec> = Vec::new();
        {
            let (scalars, fields) = (&self.layout_scalars, &self.layout_fields);
            let recorded =
                |n: &str| scalars.iter().any(|m| m == n) || fields.iter().any(|(f, _)| f == n);
            for p in &self.probes {
                let mut spec = p.spec.clone();
                let had = !spec.record.is_empty();
                spec.record.retain(|r| {
                    let ok = recorded(r);
                    if !ok {
                        self.warnings
                            .push(format!("probe {}: '{r}' is no longer recorded", spec.name));
                    }
                    ok
                });
                if had && spec.record.is_empty() {
                    self.warnings.push(format!(
                        "probe {}: removed, it records nothing now",
                        spec.name
                    ));
                } else {
                    specs.push(spec);
                }
            }
        }
        self.probes.clear();
        if let Err(problems) = self.set_probes(specs) {
            self.warnings.extend(problems);
        }
    }

    /// Replace the probes besides the implicit one (ADR 0062). Each must
    /// record signals or fields this session records; otherwise nothing
    /// changes and the problems are returned. Probes record in time only,
    /// not in sweeps, and start empty.
    pub fn set_probes(&mut self, specs: Vec<ProbeSpec>) -> Result<(), Vec<String>> {
        self.install_probes(specs)?;
        self.build_derived();
        Ok(())
    }

    /// [`Session::set_probes`] without the filters that read probes.
    fn install_probes(&mut self, specs: Vec<ProbeSpec>) -> Result<(), Vec<String>> {
        let mut problems = Vec::new();
        let mut names: Vec<&str> = Vec::new();
        // Probes take from every sample a step yields (ADR 0063), not
        // from what the history keeps.
        let (scalars, field_list) = (&self.layout_scalars, &self.layout_fields);
        let mut built = Vec::new();
        for spec in &specs {
            if let Err(e) = spec.check() {
                problems.push(e);
                continue;
            }
            if names.contains(&spec.name.as_str()) {
                problems.push(format!("probe {}: the name is used twice", spec.name));
            }
            names.push(&spec.name);
            let mut signals = Vec::new();
            let mut fields = Vec::new();
            for r in &spec.record {
                if let Some(i) = scalars.iter().position(|n| n == r) {
                    signals.push(i);
                } else if let Some(i) = field_list.iter().position(|(f, _)| f == r) {
                    fields.push(i);
                } else {
                    problems.push(format!("probe {}: the model has no '{r}'", spec.name));
                }
            }
            let probe_fields: Vec<(String, FieldShape)> =
                fields.iter().map(|&i| field_list[i].clone()).collect();
            let probe_signals: Vec<String> = signals.iter().map(|&i| scalars[i].clone()).collect();
            let mut ph = History::with_fields("t", probe_signals, probe_fields);
            // Coordinates of what it records, when it records them too.
            let has = |n: &str| spec.record.iter().any(|r| r == n);
            ph.set_coordinates(
                self.all_coordinates
                    .iter()
                    .filter(|c| has(&c.name) && has(&c.of))
                    .cloned()
                    .collect(),
            );
            spec.keep.apply(&mut ph);
            built.push(Extra {
                spec: spec.clone(),
                trigger: Trigger::new(spec.every.clone()),
                history: SharedHistory::new(ph),
                signals,
                fields,
                pending: Vec::new(),
            });
        }
        let recorded_to = self.history.read().times().last().copied();
        if !problems.is_empty() {
            return Err(problems);
        }
        // A probe whose recording is unchanged keeps what it recorded; a
        // new one records from now on.
        let eps = self.eps();
        for b in &mut built {
            let same = |o: &&mut Extra| {
                o.spec.name == b.spec.name
                    && o.spec.record == b.spec.record
                    && o.spec.every == b.spec.every
                    && o.spec.keep == b.spec.keep
            };
            if let Some(old) = self.probes.iter_mut().find(same) {
                std::mem::swap(b, old);
                b.spec = old.spec.clone();
            } else {
                b.trigger.seek(recorded_to, eps);
            }
        }
        self.probes = built;
        Ok(())
    }

    /// The filters with an input from a probe (ADR 0063), in an order
    /// where each comes after those it reads. One whose recording and
    /// probes did not change keeps what it recorded.
    fn build_derived(&mut self) {
        let mut old: Vec<Derived> = std::mem::take(&mut self.derived);
        self.derived_names.clear();
        if self.sweep.is_some() {
            return;
        }
        let params = param_keys(&self.flat, &self.ids, self.sim.model());
        let mut waiting: Vec<FilterEntry> = crate::observers::clocked(&self.filter_entries);
        let mut notes = Vec::new();
        let recorded_to = self.history.read().times().last().copied();
        while !waiting.is_empty() {
            let before = waiting.len();
            let mut k = 0;
            while k < waiting.len() {
                match self.derived_inputs(&waiting[k], &waiting) {
                    Ok(None) => k += 1,
                    Ok(Some(inputs)) => {
                        let e = waiting.remove(k);
                        match self.compile_derived(e, inputs, &params, &mut old, recorded_to) {
                            Ok(d) => self.derived.push(d),
                            Err(p) => notes.push(p),
                        }
                    }
                    Err(p) => {
                        notes.push(p);
                        waiting.remove(k);
                    }
                }
            }
            if waiting.len() == before {
                for e in waiting.drain(..) {
                    notes.push(format!(
                        "filter {}: it reads itself through other filters; ignored",
                        e.name
                    ));
                }
            }
        }
        for d in &self.derived {
            let (s, f) = d.outputs();
            self.derived_names.extend(s.into_iter().chain(f));
        }
        // Filters are set again on every probe edit: say each once.
        for n in notes {
            if !self.warnings.contains(&n) {
                self.warnings.push(n);
            }
        }
    }

    /// The inputs of filter `e`, each with its source; `Ok(None)` while a
    /// filter it reads (among `waiting`) is not built yet.
    fn derived_inputs(
        &self,
        e: &FilterEntry,
        waiting: &[FilterEntry],
    ) -> Result<Option<Vec<Input>>, String> {
        let params = param_keys(&self.flat, &self.ids, self.sim.model());
        let reads: Vec<String> = crate::observers::filter_reads(e)
            .into_iter()
            .filter(|r| r != "t" && !params.contains(r))
            .collect();
        for p in e.probe.iter().chain(e.from.values()) {
            if !self.probes.iter().any(|x| x.spec.name == *p) {
                return Err(format!(
                    "filter {}: there is no probe '{p}' to run on",
                    e.name
                ));
            }
        }
        let mut inputs = Vec::new();
        for r in reads {
            if waiting
                .iter()
                .any(|w| w.name != e.name && w.outputs().contains(&r))
            {
                return Ok(None);
            }
            if let Some(j) = self.derived.iter().position(|d| {
                let (s, f) = d.outputs();
                s.contains(&r) || f.contains(&r)
            }) {
                let (s, f) = self.derived[j].outputs();
                let (at, field) = match s.iter().position(|n| *n == r) {
                    Some(i) => (i, false),
                    None => (f.iter().position(|n| *n == r).unwrap_or(0), true),
                };
                inputs.push(Input {
                    name: r,
                    src: Src::Derived(j),
                    at,
                    field,
                });
                continue;
            }
            let src = match e.source_of(&r) {
                None => Src::Model,
                Some(p) => {
                    let i = self
                        .probes
                        .iter()
                        .position(|x| x.spec.name == p)
                        .ok_or_else(|| {
                            format!("filter {}: there is no probe '{p}' to run on", e.name)
                        })?;
                    if !self.probes[i].spec.record.contains(&r) {
                        return Err(format!(
                            "filter {}: probe {p} does not record '{r}'; ignored",
                            e.name
                        ));
                    }
                    Src::Probe(i)
                }
            };
            let (at, field) = match self.layout_scalars.iter().position(|n| *n == r) {
                Some(i) => (i, false),
                None => match self.layout_fields.iter().position(|(n, _)| *n == r) {
                    Some(i) => (i, true),
                    None => {
                        return Err(format!(
                            "filter {}: the model has no '{r}'; ignored",
                            e.name
                        ))
                    }
                },
            };
            inputs.push(Input {
                name: r,
                src,
                at,
                field,
            });
        }
        Ok(Some(inputs))
    }

    /// Compile filter `e` on its inputs, keeping the recording of an
    /// equal one in `old`.
    fn compile_derived(
        &self,
        e: FilterEntry,
        inputs: Vec<Input>,
        params: &[String],
        old: &mut Vec<Derived>,
        recorded_to: Option<f64>,
    ) -> Result<Derived, String> {
        let scalars: Vec<String> = inputs
            .iter()
            .filter(|i| !i.field)
            .map(|i| i.name.clone())
            .collect();
        let shape_of = |i: &Input| match i.src {
            Src::Derived(j) => {
                let h = self.derived[j].history.read();
                h.fields()
                    .iter()
                    .find(|f| f.name == i.name)
                    .map(|f| f.shape)
            }
            _ => Some(self.layout_fields[i.at].1),
        };
        let grids: Vec<(String, FieldShape)> = inputs
            .iter()
            .filter(|i| i.field)
            .filter_map(|i| shape_of(i).map(|s| (i.name.clone(), s)))
            .collect();
        let (filters, problems) =
            Filters::compile(std::slice::from_ref(&e), &scalars, params, &grids);
        if let Some(p) = problems.into_iter().next() {
            return Err(format!("filter {p}; ignored"));
        }
        let mut sources: Vec<Src> = Vec::new();
        for i in &inputs {
            if !sources.contains(&i.src) {
                sources.push(i.src);
            }
        }
        let key: Vec<ProbeSpec> = sources
            .iter()
            .filter_map(|s| match s {
                Src::Probe(i) => Some(self.probes[*i].spec.clone()),
                _ => None,
            })
            .collect();
        let held = vec![None; inputs.len()];
        if let Some(k) = old.iter().position(|d| d.entry == e && d.key == key) {
            let mut d = old.remove(k);
            d.filters = filters;
            d.inputs = inputs;
            d.sources = sources;
            return Ok(d);
        }
        let mut d = Derived {
            entry: e,
            key,
            filters,
            inputs,
            sources,
            held,
            history: SharedHistory::new(History::new(Vec::new())),
            pending: Vec::new(),
            now: None,
        };
        let (names, _) = d.outputs();
        let fields: Vec<(String, FieldShape)> =
            d.filters.cuts.iter().flat_map(|c| c.fields()).collect();
        let mut h = History::with_fields("t", names, fields);
        let mut coordinates = Vec::new();
        for c in &d.filters.cuts {
            coordinates.extend(c.lines().into_iter().map(|l| Coordinate {
                name: c.coordinate(),
                of: l,
                axis: 0,
            }));
        }
        h.set_coordinates(coordinates);
        d.history = SharedHistory::new(h);
        // Mid-run, it starts from what its sources recorded so far.
        if recorded_to.is_some() {
            self.reseed(&mut d);
        }
        Ok(d)
    }

    /// Take a filter's held inputs again from its sources' recordings, as
    /// they stand (after a rewind, or when it is new).
    fn reseed(&self, d: &mut Derived) {
        for (k, i) in d.inputs.iter().enumerate() {
            let h = match i.src {
                Src::Model => None,
                Src::Probe(p) => Some(self.probes[p].history.clone()),
                Src::Derived(j) => Some(self.derived[j].history.clone()),
            };
            d.held[k] = h.and_then(|h| {
                let h = h.read();
                let last = h.len().checked_sub(1)?;
                if i.field {
                    let f = h.fields().iter().find(|f| f.name == i.name)?;
                    Some(f.frame(last).to_vec()).filter(|v| !v.is_empty())
                } else {
                    let s = h.resolve(&i.name)?;
                    Some(vec![h.value(s, last)])
                }
            });
        }
        d.now = None;
    }

    /// Run the filters with clocks of their own whose sources recorded at
    /// this sample (`fired`, by probe).
    fn run_derived(&mut self, t: f64, values: &[f64], frames: &[&[f64]], fired: &[bool]) {
        if self.derived.is_empty() {
            return;
        }
        let params = self.param_values();
        let coordinates = self.all_coordinates.clone();
        let mut produced = vec![false; self.derived.len()];
        for j in 0..self.derived.len() {
            let (before, rest) = self.derived.split_at_mut(j);
            let d = &mut rest[0];
            d.now = None;
            let did = |s: &Src| match *s {
                Src::Model => true,
                Src::Probe(i) => fired.get(i).copied().unwrap_or(false),
                Src::Derived(k) => produced[k],
            };
            let go = if d.entry.on_any() {
                d.sources.iter().any(did)
            } else {
                d.sources.iter().all(did)
            };
            if !go {
                continue;
            }
            for (k, i) in d.inputs.iter().enumerate() {
                if !did(&i.src) {
                    continue;
                }
                d.held[k] = Some(match (i.src, i.field) {
                    (Src::Derived(m), field) => {
                        let (v, f) = before[m].now.as_ref().expect("it produced");
                        if field {
                            f[i.at].clone()
                        } else {
                            vec![v[i.at]]
                        }
                    }
                    (_, true) => frames[i.at].to_vec(),
                    (_, false) => vec![values[i.at]],
                });
            }
            // Until every input has a value (`any`), it gives NaN.
            let ready = d.held.iter().all(Option::is_some);
            let mut vals: Vec<f64> = d
                .inputs
                .iter()
                .zip(&d.held)
                .filter(|(i, _)| !i.field)
                .map(|(_, h)| h.as_ref().map_or(f64::NAN, |v| v[0]))
                .collect();
            let base = vals.len();
            derive(&d.filters.exprs, t, &mut vals, &params);
            let mut out: Vec<f64> = vals.split_off(base);
            if !ready {
                out.iter_mut().for_each(|v| *v = f64::NAN);
            }
            let field_names: Vec<String> = d
                .inputs
                .iter()
                .filter(|i| i.field)
                .map(|i| i.name.clone())
                .collect();
            let field_frames: Vec<Vec<f64>> = d
                .inputs
                .iter()
                .zip(&d.held)
                .filter(|(i, _)| i.field)
                .map(|(_, h)| h.clone().unwrap_or_default())
                .collect();
            let cut_frames = derived_cuts(&d.filters, &field_names, &field_frames, &coordinates);
            d.pending.push(Pending {
                index: t,
                values: out.clone(),
                frames: cut_frames.clone(),
            });
            d.flush(false);
            d.now = Some((out, cut_frames));
            produced[j] = true;
        }
    }

    /// The outputs of filters with clocks of their own, as views bind
    /// them (ADR 0063).
    pub fn derived_names(&self) -> &[String] {
        &self.derived_names
    }

    /// What each filter with a clock of its own recorded, by its name.
    pub fn derived_histories(&self) -> Vec<(String, SharedHistory)> {
        self.derived
            .iter()
            .map(|d| (d.entry.name.clone(), d.history.clone()))
            .collect()
    }

    /// Set the probes from the model file's form (ADR 0062), as the
    /// canvas edits them: what each probe recorded is kept if its
    /// recording did not change.
    pub fn set_probe_entries(&mut self, entries: &[ProbeEntry]) -> Result<(), Vec<String>> {
        if self.sweep.is_some() {
            self.file.controls.probes = entries.to_vec();
            return Ok(());
        }
        self.set_probes(entries.iter().map(ProbeSpec::from_entry).collect())
    }

    /// The probes in the model file's form.
    pub fn probe_entries(&self) -> Vec<ProbeEntry> {
        if self.sweep.is_some() {
            return self.file.controls.probes.clone();
        }
        self.probes.iter().map(|p| p.spec.to_entry()).collect()
    }

    /// Write the files of the probes that have one (`to`) into `dir`, as
    /// recorded so far: the files written, and notes on what was left out.
    pub fn write_probe_files(
        &mut self,
        dir: &std::path::Path,
    ) -> std::io::Result<(Vec<std::path::PathBuf>, Vec<String>)> {
        self.flush(true);
        let mut written = Vec::new();
        let mut notes = Vec::new();
        for p in &self.probes {
            if let Some(to) = &p.spec.to {
                let (w, n) = crate::probe::write_files(&p.history.read(), dir, to)?;
                written.extend(w);
                notes.extend(n.into_iter().map(|n| format!("probe {}: {n}", p.spec.name)));
            }
        }
        Ok((written, notes))
    }

    /// Where a probe's node sits on the data layer, without restarting
    /// its recording.
    pub fn place_probe(&mut self, name: &str, at: Option<[f64; 2]>) -> Result<(), String> {
        if self.sweep.is_some() {
            let p = self
                .file
                .controls
                .probes
                .iter_mut()
                .find(|p| p.name == name);
            p.ok_or_else(|| format!("no probe '{name}'"))?.canvas = at;
            return Ok(());
        }
        let p = self
            .probes
            .iter_mut()
            .find(|p| p.spec.name == name)
            .ok_or_else(|| format!("no probe '{name}'"))?;
        p.spec.canvas = at.map(|p| p.map(|v| (v * 10.0).round() / 10.0));
        Ok(())
    }

    /// Build the file's probes, one by one: one that records what this
    /// session does not is dropped with a warning.
    fn load_probes(&mut self) {
        let mut kept: Vec<ProbeSpec> = Vec::new();
        for (k, e) in self.file.controls.probes.clone().iter().enumerate() {
            let mut specs = kept.clone();
            specs.push(ProbeSpec::from_entry(e));
            match self.install_probes(specs.clone()) {
                Ok(()) => kept = specs,
                Err(problems) => {
                    for p in problems {
                        self.warnings
                            .push(format!("controls.probes[{k}]: {p}; ignored"));
                    }
                }
            }
        }
        if let Err(problems) = self.set_probes(kept) {
            self.warnings.extend(problems);
        }
    }

    /// The probes besides the implicit one, by name.
    pub fn probe_names(&self) -> Vec<&str> {
        self.probes.iter().map(|p| p.spec.name.as_str()).collect()
    }

    /// The probes besides the implicit one, as set.
    pub fn probe_specs(&self) -> Vec<&ProbeSpec> {
        self.probes.iter().map(|p| &p.spec).collect()
    }

    /// What probe `name` has recorded, shared for reading (`None` if
    /// there is no such probe). The implicit probe is [`Session::signals`].
    pub fn probe(&self, name: &str) -> Option<SharedHistory> {
        self.probes
            .iter()
            .find(|p| p.spec.name == name)
            .map(|p| p.history.clone())
    }

    /// The frames of the cut filters that are per sample (`whole` false)
    /// or made from the whole sweep (`whole` true), in order, given the
    /// frames of the fields they read.
    fn cut_frames<'a>(
        &self,
        frame: impl Fn(&str) -> Option<&'a [f64]>,
        whole: bool,
    ) -> Vec<Vec<f64>> {
        let coordinates = &self.all_coordinates;
        self.filters
            .cuts
            .iter()
            .filter(|c| self.whole_cut(&c.input) == whole)
            .flat_map(|c| match frame(&c.input) {
                Some(input) => {
                    let g = c.geometry(coordinates, &frame);
                    c.frames(&g, input)
                }
                // Recorded as NaN.
                None => vec![Vec::new(); c.settings.count() + 1],
            })
            .collect()
    }

    /// The per-sample cut frames from the frames of the base fields.
    fn sample_cuts(&self, before: &[&[f64]]) -> Vec<Vec<f64>> {
        let fields = &self.base_fields;
        self.cut_frames(
            |name| {
                fields
                    .iter()
                    .position(|(f, _)| f == name)
                    .and_then(|i| before.get(i).copied())
            },
            false,
        )
    }

    pub fn time(&self) -> f64 {
        self.sim.time()
    }

    pub fn labels(&self) -> Option<MethodLabels> {
        self.sim.labels()
    }

    /// Advance one communication step and record it.
    pub fn step(&mut self) -> Result<(), SimError> {
        self.apply_due();
        self.sim.step(self.dt)?;
        self.record();
        self.steps_since_snapshot += 1;
        if self.steps_since_snapshot >= SNAPSHOT_EVERY {
            self.steps_since_snapshot = 0;
            self.snapshots.push(self.sim.snapshot());
            let held = |s: &[Snapshot]| s.iter().map(Snapshot::values).sum::<usize>();
            while self.snapshots.len() > 2
                && (self.snapshots.len() > MAX_SNAPSHOTS || held(&self.snapshots) > SNAPSHOT_VALUES)
            {
                thin(&mut self.snapshots);
            }
        }
        Ok(())
    }

    /// Step until the time reaches `t` (within rounding), recording every
    /// step: how headless interfaces run a model (ADR 0039). A swept model
    /// is not run in time; this does nothing for it.
    pub fn run_until(&mut self, t: f64) -> Result<(), SimError> {
        if self.sweep.is_some() {
            return Ok(());
        }
        while self.time() < t - 1e-9 * self.dt.max(t.abs()) {
            self.step()?;
        }
        Ok(())
    }

    /// The run has reached the file's `t_end`. Returns true once, then the
    /// run may continue past it.
    pub fn take_end_reached(&mut self) -> bool {
        let reached = self
            .stop_at
            .is_some_and(|end| self.time() >= end - 1e-9 * self.dt.max(end.abs()));
        if reached {
            self.stop_at = None;
        }
        reached
    }

    /// Every parameter: the declared ones (the model's, then each
    /// instance's), then every block's in flat block order.
    pub fn params(&self) -> Vec<ParamRow> {
        let model = self.sim.model();
        let mut rows = Vec::new();
        for d in &self.plan.declared {
            let (owner, name) = d.key.rsplit_once('.').unwrap_or(("", &d.key));
            let formula = match (&d.formula, d.editable) {
                (Some(f), _) => Some(f.text.clone()),
                // Set inside another subsystem: fixed by its definition.
                (None, false) => Some(short(d.value)),
                (None, true) => None,
            };
            rows.push(ParamRow {
                key: d.key.clone(),
                block: None,
                owner: owner.into(),
                name: name.into(),
                formula,
                unit: d.unit.clone(),
                description: d.description.clone(),
                suggested: d.range.unwrap_or_else(|| suggested_range(d.value)),
                initial_condition: false,
                value: d.value,
            });
        }
        for entry in &self.flat.blocks {
            let id = self.ids[&entry.name];
            let (Ok(block), Ok(values)) = (model.block(id), model.params(id)) else {
                continue;
            };
            for (p, &value) in block.spec().params.iter().zip(values) {
                rows.push(ParamRow {
                    key: format!("{}.{}", entry.name, p.name),
                    block: Some(id),
                    owner: entry.name.clone(),
                    formula: self
                        .plan
                        .formula_of(&entry.name, &p.name)
                        .map(|f| f.text.clone()),
                    name: p.name.clone(),
                    unit: if p.unit == Unit::DIMENSIONLESS {
                        String::new()
                    } else {
                        p.unit.to_string()
                    },
                    description: p.description.clone(),
                    suggested: [p.suggested_min, p.suggested_max],
                    initial_condition: p.initial_condition,
                    value,
                });
            }
        }
        rows
    }

    /// Change a parameter by its key (`"block.param"`, or a declared
    /// parameter's), recomputing what formulas set from it (ADR 0043).
    /// The change is live input: it is recorded in the run's input track
    /// (ADR 0058), after any events due now, and drops recorded input
    /// ahead (running on from a rewind with new input is a new branch).
    pub fn set_param_key(&mut self, key: &str, value: f64) -> Result<(), String> {
        let live = self.sweep.is_none();
        if live {
            self.apply_due();
        }
        self.set_value(key, value)?;
        // Before the run starts, a value the schedule also sets is the
        // model's own: the run starts from it.
        if self.time() == 0.0 && self.snapshots.len() == 1 {
            for (k, v) in &mut self.track.base {
                if k == key {
                    *v = value;
                }
            }
        }
        if live {
            let t = self.time();
            let ahead = self.track.next_recorded;
            self.track.recorded.truncate(ahead);
            self.track.recorded.push(InputEvent {
                t,
                key: key.to_string(),
                value,
            });
            self.track.next_recorded = self.track.recorded.len();
        }
        self.after_param_change()
    }

    /// Live input from a device (keys, a view's pointer, ADR 0058): as
    /// [`Session::set_param_key`], but nothing happens (and nothing is
    /// recorded) when an input block's value is already `value`, so a held
    /// pointer or key sends only changes. Returns whether it changed.
    pub fn set_input(&mut self, key: &str, value: f64) -> Result<bool, String> {
        if self.sweep.is_none() {
            self.apply_due();
        }
        if let Some((block, name)) = key.rsplit_once('.') {
            if let Some(&id) = self.ids.get(block) {
                if self.sim.param(id, name).ok() == Some(value) {
                    return Ok(false);
                }
            }
        }
        self.set_param_key(key, value).map(|()| true)
    }

    /// Set a value by key, not recorded.
    fn set_value(&mut self, key: &str, value: f64) -> Result<(), String> {
        set_key(&mut self.sim, &mut self.plan, &self.ids, key, value)
    }

    /// Change a parameter while running, as live input (recorded, like
    /// [`Session::set_param_key`]). Initial conditions apply on
    /// [`Session::restart`], or at once if the run has not started.
    pub fn set_param(&mut self, block: BlockId, name: &str, value: f64) -> Result<(), String> {
        let flat = self
            .ids
            .iter()
            .find(|(_, &id)| id == block)
            .map(|(n, _)| n.clone())
            .ok_or("no such block")?;
        self.set_param_key(&format!("{flat}.{name}"), value)
    }

    fn after_param_change(&mut self) -> Result<(), String> {
        if self.sweep.is_some() {
            self.resample();
        } else if self.time() == 0.0 && self.snapshots.len() == 1 {
            self.restart().map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Start again at t = 0 from the current parameter values. History and
    /// plots start over.
    pub fn restart(&mut self) -> Result<(), SimError> {
        if self.sweep.is_some() {
            self.resample();
            return Ok(());
        }
        // A new run: recorded input is dropped; what the schedule sets
        // starts from the model's own values and is set again.
        self.track.recorded.clear();
        for (key, value) in self.track.base.clone() {
            if let Err(e) = self.set_value(&key, value) {
                self.warnings.push(e);
            }
        }
        self.restart_keeping_recorded()
    }

    /// Restart at t = 0 with the events at t = 0 applied first, so they
    /// set where the run starts, and the rest still ahead.
    fn restart_keeping_recorded(&mut self) -> Result<(), SimError> {
        self.track.seek(0.0, 0.0);
        // Back to t = 0 so the events there are due.
        self.sim.restart(0.0)?;
        // The model as it starts, before the events: a run's hash and the
        // saved model agree, and replaying applies the events again.
        if let Err(e) = self.capture_start() {
            self.warnings.push(e);
        }
        self.apply_due();
        self.sim.restart(0.0)?;
        self.snapshots = vec![self.sim.snapshot()];
        self.steps_since_snapshot = 0;
        self.stop_at = self.file.run.t_end;
        self.history_mut().clear();
        for p in &mut self.probes {
            p.pending.clear();
            p.history.write().clear();
            p.trigger.seek(None, 0.0);
        }
        for d in &mut self.derived {
            d.pending.clear();
            d.history.write().clear();
            d.held.iter_mut().for_each(|h| *h = None);
            d.now = None;
        }
        self.record();
        Ok(())
    }

    /// Start from the current state (ADR 0042): every block's state now
    /// becomes where the run starts, then the run restarts at t = 0 from
    /// it. Blocks whose initial-condition parameters can say it exactly
    /// (`x0` for a state `x`) get those parameters; the others get a
    /// `start` entry in the file with their state and tables. Returns a
    /// note per block.
    pub fn start_from_now(&mut self) -> Result<Vec<String>, String> {
        if self.sweep.is_some() {
            return Err("a swept model has no state to start from".into());
        }
        let t = self.time();
        let mut notes = Vec::new();
        for i in 0..self.flat.blocks.len() {
            let name = self.flat.blocks[i].name.clone();
            let id = self.ids[&name];
            let now = self.sim.current(id);
            if now.state.is_empty() && now.tables.is_empty() {
                continue;
            }
            let model = self.sim.model();
            let block = model.block(id).map_err(|e| e.to_string())?;
            let spec = block.spec();
            let params = model.params(id).map_err(|e| e.to_string())?;
            let by_params = now
                .tables
                .is_empty()
                .then(|| block.initial_params(&now.state, params))
                .flatten();
            if let Some(p) = by_params {
                let changed: Vec<(String, f64)> = spec
                    .params
                    .iter()
                    .zip(p.iter().zip(params))
                    .filter(|(_, (new, old))| new != old)
                    .map(|(q, (new, _))| (q.name.clone(), *new))
                    .collect();
                let shown: Vec<String> = changed
                    .iter()
                    .map(|(n, v)| format!("{n} = {}", crate::session::short(*v)))
                    .collect();
                for (n, v) in changed {
                    self.sim.set_param(id, &n, v).map_err(|e| e.to_string())?;
                }
                self.sim.set_start(id, None).map_err(|e| e.to_string())?;
                self.flat.blocks[i].start = None;
                notes.push(format!("{name}: {}", shown.join(", ")));
            } else {
                let entry = simsandbox_file::StartEntry::from_start(&now, spec);
                let rows: usize = now.tables.iter().map(Vec::len).sum();
                let what = if rows > 0 {
                    format!("{} values and {rows} rows", now.state.len())
                } else {
                    format!("{} values", now.state.len())
                };
                self.sim
                    .set_start(id, Some(now))
                    .map_err(|e| e.to_string())?;
                self.flat.blocks[i].start = Some(entry);
                notes.push(format!("{name}: start state saved ({what})"));
            }
        }
        if notes.is_empty() {
            return Err("no block has a state to start from".into());
        }
        self.sync_file()?;
        self.restart().map_err(|e| e.to_string())?;
        notes.insert(0, format!("Starting from the state at t = {}", short(t)));
        Ok(notes)
    }

    /// Where a block sits on the node canvas (ADR 0042); layout only, so
    /// the run goes on.
    pub fn set_position(&mut self, block: &str, at: [f64; 2]) -> Result<(), String> {
        self.set_position_in(&crate::diagram::Scope::Model, block, at)
    }

    /// Where a block, or a subsystem's port node, sits on the canvas of one
    /// level (ADR 0043); layout only.
    pub fn set_position_in(
        &mut self,
        scope: &crate::diagram::Scope,
        block: &str,
        at: [f64; 2],
    ) -> Result<(), String> {
        let mut file = std::mem::take(&mut self.file);
        let edit = crate::diagram::Edit::Move {
            block: block.into(),
            at,
        };
        let result = crate::diagram::apply_in(&mut file, scope, &Registry::new(), &edit);
        self.file = file;
        result.map(|_| ())
    }

    /// Where a filter's node sits on the data layer (ADR 0050): saved with
    /// the filters, without evaluating them again.
    pub fn place_filter(&mut self, name: &str, at: Option<[f64; 2]>) -> Result<(), String> {
        let f = self
            .filter_entries
            .iter_mut()
            .find(|f| f.name == name)
            .ok_or_else(|| format!("no filter '{name}'"))?;
        f.canvas = at.map(|p| p.map(|v| (v * 10.0).round() / 10.0));
        Ok(())
    }

    /// Whether any block starts from a saved state instead of its
    /// parameters.
    pub fn has_start_states(&self) -> bool {
        self.flat.blocks.iter().any(|b| b.start.is_some())
    }

    /// Bring the file as written up to date with what runs.
    fn sync_file(&mut self) -> Result<(), String> {
        self.flat
            .capture_params(self.sim.model(), &self.ids)
            .map_err(|e| e.to_string())?;
        self.file.capture_from(&self.flat, &self.plan);
        Ok(())
    }

    /// Start every block from its parameters again, and restart.
    pub fn forget_start_states(&mut self) -> Result<(), String> {
        for b in &mut self.flat.blocks {
            if b.start.take().is_some() {
                self.sim
                    .set_start(self.ids[&b.name], None)
                    .map_err(|e| e.to_string())?;
            }
        }
        self.sync_file()?;
        self.restart().map_err(|e| e.to_string())
    }

    /// The model file with the current parameters, step, default speed and
    /// views (ADR 0012, ADR 0013, ADR 0015), ready to write.
    pub fn save_json(
        &mut self,
        speed: f64,
        views: Option<Vec<ViewEntry>>,
    ) -> Result<String, String> {
        self.save_json_with(speed, views, None)
    }

    /// As [`Self::save_json`], with the app's arrangement of its panes
    /// (ADR 0053) if given; otherwise the file's is kept.
    pub fn save_json_with(
        &mut self,
        speed: f64,
        views: Option<Vec<ViewEntry>>,
        layout: Option<Option<simsandbox_file::layout::Layout>>,
    ) -> Result<String, String> {
        if let Some(l) = layout {
            self.file.controls.layout = l;
        }
        self.flat
            .capture_params(self.sim.model(), &self.ids)
            .map_err(|e| e.to_string())?;
        self.file.capture_from(&self.flat, &self.plan);
        self.file.run.dt = self.dt;
        self.file.controls.speed = Some(speed);
        if let Some(v) = views {
            self.file.controls.views = v;
        }
        self.file.controls.filters = self.filter_entries.clone();
        // A swept model records no probes: the file keeps its own.
        self.file.controls.probes = self.probe_entries();
        if let Some(sw) = &self.sweep {
            self.file.sample = Some(sw.sampling.clone());
        }
        Ok(self.file.to_json())
    }

    /// Keep the file as it is now, with the current parameters, as the
    /// file this run starts from.
    fn capture_start(&mut self) -> Result<(), String> {
        self.flat
            .capture_params(self.sim.model(), &self.ids)
            .map_err(|e| e.to_string())?;
        self.file.capture_from(&self.flat, &self.plan);
        self.file.run.dt = self.dt;
        self.start_file = self.file.clone();
        Ok(())
    }

    /// The sweep, if the model is swept instead of run.
    pub fn sweep(&self) -> Option<&Sweep> {
        self.sweep.as_deref()
    }

    /// Change what is swept and how (ADR 0028), and recompute. Filters that
    /// no longer apply (a cut through a grid the sweep no longer makes) are
    /// dropped; the returned notes say which.
    pub fn set_sampling(&mut self, sampling: Sampling) -> Result<Vec<String>, String> {
        if self.sweep.is_none() {
            return Err("this model has states, so it runs in time instead".into());
        }
        let plan = sampling
            .plan(&self.flat, self.sim.model(), &self.ids)
            .map_err(|e| e.to_string())?;
        self.sweep = Some(Arc::new(Sweep::new(sampling, plan, &self.ids)?));
        let entries = self.filter_entries.clone();
        let (filters, problems) = self.compile_filters(&entries);
        self.filter_entries = kept_entries(&filters, &entries);
        self.filters = filters;
        self.resample();
        Ok(problems
            .into_iter()
            .map(|p| format!("filter {p}; removed"))
            .collect())
    }

    /// Evaluate the outputs at every sample of the sweep: at once, or when
    /// deferred, by later calls to [`Session::resume_resample`].
    fn resample(&mut self) {
        if self.sweep.is_none() {
            return;
        }
        self.rebuild_history();
        let names = self.scalar_names().len();
        let grid = self.sweep.as_ref().is_some_and(|sw| sw.is_grid());
        self.job = Some(ResampleJob {
            next: 0,
            columns: vec![Vec::new(); if grid { names + 2 } else { 0 }],
            started: Instant::now(),
        });
        if !self.deferred {
            self.resume_resample(None);
        }
    }

    /// The sweep's progress while it is being evaluated: samples done and
    /// in all.
    pub fn resampling(&self) -> Option<(usize, usize)> {
        let job = self.job.as_ref()?;
        let total = self.sweep.as_ref().map_or(0, |sw| sw.plan.points.len());
        Some((job.next, total))
    }

    /// Evaluate more of the sweep, for at most `budget` (all of it if
    /// `None`). The model is left with the panel's values after each call.
    /// Returns true when the sweep is complete.
    pub fn resume_resample(&mut self, budget: Option<Duration>) -> bool {
        let (Some(sw), Some(mut job)) = (self.sweep.clone(), self.job.take()) else {
            return true;
        };
        let start = Instant::now();
        let originals: Vec<f64> = sw
            .targets
            .iter()
            .map(|(b, p)| self.sim.param(*b, p).unwrap_or(f64::NAN))
            .collect();
        let set = |sim: &mut Simulator, values: &[f64]| {
            sw.targets
                .iter()
                .zip(values)
                .all(|((b, p), &v)| sim.set_param(*b, p, v).is_ok())
                && sim.restart(0.0).is_ok()
        };
        let names = self.scalar_names().len();
        // For a grid over two parameters: every scalar signal, then the
        // swept parameters, sample by sample (cell (i, j) at j * nx + i).
        while job.next < sw.plan.points.len() {
            if budget.is_some_and(|b| start.elapsed() >= b) {
                break;
            }
            let k = job.next;
            let point = &sw.plan.points[k];
            let ok = set(&mut self.sim, point);
            let mut values = if ok {
                self.sample(k as f64)
            } else {
                vec![f64::NAN; names]
            };
            values.extend_from_slice(point);
            for (c, v) in job.columns.iter_mut().zip(&values) {
                c.push(*v);
            }
            let owned: Vec<Vec<f64>> = if ok {
                frames(&self.sim, &self.field_sources)
                    .into_iter()
                    .map(<[f64]>::to_vec)
                    .collect()
            } else {
                vec![Vec::new(); self.field_sources.len()]
            };
            let frames: Vec<&[f64]> = owned.iter().map(Vec::as_slice).collect();
            let cuts = self.sample_cuts(&frames);
            self.push(k as f64, &values, &frames, &cuts);
            job.next += 1;
        }
        // Leave the model as the panel shows it.
        set(&mut self.sim, &originals);
        if job.next < sw.plan.points.len() {
            self.job = Some(job);
            return false;
        }
        if sw.is_grid() {
            let grids: Vec<(String, Vec<f64>)> = self
                .scalar_names()
                .iter()
                .chain(sw.keys())
                .map(|n| grid_name(n))
                .zip(std::mem::take(&mut job.columns))
                .collect();
            let whole = self.cut_frames(
                |name| {
                    grids
                        .iter()
                        .find(|(n, _)| n == name)
                        .map(|(_, f)| f.as_slice())
                },
                true,
            );
            let names: Vec<String> = self
                .filters
                .cuts
                .iter()
                .filter(|c| self.whole_cut(&c.input))
                .flat_map(|c| c.fields().into_iter().map(|(n, _)| n))
                .collect();
            let mut h = self.history_mut();
            for (name, frame) in grids.into_iter().chain(names.into_iter().zip(whole)) {
                h.set_whole(&name, frame);
            }
        }
        self.last_sweep = Some(job.started.elapsed());
        true
    }

    /// Whether dragged sliders apply while dragged (ADR 0037): every method
    /// is real-time and, for a sweep, the last complete one took at most
    /// two frames.
    pub fn real_time(&self) -> bool {
        let methods = self
            .labels()
            .is_some_and(|l| l.speed == simsandbox_core::block::SpeedClass::RealTime);
        let sweep = match &self.sweep {
            None => true,
            // The last complete sweep, so a drag does not change the rule
            // halfway while each change restarts the sweep.
            Some(_) => self.last_sweep.is_some_and(|d| d <= REAL_TIME_SWEEP),
        };
        methods && sweep
    }

    /// Earliest and latest time the run can be rewound to.
    pub fn history_range(&self) -> (f64, f64) {
        let first = self.snapshots.first().map_or(0.0, |s| s.t);
        let last = self.snapshots.last().map_or(first, |s| s.t);
        (first, last)
    }

    /// Rewind to the latest snapshot at or before `t`. Everything recorded
    /// after it is discarded, so running on starts a new branch.
    pub fn rewind_to(&mut self, t: f64) {
        let keep = self.snapshots.iter().rposition(|s| s.t <= t).unwrap_or(0);
        self.snapshots.truncate(keep + 1);
        let snap = &self.snapshots[keep];
        self.sim.restore(snap);
        let t0 = snap.t;
        // Input from then on is ahead again: running on replays it.
        let eps = self.eps();
        self.track.seek(t0, eps);
        self.stop_at = self.file.run.t_end.filter(|&end| end > t0);
        self.history_mut().truncate_after(t0);
        for p in &mut self.probes {
            p.flush(true);
            p.history.write().truncate_after(t0);
            p.trigger.seek(Some(t0), eps);
        }
        // Filters with clocks of their own hold what their sources
        // recorded by then (ADR 0063).
        for j in 0..self.derived.len() {
            let mut d = std::mem::replace(
                &mut self.derived[j],
                Derived {
                    entry: FilterEntry::expression("", "0", Vec::new()),
                    key: Vec::new(),
                    filters: Filters::default(),
                    inputs: Vec::new(),
                    sources: Vec::new(),
                    held: Vec::new(),
                    history: SharedHistory::new(History::new(Vec::new())),
                    pending: Vec::new(),
                    now: None,
                },
            );
            d.flush(true);
            d.history.write().truncate_after(t0);
            self.reseed(&mut d);
            self.derived[j] = d;
        }
        self.steps_since_snapshot = 0;
    }

    fn record(&mut self) {
        let t = self.sim.time();
        let values = self.sample(t);
        let owned: Vec<Vec<f64>> = frames(&self.sim, &self.field_sources)
            .into_iter()
            .map(<[f64]>::to_vec)
            .collect();
        let frames: Vec<&[f64]> = owned.iter().map(Vec::as_slice).collect();
        let cuts = self.sample_cuts(&frames);
        self.push(t, &values, &frames, &cuts);
    }

    /// One sample: the output signals, then the expression filters computed
    /// from them, earlier filters, the current parameter values and `t`.
    fn sample(&self, t: f64) -> Vec<f64> {
        let mut values = self.output_values();
        self.append_derived(t, &mut values);
        values
    }

    fn append_derived(&self, t: f64, values: &mut Vec<f64>) {
        let exprs = &self.filters.exprs;
        if exprs.is_empty() {
            return;
        }
        // Slots: t, the output signals, the expression filters, then every
        // parameter.
        let outputs = self.sources.len();
        let mut vars = Vec::with_capacity(1 + outputs + exprs.len() + 16);
        vars.push(t);
        vars.extend_from_slice(&values[..outputs]);
        vars.extend(std::iter::repeat_n(0.0, exprs.len()));
        vars.extend(self.param_values());
        for (k, f) in exprs.iter().enumerate() {
            let v = f.expr.eval(&vars);
            vars[1 + outputs + k] = v;
            values.push(v);
        }
    }

    /// Every parameter's value, in [`Session::params`] order.
    fn param_values(&self) -> Vec<f64> {
        let model = self.sim.model();
        self.flat
            .blocks
            .iter()
            .flat_map(|e| model.params(self.ids[&e.name]).unwrap_or(&[]))
            .copied()
            .collect()
    }

    /// The filters as written (ADR 0032).
    pub fn filters(&self) -> &[FilterEntry] {
        &self.filter_entries
    }

    /// Replace the filters. All must be usable, or nothing changes and the
    /// problems are returned. The recorded history is kept and the filters
    /// are recomputed for it, using current parameter values.
    pub fn set_filters(&mut self, entries: Vec<FilterEntry>) -> Result<(), Vec<String>> {
        let mut problems = Vec::new();
        let mut names = Vec::new();
        for e in &entries {
            if names.contains(&e.name) || self.output_names.contains(&e.name) {
                problems.push(format!("{}: the name is already used", e.name));
            }
            names.push(e.name.clone());
        }
        let (filters, more) = self.compile_filters(&entries);
        problems.extend(more);
        if !problems.is_empty() {
            return Err(problems);
        }
        self.filter_entries = kept_entries(&filters, &entries);
        self.filters = filters;
        if self.sweep.is_some() {
            self.resample();
            return Ok(());
        }
        // Replay the recorded outputs and fields through the new filters;
        // probes start afresh from now.
        drop(self.history_mut());
        let old = self.replace_history(History::new(Vec::new()));
        self.build_history();
        self.replay_history(&old, true);
        self.reset_probes();
        Ok(())
    }

    fn output_values(&self) -> Vec<f64> {
        self.sources
            .iter()
            .map(|src| {
                self.sim
                    .output(src.block, &src.port)
                    .and_then(|v| v.get(src.index))
                    .copied()
                    .unwrap_or(f64::NAN)
            })
            .collect()
    }
}

/// A filter's cut frames for one sample, from its input fields' frames,
/// named by `names`.
fn derived_cuts(
    filters: &Filters,
    names: &[String],
    frames: &[Vec<f64>],
    coordinates: &[Coordinate],
) -> Vec<Vec<f64>> {
    let frame = |name: &str| -> Option<&[f64]> {
        names
            .iter()
            .position(|n| n == name)
            .and_then(|i| frames.get(i))
            .map(Vec::as_slice)
    };
    filters
        .cuts
        .iter()
        .flat_map(|c| match frame(&c.input) {
            Some(input) => {
                let g = c.geometry(coordinates, frame);
                c.frames(&g, input)
            }
            None => vec![Vec::new(); c.settings.count() + 1],
        })
        .collect()
}

/// The filters as saved: the model's, compiled, with their inputs written
/// out; those on probes as given (ADR 0063); in the given order.
fn kept_entries(filters: &Filters, given: &[FilterEntry]) -> Vec<FilterEntry> {
    let compiled = filters.entries(given);
    let derived = crate::observers::clocked(given);
    given
        .iter()
        .filter_map(|g| match &g.probe {
            _ if derived.iter().any(|d| d.name == g.name) => Some(g.clone()),
            _ => compiled.iter().find(|c| c.name == g.name).cloned(),
        })
        .collect()
}

/// Evaluate expression filters into `values`, which holds `base` signals
/// first: slots are `t`, those signals, the filters, then `params`.
fn derive(exprs: &[crate::filters::ExprFilter], t: f64, values: &mut Vec<f64>, params: &[f64]) {
    if exprs.is_empty() {
        return;
    }
    let base = values.len();
    let mut vars = Vec::with_capacity(1 + base + exprs.len() + params.len());
    vars.push(t);
    vars.extend_from_slice(values);
    vars.extend(std::iter::repeat_n(0.0, exprs.len()));
    vars.extend_from_slice(params);
    for (k, f) in exprs.iter().enumerate() {
        let v = f.expr.eval(&vars);
        vars[1 + base + k] = v;
        values.push(v);
    }
}

/// What views read, as their bindings are written (ADR 0063).
pub fn view_reads(views: &[ViewEntry]) -> Vec<String> {
    model_reads(
        views
            .iter()
            .flat_map(|v| v.bind.values().flat_map(simsandbox_file::Binding::signals))
            .collect(),
    )
}

/// Of these bindings, what is read from the model directly: not `t` and
/// not a probe's recording; sorted, once each.
fn model_reads(mut reads: Vec<String>) -> Vec<String> {
    reads.retain(|r| r != "t" && !r.starts_with("probe:"));
    reads.sort();
    reads.dedup();
    reads
}

/// One note per algebraic loop (ADR 0040), saying how it is solved.
pub fn loop_notes(sim: &Simulator) -> Vec<String> {
    let method = sim.model().loop_method().name();
    sim.loops()
        .into_iter()
        .map(|l| {
            format!(
                "algebraic loop {}: solved at every step by '{method}' (run.loops)",
                l.join(" -> ")
            )
        })
        .collect()
}

/// The 2D grids a cut filter can take: recorded grids that are not
/// themselves coordinates (in a two-parameter sweep, every result).
pub fn cut_inputs(h: &History) -> Vec<String> {
    h.fields()
        .iter()
        .filter(|f| matches!(f.shape, FieldShape::Grid { nx, ny } if nx >= 2 && ny >= 2))
        .filter(|f| !h.coordinates().iter().any(|c| c.name == f.name))
        .map(|f| f.name.clone())
        .collect()
}

/// Where the cells of grid `input` lie, from the newest sample.
pub fn cut_geometry(h: &History, input: &str) -> Geometry {
    let (nx, ny) = match h.fields().iter().find(|f| f.name == input).map(|f| f.shape) {
        Some(FieldShape::Grid { nx, ny }) => (nx, ny),
        _ => (1, 1),
    };
    let newest = |n: &str| {
        h.fields()
            .iter()
            .find(|f| f.name == n)
            .filter(|_| !h.is_empty())
            .map(|f| f.frame(h.len() - 1))
    };
    h.coordinates()
        .iter()
        .filter(|c| c.of == input && c.axis < 2)
        .fold(Geometry::cells(nx, ny), |g, c| match newest(&c.name) {
            Some(f) => g.with_coordinate(c.axis, &c.name, f),
            None => g,
        })
}

/// A new cut filter through `input`: along x at four values spread over
/// its rows, each on a row so it reads it exactly.
pub fn default_cut(h: &History, name: &str, input: &str) -> FilterEntry {
    let g = cut_geometry(h, input);
    let ny = g.ny;
    let mut rows: Vec<usize> = (0..4)
        .map(|k| (((k as f64 + 0.5) / 4.0 * ny as f64) as usize).min(ny.max(1) - 1))
        .collect();
    rows.dedup();
    let at: Vec<f64> = rows.iter().map(|&j| g.centres[1][j]).collect();
    FilterEntry {
        name: name.into(),
        type_name: "cut".into(),
        inputs: vec![input.into()],
        canvas: None,
        probe: None,
        from: Default::default(),
        when: None,
        settings: [
            ("mode".to_string(), serde_json::Value::from("along_x")),
            ("at".to_string(), serde_json::Value::from(at)),
        ]
        .into(),
    }
}

/// The current value of every recorded field (empty if missing, which the
/// history records as NaN).
fn frames<'a>(sim: &'a Simulator, sources: &[FieldSource]) -> Vec<&'a [f64]> {
    sources
        .iter()
        .map(|f| {
            sim.output(f.block, &f.port)
                .filter(|v| f.len.is_none_or(|n| v.len() == n))
                .unwrap_or(&[])
        })
        .collect()
}

/// Every parameter as `"block.param"`, in the order [`Session::params`]
/// lists them (file block order, then each block's parameters).
fn param_keys(file: &ModelFile, ids: &BTreeMap<String, BlockId>, model: &Model) -> Vec<String> {
    let mut keys = Vec::new();
    for entry in &file.blocks {
        if let Ok(b) = model.block(ids[&entry.name]) {
            for p in &b.spec().params {
                keys.push(format!("{}.{}", entry.name, p.name));
            }
        }
    }
    keys
}

impl Sweep {
    fn new(
        sampling: Sampling,
        plan: Plan,
        ids: &BTreeMap<String, BlockId>,
    ) -> Result<Self, String> {
        let targets = plan
            .keys
            .iter()
            .map(|key| {
                key.split_once('.')
                    .and_then(|(b, p)| Some((*ids.get(b)?, p.to_string())))
                    .ok_or_else(|| format!("no parameter '{key}'"))
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            sampling,
            plan,
            targets,
        })
    }
}

/// A slider range for a declared parameter that gives none: from zero to
/// twice its value (or ±1 around zero).
fn suggested_range(v: f64) -> [f64; 2] {
    if v == 0.0 {
        [-1.0, 1.0]
    } else if v > 0.0 {
        [0.0, 2.0 * v]
    } else {
        [2.0 * v, 0.0]
    }
}

/// A number in its shortest form, at most 6 significant digits.
pub(crate) fn short(v: f64) -> String {
    let r = format!("{v:.6e}").parse::<f64>().unwrap_or(v);
    let s = format!("{r}");
    if s.len() > 12 {
        format!("{r:.6e}")
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::examples::{registry, EXAMPLES};

    fn lorenz() -> Session {
        Session::load("lorenz", EXAMPLES[0].text, &registry()).unwrap()
    }

    #[test]
    fn examples_load_and_plot_what_their_controls_ask_for() {
        let s = lorenz();
        assert_eq!(
            s.signals().names(),
            ["lorenz.xyz[0]", "lorenz.xyz[1]", "lorenz.xyz[2]"]
        );
        assert!(s.warnings.is_empty());
        for e in EXAMPLES {
            Session::load(e.name, e.text, &registry()).unwrap();
        }
    }

    #[test]
    fn rewind_then_rerun_reproduces_the_run() {
        let mut s = lorenz();
        for _ in 0..300 {
            s.step().unwrap();
        }
        let reference: Vec<_> = s.signals().points(0).to_vec();

        s.rewind_to(1.234);
        // Latest snapshot at or before 1.234 (snapshots every 0.1 s).
        assert!((s.time() - 1.2).abs() < 1e-9, "{}", s.time());
        assert!(s.signals().points(0).last().unwrap().x <= s.time());
        while s.signals().points(0).len() < reference.len() {
            s.step().unwrap();
        }
        assert_eq!(s.signals().points(0), reference.as_slice());

        s.rewind_to(0.0);
        assert_eq!(s.time(), 0.0);
        assert_eq!(s.signals().points(0).len(), 1);
        assert_eq!(s.history_range(), (0.0, 0.0));
    }

    fn param(s: &Session, key: &str) -> f64 {
        s.params().iter().find(|r| r.key == key).unwrap().value
    }

    #[test]
    fn initial_conditions_apply_on_restart_or_at_once_before_running() {
        let mut s = lorenz();
        let id = s.ids["lorenz"];
        let row = s
            .params()
            .into_iter()
            .find(|r| r.key == "lorenz.x0")
            .unwrap();
        assert!(row.initial_condition);

        // Before the run starts, the plot's first point moves at once.
        s.set_param(id, "x0", 5.0).unwrap();
        assert_eq!(s.signals().points(0)[0].y, 5.0);

        for _ in 0..50 {
            s.step().unwrap();
        }
        s.set_param(id, "x0", -3.0).unwrap();
        assert!(s.time() > 0.0, "a running model is not restarted");
        s.restart().unwrap();
        assert_eq!(s.time(), 0.0);
        assert_eq!(s.signals().points(0).len(), 1);
        assert_eq!(s.signals().points(0)[0].y, -3.0);
        assert_eq!(s.history_range(), (0.0, 0.0));
    }

    #[test]
    fn saving_records_params_step_controls_and_speed() {
        let mut s = lorenz();
        let id = s.ids["lorenz"];
        s.set_param(id, "rho", 150.0).unwrap();
        s.dt = 0.005;
        s.file
            .controls
            .params
            .entry("lorenz.beta".into())
            .or_default()
            .slider = true;
        let text = s.save_json(2.5, None).unwrap();

        let again = Session::load("again", &text, &registry()).unwrap();
        assert_eq!(param(&again, "lorenz.rho"), 150.0);
        assert_eq!(again.dt, 0.005);
        assert_eq!(again.file.controls.speed, Some(2.5));
        assert!(again.file.controls.params["lorenz.beta"].slider);
        assert!(again.file.controls.params["lorenz.rho"].slider);
    }

    #[test]
    fn the_run_pauses_once_at_t_end() {
        let text = EXAMPLES[0]
            .text
            .replace(r#""dt": 0.01"#, r#""dt": 0.01, "t_end": 0.5"#);
        let mut s = Session::load("l", &text, &registry()).unwrap();
        let mut steps = 0;
        while !s.take_end_reached() {
            s.step().unwrap();
            steps += 1;
        }
        assert_eq!(steps, 50);
        s.step().unwrap();
        assert!(!s.take_end_reached(), "Run continues past t_end");
        // Rewinding before t_end re-arms it.
        s.rewind_to(0.2);
        assert!(!s.take_end_reached());
        while !s.take_end_reached() {
            s.step().unwrap();
        }
        assert!((s.time() - 0.5).abs() < 1e-9);
    }

    fn example(name_part: &str) -> Session {
        let e = EXAMPLES
            .iter()
            .find(|e| e.name.contains(name_part))
            .unwrap();
        Session::load(e.name, e.text, &registry()).unwrap()
    }

    fn exprs(pairs: &[(&str, &str)]) -> Vec<FilterEntry> {
        pairs
            .iter()
            .map(|(n, e)| FilterEntry::expression(n, e, Vec::new()))
            .collect()
    }

    fn values(s: &Session, signal: &str) -> Vec<f64> {
        let r = s.signals().resolve(signal).unwrap();
        s.signals().values(r)
    }

    /// A grid over the given `(key, range, points)`.
    fn grid_of(params: &[(&str, [f64; 2], usize)]) -> Sampling {
        Sampling {
            design: Design::Grid,
            params: params
                .iter()
                .map(|&(k, r, n)| simsandbox_file::SweepControl {
                    param: k.into(),
                    range: Some(r),
                    points: Some(n),
                })
                .collect(),
            count: None,
            seed: None,
            values: Vec::new(),
        }
    }

    #[test]
    fn formula_models_are_swept_not_run() {
        let mut s = example("Resonance curve");
        let sw = s.sweep().unwrap();
        assert_eq!(sw.keys(), ["spring.w"]);
        assert!(!sw.unordered());
        assert_eq!(s.signals().index(), "sample");
        assert_eq!(s.signals().len(), 600);
        assert!(!s.signals().unordered());
        // The swept parameter is a signal like the outputs.
        assert_eq!(
            s.signals().names(),
            ["spring.amplitude", "spring.lag", "spring.w"]
        );
        assert_eq!(s.available(&s.signals()).swept, ["spring.w"]);
        let w = values(&s, "spring.w");
        assert_eq!((w[0], w[599]), (0.0, 3.0));
        let amp = values(&s, "spring.amplitude");
        // At w = 0 the swing is F/k; near w = 1 it peaks at about F/c.
        assert!((amp[0] - 1.0).abs() < 1e-12);
        let peak = amp.iter().cloned().fold(0.0, f64::max);
        assert!((peak - 5.0).abs() < 0.1, "peak {peak}");

        // Moving a slider recomputes at once; the swept value is left alone.
        let id = s.ids["spring"];
        s.set_param(id, "c", 0.1).unwrap();
        let peak = values(&s, "spring.amplitude")
            .into_iter()
            .fold(0.0, f64::max);
        assert!((peak - 10.0).abs() < 0.3, "peak {peak}");
        assert_eq!(param(&s, "spring.w"), 1.0);

        // Sweep something else.
        s.set_sampling(grid_of(&[("spring.c", [0.1, 1.0], 10)]))
            .unwrap();
        assert_eq!(values(&s, "spring.c")[9], 1.0);
        assert!(s.signals().resolve("spring.w").is_none());
        assert!(s
            .set_sampling(grid_of(&[("spring.c", [1.0, 0.1], 10)]))
            .is_err());
        assert!(s
            .set_sampling(grid_of(&[("spring.q", [0.0, 1.0], 10)]))
            .is_err());
        assert_eq!(s.sweep().unwrap().keys(), ["spring.c"]);
        let saved = s.save_json(1.0, None).unwrap();
        let file: serde_json::Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(
            file["sample"],
            serde_json::json!({"design": "grid", "params": [{"param": "spring.c", "range": [0.1, 1.0], "points": 10}]})
        );
    }

    #[test]
    fn unordered_designs_record_samples_drawn_as_points() {
        let mut s = example("Resonance samples");
        let sw = s.sweep().unwrap();
        assert_eq!(sw.plan.design, Design::Sobol);
        assert!(sw.unordered() && !sw.is_grid());
        assert!(s.signals().unordered());
        assert_eq!(s.signals().len(), 2000);
        assert!(s.available(&s.signals()).grids.is_empty());
        let (w, c, amp) = (
            values(&s, "spring.w"),
            values(&s, "spring.c"),
            values(&s, "spring.amplitude"),
        );
        for k in [0, 1, 777, 1999] {
            let exact = 1.0 / ((1.0 - w[k].powi(2)).powi(2) + (c[k] * w[k]).powi(2)).sqrt();
            assert!((amp[k] - exact).abs() < 1e-12 * exact, "sample {k}");
        }
        // The first Sobol point is the lower corner, the second the centre.
        assert_eq!((w[0], c[0]), (0.0, 0.02));
        assert_eq!((w[1], c[1]), (1.5, 0.51));

        // Another design, a third parameter; saved and loaded back alike.
        let mut sampling = s.sweep().unwrap().sampling.clone();
        sampling.design = Design::LatinHypercube;
        sampling.count = Some(64);
        sampling.seed = Some(9);
        sampling.params.push(simsandbox_file::SweepControl {
            param: "spring.m".into(),
            range: None,
            points: None,
        });
        assert!(s.set_sampling(sampling).unwrap().is_empty());
        assert_eq!(s.signals().len(), 64);
        let m = values(&s, "spring.m");
        // The slider's range, one sample in each 64th of it.
        let mut slices: Vec<usize> = m
            .iter()
            .map(|v| ((v - 0.2) / 3.8 * 64.0) as usize)
            .collect();
        slices.sort();
        assert_eq!(slices, (0..64).collect::<Vec<_>>());
        let text = s.save_json(1.0, None).unwrap();
        let again = Session::load("again", &text, &registry()).unwrap();
        assert_eq!(again.sweep(), s.sweep());
        assert_eq!(values(&again, "spring.m"), m);
    }

    #[test]
    fn two_parameter_grids_also_offer_every_signal_as_a_grid() {
        let mut s = example("Resonance curve");
        s.set_filters(exprs(&[("half", "spring.amplitude/2")]))
            .unwrap();
        let (nx, ny) = (31, 20);
        s.set_sampling(grid_of(&[
            ("spring.w", [0.0, 3.0], nx),
            ("spring.c", [0.1, 2.0], ny),
        ]))
        .unwrap();
        assert!(s.sweep().unwrap().is_grid());
        assert!(s.signals().unordered());
        assert_eq!(s.signals().len(), nx * ny);
        let h = s.signals();
        let a = s.available(&h);
        assert_eq!(
            a.scalars,
            [
                "spring.amplitude",
                "spring.lag",
                "half",
                "spring.w",
                "spring.c"
            ]
        );
        // The swept parameters place every result's cells.
        assert_eq!(
            a.axis_choices("grid.half"),
            ["grid.spring.w", "grid.spring.c"]
        );
        assert_eq!(
            a.axis_of("grid.spring.amplitude", "grid.spring.c"),
            Ok(Some(1))
        );
        assert!(a.axis_of("grid.spring.amplitude", "t").is_err());
        assert_eq!(
            a.grids,
            [
                "grid.spring.amplitude",
                "grid.spring.lag",
                "grid.half",
                "grid.spring.w",
                "grid.spring.c"
            ]
        );
        drop(h);
        let grid = |name: &str| {
            let h = s.signals();
            let f = h.fields().iter().find(|f| f.name == name).unwrap();
            assert_eq!(f.shape, FieldShape::Grid { nx, ny });
            f.frame(0).to_vec()
        };
        let (amp, half, w, c) = (
            grid("grid.spring.amplitude"),
            grid("grid.half"),
            grid("grid.spring.w"),
            grid("grid.spring.c"),
        );
        // The grids are the samples reshaped.
        assert_eq!(amp, values(&s, "spring.amplitude"));
        // Cell (i, j) at j * nx + i: w along x, c along y; F = k = m = 1.
        for (i, j) in [(0, 0), (10, 0), (10, 7), (30, 19)] {
            let cell = j * nx + i;
            assert_eq!(w[cell], 3.0 * i as f64 / 30.0);
            assert!((c[cell] - (0.1 + 1.9 * j as f64 / 19.0)).abs() < 1e-15);
            let exact =
                1.0 / ((1.0 - w[cell].powi(2)).powi(2) + (c[cell] * w[cell]).powi(2)).sqrt();
            assert!((amp[cell] - exact).abs() < 1e-12 * exact, "{i},{j}");
            assert_eq!(half[cell], amp[cell] / 2.0);
        }
        // The panel's values are left alone.
        assert_eq!(param(&s, "spring.c"), 0.2);

        // Too many points, or one parameter twice, are refused.
        assert!(s
            .set_sampling(grid_of(&[
                ("spring.w", [0.0, 3.0], 1000),
                ("spring.c", [0.1, 2.0], 1000)
            ]))
            .is_err());
        assert!(s
            .set_sampling(grid_of(&[
                ("spring.w", [0.0, 3.0], 9),
                ("spring.w", [0.1, 2.0], 9)
            ]))
            .is_err());

        // Saved, and loaded back as the same grid.
        let text = s.save_json(1.0, None).unwrap();
        let again = Session::load("again", &text, &registry()).unwrap();
        assert_eq!(again.sweep(), s.sweep());
        assert_eq!(
            again
                .signals()
                .fields()
                .iter()
                .find(|f| f.name == "grid.spring.amplitude")
                .unwrap()
                .frame(0),
            &amp[..]
        );

        // Back to one parameter: no grids, ordered samples.
        s.set_sampling(grid_of(&[("spring.w", [0.0, 3.0], 50)]))
            .unwrap();
        assert_eq!(s.signals().len(), 50);
        assert!(!s.signals().unordered());
        assert!(s.available(&s.signals()).coordinates.is_empty());
        assert!(s.available(&s.signals()).grids.is_empty());
        assert_eq!(values(&s, "half").len(), 50);
    }

    #[test]
    fn maps_step_once_per_communication_step() {
        let mut s = example("Logistic");
        assert!(s.sweep().is_none());
        s.step().unwrap();
        let x = values(&s, "population.x");
        assert_eq!(x[0], 0.2);
        assert!((x[1] - 3.7 * 0.2 * 0.8).abs() < 1e-15);
        assert_eq!(s.time(), 1.0);
    }

    #[test]
    fn derived_signals_are_recorded_and_can_be_redefined() {
        let mut s = example("Coupled");
        // The example defines the total energy, which stays constant: the
        // two blocks are fused, so there is no coupling error (ADR 0023).
        assert!(s.filters().iter().any(|f| f.name == "energy"));
        for _ in 0..500 {
            s.step().unwrap();
        }
        let e = values(&s, "energy");
        assert_eq!(e.len(), s.signals().len());
        let drift = e.iter().map(|v| (v - e[0]).abs()).fold(0.0, f64::max);
        assert!(drift < 1e-9 * e[0], "energy drifted by {drift} of {}", e[0]);

        // Redefining recomputes the whole history (of what a view reads,
        // ADR 0063).
        s.set_view_reads(vec!["ax2".into()]);
        s.set_filters(exprs(&[("ax2", "a.x^2 + 0*t")])).unwrap();
        let ax = values(&s, "a.x");
        let ax2 = values(&s, "ax2");
        assert_eq!(ax2.len(), ax.len());
        assert!(ax.iter().zip(&ax2).all(|(x, y)| (x * x - y).abs() < 1e-15));
        assert!(s.signals().resolve("energy").is_none());

        // Problems leave everything as it was.
        let problems = s
            .set_filters(exprs(&[("b", "nope + 1"), ("a.x", "1")]))
            .unwrap_err();
        assert!(problems.iter().any(|p| p.starts_with("b:")), "{problems:?}");
        assert!(
            problems.iter().any(|p| p.starts_with("a.x:")),
            "{problems:?}"
        );
        assert!(s.signals().resolve("ax2").is_some());
        // Saved as a filter that names its inputs (ADR 0032).
        let saved = s.save_json(1.0, None).unwrap();
        let file: serde_json::Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(
            file["controls"]["filters"],
            serde_json::json!([{"name": "ax2", "type": "expression", "inputs": ["a.x"], "expr": "a.x^2 + 0*t"}])
        );
    }

    #[test]
    fn expression_filters_may_read_each_other() {
        let mut s = example("Coupled");
        s.set_view_reads(vec!["double".into()]);
        s.set_filters(exprs(&[("double", "2*sq"), ("sq", "a.x^2")]))
            .unwrap();
        for _ in 0..10 {
            s.step().unwrap();
        }
        let ax = values(&s, "a.x");
        let double = values(&s, "double");
        assert!(ax
            .iter()
            .zip(&double)
            .all(|(x, d)| (2.0 * x * x - d).abs() < 1e-14));
        // A cycle is refused.
        assert!(s.set_filters(exprs(&[("p", "q"), ("q", "p")])).is_err());
    }

    #[test]
    fn cut_filters_cut_two_parameter_sweeps_into_lines() {
        let mut s = example("Resonance map");
        let cut = default_cut(&s.signals(), "res", "grid.spring.amplitude");
        let mut filters = s.filters().to_vec();
        filters.push(cut.clone());
        s.set_filters(filters).unwrap();
        let h = s.signals();
        let a = s.available(&h);
        // Four cuts along w at fixed c, and their coordinate w.
        let lines: Vec<&str> = a
            .lines
            .iter()
            .map(String::as_str)
            .filter(|l| l.starts_with("res"))
            .collect();
        assert_eq!(lines, ["res[0]", "res[1]", "res[2]", "res[3]", "res.x"]);
        assert_eq!(a.axis_choices("res[0]"), ["t", "res.x"]);
        drop(h);
        let grid = |name: &str| {
            let h = s.signals();
            let f = h.fields().iter().find(|f| f.name == name).unwrap();
            f.frame(0).to_vec()
        };
        let (amp, c, w) = (
            grid("grid.spring.amplitude"),
            grid("grid.spring.c"),
            grid("res.x"),
        );
        let at = cut.settings["at"].as_array().unwrap()[1].as_f64().unwrap();
        // Default cuts lie on rows, so they read them exactly.
        let row = (0..120).find(|&j| c[j * 200] == at).unwrap();
        assert_eq!(grid("res[1]"), amp[row * 200..row * 200 + 200].to_vec());
        assert_eq!(w, grid("grid.spring.w")[..200].to_vec());
        let h = s.signals();
        let label = h.label(h.resolve("res[1]").unwrap());
        assert!(label.starts_with("spring.c = "), "{label}");
        assert_eq!(h.label(h.resolve("res.x").unwrap()), "spring.w");
        drop(h);
        // Saved with its input.
        let saved = s.save_json(1.0, None).unwrap();
        let again = Session::load("again", &saved, &registry()).unwrap();
        assert!(again.warnings.is_empty(), "{:?}", again.warnings);
        assert_eq!(again.filters(), s.filters());

        // A sweep with no grid drops the cuts, saying so.
        let notes = s
            .set_sampling(grid_of(&[("spring.w", [0.0, 3.0], 50)]))
            .unwrap();
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(s.filters().is_empty());
    }

    #[test]
    fn field_outputs_are_recorded_whole_and_offered_to_views() {
        let mut s = example("rod");
        let h = s.signals();
        let a = s.available(&h);
        assert!(a.scalars.is_empty());
        assert_eq!(a.grids, ["rod.temperature", "rod.position"]);
        assert_eq!(a.block_types, ["eq.heat_rod"]);
        drop(h);
        for _ in 0..20 {
            s.step().unwrap();
        }
        assert_eq!(s.signals().len(), 21);
        assert_eq!(s.signals().fields()[0].frame(20).len(), 50);
        // The hot spot cools.
        let peak = |k: usize| {
            s.signals().fields()[0]
                .frame(k)
                .iter()
                .cloned()
                .fold(0.0, f64::max)
        };
        assert!(peak(20) < peak(0));

        // Rewind and derived signals keep the frames in step with the times.
        s.rewind_to(0.5);
        let frames = |s: &Session| -> Vec<Vec<f64>> {
            (0..s.signals().len())
                .map(|k| s.signals().fields()[0].frame(k).to_vec())
                .collect()
        };
        let before = frames(&s);
        assert_eq!(before.len(), 11);
        s.set_filters(exprs(&[("one", "1 + 0*t")])).unwrap();
        assert_eq!(frames(&s), before);

        let s = example("Orbiting");
        assert_eq!(s.available(&s.signals()).points, ["disc.positions"]);
        assert_eq!(s.signals().fields()[0].frame(0).len(), 400);
    }

    #[test]
    fn bad_files_report_errors() {
        let err = Session::load("x", "{", &registry()).err().unwrap();
        assert!(err.contains("line"), "{err}");
    }
}

#[cfg(test)]
mod start_tests {
    use super::*;
    use crate::examples::{registry, EXAMPLES};

    fn open(part: &str) -> Session {
        let e = EXAMPLES.iter().find(|e| e.name.contains(part)).unwrap();
        Session::load(e.name, e.text, &registry()).unwrap()
    }

    fn reopen(s: &mut Session) -> Session {
        let text = s.save_json(1.0, None).unwrap();
        Session::load("again", &text, &registry()).unwrap()
    }

    #[test]
    fn equation_blocks_start_from_now_through_their_initial_parameters() {
        let mut s = open("Cruise");
        s.run_until(60.0).unwrap();
        let (car, cruise) = (s.ids["car"], s.ids["cruise"]);
        let v = s.sim.state(car).to_vec();
        let i = s.sim.state(cruise).to_vec();
        let notes = s.start_from_now().unwrap();
        assert!(notes[0].contains("t = 60"), "{notes:?}");
        assert!(
            notes.iter().any(|n| n.starts_with("car: v0 = ")),
            "{notes:?}"
        );
        assert_eq!(s.time(), 0.0);
        assert_eq!(s.sim.state(car), v);
        assert_eq!(s.sim.state(cruise), i);
        assert!(!s.has_start_states(), "parameters say it");
        // Saved and opened again, it starts there.
        let again = reopen(&mut s);
        assert_eq!(again.sim.state(car), v);
        assert!(!again.file.to_json().contains("\"start\""));
    }

    #[test]
    fn fields_and_populations_save_a_start_state_that_reproduces() {
        let mut s = open("Sheep");
        s.run_until(5.0).unwrap();
        let (grass, sheep) = (s.ids["grass"], s.ids["sheep"]);
        let field = s.sim.state(grass).to_vec();
        let animals = s.sim.current(sheep).tables;
        let notes = s.start_from_now().unwrap();
        assert!(
            notes
                .iter()
                .any(|n| n.contains("grass: start state saved (1600 values)")),
            "{notes:?}"
        );
        assert!(s.has_start_states());
        assert_eq!(s.sim.state(grass), field);
        assert_eq!(s.sim.current(sheep).tables, animals);
        // The same run from the saved file as from here.
        s.run_until(2.0).unwrap();
        let mut again = reopen(&mut s);
        assert_eq!(again.sim.state(grass), field);
        again.run_until(2.0).unwrap();
        s.restart().unwrap();
        s.run_until(2.0).unwrap();
        assert_eq!(again.sim.state(grass), s.sim.state(grass));
        assert_eq!(again.sim.current(sheep), s.sim.current(sheep));
        // Forgetting goes back to the parameters' start.
        again.forget_start_states().unwrap();
        assert!(again
            .sim
            .state(grass)
            .iter()
            .all(|&g| g == again.sim.state(grass)[0]));
        assert!(!again.has_start_states());
    }

    #[test]
    fn bad_start_states_are_load_errors_at_their_path() {
        let e = EXAMPLES.iter().find(|e| e.name.contains("Lorenz")).unwrap();
        let text = e.text.replacen(
            "\"type\": \"eq.lorenz\",",
            "\"type\": \"eq.lorenz\", \"start\": {\"state\": [1, 2]},",
            1,
        );
        let check = crate::describe::check(&text, &registry());
        assert_eq!(check["errors"][0]["path"], "blocks[0].start", "{check}");
        let swept = open("Resonance curve");
        let mut swept = swept;
        assert!(swept.start_from_now().is_err());
    }
}

#[cfg(test)]
mod subsystem_tests {
    use super::*;
    use crate::examples::{registry, EXAMPLES};
    use simsandbox_file::ParamValue;

    fn platoon() -> Session {
        let e = EXAMPLES
            .iter()
            .find(|e| e.name.contains("Platoon"))
            .unwrap();
        Session::load(e.name, e.text, &registry()).unwrap()
    }

    #[test]
    fn declared_parameters_are_listed_first_and_formulas_are_shown_not_edited() {
        let s = platoon();
        let rows = s.params();
        let keys: Vec<&str> = rows.iter().take(5).map(|r| r.key.as_str()).collect();
        assert_eq!(keys, ["ki", "kp", "car1.ki", "car1.kp", "car1.mass"]);
        let car1_kp = rows.iter().find(|r| r.key == "car1.kp").unwrap();
        assert_eq!(car1_kp.formula.as_deref(), Some("kp"));
        assert_eq!(car1_kp.owner, "car1");
        let mass = rows.iter().find(|r| r.key == "car2.mass").unwrap();
        assert!(mass.formula.is_none() && mass.block.is_none());
        let m = rows.iter().find(|r| r.key == "car2/car.m").unwrap();
        assert_eq!((m.value, m.formula.as_deref()), (1500.0, Some("mass")));
        let c = rows.iter().find(|r| r.key == "car2/car.c").unwrap();
        assert!(c.formula.is_none() && c.block.is_some());
    }

    #[test]
    fn changing_a_declared_parameter_reaches_every_block_using_it() {
        let mut s = platoon();
        s.set_param_key("kp", 900.0).unwrap();
        for car in ["car1", "car2", "car3"] {
            let id = s.ids[&format!("{car}/control")];
            assert_eq!(s.sim.model().param(id, "kp").unwrap(), 900.0);
        }
        s.set_param_key("car3.mass", 3000.0).unwrap();
        assert_eq!(s.sim.model().param(s.ids["car3/car"], "m").unwrap(), 3000.0);
        assert!(s
            .set_param_key("car1.kp", 1.0)
            .unwrap_err()
            .contains("computed"));
        assert!(s
            .set_param_key("car1/car.m", 1.0)
            .unwrap_err()
            .contains("computed"));
        s.set_param_key("car1/car.c", 80.0).unwrap();

        // Saved, the formulas stay formulas and only changes are written.
        let text = s.save_json(4.0, None).unwrap();
        let (f, _) = ModelFile::from_json(&text).unwrap();
        assert_eq!(f.params["kp"].value, 900.0);
        let car3 = &f.blocks[3].params;
        assert_eq!(car3["mass"], ParamValue::Number(3000.0));
        assert_eq!(car3["kp"], ParamValue::Formula("kp".into()));
        let car1 = &f.blocks[1].params;
        assert_eq!(car1["car.c"], ParamValue::Number(80.0));
        assert_eq!(car1.len(), 4, "{car1:?}");
        assert_eq!(
            f.subsystems["follower"].blocks[0].params["c"],
            ParamValue::Number(50.0)
        );
        let again = Session::load("again", &text, &registry()).unwrap();
        assert_eq!(
            again.sim.model().param(again.ids["car1/car"], "c").unwrap(),
            80.0
        );
        assert_eq!(
            again
                .sim
                .model()
                .param(again.ids["car2/control"], "kp")
                .unwrap(),
            900.0
        );
    }

    #[test]
    fn instances_start_from_a_reached_state_through_their_own_entry() {
        let mut s = platoon();
        s.run_until(20.0).unwrap();
        let v = s.sim.state(s.ids["car3/car"]).to_vec();
        s.start_from_now().unwrap();
        let text = s.save_json(4.0, None).unwrap();
        let (f, _) = ModelFile::from_json(&text).unwrap();
        // The cars' initial speeds are parameters, set for each instance.
        assert_eq!(f.blocks[3].params["car.v0"], ParamValue::Number(v[0]));
        assert_eq!(
            f.subsystems["follower"].blocks[0].params["v0"],
            ParamValue::Number(0.0)
        );
        let again = Session::load("again", &text, &registry()).unwrap();
        assert_eq!(again.sim.state(again.ids["car3/car"]), v);
    }
}

#[cfg(test)]
mod input_tests {
    use super::*;
    use crate::examples::registry;

    /// A cart pushed by an input: dv/dt = F/m - c*v.
    const CART: &str = r#"{"format": 1, "run": {"dt": 0.01},
      "blocks": [
        {"name": "push", "type": "input.value", "settings": {"unit": "N", "min": "-50", "max": "50"}},
        {"name": "cart", "type": "eq.equations", "method": {"name": "rk4"},
         "equations": ["input F [N]", "param m [kg] = 2", "param c [1/s] = 0.5",
                       "state v [m/s]", "dv/dt = F/m - c*v"]}],
      "connections": [{"from": "push.y", "to": "cart.F"}]}"#;

    fn cart(text: &str) -> Session {
        Session::load("cart", text, &registry()).unwrap()
    }

    /// Where two recordings first differ, for a readable failure.
    fn same(a: &[Vec<(u64, u64)>], b: &[Vec<(u64, u64)>], names: &[String]) {
        assert_eq!(a.len(), b.len(), "signal count");
        for (i, (x, y)) in a.iter().zip(b).enumerate() {
            let k = x.iter().zip(y).position(|(p, q)| p != q);
            assert!(
                k.is_none() && x.len() == y.len(),
                "signal {} differs at sample {:?} (lengths {} and {}): {:?} vs {:?}",
                names[i],
                k,
                x.len(),
                y.len(),
                k.map(|k| (f64::from_bits(x[k].0), f64::from_bits(x[k].1))),
                k.map(|k| (f64::from_bits(y[k].0), f64::from_bits(y[k].1)))
            );
        }
    }

    /// Every recorded signal, as bits.
    fn signals(s: &Session) -> Vec<Vec<(u64, u64)>> {
        let h = s.signals();
        (0..h.names().len())
            .map(|i| {
                h.points(i)
                    .iter()
                    .map(|p| (p.x.to_bits(), p.y.to_bits()))
                    .collect()
            })
            .collect()
    }

    /// Steer by hand: push at 0.5 s, change the mass at 1.2 s, let go at
    /// 2 s; run to 3 s.
    fn steer(s: &mut Session) {
        s.run_until(0.5).unwrap();
        s.set_param_key("push.y", 40.0).unwrap();
        s.run_until(1.2).unwrap();
        s.set_param_key("cart.m", 4.0).unwrap();
        s.run_until(2.0).unwrap();
        s.set_param_key("push.y", 0.0).unwrap();
        s.run_until(3.0).unwrap();
    }

    #[test]
    fn live_input_is_recorded_and_replays_bit_for_bit() {
        let mut s = cart(CART);
        steer(&mut s);
        let reference = signals(&s);
        assert_eq!(s.recorded().len(), 3);
        // The push moved the cart.
        let v = s.signals().points(1).last().unwrap().y;
        assert!(v > 1.0, "{v}");
        // Rewind and run on: the recorded input is replayed.
        s.rewind_to(0.3);
        assert!(s.replaying());
        s.run_until(3.0).unwrap();
        assert!(!s.replaying());
        same(&signals(&s), &reference, s.signals().names());
        // Saved as a run file, it replays the same in a fresh session.
        let run = s.run_file("cart.ssbx.json", "steer", "", "");
        let (model, _) = ModelFile::from_json(CART).unwrap();
        let loaded = simsandbox_file::load(CART, &registry()).unwrap();
        assert!(run.fits(&loaded).unwrap());
        // A different mass is a different model.
        let heavier = CART.replace("param m [kg] = 2", "param m [kg] = 3");
        let other = simsandbox_file::load(&heavier, &registry()).unwrap();
        assert!(!run.fits(&other).unwrap());
        let mut again = cart(&run.apply_to(&model).to_json());
        let track = run
            .track
            .iter()
            .map(|(t, key, value)| InputEvent {
                t: *t,
                key: key.clone(),
                value: *value,
            })
            .collect();
        again.replay(track).unwrap();
        again.run_until(3.0).unwrap();
        same(&signals(&again), &reference, s.signals().names());
        // Headless, the same track gives the same values.
        let track2 = run
            .track
            .iter()
            .map(|(t, key, value)| InputEvent {
                t: *t,
                key: key.clone(),
                value: *value,
            })
            .collect();
        let text = run.apply_to(&model).to_json();
        let r = crate::run::run_with(&text, &registry(), Some(3.0), None, track2).unwrap();
        let v = r.outputs.iter().find(|o| o.name == "cart.v").unwrap();
        let k = s
            .signals()
            .names()
            .iter()
            .position(|n| n == "cart.v")
            .unwrap();
        let live: Vec<u64> = s
            .signals()
            .points(k)
            .iter()
            .map(|p| p.y.to_bits())
            .collect();
        let headless: Vec<u64> = v.values.iter().map(|x| x.to_bits()).collect();
        assert_eq!(headless, live);
        // The run started from m = 2, not the 4 it ended with.
        let start = &s.start_file().blocks[1].params;
        assert_eq!(
            start.get("m"),
            Some(&simsandbox_file::ParamValue::Number(2.0))
        );
    }

    #[test]
    fn input_acts_in_the_next_step() {
        let mut s = cart(CART);
        s.run_until(0.5).unwrap();
        let k = s
            .signals()
            .names()
            .iter()
            .position(|n| n == "cart.v")
            .unwrap();
        let before = s.signals().points(k).last().unwrap().y;
        s.set_param_key("push.y", 40.0).unwrap();
        s.step().unwrap();
        let after = s.signals().points(k).last().unwrap().y;
        // dv = F/m dt (less the drag): about 0.2 in one 0.01 s step.
        assert!(after - before > 0.15, "{before} -> {after}");
    }

    #[test]
    fn new_input_during_a_replay_starts_a_branch() {
        let mut s = cart(CART);
        steer(&mut s);
        s.rewind_to(1.0);
        s.run_until(1.5).unwrap();
        // The mass change at 1.2 was replayed; now push the other way.
        s.set_param_key("push.y", -20.0).unwrap();
        assert!(!s.replaying());
        let keys: Vec<(String, f64)> = s
            .recorded()
            .iter()
            .map(|e| (e.key.clone(), e.value))
            .collect();
        assert_eq!(
            keys,
            [
                ("push.y".to_string(), 40.0),
                ("cart.m".to_string(), 4.0),
                ("push.y".to_string(), -20.0)
            ]
        );
        // A restart drops the recording.
        s.restart().unwrap();
        assert!(s.recorded().is_empty());
    }

    #[test]
    fn scheduled_changes_apply_in_every_run_and_bad_ones_are_reported() {
        let scheduled = CART.replace(
            r#""run": {"dt": 0.01}"#,
            r#""run": {"dt": 0.01, "inputs": [
                {"t": 1, "set": "push.y", "to": 30},
                {"t": 0, "set": "cart.v0", "to": 2}]}"#,
        );
        let mut s = cart(&scheduled);
        // The start value at t = 0 set where the run starts.
        let v0 = s.signals().points(1)[0].y;
        assert_eq!(v0, 2.0);
        s.run_until(2.0).unwrap();
        let first = signals(&s);
        assert!(s.recorded().is_empty());
        // Kept by a restart: the second run is the same.
        s.restart().unwrap();
        s.run_until(2.0).unwrap();
        same(&signals(&s), &first, s.signals().names());
        // A change to something that does not exist is a load error.
        let bad = scheduled.replace(r#""set": "push.y""#, r#""set": "push.z""#);
        let err = Session::load("cart", &bad, &registry()).err().unwrap();
        assert!(
            err.contains("run.inputs[") && err.contains("push.z"),
            "{err}"
        );
        // Out of range is noted, not refused.
        let far = scheduled.replace(r#""to": 30"#, r#""to": 300"#);
        let s = cart(&far);
        assert!(s
            .warnings
            .iter()
            .any(|w| w.contains("outside its suggested range")));
    }
}

#[cfg(test)]
mod probe_tests {
    //! Probes besides the implicit one (ADR 0062).
    use super::*;
    use crate::examples::{registry, EXAMPLES};
    use crate::probe::{Every, Keep, ProbeSpec};

    fn example(part: &str) -> Session {
        let e = EXAMPLES.iter().find(|e| e.name.contains(part)).unwrap();
        Session::load(e.name, e.text, &registry()).unwrap()
    }

    fn spec(name: &str, record: &[&str], every: Every, keep: Keep) -> ProbeSpec {
        ProbeSpec {
            every,
            keep,
            ..ProbeSpec::new(name, record.iter().map(|r| r.to_string()).collect())
        }
    }

    /// `(t, value)` bits of signal `name` in history `h`.
    fn series(h: &History, name: &str) -> Vec<(u64, u64)> {
        let k = h.names().iter().position(|n| n == name).unwrap();
        h.points(k)
            .iter()
            .map(|p| (p.x.to_bits(), p.y.to_bits()))
            .collect()
    }

    #[test]
    fn a_probe_records_a_subset_on_its_own_schedule() {
        // Cruise control, 0.05 s steps; a probe of the speed every 0.5 s.
        let mut s = example("Cruise control");
        s.set_probes(vec![spec(
            "speed",
            &["car.v"],
            Every::Seconds(0.5),
            Keep::default(),
        )])
        .unwrap();
        assert_eq!(s.probe_names(), ["speed"]);
        s.restart().unwrap();
        s.run_until(5.0).unwrap();
        s.flush(true);
        let probe = s.probe("speed").unwrap();
        let p = probe.read();
        assert_eq!(p.names(), ["car.v"]);
        let times: Vec<f64> = p.times().to_vec();
        assert_eq!(times.len(), 11, "{times:?}");
        // Every sample is the implicit probe's at the same time, exactly.
        let all = s.signals();
        let k = all.names().iter().position(|n| n == "car.v").unwrap();
        for (i, t) in times.iter().enumerate() {
            let j = all.times().iter().position(|x| x == t).unwrap();
            assert_eq!(
                p.value(SignalRef::Signal(0), i).to_bits(),
                all.points(k)[j].y.to_bits()
            );
        }
        drop((p, all));
        // Unknown names are refused, and nothing changes.
        let bad = s.set_probes(vec![spec("x", &["car.nope"], Every::Steps(1), Keep::All)]);
        assert!(bad.unwrap_err()[0].contains("the model has no 'car.nope'"));
        assert_eq!(s.probe_names(), ["speed"]);
    }

    #[test]
    fn probes_keep_the_newest_or_thin_and_replay_after_a_rewind() {
        let mut s = example("Cruise control");
        s.set_probes(vec![
            spec(
                "last",
                &["car.v"],
                Every::Steps(1),
                Keep::Last { samples: 20 },
            ),
            spec("every", &["car.v", "cruise.F"], Every::Steps(2), Keep::All),
        ])
        .unwrap();
        s.restart().unwrap();
        s.run_until(10.0).unwrap();
        s.flush(true);
        let last = s.probe("last").unwrap();
        let n = last.read().len();
        assert!((20..=22).contains(&n), "{n}");
        assert!(last.read().times()[n - 1] > 9.9);
        let every = s.probe("every").unwrap();
        let before = series(&every.read(), "cruise.F");
        assert_eq!(before.len(), 101);
        // Rewind to 4 s and run on: the same samples again, bit for bit.
        s.rewind_to(4.0);
        assert!(every.read().times().last().unwrap() <= &4.0);
        s.run_until(10.0).unwrap();
        s.flush(true);
        assert_eq!(series(&every.read(), "cruise.F"), before);
        // Restart: empty, then recording again from t = 0.
        s.restart().unwrap();
        assert_eq!(every.read().len(), 1);
    }

    #[test]
    fn probes_record_grid_fields_with_their_coordinates() {
        let mut s = example("Typed heat equation");
        s.set_probes(vec![spec(
            "snapshots",
            &["rod.T", "rod.x"],
            Every::Times(vec![0.0, 1.0, 2.0]),
            Keep::All,
        )])
        .unwrap();
        s.restart().unwrap();
        s.run_until(3.0).unwrap();
        s.flush(true);
        let p = s.probe("snapshots").unwrap();
        let h = p.read();
        assert_eq!(h.len(), 3);
        let names: Vec<&str> = h.fields().iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["rod.T", "rod.x"]);
        assert_eq!(h.fields()[0].frame(2).len(), 50);
        assert!(h
            .coordinates()
            .iter()
            .any(|c| c.name == "rod.x" && c.of == "rod.T"));
    }

    #[test]
    fn probes_survive_filter_edits_without_what_is_gone() {
        let mut s = example("Cruise control");
        let filter = FilterEntry::expression("double", "2*car.v", vec!["car.v".into()]);
        s.set_filters(vec![filter]).unwrap();
        s.set_probes(vec![spec(
            "p",
            &["car.v", "double"],
            Every::Steps(1),
            Keep::default(),
        )])
        .unwrap();
        s.set_filters(Vec::new()).unwrap();
        assert_eq!(s.probe_names(), ["p"]);
        assert_eq!(s.probe("p").unwrap().read().names(), ["car.v"]);
        assert!(s
            .warnings
            .iter()
            .any(|w| w.contains("'double' is no longer recorded")));
    }

    /// The cruise control example with `probes` (JSON) in its controls.
    fn cruise_with(probes: &str) -> String {
        let e = EXAMPLES
            .iter()
            .find(|e| e.name.contains("Cruise control"))
            .unwrap();
        let mut v: serde_json::Value = serde_json::from_str(e.text).unwrap();
        v["controls"]["probes"] = serde_json::from_str(probes).unwrap();
        v.to_string()
    }

    #[test]
    fn probes_load_from_the_file_and_save_back() {
        let text = cruise_with(
            r#"[{"name": "speed", "record": ["car.v"], "every": {"seconds": 0.5},
                 "keep": {"last": 4}, "to": "speed.csv", "canvas": [10, 20]},
                {"name": "bad", "record": ["car.nope"]}]"#,
        );
        let mut s = Session::load("cruise", &text, &registry()).unwrap();
        assert_eq!(s.probe_names(), ["speed"]);
        assert!(
            s.warnings
                .iter()
                .any(|w| w.contains("controls.probes[1]") && w.contains("car.nope")),
            "{:?}",
            s.warnings
        );
        let spec = s.probe_specs()[0].clone();
        assert_eq!(spec.every, Every::Seconds(0.5));
        assert_eq!(spec.keep, Keep::Last { samples: 4 });
        assert_eq!(spec.canvas, Some([10.0, 20.0]));
        let saved = s.save_json(1.0, None).unwrap();
        let again = Session::load("again", &saved, &registry()).unwrap();
        assert_eq!(again.probe_specs(), [&spec]);
    }

    #[test]
    fn a_headless_run_records_what_the_app_does() {
        let text = cruise_with(
            r#"[{"name": "speed", "record": ["car.v"], "every": {"steps": 10}, "keep": {"all": true},
                 "to": "speed.csv"}]"#,
        );
        let r =
            crate::run::run_probes(&text, &registry(), Some(5.0), None, Vec::new(), &[]).unwrap();
        let (spec, h) = &r.probes[0];
        assert_eq!(spec.name, "speed");
        let h = h.read();
        assert_eq!(h.len(), 11);
        // Every sample is the plain run's at the same step, exactly.
        let plain = crate::run::run(&text, &registry(), Some(5.0), None).unwrap();
        let v = &plain
            .outputs
            .iter()
            .find(|o| o.name == "car.v")
            .unwrap()
            .values;
        for (k, p) in h.points(0).iter().enumerate() {
            assert_eq!(p.y.to_bits(), v[10 * k].to_bits());
        }
        let err = crate::run::run_probes(&text, &registry(), None, None, Vec::new(), &["x".into()]);
        assert!(err.is_err_and(|e| e.contains("no probe 'x'")));
    }

    #[test]
    fn probe_edits_keep_what_unchanged_probes_recorded() {
        let text =
            cruise_with(r#"[{"name": "speed", "record": ["car.v"], "keep": {"all": true}}]"#);
        let mut s = Session::load("cruise", &text, &registry()).unwrap();
        s.run_until(1.0).unwrap();
        s.flush(true);
        let before = s.probe("speed").unwrap().read().len();
        assert_eq!(before, 21);
        // Moved, and a probe added that records nothing yet.
        let mut entries = s.probe_entries();
        entries[0].canvas = Some([5.0, 5.0]);
        entries.push(ProbeSpec::new("later", vec![]).to_entry());
        entries[1].every = Some(simsandbox_file::ProbeEvery {
            seconds: Some(0.5),
            ..Default::default()
        });
        entries[1].record = vec!["car.v".into()];
        s.set_probe_entries(&entries).unwrap();
        s.run_until(2.0).unwrap();
        s.flush(true);
        assert_eq!(s.probe("speed").unwrap().read().len(), 41);
        // The new one from now on, at multiples of 0.5 s after t = 1.
        let later: Vec<f64> = s.probe("later").unwrap().read().times().to_vec();
        assert_eq!(later.len(), 2, "{later:?}");
        assert!((later[0] - 1.5).abs() < 1e-9);
        assert_eq!(s.probe_entries()[0].canvas, Some([5.0, 5.0]));
        // A probe that records nothing is kept.
        entries[1].record.clear();
        s.set_probe_entries(&entries).unwrap();
        assert_eq!(s.probe_names(), ["speed", "later"]);
    }

    #[test]
    fn the_history_records_only_what_is_wired_and_probes_anything() {
        // Cruise control's views read car.v and cruise.F, and so on.
        let mut s = example("Cruise control");
        s.set_view_reads(vec!["car.v".into(), "probe:speed/cruise.F".into()]);
        assert_eq!(s.signals().names(), ["car.v"]);
        // Everything can still be bound and probed.
        let h = s.signals();
        assert!(s.available(&h).scalars.contains(&"cruise.I".to_string()));
        drop(h);
        s.set_probes(vec![spec("i", &["cruise.I"], Every::Steps(1), Keep::All)])
            .unwrap();
        s.run_until(1.0).unwrap();
        s.flush(true);
        // Added after the sample at t = 0: every step since.
        assert_eq!(s.probe("i").unwrap().read().len(), 20);
        // Wiring cruise.I to a view records it from now; car.v keeps its past.
        s.set_view_reads(vec!["car.v".into(), "cruise.I".into()]);
        s.run_until(2.0).unwrap();
        s.flush(true);
        let h = s.signals();
        assert_eq!(h.names(), ["car.v", "cruise.I"]);
        let i = h.values(h.resolve("cruise.I").unwrap());
        assert_eq!(h.len(), 41);
        assert!(i[..21].iter().all(|v| v.is_nan()));
        assert!(i[21..].iter().all(|v| v.is_finite()));
        assert!(h
            .values(h.resolve("car.v").unwrap())
            .iter()
            .all(|v| v.is_finite()));
    }

    /// The values of `name` in a filter's own recording, with its times.
    fn derived_series(s: &Session, filter: &str, name: &str) -> (Vec<f64>, Vec<f64>) {
        let (_, h) = s
            .derived_histories()
            .into_iter()
            .find(|(n, _)| n == filter)
            .unwrap();
        let h = h.read();
        let r = h.resolve(name).unwrap();
        (h.times().to_vec(), h.values(r))
    }

    #[test]
    fn filters_on_a_probe_run_on_its_samples() {
        let text = cruise_with(
            r#"[{"name": "speed", "record": ["car.v", "cruise.F"], "every": {"seconds": 0.5},
                 "keep": {"all": true}}]"#,
        );
        let mut s = Session::load("cruise", &text, &registry()).unwrap();
        let mut filters = s.filters().to_vec();
        let mut power = FilterEntry::expression("power", "car.v*cruise.F", Vec::new());
        power.probe = Some("speed".into());
        filters.push(power);
        s.set_filters(filters.clone()).unwrap();
        s.run_until(2.0).unwrap();
        s.flush(true);
        // Every 0.5 s, from the probe's samples, in a recording of its own.
        let (t, w) = derived_series(&s, "power", "power");
        let p = s.probe("speed").unwrap();
        let h = p.read();
        assert_eq!(t, h.times());
        let (v, f) = (
            h.values(SignalRef::Signal(0)),
            h.values(SignalRef::Signal(1)),
        );
        assert!(w
            .iter()
            .zip(v.iter().zip(&f))
            .all(|(w, (v, f))| (w - v * f).abs() < 1e-9));
        drop(h);
        assert!(s.signals().resolve("power").is_none());
        assert!(s.derived_names().contains(&"power".to_string()));
        let saved = s.save_json(1.0, None).unwrap();
        assert!(saved.contains("\"probe\": \"speed\""));
        // A filter on a probe reads only what the probe records.
        let mut bad = FilterEntry::expression("pi", "cruise.I*2", Vec::new());
        bad.probe = Some("speed".into());
        filters.push(bad);
        s.set_filters(filters.clone()).unwrap();
        assert!(s
            .warnings
            .iter()
            .any(|w| w.contains("pi") && w.contains("does not record")));
        let mut nowhere = FilterEntry::expression("x", "1", Vec::new());
        nowhere.probe = Some("gone".into());
        s.set_filters(vec![nowhere]).unwrap();
        assert!(s.warnings.iter().any(|w| w.contains("no probe 'gone'")));
    }

    #[test]
    fn filters_on_several_probes_run_paired_or_on_any() {
        // car.v every 0.2 s, cruise.F every 0.3 s (steps of 0.05 s).
        let text = cruise_with(
            r#"[{"name": "a", "record": ["car.v"], "every": {"steps": 4}, "keep": {"all": true}},
                {"name": "b", "record": ["cruise.F"], "every": {"steps": 6}, "keep": {"all": true}}]"#,
        );
        let mut s = Session::load("cruise", &text, &registry()).unwrap();
        let both = |name: &str, when: Option<&str>| {
            let mut f = FilterEntry::expression(name, "car.v*cruise.F", Vec::new());
            f.from = [
                ("car.v".into(), "a".into()),
                ("cruise.F".into(), "b".into()),
            ]
            .into();
            f.when = when.map(String::from);
            f
        };
        // And a filter reading a filter: on its clock.
        let half = FilterEntry::expression("half", "paired/2", Vec::new());
        s.set_filters(vec![
            both("paired", None),
            both("latest", Some("any")),
            half,
        ])
        .unwrap();
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        s.run_until(1.2).unwrap();
        s.flush(true);
        let step =
            |t: &[f64]| -> Vec<i64> { t.iter().map(|t| (t / 0.05).round() as i64).collect() };
        // Paired: where both recorded, every 12 steps.
        let (t, w) = derived_series(&s, "paired", "paired");
        assert_eq!(step(&t), [12, 24]);
        let (th, h) = derived_series(&s, "half", "half");
        assert_eq!(th, t);
        assert!(h.iter().zip(&w).all(|(h, w)| (2.0 * h - w).abs() < 1e-9));
        // Any: whenever either recorded (steps 4, 6, 8, 12, ...), with the
        // other's latest value; NaN until both have one.
        let (t, w) = derived_series(&s, "latest", "latest");
        assert_eq!(step(&t), [4, 6, 8, 12, 16, 18, 20, 24]);
        assert!(w[0].is_nan() && w[1].is_finite());
        let a = s.probe("a").unwrap();
        let b = s.probe("b").unwrap();
        let (a, b) = (a.read(), b.read());
        // At step 8: car.v at 8, cruise.F held from 6.
        let (v8, f6) = (
            a.values(SignalRef::Signal(0))[1],
            b.values(SignalRef::Signal(0))[0],
        );
        assert!((w[2] - v8 * f6).abs() < 1e-9);
        drop((a, b));
        // After a rewind it records the same again.
        let before = derived_series(&s, "latest", "latest").1;
        s.rewind_to(0.5);
        s.run_until(1.2).unwrap();
        s.flush(true);
        let again = derived_series(&s, "latest", "latest").1;
        assert_eq!(
            before.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            again.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
        );
    }
}
