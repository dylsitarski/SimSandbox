//! `plot.mesh` (ADR 0086): a field over a mesh's cells or points, drawn
//! on the mesh as its newest frame has it: in 2D each cell filled, in 3D
//! the outer surface, turned with the mouse. The triangles are built here
//! on the CPU; they are drawn by the egui painter (headless, and wherever
//! the GPU path is not installed) or by wgpu.

use crate::builtin::bindings;
use crate::colour::sequential;
use crate::history::{History, SignalRef};
use crate::view::{
    always, Available, Preset, Slot, SlotKind, StyleProperty, View, ViewInput, ViewRegistry,
    ViewType,
};
use crate::view3d::{Camera, Frame, Normaliser};
use egui::{Color32, Mesh, Pos2, Shape, Stroke};
use std::collections::HashMap;

/// Cells outlined below this many.
const OUTLINE_BELOW: usize = 4000;

const VALUE: Slot = Slot {
    name: "value",
    kind: SlotKind::Rows,
    of: None,
    applies: always,
    many: false,
    required: true,
    description:
        "A quantity over the mesh's cells (each cell filled) or its points (coloured between them)",
};

const MESH: Slot = Slot {
    name: "mesh",
    kind: SlotKind::Mesh,
    of: None,
    applies: always,
    many: false,
    required: true,
    description: "The mesh's cells: each its corners, as a table of cells gives them",
};

const fn coordinate(name: &'static str, required: bool) -> Slot {
    Slot {
        name,
        kind: SlotKind::Rows,
        of: None,
        applies: always,
        many: false,
        required,
        description: "A coordinate of the mesh's points",
    }
}

const SLOTS: &[Slot] = &[
    VALUE,
    MESH,
    coordinate("x", true),
    coordinate("y", true),
    coordinate("z", false),
];

const LOG_COLOUR: StyleProperty = StyleProperty {
    name: "log_colour",
    default: 0.0,
    suggested: [0.0, 1.0],
    unit: "",
    description: "1 colours by the logarithm of the value; 0 linearly",
    applies: always,
};

const EDGES: StyleProperty = StyleProperty {
    name: "edges",
    default: 1.0,
    suggested: [0.0, 1.0],
    unit: "",
    description: "1 outlines the cells (below a few thousand), 0 not",
    applies: always,
};

/// The first mesh with a field over it.
fn mesh_bindings(a: &Available<'_>) -> Option<crate::view::Bindings> {
    let m = a
        .meshes
        .iter()
        .find(|m| !m.over_cells.is_empty() || !m.over_points.is_empty())?;
    let value = m.over_cells.first().or(m.over_points.first())?;
    let mut pairs = vec![("value", value.as_str()), ("mesh", m.cells.as_str())];
    for (slot, c) in ["x", "y", "z"].iter().zip(&m.coordinates) {
        pairs.push((slot, c.as_str()));
    }
    Some(bindings(&pairs))
}

pub fn register(r: &mut ViewRegistry) -> Result<(), String> {
    r.register(ViewType {
        name: "plot.mesh",
        label: "Mesh",
        description:
            "A field over a mesh as it is now: cells filled in 2D, the outer surface in 3D",
        slots: SLOTS,
        style: &[LOG_COLOUR, EDGES],
        applies_to: &[],
        swap: None,
        presets: &[Preset {
            label: "Mesh",
            description: "A field over a mesh",
            bindings: mesh_bindings,
        }],
        create: || Box::new(MeshView::default()),
    })
}

/// Triangles to draw, each three corners with a value at each, and the
/// cells' outlines (pairs of corners), in data coordinates.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Triangles {
    pub dim: usize,
    pub positions: Vec<[f64; 3]>,
    pub values: Vec<f64>,
    /// Each polygon drawn: its first triangle and how many (a fan from
    /// its first corner, so its corners are the first triangle's first
    /// two, then each triangle's third).
    pub polygons: Vec<(usize, usize)>,
    /// Cells drawn.
    pub cells: usize,
}

/// A rows frame's values by id.
fn by_id(h: &History, f: usize, k: usize) -> Option<HashMap<u64, f64>> {
    let (_, ids, v) = h.fields()[f].rows(k)?;
    Some(ids.iter().zip(v).map(|(&i, &x)| (i as u64, x)).collect())
}

