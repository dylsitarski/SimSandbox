//! One open model: the running simulator, its snapshots for rewind, the
//! recorded signals the views draw from, and its parameters.

use crate::filters::Filters;
use simsandbox_core::block::{MethodLabels, Shape};
use simsandbox_core::graph::{BlockId, Model};
use simsandbox_core::registry::Registry;
use simsandbox_core::sim::{Simulator, Snapshot};
use simsandbox_core::units::Unit;
use simsandbox_core::SimError;
use simsandbox_file::{
    Design, FilterEntry, FusedGroup, ModelFile, ParamPlan, Plan, Sampling, ViewEntry,
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
/// When the snapshots grow past this, every other one is dropped: the whole
/// run stays reachable, at coarser resolution.
const MAX_SNAPSHOTS: usize = 10_000;

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
        let plots = &loaded.file.controls.plots;
        for entry in &loaded.flat.blocks {
            let id = loaded.ids[&entry.name];
            let block = loaded.model.block(id).map_err(|e| e.to_string())?;
            if !block_types.contains(&entry.type_name) {
                block_types.push(entry.type_name.clone());
            }
            let spec = block.spec();
            for port in spec.outputs() {
                let key = format!("{}.{}", entry.name, port.name);
                // Plot what the file asks for, or every output if it says nothing.
                if !plots.is_empty() && !plots.contains(&key) {
                    continue;
                }
                let field = match port.shape {
                    Shape::Grid { nx, ny } => Some(FieldShape::Grid { nx, ny }),
                    Shape::Points { count, dim } => Some(FieldShape::Points { count, dim }),
                    Shape::Population { dim } => Some(FieldShape::Population { dim }),
                    Shape::Scalar | Shape::Vector(_) => None,
                };
                if let Some(shape) = field {
                    if let Some(c) = &port.coordinate {
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
        };
        let (filters, problems) = session.compile_filters(&entries);
        session.filter_entries = filters.entries(&entries);
        session.filters = filters;
        for p in problems {
            session
                .warnings
                .push(format!("controls.filters.{p}; ignored"));
        }
        if session.sweep.is_some() {
            session.resample();
        } else {
            session.rebuild_history();
            session.record();
        }
        Ok(session)
    }

    /// Names of the recorded grids, point sets, one-row grids and the
    /// model's block types: what views can bind besides scalars.
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
        if self.pending.is_empty() {
            return true;
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
        if let Some(mut h) = self.history.try_write() {
            for p in self.pending.drain(..) {
                let frames: Vec<&[f64]> = p.frames.iter().map(Vec::as_slice).collect();
                h.push_sample(p.index, &p.values, &frames);
            }
            h.push_sample(index, values, &frames);
            return;
        }
        self.pending.push(Pending {
            index,
            values: values.to_vec(),
            frames: frames.iter().map(|f| f.to_vec()).collect(),
        });
    }

    /// What views can bind, with `h` this session's history: the scalar
    /// signals, grids, point sets and the model's block types.
    pub fn available<'a>(&'a self, h: &'a History) -> Available<'a> {
        Available {
            scalars: h.names(),
            grids: &self.grids,
            points: &self.points,
            block_types: &self.block_types,
            lines: &self.lines,
            coordinates: h.coordinates(),
            swept: self.sweep.as_ref().map_or(&[], |sw| sw.keys()),
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
    fn compile_filters(&self, entries: &[FilterEntry]) -> (Filters, Vec<String>) {
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
        let mut h = History::with_fields(index, scalars, fields);
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
        h.set_coordinates(coordinates);
        for (name, label) in labels {
            h.set_label(&name, &label);
        }
        self.lines = h
            .fields()
            .iter()
            .filter(|f| matches!(f.shape, FieldShape::Grid { ny: 1, .. }))
            .map(|f| f.name.clone())
            .collect();
        self.replace_history(h);
    }

    /// The frames of the cut filters that are per sample (`whole` false)
    /// or made from the whole sweep (`whole` true), in order, given the
    /// frames of the fields they read.
    fn cut_frames<'a>(
        &self,
        frame: impl Fn(&str) -> Option<&'a [f64]>,
        whole: bool,
    ) -> Vec<Vec<f64>> {
        let h = self.signals();
        let coordinates = h.coordinates();
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
        self.sim.step(self.dt)?;
        self.record();
        self.steps_since_snapshot += 1;
        if self.steps_since_snapshot >= SNAPSHOT_EVERY {
            self.steps_since_snapshot = 0;
            self.snapshots.push(self.sim.snapshot());
            if self.snapshots.len() > MAX_SNAPSHOTS {
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
    pub fn set_param_key(&mut self, key: &str, value: f64) -> Result<(), String> {
        if self.plan.declared(key).is_some() {
            let set = self.plan.set(key, value)?;
            for (block, param, v) in set {
                let id = self.ids[&block];
                self.sim
                    .set_param(id, &param, v)
                    .map_err(|e| e.to_string())?;
            }
            return self.after_param_change();
        }
        let (block, name) = key
            .rsplit_once('.')
            .ok_or_else(|| format!("'{key}' is not 'block.param'"))?;
        let id = *self
            .ids
            .get(block)
            .ok_or_else(|| format!("no block '{block}'"))?;
        if let Some(f) = self.plan.formula_of(block, name) {
            return Err(format!("{key} is computed from '{}'", f.text));
        }
        self.set_param(id, name, value)
    }

    /// Change a parameter while running. Initial conditions apply on
    /// [`Session::restart`], or at once if the run has not started.
    pub fn set_param(&mut self, block: BlockId, name: &str, value: f64) -> Result<(), String> {
        self.sim
            .set_param(block, name, value)
            .map_err(|e| e.to_string())?;
        self.after_param_change()
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
        self.sim.restart(0.0)?;
        self.snapshots = vec![self.sim.snapshot()];
        self.steps_since_snapshot = 0;
        self.stop_at = self.file.run.t_end;
        self.history_mut().clear();
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
        if let Some(sw) = &self.sweep {
            self.file.sample = Some(sw.sampling.clone());
        }
        Ok(self.file.to_json())
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
        self.filter_entries = filters.entries(&entries);
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
        self.stop_at = self.file.run.t_end.filter(|&end| end > t0);
        self.history_mut().truncate_after(t0);
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
        self.filter_entries = filters.entries(&entries);
        self.filters = filters;
        if self.sweep.is_some() {
            self.resample();
            return Ok(());
        }
        // Replay the recorded outputs and fields through the new filters.
        drop(self.history_mut());
        let old = self.replace_history(History::new(Vec::new()));
        self.rebuild_history();
        let outputs = self.output_names.len();
        let base = self.base_fields.len();
        for k in 0..old.len() {
            let t = old.times()[k];
            let mut values: Vec<f64> = (0..outputs)
                .map(|i| old.value(SignalRef::Signal(i), k))
                .collect();
            self.append_derived(t, &mut values);
            let frames: Vec<&[f64]> = old.fields()[..base].iter().map(|f| f.frame(k)).collect();
            let cuts = self.sample_cuts(&frames);
            self.push(t, &values, &frames, &cuts);
        }
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

/// One note per algebraic loop (ADR 0040), saying how it is solved.
pub fn loop_notes(sim: &Simulator) -> Vec<String> {
    let method = sim.model().loop_method().name();
    sim.loops()
        .into_iter()
        .map(|l| {
            format!(
                "algebraic loop {}: solved at every step by '{method}' (run.loops)",
                l.join(" → ")
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

        // Redefining recomputes the whole history.
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
