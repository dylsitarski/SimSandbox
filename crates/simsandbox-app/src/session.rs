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
    FilterEntry, FusedGroup, ModelFile, SweepControl, ViewEntry, DEFAULT_GRID_POINTS,
    DEFAULT_SWEEP_POINTS, MAX_GRID_EVALUATIONS,
};
use simsandbox_views::cut::Geometry;
use simsandbox_views::history::thin;
use simsandbox_views::{Available, Coordinate, FieldShape, History, SignalRef};
use std::collections::BTreeMap;

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
    len: usize,
}

/// One parameter as the panel shows it.
#[derive(Clone)]
pub struct ParamRow {
    /// `"block.param"`, the key used in `controls.params`.
    pub key: String,
    pub block: BlockId,
    pub name: String,
    pub unit: String,
    pub description: String,
    pub suggested: [f64; 2],
    pub initial_condition: bool,
    pub value: f64,
}

/// One swept parameter: `points` values evenly spread over `range`.
#[derive(Debug, Clone, PartialEq)]
pub struct SweepAxis {
    /// `"block.param"`.
    pub key: String,
    pub block: BlockId,
    pub param: String,
    pub range: [f64; 2],
    pub points: usize,
}

impl SweepAxis {
    /// The `k`th value. Weighted this way, the first and last points hit the
    /// range ends exactly.
    pub fn value(&self, k: usize) -> f64 {
        let f = k as f64 / (self.points - 1) as f64;
        self.range[0] * (1.0 - f) + self.range[1] * f
    }
}

/// Parameters swept across ranges, for models with no states: the outputs
/// are evaluated at each value instead of running in time. One parameter
/// (ADR 0021) gives signals against it; two (ADR 0022) give every output
/// as a grid over both, first parameter along x.
#[derive(Debug, Clone, PartialEq)]
pub struct Sweep {
    pub axes: Vec<SweepAxis>,
}

impl Sweep {
    /// Two parameters: the results are grids.
    pub fn is_grid(&self) -> bool {
        self.axes.len() == 2
    }
}

pub struct Session {
    /// Display name, e.g. the file name.
    pub title: String,
    pub description: Option<String>,
    /// The file as loaded; the source for saving.
    pub file: ModelFile,
    pub ids: BTreeMap<String, BlockId>,
    pub warnings: Vec<String>,
    /// Blocks solved together as one system (ADR 0023).
    pub groups: Vec<FusedGroup>,
    pub dt: f64,
    /// Recorded signals, one per plotted output element (ADR 0015); in a
    /// two-parameter sweep, one grid per output element instead.
    pub signals: History,
    sources: Vec<Source>,
    /// Names of the scalar output signals, in `sources` order.
    output_names: Vec<String>,
    /// Grid and point-set outputs and their shapes.
    base_fields: Vec<(String, FieldShape)>,
    /// The swept parameters' keys when their values are recorded as
    /// coordinate grids (two-parameter sweeps).
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
    sweep: Option<Sweep>,
    /// Filters as written, for saving and editing (ADR 0030, ADR 0032).
    filter_entries: Vec<FilterEntry>,
    /// Filters compiled: expressions recorded after the output signals,
    /// cut fields after the recorded fields.
    filters: Filters,
}