/// The triangles of the mesh with cells field `mesh`, coordinates `coords`
/// and values `value`, at sample `k` (ADR 0086). In 2D, each cell's
/// polygon; in 3D, the sides of one cell only (the outer surface), or
/// every polygon of a mesh of 2D cells. A value over the cells colours a
/// cell; over the points, its corners.
pub fn triangles(
    h: &History,
    mesh: usize,
    coords: &[usize],
    value: usize,
    k: usize,
) -> Option<Triangles> {
    let shape = h.mesh(&h.fields()[mesh].name)?.clone();
    let (_, cell_ids, corners) = h.fields()[mesh].rows(k)?;
    let nc = corners.len().checked_div(cell_ids.len().max(1))?;
    let xyz: Vec<HashMap<u64, f64>> = coords
        .iter()
        .map(|&c| by_id(h, c, k))
        .collect::<Option<_>>()?;
    let point = |id: u64| -> Option<[f64; 3]> {
        let mut p = [0.0; 3];
        for (a, m) in xyz.iter().enumerate() {
            p[a] = *m.get(&id)?;
        }
        Some(p)
    };
    let values = by_id(h, value, k)?;
    let field = &h.fields()[value].name;
    let over_cells = h.table(field).is_some() && h.table(field) == h.table(&h.fields()[mesh].name);
    // Which polygons: in 3D a mesh of 3D cells shows its sides that belong
    // to one cell only.
    let solid = shape.dim == 3 && shape.faces.len() > 1;
    let mut count: HashMap<Vec<u64>, usize> = HashMap::new();
    if solid {
        for c in corners.chunks_exact(nc) {
            for f in &shape.faces {
                let mut key: Vec<u64> = f.iter().map(|&p| c[p] as u64).collect();
                key.sort_unstable();
                *count.entry(key).or_default() += 1;
            }
        }
    }
    let mut out = Triangles {
        dim: shape.dim,
        ..Triangles::default()
    };
    for (r, c) in corners.chunks_exact(nc).enumerate() {
        let cell_value = values.get(&(cell_ids[r] as u64)).copied();
        out.cells += 1;
        for f in &shape.faces {
            let ids: Vec<u64> = f.iter().map(|&p| c[p] as u64).collect();
            if solid {
                let mut key = ids.clone();
                key.sort_unstable();
                if count.get(&key) != Some(&1) {
                    continue;
                }
            }
            let Some(pts) = ids.iter().map(|&i| point(i)).collect::<Option<Vec<_>>>() else {
                continue;
            };
            let vals: Vec<f64> = ids
                .iter()
                .map(|i| {
                    if over_cells {
                        cell_value.unwrap_or(f64::NAN)
                    } else {
                        values.get(i).copied().unwrap_or(f64::NAN)
                    }
                })
                .collect();
            // A fan from the first corner.
            out.polygons.push((out.positions.len() / 3, pts.len() - 2));
            for t in 1..pts.len() - 1 {
                for v in [0, t, t + 1] {
                    out.positions.push(pts[v]);
                    out.values.push(vals[v]);
                }
            }
        }
    }
    Some(out)
}

impl Triangles {
    /// The corners of polygon `p`, in order around.
    pub fn corners(&self, p: usize) -> Vec<[f64; 3]> {
        let (first, n) = self.polygons[p];
        let mut c = vec![self.positions[3 * first], self.positions[3 * first + 1]];
        c.extend((0..n).map(|t| self.positions[3 * (first + t) + 2]));
        c
    }
}

/// The smallest and largest finite value (on the logarithm with `log`).
fn range(values: &[f64], log: bool) -> Option<[f64; 2]> {
    let scale = |v: f64| if log { v.ln() } else { v };
    let (lo, hi) = values
        .iter()
        .map(|&v| scale(v))
        .filter(|v| v.is_finite())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| {
            (a.min(v), b.max(v))
        });
    (lo <= hi).then_some([lo, hi])
}

/// Each value's colour on the sequential scale over `[lo, hi]`.
pub fn colours(values: &[f64], log: bool, [lo, hi]: [f64; 2]) -> Vec<Color32> {
    let span = (hi - lo).max(1e-300);
    values
        .iter()
        .map(|&v| {
            let v = if log { v.ln() } else { v };
            if v.is_finite() {
                sequential(((v - lo) / span) as f32)
            } else {
                Color32::from_gray(90)
            }
        })
        .collect()
}

