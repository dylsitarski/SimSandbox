//! The catalogue: what a step of the model yields (its outputs and the
//! filters computed from them), how a sample is taken from the simulator,
//! and what views and interfaces can bind.

use super::{param_keys, param_values, Session};
use crate::filters::Filters;
use simsandbox_core::block::{BlockSpec, MeshOf, PortSpec, Shape};
use simsandbox_core::graph::{BlockId, Model};
use simsandbox_core::sim::Simulator;
use simsandbox_core::units::Unit;
use simsandbox_file::{FilterEntry, Loaded};
use simsandbox_views::cut::Geometry;
use simsandbox_views::{Available, Coordinate, FieldShape, History, MeshInfo, MeshShape};
use std::collections::BTreeMap;

/// Where a recorded signal comes from: an element of an output port.
#[derive(Clone)]
pub(super) struct Source {
    block: BlockId,
    port: String,
    /// Its place among the block's fixed-length output values
    /// ([`Simulator::outputs_of`]), found once.
    at: usize,
}

/// A whole output port recorded as a field (a grid or a point set).
#[derive(Clone)]
pub(super) struct FieldSource {
    block: BlockId,
    port: String,
    /// Values per frame; `None` for a population, whose frames vary.
    len: Option<usize>,
    /// Where its values are among the block's fixed-length output values
    /// ([`Simulator::outputs_of`]); `None` for one of varying length.
    at: Option<usize>,
}

/// The session's catalogue: every output of the model (ADR 0063: any can
/// be recorded), the filters computed from them (ADR 0030–0032), and the
/// layout of a whole sample they make together, which the history and
/// probes take from and views bind by name. Set at loading; the layout
/// changes only with the filters.
#[derive(Default)]
pub(super) struct Catalogue {
    pub(super) sources: Vec<Source>,
    /// Names of the scalar output signals, in `sources` order.
    pub(super) output_names: Vec<String>,
    /// Grid and point-set outputs and their shapes.
    pub(super) base_fields: Vec<(String, FieldShape)>,
    /// Coordinates among the grid outputs.
    base_coordinates: Vec<Coordinate>,
    /// Grid and point-set outputs, recorded whole (ADR 0016).
    field_sources: Vec<FieldSource>,
    /// Meshes views can draw, and how (ADR 0086).
    meshes: Vec<MeshInfo>,
    mesh_shapes: Vec<(String, MeshShape)>,
    /// The model's block types, which views can bind.
    pub(super) block_types: Vec<String>,
    /// Its `input.pointer` blocks, which a scene's layers grab with
    /// (ADR 0093 point 6).
    pointers: Vec<String>,
    /// Every scalar and per-sample field a step yields, in sample order:
    /// what probes take from and views can bind.
    pub(super) layout_scalars: Vec<String>,
    pub(super) layout_fields: Vec<(String, FieldShape)>,
    /// Which fields place which others' cells, among everything a step
    /// yields.
    pub(super) all_coordinates: Vec<Coordinate>,
    /// Names of the grids (and fields of more axes), point sets, grids
    /// with one row (which views can draw over time) and fields over a
    /// table's rows (ADR 0086) a step yields.
    grids: Vec<String>,
    points: Vec<String>,
    lines: Vec<String>,
    rows: Vec<String>,
    /// Filters as written, for saving and editing (ADR 0030, ADR 0032).
    pub(super) filter_entries: Vec<FilterEntry>,
    /// Filters compiled: expressions recorded after the output signals,
    /// cut fields after the recorded fields.
    pub(super) filters: Filters,
}

/// The field a port's shape records, if it is one.
fn field_shape(shape: Shape) -> Option<FieldShape> {
    match shape {
        Shape::Grid { nx, ny } => Some(FieldShape::Grid { nx, ny }),
        Shape::Points { count, dim } => Some(FieldShape::Points { count, dim }),
        Shape::Population { dim } => Some(FieldShape::Population { dim }),
        Shape::Tensor { rank, dims } => Some(FieldShape::Tensor { rank, dims }),
        // Recorded with their ids (ADR 0079 stage 1c).
        Shape::Rows { dim } => Some(FieldShape::Rows { dim }),
        Shape::Scalar | Shape::Vector(_) => None,
    }
}

