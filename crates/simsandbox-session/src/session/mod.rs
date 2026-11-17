//! One open model: the running simulator, its snapshots for rewind, the
//! recorded signals the views draw from, and its parameters.

use crate::filters::Filters;
use crate::probe::{ProbeSpec, Trigger};
use simsandbox_core::block::{MethodLabels, Shape, TimeSemantics};
use simsandbox_core::graph::{BlockId, Model};
use simsandbox_core::registry::Registry;
use simsandbox_core::sim::{Simulator, Snapshot};
use simsandbox_core::units::Unit;
use simsandbox_core::SimError;
use simsandbox_file::{FilterEntry, FusedGroup, ModelFile, ParamPlan, ProbeEntry, ViewEntry};
use simsandbox_views::cut::Geometry;
use simsandbox_views::history::thin;
use simsandbox_views::{
    Available, Coordinate, FieldShape, History, MeshInfo, MeshShape, SignalRef,
};
use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

mod inputs;

pub use inputs::InputEvent;
pub(crate) use inputs::{set_key, Track};

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
    /// Whether it keeps samples in memory: only while something reads
    /// them (views, filters, a file written when asked, a headless run),
    /// so a probe feeding a block or writing as it records costs none.
    in_memory: bool,
    /// Its file, written as it records (ADR 0063).
    stream: Option<crate::probe::Stream>,
}

impl Extra {
    /// Its signals' names and its fields, as recorded.
    fn layout(&self) -> (Vec<String>, Vec<(String, FieldShape)>) {
        let h = self.history.read();
        (
            h.names().to_vec(),
            h.fields()
                .iter()
                .map(|f| (f.name.clone(), f.shape))
                .collect(),
        )
    }
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

/// When the snapshots grow past this many, or past the recorder's memory
/// budget (ADR 0064), every other one is dropped: the whole run stays
/// reachable, at coarser resolution. Gray–Scott (128 × 128, its outputs
/// computed again on rewind, ADR 0063) takes about 0.26 MB a snapshot, so
/// the default 256 MB keeps about 1 000.
const MAX_SNAPSHOTS: usize = 10_000;

/// The app's memory budget by default, in MB (ADR 0062 step 5): shared by
/// snapshots, the history and probes, a quarter each by default.
pub const DEFAULT_MEMORY_MB: f64 = 1024.0;

/// What a session's recordings hold against its memory budget (ADR 0062
/// step 5), in bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryUse {
    pub budget: usize,
    pub held: usize,
    /// Each part: `snapshots`, `history`, `probe NAME`, `filter NAME`.
    pub parts: Vec<(String, usize)>,
}

impl MemoryUse {
    /// The part holding most, if any holds anything.
    pub fn largest(&self) -> Option<&(String, usize)> {
        self.parts.iter().filter(|p| p.1 > 0).max_by_key(|p| p.1)
    }

    pub fn over(&self) -> bool {
        self.held > self.budget
    }
}

/// Where a recorded signal comes from: an element of an output port.
#[derive(Clone)]
struct Source {
    block: BlockId,
    port: String,
    index: usize,
}

/// A whole output port recorded as a field (a grid or a point set).
#[derive(Clone)]
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
    /// The signal driving it (ADR 0075), `block.port`: then its value is
    /// that signal's, not set by hand.
    pub driven: Option<String>,
}

/// The parameters of `model` driven by wires, as `block.param` to the
/// `block.port` driving each (ADR 0075).
pub fn drives(model: &simsandbox_core::graph::Model) -> Vec<(String, String)> {
    let name = |id: BlockId| {
        model
            .block(id)
            .map(|b| b.spec().name.clone())
            .unwrap_or_default()
    };
    model
        .drives()
        .iter()
        .map(|c| {
            (
                format!("{}.{}", name(c.to.block), c.to.port),
                format!("{}.{}", name(c.from.block), c.from.port),
            )
        })
        .collect()
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
    /// Recorded signals, one per plotted output element (ADR 0015);
    /// shared with the window.
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
    /// Probes whose recordings views read.
    view_probes: Vec<String>,
    /// Keep every probe's samples (a headless run).
    record_all_probes: bool,
    /// Where probes write their files as they record.
    probe_dir: Option<std::path::PathBuf>,
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
    /// Fields over a table's rows, one value each (ADR 0086).
    rows: Vec<String>,
    /// Meshes views can draw, and how (ADR 0086).
    meshes: Vec<MeshInfo>,
    mesh_shapes: Vec<(String, MeshShape)>,
    block_types: Vec<String>,
    /// Where the app pauses (the file's `run.t_end`); cleared once reached so
    /// Run continues past it (ADR 0013).
    stop_at: Option<f64>,
    sim: Simulator,
    snapshots: Vec<Snapshot>,
    /// The app's memory budget in MB (ADR 0062 step 5).
    memory_mb: f64,
    steps_since_snapshot: usize,
    /// Built without running its studies ([`crate::study::without_running_studies`]),
    /// or (evaluated later) changed since last evaluated: [`Session::evaluate`]
    /// brings it up to date.
    unevaluated: bool,
    /// Nothing in it runs in time (ADR 0072): evaluated, not stepped.
    timeless: bool,
    /// Only maps, and nothing reads time (ADR 0083): it iterates.
    iterates: bool,
    /// An iterating model has rounds still to run: since it opened or last
    /// changed, it has neither held still nor reached its last round.
    rounds_pending: bool,
    /// Its last round changed no state.
    held_still: bool,
    /// Changes to a model with nothing in time leave it unevaluated, for
    /// the caller to [`Session::evaluate`] (the app's worker, ADR 0072),
    /// instead of evaluating it at once.
    evaluate_later: bool,
    /// The recorder (ADR 0064): without one, no snapshots and no rewind.
    recorder: Option<simsandbox_file::RecorderEntry>,
    /// Filters as written, for saving and editing (ADR 0030, ADR 0032).
    filter_entries: Vec<FilterEntry>,
    /// Filters compiled: expressions recorded after the output signals,
    /// cut fields after the recorded fields.
    filters: Filters,
    /// The schedule and the recorded live input (ADR 0058).
    track: Track,
    /// The history lacks what views read now for its samples up to this
    /// time: wired after they were recorded (ADR 0063). Filled on demand
    /// by [`Session::recompute`].
    gaps_until: Option<f64>,
    /// The file as it was when the current run started (its parameters at
    /// t = 0), which a run file is made with (ADR 0059).
    start_file: ModelFile,
    /// Digests of the files blocks read, for the model hash (ADR 0063).
    data: BTreeMap<String, String>,
}