#[derive(Default)]
struct MeshView {
    camera: Camera,
    /// A plane through a 3D mesh, drawn flat instead of its surface.
    cut: Option<Cut>,
}

/// A plane through a 3D mesh at one coordinate (ADR 0086): saved as the
/// view's selection, `{"cut": {"axis": "z", "at": 0.5}}`.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Cut {
    #[serde(with = "axis_name")]
    pub axis: usize,
    pub at: f64,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Selection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cut: Option<Cut>,
}

/// The coordinate slots' names for a cut's axis.
mod axis_name {
    const NAMES: [&str; 3] = ["x", "y", "z"];
    pub fn serialize<S: serde::Serializer>(a: &usize, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(NAMES[(*a).min(2)])
    }
    pub fn deserialize<'d, D: serde::Deserializer<'d>>(d: D) -> Result<usize, D::Error> {
        let n = <String as serde::Deserialize>::deserialize(d)?;
        NAMES
            .iter()
            .position(|m| *m == n)
            .ok_or_else(|| serde::de::Error::custom(format!("a cut runs along x, y or z, not {n}")))
    }
}

impl MeshView {
    fn bound(input: &ViewInput<'_>, slot: &str) -> Option<usize> {
        match input.one(slot) {
            Some(SignalRef::Field(f)) => Some(f),
            _ => None,
        }
    }
}

impl MeshView {
    /// A cut's axis and place, the place over the coordinate's range.
    fn cut_controls(&mut self, ui: &mut egui::Ui, h: &History, coords: &[usize], k: usize) {
        let names = ["x", "y", "z"];
        ui.horizontal(|ui| {
            let now = self.cut.map_or("none", |c| names[c.axis]);
            egui::ComboBox::from_id_salt("plot.mesh.cut")
                .selected_text(format!("cut: {now}"))
                .show_ui(ui, |ui| {
                    if ui.selectable_label(self.cut.is_none(), "none").clicked() {
                        self.cut = None;
                    }
                    for (a, n) in names.iter().enumerate() {
                        let picked = self.cut.is_some_and(|c| c.axis == a);
                        if ui.selectable_label(picked, *n).clicked() && !picked {
                            let r = coordinate_range(h, coords[a], k);
                            self.cut = Some(Cut {
                                axis: a,
                                at: 0.5 * (r[0] + r[1]),
                            });
                        }
                    }
                });
            if let Some(c) = &mut self.cut {
                let r = coordinate_range(h, coords[c.axis], k);
                ui.add(egui::Slider::new(&mut c.at, r[0]..=r[1]));
            }
        });
    }
}

/// The smallest and largest value of a coordinate over the points.
fn coordinate_range(h: &History, f: usize, k: usize) -> [f64; 2] {
    let values = h.fields()[f].rows(k).map(|r| r.2).unwrap_or(&[]);
    let (lo, hi) = values
        .iter()
        .filter(|v| v.is_finite())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &v| {
            (a.min(v), b.max(v))
        });
    if lo <= hi {
        [lo, hi]
    } else {
        [0.0, 1.0]
    }
}