impl Catalogue {
    /// Every output of a loaded model, without filters yet.
    pub(super) fn of(loaded: &Loaded) -> Result<Self, String> {
        let mut c = Self::default();
        for entry in &loaded.flat.blocks {
            let id = loaded.ids[&entry.name];
            let block = loaded.model.block(id).map_err(|e| e.to_string())?;
            if !c.block_types.contains(&entry.type_name) {
                c.block_types.push(entry.type_name.clone());
            }
            if entry.type_name == simsandbox_eq::input::POINTER {
                c.pointers.push(entry.name.clone());
            }
            let spec = block.spec();
            for port in spec.outputs() {
                c.add_output(&entry.name, id, spec, port);
            }
        }
        c.add_aliases(&loaded.aliases);
        c.keep_coordinates_that_fit();
        Ok(c)
    }

    /// One output port: every output can be recorded; what is, wiring
    /// decides (ADR 0063).
    fn add_output(&mut self, block: &str, id: BlockId, spec: &BlockSpec, port: &PortSpec) {
        let key = format!("{block}.{}", port.name);
        // Its place among the block's fixed-length outputs, as the
        // simulator lays them out (ports in order).
        let offset: usize = spec
            .outputs()
            .take_while(|p| p.name != port.name)
            .map(|p| p.shape.len())
            .sum();
        let Some(shape) = field_shape(port.shape) else {
            for index in 0..port.shape.len() {
                self.output_names.push(match port.shape {
                    Shape::Vector(_) => format!("{key}[{index}]"),
                    _ => key.clone(),
                });
                self.sources.push(Source {
                    block: id,
                    port: port.name.clone(),
                    at: offset + index,
                });
            }
            return;
        };
        for c in &port.coordinate {
            self.base_coordinates.push(Coordinate {
                name: key.clone(),
                of: format!("{block}.{}", c.port),
                axis: c.axis,
            });
        }
        if let Some(m) = &port.mesh {
            self.add_mesh(block, &key, spec, port, m);
        }
        // Rows of one table place each other, by id (ADR 0086): any of
        // its quantities can be the other's axis.
        if let (Shape::Rows { dim: 1 }, Some(table)) = (port.shape, port.axis_names.first()) {
            for other in spec.outputs().filter(|o| {
                o.name != port.name
                    && matches!(o.shape, Shape::Rows { dim: 1 })
                    && o.axis_names.first() == Some(table)
            }) {
                self.base_coordinates.push(Coordinate {
                    name: key.clone(),
                    of: format!("{block}.{}", other.name),
                    axis: 0,
                });
            }
        }
        self.base_fields.push((key, shape));
        self.field_sources.push(FieldSource {
            block: id,
            port: port.name.clone(),
            len: (!port.shape.is_variable()).then(|| port.shape.len()),
            at: (!port.shape.is_variable()).then_some(offset),
        });
    }

    /// A mesh's cells, `key`, with the fields over its cells and points
    /// (ADR 0086).
    fn add_mesh(&mut self, block: &str, key: &str, spec: &BlockSpec, port: &PortSpec, m: &MeshOf) {
        let table = |o: &PortSpec| {
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
                .map(|o| format!("{block}.{}", o.name))
                .collect()
        };
        let qualified: Vec<String> = m
            .coordinates
            .iter()
            .map(|c| format!("{block}.{c}"))
            .collect();
        self.meshes.push(MeshInfo {
            cells: key.to_string(),
            coordinates: qualified.clone(),
            over_cells: over(port.axis_names.first().cloned()),
            over_points: over(points)
                .into_iter()
                .filter(|f| !qualified.contains(f))
                .collect(),
        });
        self.mesh_shapes.push((
            key.to_string(),
            MeshShape {
                dim: m.dim,
                faces: m.faces.clone(),
                coordinates: qualified,
            },
        ));
    }

    /// Declared outputs of used models (ADR 0066 step 4b) are recorded
    /// under their names too, from the same outputs.
    fn add_aliases(&mut self, aliases: &BTreeMap<String, String>) {
        for (alias, target) in aliases {
            if let Some(k) = self.output_names.iter().position(|n| n == target) {
                self.output_names.push(alias.clone());
                self.sources.push(self.sources[k].clone());
            } else if let Some(k) = self.base_fields.iter().position(|(n, _)| n == target) {
                self.base_fields
                    .push((alias.clone(), self.base_fields[k].1));
                self.field_sources.push(self.field_sources[k].clone());
            }
        }
    }