impl Session {
    /// Load a model file. Errors are formatted for display.
    pub fn load(title: &str, text: &str, registry: &Registry) -> Result<Self, String> {
        Self::load_in(title, text, registry, None)
    }

    /// Load a model file saved in `folder`, whose blocks read the files
    /// they name from there (ADR 0063).
    pub fn load_in(
        title: &str,
        text: &str,
        registry: &Registry,
        folder: Option<&std::path::Path>,
    ) -> Result<Self, String> {
        // A model that uses model files runs as one simulator with clocks
        // (ADR 0066 step 4b).
        let loaded = if simsandbox_file::compose::is_composition(text) {
            simsandbox_file::compose::load_merged(text, registry, folder)
        } else {
            simsandbox_file::load_in(text, registry, folder)
        }
        .map_err(|e| e.to_string())?;
        let mut names = Vec::new();
        let mut sources = Vec::new();
        let mut fields = Vec::new();
        let mut field_sources = Vec::new();
        let mut meshes: Vec<MeshInfo> = Vec::new();
        let mut mesh_shapes: Vec<(String, MeshShape)> = Vec::new();
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
                // (ADR 0063).
                let key = format!("{}.{}", entry.name, port.name);
                let field = match port.shape {
                    Shape::Grid { nx, ny } => Some(FieldShape::Grid { nx, ny }),
                    Shape::Points { count, dim } => Some(FieldShape::Points { count, dim }),
                    Shape::Population { dim } => Some(FieldShape::Population { dim }),
                    Shape::Tensor { rank, dims } => Some(FieldShape::Tensor { rank, dims }),
                    // Recorded with their ids (ADR 0079 stage 1c).
                    Shape::Rows { dim } => Some(FieldShape::Rows { dim }),
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
                    // A mesh's cells, with the fields over its cells and
                    // points (ADR 0086).
                    if let Some(m) = &port.mesh {
                        let table = |o: &simsandbox_core::block::PortSpec| {
                            o.axis_names
                                .first()
                                .cloned()
                                .filter(|_| matches!(o.shape, Shape::Rows { dim: 1 }))
                        };
                        let points = spec
                            .outputs()
                            .find(|o| Some(&o.name) == m.coordinates.first())
                            .and_then(table);
                        let over = |t: Option<String>| -> Vec<String> {
                            spec.outputs()
                                .filter(|o| t.is_some() && table(o) == t)
                                .map(|o| format!("{}.{}", entry.name, o.name))
                                .collect()
                        };
                        let qualified: Vec<String> = m
                            .coordinates
                            .iter()
                            .map(|c| format!("{}.{}", entry.name, c))
                            .collect();
                        meshes.push(MeshInfo {
                            cells: key.clone(),
                            coordinates: qualified.clone(),
                            over_cells: over(port.axis_names.first().cloned()),
                            over_points: over(points)
                                .into_iter()
                                .filter(|f| !qualified.contains(f))
                                .collect(),
                        });
                        mesh_shapes.push((
                            key.clone(),
                            MeshShape {
                                dim: m.dim,
                                faces: m.faces.clone(),
                                coordinates: qualified,
                            },
                        ));
                    }
                    // Rows of one table place each other, by id (ADR 0086):
                    // any of its quantities can be the other's axis.
                    if let (Shape::Rows { dim: 1 }, Some(table)) =
                        (port.shape, port.axis_names.first())
                    {
                        for other in spec.outputs().filter(|o| {
                            o.name != port.name
                                && matches!(o.shape, Shape::Rows { dim: 1 })
                                && o.axis_names.first() == Some(table)
                        }) {
                            coordinates.push(Coordinate {
                                name: key.clone(),
                                of: format!("{}.{}", entry.name, other.name),
                                axis: 0,
                            });
                        }
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
        // Declared outputs of used models (ADR 0066 step 4b) are recorded
        // under their names too, from the same outputs.
        for (alias, target) in &loaded.aliases {
            if let Some(k) = names.iter().position(|n| n == target) {
                names.push(alias.clone());
                sources.push(sources[k].clone());
            } else if let Some(k) = fields.iter().position(|(n, _)| n == target) {
                fields.push((alias.clone(), fields[k].1));
                field_sources.push(field_sources[k].clone());
            }
        }
        // A coordinate needs the field it places, recorded, of the same
        // shape, or a line along its axis (ADR 0080).
        coordinates.retain(|c| {
            let shape = |n: &str| fields.iter().find(|(f, _)| f == n).map(|(_, s)| *s);
            match (shape(&c.name), shape(&c.of)) {
                (Some(a), Some(b)) if a == b => true,
                (Some(FieldShape::Grid { nx: n, ny: 1 }), Some(FieldShape::Grid { nx, ny })) => {
                    n == if c.axis == 0 { nx } else { ny }
                }
                // Along one axis of a field of more (ADR 0086).
                (
                    Some(FieldShape::Grid { nx: n, ny: 1 }),
                    Some(FieldShape::Tensor { rank, dims }),
                ) => c.axis < rank && dims[c.axis] == n,
                _ => false,
            }
        });
        // Where a run only data drives ends (ADR 0067 step 4).
        let data_end = crate::run::data_end(&loaded.model);
        let sim = Simulator::new(loaded.model, 0.0).map_err(|e| e.to_string())?;
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
        let start_file = loaded.file.clone();
        let data = loaded.data;
        let mut warnings = loaded.warnings;
        warnings.extend(loop_notes(&sim));
        let entries = loaded.file.controls.filters.clone();
        let mut session = Self {
            title: title.into(),
            description: loaded.file.description.clone(),
            dt: loaded.file.run.dt,
            stop_at: loaded.file.run.t_end.or(data_end),
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
            view_probes: Vec::new(),
            record_all_probes: false,
            probe_dir: None,
            sources,
            output_names: names,
            base_fields: fields,
            base_coordinates: coordinates,
            lines: Vec::new(),
            field_sources,
            grids: Vec::new(),
            points: Vec::new(),
            rows: Vec::new(),
            meshes,
            mesh_shapes,
            block_types,
            snapshots: vec![sim.snapshot()],
            memory_mb: DEFAULT_MEMORY_MB,
            sim,
            steps_since_snapshot: 0,
            recorder: start_file.controls.recorder.clone(),
            filter_entries: Vec::new(),
            filters: Filters::default(),
            track: Track::new(schedule, Vec::new()),
            gaps_until: None,
            start_file,
            data,
            unevaluated: false,
            timeless: false,
            iterates: false,
            rounds_pending: false,
            held_still: false,
            evaluate_later: false,
        };
        session.timeless = session.nothing_in_time();
        session.iterates = !session.timeless && session.only_rounds();
        session.rounds_pending = session.iterates;
        session.unevaluated = !crate::study::studies_run()
            && session.block_types.iter().any(|t| t == crate::study::STUDY);
        session.check_schedule()?;
        let (filters, problems) = session.compile_filters(&entries);
        session.filter_entries = kept_entries(&filters, &entries);
        session.filters = filters;
        for p in problems {
            session
                .warnings
                .push(format!("controls.filters.{p}; ignored"));
        }
        // No probes yet: they are loaded next, and the filters that read
        // them with them.
        session.build_history();
        session.load_probes();
        // Scheduled changes at t = 0 set where the run starts.
        session.restart().map_err(|e| e.to_string())?;
        Ok(session)
    }

    /// How close two times must be to count as the same step boundary.
    fn eps(&self) -> f64 {
        1e-9 * self.dt.max(self.time().abs())
    }

    /// The file as it was when the current run started.
    pub fn start_file(&self) -> &ModelFile {
        &self.start_file
    }

    /// Names of the recorded grids, point sets, one-row grids and the
    /// model's block types: what views can bind besides scalars.
    /// Every scalar a step yields, which views can bind (ADR 0063).
    pub fn bindable_scalars(&self) -> &[String] {
        &self.layout_scalars
    }

    /// The 2D grids a cut filter can take, recorded or not (ADR 0063).
    pub fn cut_inputs(&self) -> Vec<String> {
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

    /// The meshes views can draw (ADR 0086).
    pub fn meshes(&self) -> &[MeshInfo] {
        &self.meshes
    }

    pub fn bindable(&self) -> [&[String]; 5] {
        [
            &self.grids,
            &self.points,
            &self.lines,
            &self.block_types,
            &self.rows,
        ]
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
            if wait {
                if let Some(w) = &mut p.stream {
                    let _ = w.flush();
                }
            }
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
        if !self.probes.is_empty() || !self.derived.is_empty() {
            let (step, eps) = (self.sim.step_count(), self.eps());
            let mut fired = Vec::with_capacity(self.probes.len());
            for p in &mut self.probes {
                let due = p.trigger.due(index, step, eps);
                fired.push(due);
                if !due {
                    continue;
                }
                let own: Vec<f64> = p.signals.iter().map(|&i| values[i]).collect();
                if p.stream.is_some() {
                    let fr: Vec<&[f64]> = p.fields.iter().map(|&i| frames[i]).collect();
                    let (_, fields) = p.layout();
                    let w = p.stream.as_mut().expect("checked");
                    if let Err(e) = w.write(index, &own, &fr, &fields) {
                        let note = format!("probe {}: writing its file failed: {e}", p.spec.name);
                        p.stream = None;
                        if !self.warnings.contains(&note) {
                            self.warnings.push(note);
                        }
                    }
                }
                if !p.in_memory {
                    continue;
                }
                p.pending.push(Pending {
                    index,
                    values: own,
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
        let mut probes: Vec<String> = reads
            .iter()
            .filter_map(|r| {
                simsandbox_views::history::split_source(r)
                    .0
                    .map(String::from)
            })
            .collect();
        probes.sort();
        probes.dedup();
        if probes != self.view_probes {
            self.view_probes = probes;
            self.update_probe_io();
        }
        let reads = Some(model_reads(reads));
        if self.view_reads == reads {
            return;
        }
        self.view_reads = reads;
        let (signals, fields) = self.kept();
        if signals == self.keep_signals && fields == self.keep_fields {
            return;
        }
        drop(self.history_mut());
        let old = self.replace_history(History::new(Vec::new()));
        self.build_history();
        // The sample at now is had again from the model as it is.
        let mut fills = BTreeMap::new();
        if let Some(&last) = old.times().last() {
            if (last - self.time()).abs() <= self.eps() {
                fills.insert(old.len() - 1, self.sample_now());
            }
        }
        self.replay_history(&old, false, &fills);
    }

    /// The places in the layout the history keeps: everything when no
    /// views are known, else what views read from the model
    /// and what filters read, with the coordinates cuts need.
    fn kept(&self) -> (Vec<usize>, Vec<usize>) {
        let Some(reads) = &self.view_reads else {
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
    ///
    /// Samples in `fills` (by index in `old`: every scalar, and the kept
    /// fields' frames) are taken from there instead. Names `old` lacks
    /// mark the samples before as gaps, unless recomputed.
    fn replay_history(&mut self, old: &History, recompute: bool, fills: &BTreeMap<usize, Pending>) {
        let signal_at: Vec<Option<usize>> = self
            .layout_scalars
            .iter()
            .map(|n| old.names().iter().position(|m| m == n))
            .collect();
        let field_at: Vec<Option<usize>> = self
            .layout_fields
            .iter()
            .map(|(n, _)| old.fields().iter().position(|f| &f.name == n))
            .collect();
        let outputs = self.output_names.len();
        let base = self.base_fields.len();
        let lacks = self
            .keep_signals
            .iter()
            .any(|&i| signal_at[i].is_none() && (!recompute || i < outputs))
            || self
                .keep_fields
                .iter()
                .any(|&i| field_at[i].is_none() && (!recompute || i < base));
        // The latest sample neither filled nor complete.
        let unfilled = (0..old.len()).rev().find(|k| !fills.contains_key(k));
        if let (true, Some(k)) = (lacks, unfilled) {
            let last = old.times()[k];
            self.gaps_until = Some(self.gaps_until.map_or(last, |g| g.max(last)));
        }
        for k in 0..old.len() {
            let t = old.times()[k];
            if let Some(f) = fills.get(&k) {
                let frames: Vec<&[f64]> = f.frames.iter().map(Vec::as_slice).collect();
                self.push_kept(t, &f.values, &frames);
                continue;
            }
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
            rows: &self.rows,
            meshes: &self.meshes,
            coordinates: &self.all_coordinates,
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
        Filters::compile(entries, &self.output_names, &params, &self.base_fields)
    }

    /// The layout of a sample and an empty history keeping what is wired.
    fn build_history(&mut self) {
        let names = self.scalar_names();
        let mut fields = self.base_fields.clone();
        let mut coordinates = self.base_coordinates.clone();
        let scalars = names;
        let mut labels = Vec::new();
        for c in &self.filters.cuts {
            fields.extend(c.fields());
            let coordinate = c.coordinate();
            // For labels only the coordinates' names matter.
            let mut g = Geometry::cells(c.nx, c.ny);
            for k in coordinates.iter().filter(|k| k.of == c.input && k.axis < 2) {
                g.names[k.axis] = Some(k.name.clone());
            }
            labels.extend(c.labels(&g));
            coordinates.extend(c.lines().into_iter().map(|l| Coordinate {
                name: coordinate.clone(),
                of: l,
                axis: 0,
            }));
        }
        let index = "t";
        let all: Vec<&(String, FieldShape)> = fields.iter().collect();
        // What views can draw: grids and tensors, points and populations.
        let of_kind = |grid: bool| -> Vec<String> {
            all.iter()
                .filter(|(_, s)| {
                    if grid {
                        // Fields of more axes too: views leave the others
                        // over (ADR 0086).
                        matches!(s, FieldShape::Grid { .. } | FieldShape::Tensor { .. })
                    } else {
                        matches!(s, FieldShape::Points { .. } | FieldShape::Population { .. })
                    }
                })
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
        self.rows = all
            .iter()
            .filter(|(_, s)| matches!(s, FieldShape::Rows { dim: 1 }))
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
        h.set_field_budget(self.share());
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
        // Outputs' units, for views that print them.
        let model = self.sim.model();
        let unit = |block: BlockId, port: &str| {
            model
                .block(block)
                .ok()
                .and_then(|b| b.spec().outputs().find(|p| p.name == port).map(|p| p.unit))
                .filter(|u| *u != Unit::DIMENSIONLESS)
                .map(|u| u.to_string())
        };
        for (name, s) in self.output_names.iter().zip(&self.sources) {
            if let Some(u) = unit(s.block, &s.port) {
                h.set_unit(name, &u);
            }
        }
        for ((name, _), f) in self.base_fields.iter().zip(&self.field_sources) {
            if let Some(u) = unit(f.block, &f.port) {
                h.set_unit(name, &u);
            }
            // Its axes' names, for views that leave some over (ADR 0086).
            let axes = model
                .block(f.block)
                .ok()
                .and_then(|b| {
                    b.spec()
                        .outputs()
                        .find(|p| p.name == f.port)
                        .map(|p| p.axis_names.clone())
                })
                .unwrap_or_default();
            if !axes.is_empty() {
                h.set_axes(name, axes);
            }
        }
        for (name, m) in &self.mesh_shapes {
            h.set_mesh(name, m.clone());
        }
        // A cut's lines have its grid's unit; its coordinate, the unit of
        // the grid's coordinate along the cut.
        for c in &self.filters.cuts {
            if let Some(u) = h.unit(&c.input).map(str::to_string) {
                for l in c.lines() {
                    h.set_unit(&l, &u);
                }
            }
            let axis = match c.settings.mode {
                simsandbox_views::cut::Mode::AlongX => Some(0),
                simsandbox_views::cut::Mode::AlongY => Some(1),
                simsandbox_views::cut::Mode::Free => None,
            };
            let along = axis.and_then(|a| {
                self.all_coordinates
                    .iter()
                    .find(|k| k.of == c.input && k.axis == a)
                    .and_then(|k| h.unit(&k.name).map(str::to_string))
            });
            if let Some(u) = along {
                h.set_unit(&c.coordinate(), &u);
            }
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
        self.update_probe_io();
        Ok(())
    }

    /// Which probes keep samples in memory, and which write their files
    /// as they record (ADR 0063): call after anything those depend on
    /// changes.
    fn update_probe_io(&mut self) {
        let read_by_filters: Vec<usize> = self
            .derived
            .iter()
            .flat_map(|d| d.sources.iter())
            .filter_map(|s| match s {
                Src::Probe(i) => Some(*i),
                _ => None,
            })
            .collect();
        let indexed = self.recorder.is_some();
        let mut notes = Vec::new();
        for (i, p) in self.probes.iter_mut().enumerate() {
            let s = &p.spec;
            let in_memory = self.record_all_probes
                || self.view_probes.contains(&s.name)
                || read_by_filters.contains(&i)
                || (s.to.is_some() && !s.stream);
            if in_memory != p.in_memory {
                // Started or stopped: from now on, nothing before.
                p.pending.clear();
                p.history.write().clear();
                p.in_memory = in_memory;
            }
            let wanted = match (&s.to, &self.probe_dir) {
                (Some(to), Some(dir)) if s.stream => Some(dir.join(to)),
                _ => None,
            };
            let current = p.stream.as_ref().map(|w| w.paths[0].clone());
            if wanted != current {
                p.stream = None;
                if let (Some(_), Some(dir), Some(to)) = (&wanted, &self.probe_dir, &s.to) {
                    let (names, fields) = p.layout();
                    match crate::probe::Stream::new(dir, to, &names, &fields, indexed) {
                        Ok((w, n)) => {
                            notes.extend(n.into_iter().map(|n| format!("probe {}: {n}", s.name)));
                            p.stream = Some(w);
                        }
                        Err(e) => notes.push(format!("probe {}: cannot write {to}: {e}", s.name)),
                    }
                }
            }
            if let Some(w) = &mut p.stream {
                w.set_indexed(indexed);
            }
            if s.stream && s.to.is_some() && self.probe_dir.is_none() {
                notes.push(format!(
                    "probe {}: it writes its file once the model is saved",
                    s.name
                ));
            }
        }
        for n in notes {
            if !self.warnings.contains(&n) {
                self.warnings.push(n);
            }
        }
    }

    /// Where probes write their files as they record: beside the model
    /// (ADR 0063). `None` while the model has no file.
    pub fn set_probe_dir(&mut self, dir: Option<std::path::PathBuf>) {
        if self.probe_dir != dir {
            self.probe_dir = dir;
            self.update_probe_io();
        }
    }

    /// Keep every probe's samples in memory, whatever reads them: a
    /// headless run returns them all.
    pub fn record_all_probes(&mut self) {
        self.record_all_probes = true;
        self.update_probe_io();
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
            ph.set_field_budget(self.share());
            spec.keep.apply(&mut ph);
            built.push(Extra {
                spec: spec.clone(),
                trigger: Trigger::new(spec.every.clone()),
                history: SharedHistory::new(ph),
                signals,
                fields,
                pending: Vec::new(),
                in_memory: true,
                stream: None,
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
        h.set_field_budget(self.share());
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
        self.set_probes(entries.iter().map(ProbeSpec::from_entry).collect())
    }

    /// The probes in the model file's form.
    pub fn probe_entries(&self) -> Vec<ProbeEntry> {
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
        for p in &mut self.probes {
            // Written as it records: already there, flushed above.
            if let Some(w) = &p.stream {
                written.extend(w.paths.iter().cloned());
                continue;
            }
            if let Some(to) = p.spec.to.as_ref().filter(|_| !p.spec.stream) {
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

    /// The frames of the cut filters, in order, given the frames of the
    /// fields they read.
    fn cut_frames<'a>(&self, frame: impl Fn(&str) -> Option<&'a [f64]>) -> Vec<Vec<f64>> {
        let coordinates = &self.all_coordinates;
        self.filters
            .cuts
            .iter()
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
        self.cut_frames(|name| {
            fields
                .iter()
                .position(|(f, _)| f == name)
                .and_then(|i| before.get(i).copied())
        })
    }

    pub fn time(&self) -> f64 {
        self.sim.time()
    }

    pub fn labels(&self) -> Option<MethodLabels> {
        self.sim.labels()
    }

    /// Advance one communication step and record it. A step whose study
    /// was given up (ADR 0071 step 2, [`crate::run::given_up`]) leaves the
    /// model as it was before it.
    pub fn step(&mut self) -> Result<(), SimError> {
        self.apply_due();
        let studied = self.block_types.iter().any(|t| t == simsandbox_file::STUDY);
        let before = studied.then(|| self.sim.snapshot());
        let states_before = self.iterates.then(|| self.state_bits());
        if let Err(e) = self.sim.step(self.dt) {
            if let Some(b) = before.filter(|_| crate::run::given_up(&e)) {
                self.sim.restore(&b);
            }
            return Err(e);
        }
        // A round that changes no state is the last that changes anything
        // (ADR 0083).
        if let Some(b) = states_before {
            if b == self.state_bits() {
                self.held_still = true;
                self.rounds_pending = false;
            }
        }
        self.record();
        let Some(r) = &self.recorder else {
            return Ok(());
        };
        self.steps_since_snapshot += 1;
        if self.steps_since_snapshot as u64 >= r.every() {
            self.steps_since_snapshot = 0;
            self.snapshots.push(self.sim.snapshot());
            self.fit_snapshots();
        }
        Ok(())
    }

    /// Step until the time reaches `t` (within rounding), recording every
    /// step: how headless interfaces run a model (ADR 0039).
    /// How long the next step is in simulated time: `dt`, or with several
    /// clocks the time to the next instant of any (ADR 0066 step 4b).
    pub fn step_dt(&self) -> f64 {
        self.sim
            .next_instant()
            .map_or(self.dt, |next| (next - self.time()).max(0.0))
    }

    pub fn run_until(&mut self, t: f64) -> Result<(), SimError> {
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

    /// The kernel model being run.
    pub fn sim_model(&self) -> &simsandbox_core::graph::Model {
        self.sim.model()
    }

    /// Each activated use (ADR 0088), by path: whether it is held now
    /// and how often it has started again since the run began.
    pub fn activations(&self) -> Vec<(String, bool, u64)> {
        self.sim
            .activations()
            .into_iter()
            .map(|(name, held, starts)| (name.to_string(), held, starts))
            .collect()
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
                driven: None,
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
                    driven: None,
                });
            }
        }
        // What wires drive (ADR 0075); a declared parameter of a use is
        // driven when the parameters set to it are.
        let driven = drives(model);
        for r in &mut rows {
            r.driven = driven
                .iter()
                .find(|(to, _)| *to == r.key)
                .map(|(_, from)| from.clone());
        }
        let blocks: Vec<(String, Option<String>, Option<String>)> = rows
            .iter()
            .filter(|r| r.block.is_some())
            .map(|r| (r.owner.clone(), r.formula.clone(), r.driven.clone()))
            .collect();
        for r in rows
            .iter_mut()
            .filter(|r| r.block.is_none() && !r.owner.is_empty())
        {
            let inside = format!("{}/", r.owner);
            r.driven = blocks
                .iter()
                .find(|(owner, formula, d)| {
                    owner.starts_with(&inside) && formula.as_deref() == Some(&r.name) && d.is_some()
                })
                .and_then(|(_, _, d)| d.clone());
        }
        rows
    }

    fn after_param_change(&mut self) -> Result<(), String> {
        if self.timeless && self.evaluate_later {
            self.unevaluated = true;
            return Ok(());
        }
        // An iteration starts again from round 0 (ADR 0083).
        if self.iterates && self.evaluate_later {
            return self.restart().map_err(|e| e.to_string());
        }
        if self.time() == 0.0 && self.sim.step_count() == 0 {
            self.restart().map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Whether it was built without running its studies, or changed since
    /// last evaluated with evaluation left for later: [`Session::evaluate`]
    /// brings it up to date.
    pub fn unevaluated(&self) -> bool {
        self.unevaluated
    }

    /// Nothing in it runs in time (ADR 0072): no block has states or
    /// tables or reads recorded data, there are no clocks and no scheduled
    /// inputs. The app evaluates such a model and never steps it.
    pub fn timeless(&self) -> bool {
        self.timeless
    }

    /// Only maps, with a last round, and nothing reads time or happens at a
    /// time (ADR 0083): a `t_end`; no continuous states, tables, clocks,
    /// data or scheduled inputs; and every block says it never reads `t`. The app iterates such a model
    /// as fast as it can instead of pacing it.
    pub fn iterates(&self) -> bool {
        self.iterates
    }

    /// An iterating model still has rounds to run: it has neither held
    /// still nor reached its last round since it opened or last changed.
    pub fn rounds_pending(&self) -> bool {
        self.rounds_pending
    }

    /// Stop iterating (its last round was reached, or a step failed).
    pub fn stop_rounds(&mut self) {
        self.rounds_pending = false;
    }

    /// Its last round changed no state, so no later round changes anything.
    pub fn held_still(&self) -> bool {
        self.held_still
    }

    /// Rounds run since it started.
    pub fn round(&self) -> u64 {
        self.sim.step_count()
    }

    /// Every block's state and tables (rows, ids and values), by bits.
    fn state_bits(&self) -> Vec<u64> {
        let mut bits = Vec::new();
        for &id in self.ids.values() {
            bits.extend(self.sim.state(id).iter().map(|v| v.to_bits()));
            for t in self.sim.tables(id) {
                bits.push(t.len() as u64);
                bits.extend(t.ids());
                for c in 0..t.columns() {
                    bits.extend(t.column(c).iter().map(|v| v.to_bits()));
                }
            }
        }
        bits
    }

    fn only_rounds(&self) -> bool {
        let model = self.sim.model();
        // A last round: an open-ended map (the logistic map) is watched as
        // it goes, paced as any model.
        self.file.run.t_end.is_some()
            && model.clocks().0.is_empty()
            && self.track.schedule().is_empty()
            && self.ids.values().all(|&id| {
                model.block(id).is_ok_and(|b| {
                    // Tables only of blocks that say they never read
                    // time: rows that change by rules each round. A block
                    // integrated in time runs in time, whatever holds its
                    // states (rows of tables, ADR 0085 step 3).
                    b.reads_time() == Some(false)
                        && b.instants().is_none()
                        && b.spec().time != TimeSemantics::Continuous
                        && (b.spec().states.is_empty()
                            || b.spec().time == TimeSemantics::DiscreteEvent)
                })
            })
    }

    fn nothing_in_time(&self) -> bool {
        let model = self.sim.model();
        model.clocks().0.is_empty()
            && self.track.schedule().is_empty()
            && self.ids.values().all(|&id| {
                model.block(id).is_ok_and(|b| {
                    b.spec().states.is_empty()
                        && b.spec().tables.is_empty()
                        && b.instants().is_none()
                })
            })
    }

    /// Leave changes to a model with nothing in time unevaluated, for the
    /// caller to [`Session::evaluate`] where it can show progress and give
    /// a study up (the app's worker, ADR 0072).
    pub fn evaluate_later(&mut self) {
        self.evaluate_later = true;
    }

    /// Bring the model up to date (a restart: the history starts over with
    /// its values): run the studies of a model built without running them,
    /// or evaluate a model with nothing in time after a change. A study
    /// given up ([`crate::run::given_up`]) leaves it unevaluated, to run
    /// again.
    pub fn evaluate(&mut self) -> Result<(), SimError> {
        let r = self.restart();
        // A study that failed is reported once, not run again and again.
        if r.as_ref().is_err_and(|e| !crate::run::given_up(e)) {
            self.unevaluated = false;
        }
        r
    }

    /// Start again at t = 0 from the current parameter values. History and
    /// plots start over.
    pub fn restart(&mut self) -> Result<(), SimError> {
        // A new run: recorded input is dropped; what the schedule sets
        // starts from the model's own values and is set again.
        self.track.clear_recorded();
        for (key, value) in self.track.base().to_vec() {
            if let Err(e) = self.set_value(&key, value) {
                self.warnings.push(e);
            }
        }
        let r = self.restart_keeping_recorded();
        self.rounds_pending = self.iterates;
        self.held_still = false;
        // Any restart outside `without_running_studies` runs them.
        if r.is_ok() && crate::study::studies_run() {
            self.unevaluated = false;
        }
        r
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
        self.snapshots = match self.recorder {
            Some(_) => vec![self.sim.snapshot()],
            None => Vec::new(),
        };
        self.steps_since_snapshot = 0;
        self.stop_at = self
            .file
            .run
            .t_end
            .or_else(|| crate::run::data_end(self.sim.model()));
        self.history_mut().clear();
        self.gaps_until = None;
        for p in &mut self.probes {
            p.pending.clear();
            p.history.write().clear();
            p.trigger.seek(None, 0.0);
            if let Some(w) = &mut p.stream {
                let _ = w.truncate_after(None);
            }
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
        self.file.controls.recorder = self.recorder.clone();
        self.file.controls.probes = self.probe_entries();
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

    /// Whether dragged sliders apply while dragged (ADR 0037): every method
    /// is real-time.
    pub fn real_time(&self) -> bool {
        self.labels()
            .is_some_and(|l| l.speed == simsandbox_core::block::SpeedClass::RealTime)
    }

    /// Thin the snapshots to the recorder's budget.
    fn fit_snapshots(&mut self) {
        let Some(r) = &self.recorder else {
            return;
        };
        // Its own budget, or its share of the app's.
        let mb = r.mb.unwrap_or(self.memory_mb / 4.0);
        let budget = (mb * 1e6 / 8.0) as usize;
        let held = |s: &[Snapshot]| s.iter().map(Snapshot::values).sum::<usize>();
        while self.snapshots.len() > 2
            && (self.snapshots.len() > MAX_SNAPSHOTS || held(&self.snapshots) > budget)
        {
            thin(&mut self.snapshots);
        }
    }

    /// A quarter of the memory budget, in bytes: the default share of
    /// snapshots, the history and each probe (ADR 0062 step 5).
    fn share(&self) -> usize {
        (self.memory_mb * 1e6 / 4.0) as usize
    }

    /// The app's memory budget in MB (ADR 0062 step 5).
    pub fn memory_budget(&self) -> f64 {
        self.memory_mb
    }

    /// Set the memory budget in MB (ADR 0062 step 5): snapshots (unless
    /// the recorder sets its own), the history, each probe that keeps
    /// what it records by default, and each filter thin past a quarter of
    /// it. What is held now thins at the next sample.
    pub fn set_memory_budget(&mut self, mb: f64) {
        if !(mb.is_finite() && mb > 0.0) {
            return;
        }
        self.memory_mb = mb;
        let share = self.share();
        self.history.write().set_field_budget(share);
        for p in &self.probes {
            p.history.write().set_field_budget(share);
        }
        for d in &self.derived {
            d.history.write().set_field_budget(share);
        }
        self.fit_snapshots();
    }

    /// What the recordings hold against the memory budget (ADR 0062 step
    /// 5): snapshots, the history, each probe and filter.
    pub fn memory_use(&self) -> MemoryUse {
        let mut parts = vec![
            (
                "snapshots".to_string(),
                8 * self.snapshots.iter().map(Snapshot::values).sum::<usize>(),
            ),
            ("history".to_string(), self.history.read().bytes()),
        ];
        for p in &self.probes {
            parts.push((format!("probe {}", p.spec.name), p.history.read().bytes()));
        }
        for d in &self.derived {
            parts.push((format!("filter {}", d.entry.name), d.history.read().bytes()));
        }
        MemoryUse {
            budget: (self.memory_mb * 1e6) as usize,
            held: parts.iter().map(|p| p.1).sum(),
            parts,
        }
    }

    /// The recorder, if the model has one (ADR 0064).
    pub fn recorder(&self) -> Option<&simsandbox_file::RecorderEntry> {
        self.recorder.as_ref()
    }

    /// Add, change or remove the recorder (ADR 0064). Added during a run,
    /// it can rewind to now and later; removed, its snapshots go.
    pub fn set_recorder(&mut self, recorder: Option<simsandbox_file::RecorderEntry>) {
        match (&self.recorder, &recorder) {
            (None, Some(_)) => {
                self.snapshots = vec![self.sim.snapshot()];
                self.steps_since_snapshot = 0;
            }
            (_, None) => self.snapshots.clear(),
            _ => {}
        }
        self.recorder = recorder;
        self.fit_snapshots();
        self.update_probe_io();
    }

    /// Earliest and latest time the run can be rewound to.
    pub fn history_range(&self) -> (f64, f64) {
        let first = self.snapshots.first().map_or(self.time(), |s| s.t);
        let last = self.snapshots.last().map_or(first, |s| s.t);
        (first, last)
    }

    /// Every scalar and every field's frame of the model now, `keep`
    /// fields only (the rest empty), as a sample at its time.
    fn sample_now(&self) -> Pending {
        let t = self.sim.time();
        let values = self.sample(t);
        let before = frames(&self.sim, &self.field_sources);
        let cuts = self.sample_cuts(&before);
        let mut all: Vec<Vec<f64>> = before.iter().map(|f| f.to_vec()).chain(cuts).collect();
        for (i, f) in all.iter_mut().enumerate() {
            if !self.keep_fields.contains(&i) {
                *f = Vec::new();
            }
        }
        Pending {
            index: t,
            values,
            frames: all,
        }
    }

    /// Whether the history lacks, for samples it holds, something views
    /// read now (wired after those samples were recorded, ADR 0063):
    /// [`Session::recompute`] fills it.
    pub fn has_gaps(&self) -> bool {
        let first = self.history.read().times().first().copied();
        matches!((self.gaps_until, first), (Some(g), Some(f)) if g >= f)
    }

    /// Fill the history's gaps (ADR 0062, ADR 0063: on demand, never
    /// automatically): run again from the latest snapshot before them,
    /// replaying the input as recorded, and record what views read now at
    /// the times of the samples held. The model is put back as it was, at
    /// the same time; it computes the same values bit for bit, so samples
    /// recorded before are unchanged. Probes and filters with clocks of
    /// their own are not filled. Needs the recorder's snapshots.
    pub fn recompute(&mut self) -> Result<(), String> {
        if !self.has_gaps() {
            return Ok(());
        }
        let until = self.gaps_until.expect("has gaps");
        let first = self.history.read().times()[0];
        let Some(snap) = self.snapshots.iter().rev().find(|s| s.t <= first) else {
            return Err("recomputing needs the recorder's snapshots (ADR 0064)".into());
        };
        let snap = snap.clone();
        drop(self.history_mut());
        let times: Vec<f64> = self.history.read().times().to_vec();
        let now = self.sim.snapshot();
        let track = self.track.position();
        let plan = self.plan.clone();
        let warnings = self.warnings.len();
        let eps = self.eps();
        self.sim.restore(&snap);
        self.track.seek(snap.t, eps);
        let mut fills: BTreeMap<usize, Pending> = BTreeMap::new();
        let mut k = times.partition_point(|&t| t < snap.t - eps);
        let mut failed = None;
        while k < times.len() && times[k] <= until + eps {
            let t = self.sim.time();
            if (times[k] - t).abs() <= eps {
                fills.insert(k, self.sample_now());
                k += 1;
                continue;
            }
            if times[k] < t - eps {
                // A sample between steps (none are, but never loop).
                k += 1;
                continue;
            }
            self.apply_due();
            if let Err(e) = self.sim.step(self.dt) {
                failed = Some(e.to_string());
                break;
            }
        }
        self.sim.restore(&now);
        self.track.set_position(track);
        self.plan = plan;
        self.warnings.truncate(warnings);
        if let Some(e) = failed {
            return Err(format!("recomputing failed: {e}"));
        }
        let old = self.replace_history(History::new(Vec::new()));
        self.gaps_until = None;
        self.build_history();
        self.replay_history(&old, false, &fills);
        Ok(())
    }

    /// Rewind to the latest snapshot at or before `t`. Everything recorded
    /// after it is discarded, so running on starts a new branch.
    /// Without a recorder (ADR 0064) there is nothing to rewind to.
    pub fn rewind_to(&mut self, t: f64) {
        if self.snapshots.is_empty() {
            return;
        }
        let keep = self.snapshots.iter().rposition(|s| s.t <= t).unwrap_or(0);
        self.snapshots.truncate(keep + 1);
        let snap = &self.snapshots[keep];
        self.sim.restore(snap);
        let t0 = snap.t;
        // Input from then on is ahead again: running on replays it.
        let eps = self.eps();
        self.track.seek(t0, eps);
        self.stop_at = self
            .file
            .run
            .t_end
            .or_else(|| crate::run::data_end(self.sim.model()))
            .filter(|&end| end > t0);
        self.history_mut().truncate_after(t0);
        for p in &mut self.probes {
            p.flush(true);
            p.history.write().truncate_after(t0);
            p.trigger.seek(Some(t0), eps);
            if let Some(w) = &mut p.stream {
                let _ = w.truncate_after(Some(t0));
            }
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
        // Replay the recorded outputs and fields through the new filters;
        // probes start afresh from now.
        drop(self.history_mut());
        let old = self.replace_history(History::new(Vec::new()));
        self.build_history();
        self.replay_history(&old, true, &BTreeMap::new());
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

/// One note per algebraic loop (ADR 0040), saying how it is solved, and
/// one per loop of equation blocks stepped apart ([`apart_notes`]).
pub fn loop_notes(sim: &Simulator) -> Vec<String> {
    let method = sim.model().loop_method().name();
    let mut notes: Vec<String> = sim
        .loops()
        .into_iter()
        .map(|l| algebraic_note(&l, method))
        .collect();
    notes.extend(apart_notes(sim.model()));
    notes
}

/// The note of [`loop_notes`] for one algebraic loop, by its outputs.
pub(crate) fn algebraic_note(outputs: &[String], method: &str) -> String {
    format!(
        "algebraic loop {}: solved at every step by '{method}' (run.loops)",
        outputs.join(" -> ")
    )
}

/// One note per loop of equation blocks that could be solved together
/// and are not (ADR 0095): blocks wired in a circle, continuous or
/// stateless, at least one with states, not all in one fused group
/// (different methods, `fusion: off`, different clocks, an activated
/// part's edge). Each then reads the others' outputs as they were at the
/// start of the step, an error of the order of the step whatever the
/// methods' own accuracy, which their labels do not show.
pub fn apart_notes(model: &simsandbox_core::graph::Model) -> Vec<String> {
    apart_loops(model).iter().map(|l| apart_note(l)).collect()
}

/// The note of [`apart_notes`] for one loop, by its blocks' names.
pub(crate) fn apart_note(names: &[String]) -> String {
    format!(
        "{} depend on each other and are stepped apart: each reads the others' outputs \
         from the start of the step, an error of the order of run.dt that the accuracy \
         label does not show. To solve them together give them one method and leave \
         fusion on, or list them in 'groups'",
        names.join(", ")
    )
}

/// The loops [`apart_notes`] reports, each as its blocks' names.
pub(crate) fn apart_loops(model: &simsandbox_core::graph::Model) -> Vec<Vec<String>> {
    let n = model.len();
    let mut next = vec![Vec::new(); n];
    let mut prev = vec![Vec::new(); n];
    for c in model.connections() {
        let (a, b) = (c.from.block.0, c.to.block.0);
        if a != b && model.is_fusable(c.from.block) && model.is_fusable(c.to.block) {
            next[a].push(b);
            prev[b].push(a);
        }
    }
    // Strongly connected parts (Kosaraju, without recursion): blocks in
    // the order they are finished going forward, then gathered going
    // backward from the last finished.
    let mut order = Vec::with_capacity(n);
    let mut seen = vec![false; n];
    for start in 0..n {
        if seen[start] {
            continue;
        }
        seen[start] = true;
        let mut stack = vec![(start, 0)];
        while let Some((v, k)) = stack.last_mut() {
            if let Some(&w) = next[*v].get(*k) {
                *k += 1;
                if !seen[w] {
                    seen[w] = true;
                    stack.push((w, 0));
                }
            } else {
                order.push(*v);
                stack.pop();
            }
        }
    }
    let mut part = vec![usize::MAX; n];
    let mut parts: Vec<Vec<usize>> = Vec::new();
    for &start in order.iter().rev() {
        if part[start] != usize::MAX {
            continue;
        }
        let k = parts.len();
        part[start] = k;
        let mut members = vec![start];
        let mut at = 0;
        while at < members.len() {
            for &w in &prev[members[at]] {
                if part[w] == usize::MAX {
                    part[w] = k;
                    members.push(w);
                }
            }
            at += 1;
        }
        parts.push(members);
    }
    let id = simsandbox_core::graph::BlockId;
    let mut notes = Vec::new();
    for mut members in parts {
        let together = model
            .group_of(id(members[0]))
            .is_some_and(|g| members.iter().all(|&m| model.group_of(id(m)) == Some(g)));
        if members.len() < 2 || together || members.iter().all(|&m| model.is_algebraic(id(m))) {
            continue;
        }
        members.sort_unstable();
        let names: Vec<String> = members
            .iter()
            .filter_map(|&m| model.block(id(m)).ok().map(|b| b.spec().name.clone()))
            .collect();
        notes.push(names);
    }
    notes
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
mod tests;