/// A 3D mesh cut by the plane `cut` (ADR 0086): each cell it crosses as
/// the polygon where it does, in the other two coordinates (in order), a
/// value over the cells filling it, one over the points interpolated
/// along its edges. A corner on the plane counts as above it, so a plane
/// along cells' sides draws each side once.
pub fn section(
    h: &History,
    mesh: usize,
    coords: &[usize],
    value: usize,
    k: usize,
    cut: Cut,
) -> Option<Triangles> {
    let shape = h.mesh(&h.fields()[mesh].name)?.clone();
    let (_, cell_ids, corners) = h.fields()[mesh].rows(k)?;
    let nc = corners.len().checked_div(cell_ids.len().max(1))?;
    let xyz: Vec<HashMap<u64, f64>> = coords
        .iter()
        .map(|&c| by_id(h, c, k))
        .collect::<Option<_>>()?;
    let point = |id: u64| -> Option<[f64; 3]> {
        let mut p = [0.0; 3];
        for (a, m) in xyz.iter().enumerate() {
            p[a] = *m.get(&id)?;
        }
        Some(p)
    };
    let values = by_id(h, value, k)?;
    let field = &h.fields()[value].name;
    let over_cells = h.table(field).is_some() && h.table(field) == h.table(&h.fields()[mesh].name);
    // The cells' edges, as pairs of corner places, from their sides.
    let mut edges: Vec<(usize, usize)> = Vec::new();
    for f in &shape.faces {
        for e in 0..f.len() {
            let (a, b) = (f[e], f[(e + 1) % f.len()]);
            let pair = (a.min(b), a.max(b));
            if !edges.contains(&pair) {
                edges.push(pair);
            }
        }
    }
    let (u, v) = match cut.axis {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    let mut out = Triangles {
        dim: 2,
        ..Triangles::default()
    };
    for (r, c) in corners.chunks_exact(nc).enumerate() {
        let ids: Vec<u64> = c.iter().map(|&p| p as u64).collect();
        let Some(pts) = ids.iter().map(|&i| point(i)).collect::<Option<Vec<_>>>() else {
            continue;
        };
        let d: Vec<f64> = pts.iter().map(|p| p[cut.axis] - cut.at).collect();
        let cell_value = values
            .get(&(cell_ids[r] as u64))
            .copied()
            .unwrap_or(f64::NAN);
        let mut hits: Vec<([f64; 2], f64)> = Vec::new();
        for &(a, b) in &edges {
            if (d[a] < 0.0) == (d[b] < 0.0) {
                continue;
            }
            let t = d[a] / (d[a] - d[b]);
            let at = |i: usize| pts[a][i] + t * (pts[b][i] - pts[a][i]);
            let q = [at(u), at(v)];
            let val = if over_cells {
                cell_value
            } else {
                let va = values.get(&ids[a]).copied().unwrap_or(f64::NAN);
                let vb = values.get(&ids[b]).copied().unwrap_or(f64::NAN);
                va + t * (vb - va)
            };
            if !hits.iter().any(|(p, _)| *p == q) {
                hits.push((q, val));
            }
        }
        if hits.len() < 3 {
            continue;
        }
        // Around their centre: a cell is convex, so is its section.
        let n = hits.len() as f64;
        let m = [
            hits.iter().map(|h| h.0[0]).sum::<f64>() / n,
            hits.iter().map(|h| h.0[1]).sum::<f64>() / n,
        ];
        hits.sort_by(|a, b| {
            let ang = |p: &[f64; 2]| (p[1] - m[1]).atan2(p[0] - m[0]);
            ang(&a.0).total_cmp(&ang(&b.0))
        });
        out.cells += 1;
        out.polygons.push((out.positions.len() / 3, hits.len() - 2));
        for t in 1..hits.len() - 1 {
            for i in [0, t, t + 1] {
                out.positions.push([hits[i].0[0], hits[i].0[1], 0.0]);
                out.values.push(hits[i].1);
            }
        }
    }
    Some(out)
}

impl View for MeshView {
    fn show(&mut self, ui: &mut egui::Ui, input: &ViewInput<'_>) {
        let h = input.history;
        let (Some(value), Some(mesh), Some(x), Some(y)) = (
            Self::bound(input, "value"),
            Self::bound(input, "mesh"),
            Self::bound(input, "x"),
            Self::bound(input, "y"),
        ) else {
            return;
        };
        let Some(k) = h.len().checked_sub(1) else {
            return;
        };
        let mut coords = vec![x, y];
        coords.extend(Self::bound(input, "z"));
        // A 3D mesh may be cut by a plane at one coordinate.
        if coords.len() == 3 && h.mesh(&h.fields()[mesh].name).is_some_and(|m| m.dim == 3) {
            self.cut_controls(ui, h, &coords, k);
        } else {
            self.cut = None;
        }
        let built = match self.cut {
            Some(c) => section(h, mesh, &coords, value, k, c),
            None => triangles(h, mesh, &coords, value, k),
        };
        let Some(tri) = built else {
            ui.weak("nothing to draw yet");
            return;
        };
        let log = input.style("log_colour") != 0.0;
        let Some(r) = range(&tri.values, log) else {
            ui.weak(format!(
                "{}: no values",
                h.labelled(SignalRef::Field(value))
            ));
            return;
        };
        let shown = |v: f64| if log { v.exp() } else { v };
        ui.weak(format!(
            "{}: {} (dark) to {} (bright){}, {} cells",
            h.labelled(SignalRef::Field(value)),
            crate::fields::fmt(shown(r[0])),
            crate::fields::fmt(shown(r[1])),
            if log { ", log scale" } else { "" },
            tri.cells
        ));
        let colour = colours(&tri.values, log, r);
        let edges = input.style("edges") != 0.0 && tri.cells < OUTLINE_BELOW;
        let outline = Stroke::new(0.5, ui.visuals().extreme_bg_color.gamma_multiply(0.7));
        if self.cut.is_none() && tri.dim == 3 && coords.len() == 3 {
            draw_3d(
                ui,
                &mut self.camera,
                &tri,
                &colour,
                edges.then_some(outline),
            );
        } else {
            draw_2d(ui, &tri, &colour, edges.then_some(outline));
        }
    }

    fn camera(&self) -> Option<[f64; 3]> {
        let c = self.camera;
        Some([c.yaw.into(), c.pitch.into(), c.zoom.into()])
    }

    fn set_camera(&mut self, [yaw, pitch, zoom]: [f64; 3]) {
        self.camera = Camera {
            yaw: yaw as f32,
            pitch: pitch as f32,
            zoom: zoom as f32,
        };
    }

    fn selection(&self) -> Option<serde_json::Value> {
        self.cut
            .map(|cut| serde_json::to_value(Selection { cut: Some(cut) }).expect("plain data"))
    }

    fn set_selection(&mut self, selection: &serde_json::Value) -> Result<(), String> {
        let s: Selection = serde_json::from_value(selection.clone()).map_err(|e| e.to_string())?;
        self.cut = s.cut;
        Ok(())
    }
}

/// Filled cells in the plane, the axes' proportions kept.
fn draw_2d(ui: &mut egui::Ui, tri: &Triangles, colour: &[Color32], outline: Option<Stroke>) {
    let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for p in &tri.positions {
        for a in 0..2 {
            lo[a] = lo[a].min(p[a]);
            hi[a] = hi[a].max(p[a]);
        }
    }
    if lo[0] > hi[0] {
        return;
    }
    let span = [(hi[0] - lo[0]).max(1e-12), (hi[1] - lo[1]).max(1e-12)];
    let inner = rect.shrink(8.0);
    let scale = (inner.width() as f64 / span[0]).min(inner.height() as f64 / span[1]);
    let centre = inner.center();
    let at = |p: &[f64; 3]| {
        Pos2::new(
            centre.x + ((p[0] - 0.5 * (lo[0] + hi[0])) * scale) as f32,
            centre.y - ((p[1] - 0.5 * (lo[1] + hi[1])) * scale) as f32,
        )
    };
    let painter = ui.painter_at(rect);
    #[cfg(feature = "gpu")]
    if crate::gpu::installed() {
        let corners: Vec<(Pos2, f32, Color32)> = tri
            .positions
            .iter()
            .zip(colour)
            .map(|(p, &c)| (at(p), 0.5, c))
            .collect();
        let mut lines = Vec::new();
        if let Some(stroke) = outline {
            for p in 0..tri.polygons.len() {
                let c = tri.corners(p);
                for e in 0..c.len() {
                    for q in [&c[e], &c[(e + 1) % c.len()]] {
                        lines.push((at(q), 0.4, stroke.color));
                    }
                }
            }
        }
        on_gpu(
            ui,
            &painter,
            rect,
            &corners,
            &lines,
            ui.visuals().panel_fill,
        );
        return;
    }
    let mut mesh = Mesh::default();
    for (p, &c) in tri.positions.iter().zip(colour) {
        mesh.colored_vertex(at(p), c);
    }
    for t in 0..tri.positions.len() as u32 / 3 {
        mesh.add_triangle(3 * t, 3 * t + 1, 3 * t + 2);
    }
    painter.add(Shape::mesh(mesh));
    if let Some(stroke) = outline {
        for p in 0..tri.polygons.len() {
            let c: Vec<Pos2> = tri.corners(p).iter().map(at).collect();
            painter.add(Shape::closed_line(c, stroke));
        }
    }
}

/// The outer surface, shaded by how it faces the eye and drawn furthest
/// first.
fn draw_3d(
    ui: &mut egui::Ui,
    camera: &mut Camera,
    tri: &Triangles,
    colour: &[Color32],
    outline: Option<Stroke>,
) {
    let Some(norm) = Normaliser::fit(tri.positions.iter().copied()) else {
        return;
    };
    let (lo, hi) = (norm.apply(norm.lo), norm.apply(norm.hi));
    let frame = Frame::begin(ui, camera, lo, hi, false);
    let projected: Vec<[f32; 3]> = tri
        .positions
        .iter()
        .map(|&p| frame.camera.project(norm.apply(p)))
        .collect();
    let light = |t: usize| {
        let [a, b, c] = [0, 1, 2].map(|v| projected[3 * t + v]);
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [
            u[1] * w[2] - u[2] * w[1],
            u[2] * w[0] - u[0] * w[2],
            u[0] * w[1] - u[1] * w[0],
        ];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12);
        // Lit from above and in front of the eye (right, up, into the
        // picture), either side of a face alike.
        const LIGHT: [f32; 3] = [0.3, 0.7, -0.65];
        let d = (n[0] * LIGHT[0] + n[1] * LIGHT[1] + n[2] * LIGHT[2]) / len;
        0.4 + 0.6 * d.abs()
    };
    let screen: Vec<Pos2> = tri
        .positions
        .iter()
        .map(|&p| frame.to_screen(norm.apply(p)))
        .collect();
    #[cfg(feature = "gpu")]
    if crate::gpu::installed() {
        // Depth from the eye: the GPU keeps the nearest.
        let z = |d: f32| (0.5 + d / 8.0).clamp(0.0, 1.0);
        let corners: Vec<(Pos2, f32, Color32)> = (0..screen.len())
            .map(|v| {
                (
                    screen[v],
                    z(projected[v][2]),
                    shade(colour[v], light(v / 3)),
                )
            })
            .collect();
        let mut lines = Vec::new();
        if let Some(stroke) = outline {
            for &(first, n) in &tri.polygons {
                let mut c: Vec<usize> = vec![3 * first, 3 * first + 1];
                c.extend((0..n).map(|t| 3 * (first + t) + 2));
                for e in 0..c.len() {
                    for v in [c[e], c[(e + 1) % c.len()]] {
                        lines.push((screen[v], z(projected[v][2]) - 2e-3, stroke.color));
                    }
                }
            }
        }
        on_gpu(
            ui,
            &frame.painter,
            frame.rect,
            &corners,
            &lines,
            ui.visuals().extreme_bg_color,
        );
        frame.hint(ui.visuals());
        return;
    }
    // Polygons furthest first, each with its outline, so nearer ones
    // cover them (the painter's algorithm).
    let depth = |&(first, n): &(usize, usize)| {
        (3 * first..3 * (first + n))
            .map(|v| projected[v][2])
            .sum::<f32>()
            / (3 * n) as f32
    };
    let mut order: Vec<usize> = (0..tri.polygons.len()).collect();
    order.sort_by(|&a, &b| depth(&tri.polygons[b]).total_cmp(&depth(&tri.polygons[a])));
    let mut mesh = Mesh::default();
    for p in order {
        let (first, n) = tri.polygons[p];
        for t in first..first + n {
            let base = mesh.vertices.len() as u32;
            let l = light(t);
            for v in 3 * t..3 * t + 3 {
                mesh.colored_vertex(screen[v], shade(colour[v], l));
            }
            mesh.add_triangle(base, base + 1, base + 2);
        }
        if let Some(stroke) = outline {
            frame.painter.add(Shape::mesh(std::mem::take(&mut mesh)));
            let mut c = vec![screen[3 * first], screen[3 * first + 1]];
            c.extend((0..n).map(|t| screen[3 * (first + t) + 2]));
            frame.painter.add(Shape::closed_line(c, stroke));
        }
    }
    frame.painter.add(Shape::mesh(mesh));
    frame.hint(ui.visuals());
}