    /// A coordinate needs the field it places, recorded, of the same
    /// shape, or a line along its axis (ADR 0080).
    fn keep_coordinates_that_fit(&mut self) {
        let fields = &self.base_fields;
        self.base_coordinates.retain(|c| {
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
    }

    /// Compile filters on these outputs and `params` (every parameter's
    /// key); problems are `name: reason`. Filters on probes are compiled
    /// with their probes (ADR 0063).
    pub(super) fn compile(
        &self,
        entries: &[FilterEntry],
        params: &[String],
    ) -> (Filters, Vec<String>) {
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
        Filters::compile(entries, &self.output_names, params, &self.base_fields)
    }

    /// Take compiled filters and the entries they were compiled from.
    pub(super) fn set_filters(&mut self, filters: Filters, entries: &[FilterEntry]) {
        self.filter_entries = kept_entries(&filters, entries);
        self.filters = filters;
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

    /// Lay out a sample with the filters as they are: its scalars, fields
    /// and coordinates, and what views can draw. Returns the cuts' labels.
    pub(super) fn lay_out(&mut self) -> Vec<(String, String)> {
        let scalars = self.scalar_names();
        let mut fields = self.base_fields.clone();
        let mut coordinates = self.base_coordinates.clone();
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
        labels
    }

    /// Tell a new history `h` what it records: coordinates among its
    /// fields, the cuts' `labels`, units, axes' names and meshes.
    pub(super) fn describe(&self, h: &mut History, labels: Vec<(String, String)>, model: &Model) {
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
    }

    /// The output signals' values now (NaN where missing).
    pub(super) fn output_values(&self, sim: &Simulator) -> Vec<f64> {
        self.sources
            .iter()
            .map(|src| {
                sim.outputs_of(src.block)
                    .and_then(|v| v.get(src.at))
                    .copied()
                    .unwrap_or(f64::NAN)
            })
            .collect()
    }

    /// The current value of every field output (empty if missing, which
    /// the history records as NaN).
    pub(super) fn frames<'a>(&self, sim: &'a Simulator) -> Vec<&'a [f64]> {
        self.field_sources
            .iter()
            .map(|f| {
                let fixed = |at: usize| {
                    let n = f.len.unwrap_or(0);
                    sim.outputs_of(f.block).and_then(|v| v.get(at..at + n))
                };
                match f.at {
                    Some(at) => fixed(at),
                    None => sim.output(f.block, &f.port),
                }
                .filter(|v| f.len.is_none_or(|n| v.len() == n))
                .unwrap_or(&[])
            })
            .collect()
    }

    /// Append the expression filters, computed from the output signals
    /// first in `values`, earlier filters, `t` and every parameter's
    /// value (asked for only when there are filters).
    pub(super) fn append_derived(
        &self,
        t: f64,
        values: &mut Vec<f64>,
        params: impl FnOnce() -> Vec<f64>,
    ) {
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
        vars.extend(params());
        for (k, f) in exprs.iter().enumerate() {
            let v = f.expr.eval(&vars);
            vars[1 + outputs + k] = v;
            values.push(v);
        }
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
    pub(super) fn sample_cuts(&self, before: &[&[f64]]) -> Vec<Vec<f64>> {
        let fields = &self.base_fields;
        self.cut_frames(|name| {
            fields
                .iter()
                .position(|(f, _)| f == name)
                .and_then(|i| before.get(i).copied())
        })
    }

    /// The 2D grids a cut filter can take, recorded or not (ADR 0063).
    fn cut_inputs(&self) -> Vec<String> {
        self.layout_fields
            .iter()
            .filter(|(_, s)| matches!(s, FieldShape::Grid { nx, ny } if *nx >= 2 && *ny >= 2))
            .filter(|(n, _)| !self.all_coordinates.iter().any(|c| &c.name == n))
            .map(|(n, _)| n.clone())
            .collect()
    }
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

impl Session {
    /// Every scalar a step yields, which views can bind (ADR 0063).
    pub fn bindable_scalars(&self) -> &[String] {
        &self.catalogue.layout_scalars
    }

    /// The 2D grids a cut filter can take, recorded or not (ADR 0063).
    pub fn cut_inputs(&self) -> Vec<String> {
        self.catalogue.cut_inputs()
    }

    /// Which fields place which others' cells, among all a step yields.
    pub fn all_coordinates(&self) -> &[Coordinate] {
        &self.catalogue.all_coordinates
    }

    /// The meshes views can draw (ADR 0086).
    pub fn meshes(&self) -> &[MeshInfo] {
        &self.catalogue.meshes
    }

    /// Names of the grids, point sets, one-row grids, the model's block
    /// types and fields over rows: what views can bind besides scalars.
    pub fn bindable(&self) -> [&[String]; 5] {
        let c = &self.catalogue;
        [&c.grids, &c.points, &c.lines, &c.block_types, &c.rows]
    }

    /// The model's `input.pointer` blocks, which a scene's layers grab
    /// with (ADR 0093 point 6).
    pub fn pointers(&self) -> &[String] {
        &self.catalogue.pointers
    }

    /// What views can bind: everything a step yields (ADR 0063), whether
    /// or not the history `h` records it now; and the model's block types.
    pub fn available<'a>(&'a self, h: &'a History) -> Available<'a> {
        let c = &self.catalogue;
        Available {
            scalars: &c.layout_scalars,
            grids: &c.grids,
            points: &c.points,
            block_types: &c.block_types,
            lines: &c.lines,
            rows: &c.rows,
            meshes: &c.meshes,
            coordinates: &c.all_coordinates,
            derived: self.probes.derived_names(),
            // Units and tables of every output, recorded or not.
            meta: Some(h),
            pointers: Some(&c.pointers),
        }
    }

    /// The filters as written (ADR 0032).
    pub fn filters(&self) -> &[FilterEntry] {
        &self.catalogue.filter_entries
    }

    /// Where a filter's node sits on the data layer (ADR 0050): saved with
    /// the filters, without evaluating them again.
    pub fn place_filter(&mut self, name: &str, at: Option<[f64; 2]>) -> Result<(), String> {
        let f = self
            .catalogue
            .filter_entries
            .iter_mut()
            .find(|f| f.name == name)
            .ok_or_else(|| format!("no filter '{name}'"))?;
        f.canvas = at.map(|p| p.map(|v| (v * 10.0).round() / 10.0));
        Ok(())
    }

    /// Replace the filters. All must be usable, or nothing changes and the
    /// problems are returned. The recorded history is kept and the filters
    /// are recomputed for it, using current parameter values.
    pub fn set_filters(&mut self, entries: Vec<FilterEntry>) -> Result<(), Vec<String>> {
        let mut problems = Vec::new();
        let mut names = Vec::new();
        for e in &entries {
            if names.contains(&e.name) || self.catalogue.output_names.contains(&e.name) {
                problems.push(format!("{}: the name is already used", e.name));
            }
            names.push(e.name.clone());
        }
        let (filters, more) = self.compile_filters(&entries);
        problems.extend(more);
        if !problems.is_empty() {
            return Err(problems);
        }
        self.catalogue.set_filters(filters, &entries);
        // Replay the recorded outputs and fields through the new filters;
        // probes start afresh from now.
        let old = self.recording.take();
        self.build_history();
        self.replay_history(&old, true, &BTreeMap::new());
        self.reset_probes();
        Ok(())
    }

    /// Compile filters for this model; problems are `name: reason`.
    pub(super) fn compile_filters(&self, entries: &[FilterEntry]) -> (Filters, Vec<String>) {
        let params = param_keys(&self.flat, &self.ids, self.sim.model());
        self.catalogue.compile(entries, &params)
    }

    /// One sample: the output signals, then the expression filters computed
    /// from them, earlier filters, the current parameter values and `t`.
    pub(super) fn sample(&self, t: f64) -> Vec<f64> {
        let mut values = self.catalogue.output_values(&self.sim);
        self.catalogue
            .append_derived(t, &mut values, || self.param_values());
        values
    }

    /// Every parameter's value, in [`Session::params`] order.
    pub(super) fn param_values(&self) -> Vec<f64> {
        param_values(&self.sim, &self.flat, &self.ids)
    }
}