impl Session {
    /// Load a model file. Errors are formatted for display.
    pub fn load(title: &str, text: &str, registry: &Registry) -> Result<Self, String> {
        let loaded = simsandbox_file::load(text, registry).map_err(|e| e.to_string())?;
        let mut names = Vec::new();
        let mut sources = Vec::new();
        let mut fields = Vec::new();
        let mut field_sources = Vec::new();
        let mut coordinates: Vec<Coordinate> = Vec::new();
        let mut block_types = Vec::new();
        let plots = &loaded.file.controls.plots;
        for entry in &loaded.file.blocks {
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
                        len: port.shape.len(),
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
        let sim = Simulator::new(loaded.model, 0.0).map_err(|e| e.to_string())?;
        let sweep = default_sweep(&loaded.file, &loaded.ids, sim.model());
        let warnings = loaded.warnings;
        let entries = loaded.file.controls.filters.clone();
        let mut session = Self {
            title: title.into(),
            description: loaded.file.description.clone(),
            dt: loaded.file.run.dt,
            stop_at: loaded.file.run.t_end,
            file: loaded.file,
            ids: loaded.ids,
            groups: loaded.groups,
            warnings,
            signals: History::new(Vec::new()),
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
            session.recompute_sweep();
        } else {
            session.rebuild_history();
            session.record();
        }
        Ok(session)
    }

    /// What views can bind: the scalar signals, grids, point sets and the
    /// model's block types.
    pub fn available(&self) -> Available<'_> {
        Available {
            scalars: self.signals.names(),
            grids: &self.grids,
            points: &self.points,
            block_types: &self.block_types,
            lines: &self.lines,
            coordinates: self.signals.coordinates(),
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
    /// input: the grid outputs, or in a two-parameter sweep a grid per
    /// scalar (named in `scalars`) and per swept parameter.
    fn source_grids(&self, scalars: &[String]) -> Vec<(String, FieldShape)> {
        match &self.sweep {
            Some(sw) if sw.is_grid() => {
                let shape = FieldShape::Grid {
                    nx: sw.axes[0].points,
                    ny: sw.axes[1].points,
                };
                scalars
                    .iter()
                    .cloned()
                    .chain(sw.axes.iter().map(|a| a.key.clone()))
                    .map(|n| (n, shape))
                    .collect()
            }
            _ => self.base_fields.clone(),
        }
    }

    /// Compile filters for this model; problems are `name: reason`.
    fn compile_filters(&self, entries: &[FilterEntry]) -> (Filters, Vec<String>) {
        let params = param_keys(&self.file, &self.ids, self.sim.model());
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

    /// An empty history for the current mode: signals against time or one
    /// swept parameter, or grids over two swept parameters; then the cut
    /// filters' fields.
    fn rebuild_history(&mut self) {
        let names = self.scalar_names();
        let mut fields = self.source_grids(&names);
        let mut coordinates = match &self.sweep {
            // The swept parameters' values place every result's cells.
            Some(sw) if sw.is_grid() => sw
                .axes
                .iter()
                .enumerate()
                .flat_map(|(axis, a)| {
                    names.iter().map(move |n| Coordinate {
                        name: a.key.clone(),
                        of: n.clone(),
                        axis,
                    })
                })
                .collect(),
            _ => self.base_coordinates.clone(),
        };
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
        let index = match &self.sweep {
            Some(sw) if sw.is_grid() => "sweep",
            Some(sw) => sw.axes[0].key.as_str(),
            None => "t",
        };
        let scalars = match &self.sweep {
            Some(sw) if sw.is_grid() => Vec::new(),
            _ => names,
        };
        let of_kind = |grid: bool| -> Vec<String> {
            fields
                .iter()
                .filter(|(_, s)| matches!(s, FieldShape::Grid { .. }) == grid)
                .map(|(n, _)| n.clone())
                .collect()
        };
        self.grids = of_kind(true);
        self.points = of_kind(false);
        self.signals = History::with_fields(index, scalars, fields);
        self.signals.set_coordinates(coordinates);
        for (name, label) in labels {
            self.signals.set_label(&name, &label);
        }
        self.lines = self
            .signals
            .fields()
            .iter()
            .filter(|f| matches!(f.shape, FieldShape::Grid { ny: 1, .. }))
            .map(|f| f.name.clone())
            .collect();
    }

    /// The cut filters' frames for one sample, from the frames of the
    /// fields recorded before them (in history order).
    fn cut_frames(&self, before: &[&[f64]]) -> Vec<Vec<f64>> {
        let fields = self.signals.fields();
        let frame = |name: &str| -> Option<&[f64]> {
            fields
                .iter()
                .position(|f| f.name == name)
                .and_then(|i| before.get(i).copied())
        };
        let coordinates = self.signals.coordinates();
        self.filters
            .cuts
            .iter()
            .flat_map(|c| match frame(&c.input) {
                Some(input) => {
                    let g = c.geometry(coordinates, frame);
                    c.frames(&g, input)
                }
                // Recorded as NaN.
                None => vec![Vec::new(); c.settings.count() + 1],
            })
            .collect()
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

    /// Every parameter, in file block order.
    pub fn params(&self) -> Vec<ParamRow> {
        let model = self.sim.model();
        let mut rows = Vec::new();
        for entry in &self.file.blocks {
            let id = self.ids[&entry.name];
            let (Ok(block), Ok(values)) = (model.block(id), model.params(id)) else {
                continue;
            };
            for (p, &value) in block.spec().params.iter().zip(values) {
                rows.push(ParamRow {
                    key: format!("{}.{}", entry.name, p.name),
                    block: id,
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

    /// Change a parameter while running. Initial conditions apply on
    /// [`Session::restart`], or at once if the run has not started.
    pub fn set_param(&mut self, block: BlockId, name: &str, value: f64) -> Result<(), String> {
        self.sim
            .set_param(block, name, value)
            .map_err(|e| e.to_string())?;
        if self.sweep.is_some() {
            self.recompute_sweep();
        } else if self.time() == 0.0 && self.snapshots.len() == 1 {
            self.restart().map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Start again at t = 0 from the current parameter values. History and
    /// plots start over.
    pub fn restart(&mut self) -> Result<(), SimError> {
        if self.sweep.is_some() {
            self.recompute_sweep();
            return Ok(());
        }
        self.sim.restart(0.0)?;
        self.snapshots = vec![self.sim.snapshot()];
        self.steps_since_snapshot = 0;
        self.stop_at = self.file.run.t_end;
        self.signals.clear();
        self.record();
        Ok(())
    }

    /// The model file with the current parameters, step, default speed and
    /// views (ADR 0012, ADR 0013, ADR 0015), ready to write.
    pub fn save_json(
        &mut self,
        speed: f64,
        views: Option<Vec<ViewEntry>>,
    ) -> Result<String, String> {
        self.file
            .capture_params(self.sim.model(), &self.ids)
            .map_err(|e| e.to_string())?;
        self.file.run.dt = self.dt;
        self.file.controls.speed = Some(speed);
        if let Some(v) = views {
            self.file.controls.views = v;
        }
        self.file.controls.filters = self.filter_entries.clone();
        if let Some(sw) = &self.sweep {
            self.file.controls.sweep = sw
                .axes
                .iter()
                .map(|a| SweepControl {
                    param: a.key.clone(),
                    range: Some(a.range),
                    points: Some(a.points),
                })
                .collect();
        }
        Ok(self.file.to_json())
    }

    /// The slider range the file sets for a parameter, if any.
    pub fn slider_range(&self, key: &str) -> Option<[f64; 2]> {
        self.file.controls.params.get(key).and_then(|c| c.range)
    }

    /// The sweep, if the model is swept instead of run.
    pub fn sweep(&self) -> Option<&Sweep> {
        self.sweep.as_ref()
    }

    /// Change what is swept: one or two `("block.param", range, points)`.
    /// Recomputes at once.
    pub fn set_sweep(&mut self, axes: &[(String, [f64; 2], usize)]) -> Result<(), String> {
        if self.sweep.is_none() {
            return Err("this model has states, so it runs in time instead".into());
        }
        if !(1..=2).contains(&axes.len()) {
            return Err("one or two parameters can be swept".into());
        }
        let mut built: Vec<SweepAxis> = Vec::new();
        for (key, range, points) in axes {
            let (block, param) = key
                .split_once('.')
                .and_then(|(b, p)| Some((*self.ids.get(b)?, p.to_string())))
                .ok_or_else(|| format!("no parameter '{key}'"))?;
            if self.sim.param(block, &param).is_err() {
                return Err(format!("no parameter '{key}'"));
            }
            if built.iter().any(|a| a.key == *key) {
                return Err(format!("'{key}' is swept twice"));
            }
            if !(range[0].is_finite() && range[1].is_finite() && range[0] < range[1]) {
                return Err(format!(
                    "[{}, {}] is not an increasing range",
                    range[0], range[1]
                ));
            }
            built.push(SweepAxis {
                key: key.clone(),
                block,
                param,
                range: *range,
                points: (*points).clamp(2, 100_000),
            });
        }
        if let [a, b] = built.as_slice() {
            if a.points * b.points > MAX_GRID_EVALUATIONS {
                return Err(format!(
                    "{} × {} points is more than {MAX_GRID_EVALUATIONS}",
                    a.points, b.points
                ));
            }
        }
        self.sweep = Some(Sweep { axes: built });
        self.recompute_sweep();
        Ok(())
    }

    /// Evaluate the outputs at every value of the swept parameters.
    fn recompute_sweep(&mut self) {
        let Some(sw) = self.sweep.clone() else {
            return;
        };
        self.rebuild_history();
        let originals: Vec<f64> = sw
            .axes
            .iter()
            .map(|a| self.sim.param(a.block, &a.param).unwrap_or(f64::NAN))
            .collect();
        let set = |sim: &mut Simulator, values: &[f64]| {
            sw.axes
                .iter()
                .zip(values)
                .all(|(a, &v)| sim.set_param(a.block, &a.param, v).is_ok())
                && sim.restart(0.0).is_ok()
        };
        if let [x, y] = sw.axes.as_slice() {
            // Grids over (x, y), row by row: cell (i, j) at j * nx + i.
            let (nx, ny) = (x.points, y.points);
            let names = self.scalar_names().len();
            let mut grids = vec![vec![f64::NAN; nx * ny]; names + 2];
            for j in 0..ny {
                for i in 0..nx {
                    let (vx, vy) = (x.value(i), y.value(j));
                    let cell = j * nx + i;
                    grids[names][cell] = vx;
                    grids[names + 1][cell] = vy;
                    if set(&mut self.sim, &[vx, vy]) {
                        for (g, v) in grids.iter_mut().zip(self.sample(0.0)) {
                            g[cell] = v;
                        }
                    }
                }
            }
            let frames: Vec<&[f64]> = grids.iter().map(Vec::as_slice).collect();
            let cuts = self.cut_frames(&frames);
            push(&mut self.signals, 0.0, &[], &frames, &cuts);
        } else {
            let a = &sw.axes[0];
            for k in 0..a.points {
                let v = a.value(k);
                if !set(&mut self.sim, &[v]) {
                    continue;
                }
                let values = self.sample(v);
                let frames = frames(&self.sim, &self.field_sources);
                let cuts = self.cut_frames(&frames);
                push(&mut self.signals, v, &values, &frames, &cuts);
            }
        }
        // Leave the model as the panel shows it.
        set(&mut self.sim, &originals);
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
        self.signals.truncate_after(t0);
        self.steps_since_snapshot = 0;
    }

    fn record(&mut self) {
        let t = self.sim.time();
        let values = self.sample(t);
        let frames = frames(&self.sim, &self.field_sources);
        let cuts = self.cut_frames(&frames);
        push(&mut self.signals, t, &values, &frames, &cuts);
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
        vars.extend(self.params().iter().map(|r| r.value));
        for (k, f) in exprs.iter().enumerate() {
            let v = f.expr.eval(&vars);
            vars[1 + outputs + k] = v;
            values.push(v);
        }
    }

    /// The filters as written (ADR 0032).
    pub fn filters(&self) -> &[FilterEntry] {
        &self.filter_entries
    }

    /// The 2D grids a cut filter can take: recorded grids that are not
    /// themselves coordinates (in a two-parameter sweep, every result).
    pub fn cut_inputs(&self) -> Vec<String> {
        self.signals
            .fields()
            .iter()
            .filter(|f| matches!(f.shape, FieldShape::Grid { nx, ny } if nx >= 2 && ny >= 2))
            .filter(|f| !self.signals.coordinates().iter().any(|c| c.name == f.name))
            .map(|f| f.name.clone())
            .collect()
    }

    /// Where the cells of grid `input` lie, from the newest sample.
    pub fn cut_geometry(&self, input: &str) -> Geometry {
        let h = &self.signals;
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
    pub fn default_cut(&self, name: &str, input: &str) -> FilterEntry {
        let g = self.cut_geometry(input);
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
            settings: [
                ("mode".to_string(), serde_json::Value::from("along_x")),
                ("at".to_string(), serde_json::Value::from(at)),
            ]
            .into(),
        }
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
            self.recompute_sweep();
            return Ok(());
        }
        // Replay the recorded outputs and fields through the new filters.
        let old = std::mem::replace(&mut self.signals, History::new(Vec::new()));
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
            let cuts = self.cut_frames(&frames);
            push(&mut self.signals, t, &values, &frames, &cuts);
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

/// Record one sample: scalar values, the frames of the fields before the
/// cuts, then the cuts' frames.
fn push(history: &mut History, index: f64, values: &[f64], before: &[&[f64]], cuts: &[Vec<f64>]) {
    let frames: Vec<&[f64]> = before
        .iter()
        .copied()
        .chain(cuts.iter().map(Vec::as_slice))
        .collect();
    history.push_sample(index, values, &frames);
}

/// The current value of every recorded field (empty if missing, which the
/// history records as NaN).
fn frames<'a>(sim: &'a Simulator, sources: &[FieldSource]) -> Vec<&'a [f64]> {
    sources
        .iter()
        .map(|f| {
            sim.output(f.block, &f.port)
                .filter(|v| v.len() == f.len)
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

/// The sweep for a model with no states: the file's `controls.sweep`, or
/// the first parameter over its slider or suggested range. `None` if any
/// block has states.
fn default_sweep(
    file: &ModelFile,
    ids: &BTreeMap<String, BlockId>,
    model: &Model,
) -> Option<Sweep> {
    let mut first = None;
    for entry in &file.blocks {
        let spec = model.block(ids[&entry.name]).ok()?.spec();
        if !spec.states.is_empty() {
            return None;
        }
        if first.is_none() {
            if let Some(p) = spec.params.first() {
                first = Some(format!("{}.{}", entry.name, p.name));
            }
        }
    }
    let saved = &file.controls.sweep;
    let wanted: Vec<(String, Option<[f64; 2]>, Option<usize>)> = if saved.is_empty() {
        vec![(first?, None, None)]
    } else {
        saved
            .iter()
            .map(|s| (s.param.clone(), s.range, s.points))
            .collect()
    };
    let default_points = if wanted.len() == 2 {
        DEFAULT_GRID_POINTS
    } else {
        DEFAULT_SWEEP_POINTS
    };
    let mut axes = Vec::new();
    for (key, range, points) in wanted {
        let (block_name, param) = key.split_once('.')?;
        let block = *ids.get(block_name)?;
        let spec = model
            .block(block)
            .ok()?
            .spec()
            .params
            .iter()
            .find(|p| p.name == param)?
            .clone();
        let range = range
            .or_else(|| file.controls.params.get(&key).and_then(|c| c.range))
            .unwrap_or([spec.suggested_min, spec.suggested_max]);
        axes.push(SweepAxis {
            key: key.clone(),
            block,
            param: param.into(),
            range,
            points: points.unwrap_or(default_points),
        });
    }
    Some(Sweep { axes })
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
            s.signals.names(),
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
        let reference: Vec<_> = s.signals.points(0).to_vec();

        s.rewind_to(1.234);
        // Latest snapshot at or before 1.234 (snapshots every 0.1 s).
        assert!((s.time() - 1.2).abs() < 1e-9, "{}", s.time());
        assert!(s.signals.points(0).last().unwrap().x <= s.time());
        while s.signals.points(0).len() < reference.len() {
            s.step().unwrap();
        }
        assert_eq!(s.signals.points(0), reference.as_slice());

        s.rewind_to(0.0);
        assert_eq!(s.time(), 0.0);
        assert_eq!(s.signals.points(0).len(), 1);
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
        assert_eq!(s.signals.points(0)[0].y, 5.0);

        for _ in 0..50 {
            s.step().unwrap();
        }
        s.set_param(id, "x0", -3.0).unwrap();
        assert!(s.time() > 0.0, "a running model is not restarted");
        s.restart().unwrap();
        assert_eq!(s.time(), 0.0);
        assert_eq!(s.signals.points(0).len(), 1);
        assert_eq!(s.signals.points(0)[0].y, -3.0);
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
        let r = s.signals.resolve(signal).unwrap();
        s.signals.values(r)
    }

    #[test]
    fn formula_models_are_swept_not_run() {
        let mut s = example("Resonance");
        let sw = s.sweep().unwrap().axes[0].clone();
        assert_eq!(
            (sw.key.as_str(), sw.range, sw.points),
            ("spring.w", [0.0, 3.0], 600)
        );
        assert_eq!(s.signals.index(), "spring.w");
        assert_eq!(s.signals.len(), 600);
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
        assert_eq!(
            s.params().iter().find(|r| r.name == "w").unwrap().value,
            1.0
        );

        // Sweep something else.
        let axis = |key: &str, range: [f64; 2], points| (key.to_string(), range, points);
        s.set_sweep(&[axis("spring.c", [0.1, 1.0], 10)]).unwrap();
        assert_eq!(s.signals.index(), "spring.c");
        assert_eq!(s.signals.times()[9], 1.0);
        assert!(s.set_sweep(&[axis("spring.c", [1.0, 0.1], 10)]).is_err());
        assert!(s.set_sweep(&[axis("spring.q", [0.0, 1.0], 10)]).is_err());
        let saved = s.save_json(1.0, None).unwrap();
        assert!(saved.contains("\"param\": \"spring.c\""));
    }

    #[test]
    fn two_parameter_sweeps_record_every_output_as_a_grid() {
        let mut s = example("Resonance");
        s.set_filters(exprs(&[("half", "spring.amplitude/2")]))
            .unwrap();
        let axis = |key: &str, range: [f64; 2], points| (key.to_string(), range, points);
        let (nx, ny) = (31, 20);
        s.set_sweep(&[
            axis("spring.w", [0.0, 3.0], nx),
            axis("spring.c", [0.1, 2.0], ny),
        ])
        .unwrap();
        assert!(s.sweep().unwrap().is_grid());
        let a = s.available();
        assert!(a.scalars.is_empty());
        // The swept parameters place every result's cells.
        assert_eq!(a.axis_choices("half"), ["spring.w", "spring.c"]);
        assert_eq!(a.axis_of("spring.amplitude", "spring.c"), Ok(Some(1)));
        assert!(a.axis_of("spring.amplitude", "t").is_err());
        assert_eq!(
            a.grids,
            [
                "spring.amplitude",
                "spring.lag",
                "half",
                "spring.w",
                "spring.c"
            ]
        );
        let grid = |name: &str| {
            let f = s.signals.fields().iter().find(|f| f.name == name).unwrap();
            assert_eq!(f.shape, FieldShape::Grid { nx, ny });
            f.frame(0).to_vec()
        };
        let (amp, half, w, c) = (
            grid("spring.amplitude"),
            grid("half"),
            grid("spring.w"),
            grid("spring.c"),
        );
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
        assert_eq!(
            s.params().iter().find(|r| r.name == "c").unwrap().value,
            0.2
        );

        // Too many points, or one parameter twice, are refused.
        assert!(s
            .set_sweep(&[
                axis("spring.w", [0.0, 3.0], 1000),
                axis("spring.c", [0.1, 2.0], 1000)
            ])
            .is_err());
        assert!(s
            .set_sweep(&[
                axis("spring.w", [0.0, 3.0], 9),
                axis("spring.w", [0.1, 2.0], 9)
            ])
            .is_err());

        // Saved as a list, and loaded back as a grid sweep.
        let text = s.save_json(1.0, None).unwrap();
        let again = Session::load("again", &text, &registry()).unwrap();
        assert_eq!(again.sweep(), s.sweep());
        assert_eq!(again.signals.fields()[0].frame(0), &amp[..]);

        // Back to one parameter: signals again.
        s.set_sweep(&[axis("spring.w", [0.0, 3.0], 50)]).unwrap();
        assert_eq!(s.signals.len(), 50);
        assert!(s.available().coordinates.is_empty());
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
        assert_eq!(e.len(), s.signals.len());
        let drift = e.iter().map(|v| (v - e[0]).abs()).fold(0.0, f64::max);
        assert!(drift < 1e-9 * e[0], "energy drifted by {drift} of {}", e[0]);

        // Redefining recomputes the whole history.
        s.set_filters(exprs(&[("ax2", "a.x^2 + 0*t")])).unwrap();
        let ax = values(&s, "a.x");
        let ax2 = values(&s, "ax2");
        assert_eq!(ax2.len(), ax.len());
        assert!(ax.iter().zip(&ax2).all(|(x, y)| (x * x - y).abs() < 1e-15));
        assert!(s.signals.resolve("energy").is_none());

        // Problems leave everything as it was.
        let problems = s
            .set_filters(exprs(&[("b", "nope + 1"), ("a.x", "1")]))
            .unwrap_err();
        assert!(problems.iter().any(|p| p.starts_with("b:")), "{problems:?}");
        assert!(
            problems.iter().any(|p| p.starts_with("a.x:")),
            "{problems:?}"
        );
        assert!(s.signals.resolve("ax2").is_some());
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
        let cut = s.default_cut("res", "spring.amplitude");
        let mut filters = s.filters().to_vec();
        filters.push(cut.clone());
        s.set_filters(filters).unwrap();
        let a = s.available();
        // Four cuts along w at fixed c, and their coordinate w.
        let lines: Vec<&str> = a
            .lines
            .iter()
            .map(String::as_str)
            .filter(|l| l.starts_with("res"))
            .collect();
        assert_eq!(lines, ["res[0]", "res[1]", "res[2]", "res[3]", "res.x"]);
        assert_eq!(a.axis_choices("res[0]"), ["t", "res.x"]);
        let grid = |name: &str| {
            let f = s.signals.fields().iter().find(|f| f.name == name).unwrap();
            f.frame(0).to_vec()
        };
        let (amp, c, w) = (grid("spring.amplitude"), grid("spring.c"), grid("res.x"));
        let at = cut.settings["at"].as_array().unwrap()[1].as_f64().unwrap();
        // Default cuts lie on rows, so they read them exactly.
        let row = (0..120).find(|&j| c[j * 200] == at).unwrap();
        assert_eq!(grid("res[1]"), amp[row * 200..row * 200 + 200].to_vec());
        assert_eq!(w, grid("spring.w")[..200].to_vec());
        let label = s.signals.label(s.signals.resolve("res[1]").unwrap());
        assert!(label.starts_with("spring.c = "), "{label}");
        assert_eq!(
            s.signals.label(s.signals.resolve("res.x").unwrap()),
            "spring.w"
        );
        // Saved with its input.
        let saved = s.save_json(1.0, None).unwrap();
        let again = Session::load("again", &saved, &registry()).unwrap();
        assert!(again.warnings.is_empty(), "{:?}", again.warnings);
        assert_eq!(again.filters(), s.filters());
    }

    #[test]
    fn field_outputs_are_recorded_whole_and_offered_to_views() {
        let mut s = example("rod");
        let a = s.available();
        assert!(a.scalars.is_empty());
        assert_eq!(a.grids, ["rod.temperature", "rod.position"]);
        assert_eq!(a.block_types, ["eq.heat_rod"]);
        for _ in 0..20 {
            s.step().unwrap();
        }
        assert_eq!(s.signals.len(), 21);
        assert_eq!(s.signals.fields()[0].frame(20).len(), 50);
        // The hot spot cools.
        let peak = |k: usize| {
            s.signals.fields()[0]
                .frame(k)
                .iter()
                .cloned()
                .fold(0.0, f64::max)
        };
        assert!(peak(20) < peak(0));

        // Rewind and derived signals keep the frames in step with the times.
        s.rewind_to(0.5);
        let frames = |s: &Session| -> Vec<Vec<f64>> {
            (0..s.signals.len())
                .map(|k| s.signals.fields()[0].frame(k).to_vec())
                .collect()
        };
        let before = frames(&s);
        assert_eq!(before.len(), 11);
        s.set_filters(exprs(&[("one", "1 + 0*t")])).unwrap();
        assert_eq!(frames(&s), before);

        let s = example("Orbiting");
        assert_eq!(s.available().points, ["disc.positions"]);
        assert_eq!(s.signals.fields()[0].frame(0).len(), 400);
    }

    #[test]
    fn bad_files_report_errors() {
        let err = Session::load("x", "{", &registry()).err().unwrap();
        assert!(err.contains("line"), "{err}");
    }
}