/// Draw triangles (corners in threes) and line segments (in twos), placed
/// on screen with their depth, by wgpu over `rect` (ADR 0086).
#[cfg(feature = "gpu")]
fn on_gpu(
    ui: &egui::Ui,
    painter: &egui::Painter,
    rect: egui::Rect,
    triangles: &[(Pos2, f32, Color32)],
    lines: &[(Pos2, f32, Color32)],
    background: Color32,
) {
    let c = rect.center();
    let (hw, hh) = (rect.width() / 2.0, rect.height() / 2.0);
    let vertex = |&(p, z, colour): &(Pos2, f32, Color32)| {
        crate::gpu::Vertex::new([(p.x - c.x) / hw, (c.y - p.y) / hh, z], colour)
    };
    let ppp = ui.ctx().pixels_per_point();
    crate::gpu::paint(
        painter,
        rect,
        crate::gpu::MeshCallback {
            id: ui.id().value(),
            size: [
                (rect.width() * ppp).round() as u32,
                (rect.height() * ppp).round() as u32,
            ],
            background,
            triangles: triangles.iter().map(vertex).collect(),
            lines: lines.iter().map(vertex).collect(),
        },
    );
}

fn shade(c: Color32, light: f32) -> Color32 {
    let f = |v: u8| (v as f32 * light).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(f(c.r()), f(c.g()), f(c.b()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{FieldShape, MeshShape};

    /// A history with a mesh's cells `c`, coordinates `x`, `y`, `z` over
    /// points and a value `u` over cells, one sample.
    fn history(shape: MeshShape, cells: &[Vec<f64>], pts: &[[f64; 3]], u: &[f64]) -> History {
        let nc = cells[0].len();
        let rows = |dim| FieldShape::Rows { dim };
        let mut h = History::with_fields(
            "t",
            vec![],
            vec![
                ("m.c".into(), rows(nc)),
                ("m.x".into(), rows(1)),
                ("m.y".into(), rows(1)),
                ("m.z".into(), rows(1)),
                ("m.u".into(), rows(1)),
            ],
        );
        h.set_axes("m.c", vec!["c".into()]);
        h.set_axes("m.u", vec!["c".into()]);
        for n in ["m.x", "m.y", "m.z"] {
            h.set_axes(n, vec!["pts".into()]);
        }
        h.set_mesh("m.c", shape);
        let frame = |values: Vec<f64>, n: usize| {
            let mut f = vec![0.0, n as f64];
            f.extend((0..n).map(|i| i as f64));
            f.extend(values);
            f
        };
        let c = frame(cells.concat(), cells.len());
        let coords: Vec<Vec<f64>> = (0..3)
            .map(|a| frame(pts.iter().map(|p| p[a]).collect(), pts.len()))
            .collect();
        let uf = frame(u.to_vec(), u.len());
        h.push_sample(0.0, &[], &[&c, &coords[0], &coords[1], &coords[2], &uf]);
        h
    }

    #[test]
    fn cells_in_the_plane_are_filled_by_their_values() {
        // A unit square as two triangles.
        let shape = MeshShape {
            dim: 2,
            faces: vec![vec![0, 1, 2]],
            coordinates: vec!["m.x".into(), "m.y".into()],
        };
        let pts = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        let h = history(
            shape,
            &[vec![0.0, 1.0, 2.0], vec![0.0, 2.0, 3.0]],
            &pts,
            &[5.0, 7.0],
        );
        let t = triangles(&h, 0, &[1, 2], 4, 0).unwrap();
        assert_eq!((t.cells, t.polygons.len(), t.positions.len()), (2, 2, 6));
        assert_eq!(t.values, [5.0, 5.0, 5.0, 7.0, 7.0, 7.0]);
        assert_eq!(
            t.corners(1),
            [[0.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]]
        );
    }

    #[test]
    fn a_solid_shows_its_outer_sides_only() {
        // Two unit cubes side by side (hexahedra, Gmsh's corner order):
        // twelve sides, two of them the shared one: ten outer.
        let faces = vec![
            vec![0, 3, 2, 1],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
            vec![4, 5, 6, 7],
        ];
        let shape = MeshShape {
            dim: 3,
            faces,
            coordinates: vec!["m.x".into(), "m.y".into(), "m.z".into()],
        };
        // Points: x in 0, 1, 2 (fastest), y in 0, 1, z in 0, 1.
        let pts: Vec<[f64; 3]> = (0..12)
            .map(|i| [(i % 3) as f64, ((i / 3) % 2) as f64, (i / 6) as f64])
            .collect();
        let hex = |o: usize| -> Vec<f64> {
            [0, 1, 4, 3, 6, 7, 10, 9]
                .iter()
                .map(|&k| (o + k) as f64)
                .collect()
        };
        let h = history(shape, &[hex(0), hex(1)], &pts, &[1.0, 2.0]);
        let t = triangles(&h, 0, &[1, 2, 3], 4, 0).unwrap();
        assert_eq!(t.polygons.len(), 10);
        assert_eq!(t.positions.len(), 10 * 2 * 3);
    }

    /// The area of a section's polygons (shoelace).
    fn area(t: &Triangles) -> f64 {
        (0..t.polygons.len())
            .map(|p| {
                let c = t.corners(p);
                let n = c.len();
                0.5 * (0..n)
                    .map(|k| c[k][0] * c[(k + 1) % n][1] - c[(k + 1) % n][0] * c[k][1])
                    .sum::<f64>()
                    .abs()
            })
            .sum()
    }

    fn hexa_faces() -> Vec<Vec<usize>> {
        vec![
            vec![0, 3, 2, 1],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
            vec![4, 5, 6, 7],
        ]
    }

    #[test]
    fn a_plane_cuts_each_cell_once() {
        let shape = MeshShape {
            dim: 3,
            faces: hexa_faces(),
            coordinates: vec!["m.x".into(), "m.y".into(), "m.z".into()],
        };
        let pts: Vec<[f64; 3]> = (0..12)
            .map(|i| [(i % 3) as f64, ((i / 3) % 2) as f64, (i / 6) as f64])
            .collect();
        let hex = |o: usize| -> Vec<f64> {
            [0, 1, 4, 3, 6, 7, 10, 9]
                .iter()
                .map(|&k| (o + k) as f64)
                .collect()
        };
        let h = history(shape, &[hex(0), hex(1)], &pts, &[1.0, 2.0]);
        let cut = |axis, at| section(&h, 0, &[1, 2, 3], 4, 0, Cut { axis, at }).unwrap();
        // Across both cubes: two squares, each its cube's value.
        let z = cut(2, 0.3);
        assert_eq!((z.polygons.len(), area(&z)), (2, 2.0));
        assert!(z.values[..6].iter().all(|&v| v == 1.0) && z.values[6..].iter().all(|&v| v == 2.0));
        // Through one cube, and along the side they share: once.
        assert_eq!(area(&cut(0, 0.5)), 1.0);
        let shared = cut(0, 1.0);
        assert_eq!((shared.polygons.len(), area(&shared)), (1, 1.0));
        // Outside the mesh: nothing.
        assert!(cut(1, 3.0).polygons.is_empty());
    }

    #[test]
    fn a_plane_through_tetrahedra_fills_the_cube_section() {
        // The unit cube as six tetrahedra about its diagonal (Kuhn), and a
        // value over the points: their height.
        let shape = MeshShape {
            dim: 3,
            faces: vec![vec![0, 2, 1], vec![0, 1, 3], vec![0, 3, 2], vec![1, 2, 3]],
            coordinates: vec!["m.x".into(), "m.y".into(), "m.z".into()],
        };
        let pts: Vec<[f64; 3]> = (0..8)
            .map(|i| [(i & 1) as f64, ((i >> 1) & 1) as f64, ((i >> 2) & 1) as f64])
            .collect();
        let mut cells = Vec::new();
        for p in [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            let mut at = 0usize;
            let mut c = vec![0.0];
            for a in p {
                at |= 1 << a;
                c.push(at as f64);
            }
            cells.push(c);
        }
        let heights: Vec<f64> = pts.iter().map(|p| p[2]).collect();
        // The value over the points: the history's `u` is over the cells'
        // table unless named otherwise.
        let mut h = history(shape, &cells, &pts, &heights);
        h.set_axes("m.u", vec!["pts".into()]);
        let s = section(&h, 0, &[1, 2, 3], 4, 0, Cut { axis: 2, at: 0.3 }).unwrap();
        assert!((area(&s) - 1.0).abs() < 1e-12, "{}", area(&s));
        assert_eq!(s.polygons.len(), 6);
        assert!(s.values.iter().all(|&v| (v - 0.3).abs() < 1e-12));
    }
}
