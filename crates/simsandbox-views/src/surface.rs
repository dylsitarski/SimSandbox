//! Views of a grid field as a shape (ADR 0022): `plot.surface` (heights
//! over the two axes, a rotatable 3D surface projected on the CPU like
//! `plot.3d`) and `plot.contour` (lines of equal value, by marching
//! squares). Both lay the field over its axes as the heat map does (ADR
//! 0024), so they show 2D grids over their coordinates and lines of cells
//! over time alike.

use crate::builtin::scroll_to_zoom;
use crate::colour::sequential;
use crate::fields::{fmt, grid_bindings, sheet, FIELD, HEAT_TRAIL, MAX_COLUMNS, X_AXIS, Y_AXIS};
use crate::view::{always, Preset, StyleProperty, Swap, View, ViewInput, ViewRegistry, ViewType};
use crate::view3d::{Camera, Frame};
use egui::{Color32, Mesh, Pos2, Shape, Stroke};
use egui_plot::{Line, Plot};
use std::collections::{HashMap, VecDeque};

/// At most this many quads are drawn; larger grids are strided.
const MAX_QUADS: usize = 40_000;
/// Below this many quads each is outlined, so the grid shows.
const OUTLINE_BELOW: usize = 2_500;
/// Half the height of the box the surface is drawn in; across it is 1.
const Z_HALF: f32 = 0.7;
/// Where the light comes from, in the box's coordinates.
const LIGHT: [f32; 3] = [-0.4, -0.6, 0.7];

const LOG_SCALE: StyleProperty = StyleProperty {
    name: "log_scale",
    default: 0.0,
    suggested: [0.0, 1.0],
    unit: "",
    description: "1 uses the logarithm of the value (heights, colours and contour levels), \
                  for fields spanning orders of magnitude; values that are not positive are \
                  left out. 0 is linear",
    applies: always,
};

const LEVELS: StyleProperty = StyleProperty {
    name: "levels",
    default: 10.0,
    suggested: [1.0, 40.0],
    unit: "",
    description: "Number of contour lines, evenly spaced between the smallest and largest \
                  value (evenly in the logarithm with log_scale)",
    applies: always,
};

pub fn register(r: &mut ViewRegistry) -> Result<(), String> {
    r.register(ViewType {
        name: "plot.surface",
        label: "Surface",
        description: "A grid field as heights over its two axes, coloured by value",
        slots: &[FIELD, X_AXIS, Y_AXIS],
        style: &[HEAT_TRAIL, LOG_SCALE],
        applies_to: &[],
        swap: Some(Swap::Slots(["x", "y"])),
        presets: &[Preset {
            label: "Surface",
            description: "A grid field as a 3D surface",
            bindings: grid_bindings,
        }],
        create: || Box::new(Surface::default()),
    })?;
    r.register(ViewType {
        name: "plot.contour",
        label: "Contours",
        description: "Lines of equal value through a grid field, over its two axes",
        slots: &[FIELD, X_AXIS, Y_AXIS],
        style: &[HEAT_TRAIL, LEVELS, LOG_SCALE],
        applies_to: &[],
        swap: Some(Swap::Slots(["x", "y"])),
        presets: &[Preset {
            label: "Contours",
            description: "Lines of equal value through a grid field",
            bindings: grid_bindings,
        }],
        create: || Box::new(Contour),
    })
}

/// The values to draw: as they are, or their logarithm (NaN where not
/// positive).
fn scaled(z: &[f64], log: bool) -> Vec<f64> {
    z.iter()
        .map(|&v| match (log, v > 0.0) {
            (false, _) => v,
            (true, true) => v.ln(),
            (true, false) => f64::NAN,
        })
        .collect()
}

/// Smallest and largest finite value.
fn finite_range(v: &[f64]) -> Option<[f64; 2]> {
    let (lo, hi) = v
        .iter()
        .filter(|v| v.is_finite())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &v| {
            (a.min(v), b.max(v))
        });
    (lo <= hi).then_some([lo, hi])
}

/// `v` within `[lo, hi]` as a fraction (0 for an empty range).
fn fraction(v: f64, [lo, hi]: [f64; 2]) -> f64 {
    if hi > lo {
        (v - lo) / (hi - lo)
    } else {
        0.5
    }
}

/// The value range shown above a view, in the values' own units.
fn range_text(label: &str, [lo, hi]: [f64; 2], log: bool) -> String {
    let un = |v: f64| if log { v.exp() } else { v };
    format!(
        "{label}: {} (dark) to {} (bright){}",
        fmt(un(lo)),
        fmt(un(hi)),
        if log { ", log scale" } else { "" }
    )
}

/// A number in at most four significant digits, for axis ends.
fn short(v: f64) -> String {
    if v == 0.0 || !v.is_finite() {
        return format!("{v}");
    }
    let digits = (3 - v.abs().log10().floor() as i32).clamp(0, 12) as usize;
    let s = format!("{v:.digits$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

/// One quad of a surface: its corner nodes (anticlockwise from the low
/// corner), its depth (larger is further) and its colour.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Quad {
    nodes: [usize; 4],
    depth: f32,
    colour: Color32,
}

/// Positions taken along an axis of `n` nodes at stride `s`, always ending
/// at the last.
fn strided(n: usize, s: usize) -> Vec<usize> {
    let mut v: Vec<usize> = (0..n).step_by(s.max(1)).collect();
    if v.last() != Some(&(n - 1)) {
        v.push(n - 1);
    }
    v
}

/// The quads of a `w` × `h` grid of nodes (`nodes[j * w + i]`, in the box's
/// coordinates, `None` where the value is missing), strided to at most
/// [`MAX_QUADS`], coloured by `fractions` and shaded by their slope, sorted
/// furthest first so that drawing in order hides what is behind.
fn surface_quads(
    w: usize,
    h: usize,
    nodes: &[Option<[f32; 3]>],
    fractions: &[f64],
    camera: &Camera,
) -> Vec<Quad> {
    if w < 2 || h < 2 {
        return Vec::new();
    }
    let s = (((w - 1) * (h - 1)) as f64 / MAX_QUADS as f64)
        .sqrt()
        .ceil()
        .max(1.0) as usize;
    let (xs, ys) = (strided(w, s), strided(h, s));
    let light = {
        let n = LIGHT.iter().map(|v| v * v).sum::<f32>().sqrt();
        LIGHT.map(|v| v / n)
    };
    let mut quads = Vec::new();
    for jj in ys.windows(2) {
        for ii in xs.windows(2) {
            let ids = [
                jj[0] * w + ii[0],
                jj[0] * w + ii[1],
                jj[1] * w + ii[1],
                jj[1] * w + ii[0],
            ];
            let Some(p) = ids
                .iter()
                .map(|&k| nodes[k])
                .collect::<Option<Vec<[f32; 3]>>>()
            else {
                continue;
            };
            let d = |a: usize, b: usize| [0, 1, 2].map(|k| p[b][k] - p[a][k]);
            let (u, v) = (d(0, 2), d(1, 3));
            let n = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let len = n.iter().map(|c| c * c).sum::<f32>().sqrt().max(1e-12);
            let lit = (n.iter().zip(light).map(|(a, b)| a * b).sum::<f32>() / len).abs();
            let shade = 0.45 + 0.55 * lit;
            let f = ids.iter().map(|&k| fractions[k]).sum::<f64>() / 4.0;
            let c = sequential(f as f32);
            let colour = Color32::from_rgb(
                (c.r() as f32 * shade) as u8,
                (c.g() as f32 * shade) as u8,
                (c.b() as f32 * shade) as u8,
            );
            let centre = [0, 1, 2].map(|k| p.iter().map(|q| q[k]).sum::<f32>() / 4.0);
            quads.push(Quad {
                nodes: ids,
                depth: camera.project(centre)[2],
                colour,
            });
        }
    }
    quads.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    quads
}

/// A grid field as heights, drawn into a box: across and into the picture
/// are its axes, up is its value.
#[derive(Default)]
struct Surface {
    camera: Camera,
}

impl View for Surface {
    fn show(&mut self, ui: &mut egui::Ui, input: &ViewInput<'_>) {
        let Some(sh) = sheet(input, MAX_COLUMNS) else {
            return;
        };
        let log = input.style("log_scale") != 0.0;
        let z = scaled(&sh.z, log);
        let Some(zr) = finite_range(&z) else {
            ui.weak(format!("{}: nothing to draw", sh.labels[2]));
            return;
        };
        ui.weak(range_text(&sh.labels[2], zr, log));
        let (w, h) = (sh.x.len(), sh.y.len());
        let xr = finite_range(&sh.x).unwrap_or([0.0, 1.0]);
        let yr = finite_range(&sh.y).unwrap_or([0.0, 1.0]);
        let unit = |v: f64, r: [f64; 2], half: f32| (2.0 * fraction(v, r) - 1.0) as f32 * half;
        let nodes: Vec<Option<[f32; 3]>> = (0..w * h)
            .map(|k| {
                let p = [sh.x[k % w], sh.y[k / w], z[k]];
                p.iter().all(|v| v.is_finite()).then(|| {
                    [
                        unit(p[0], xr, 1.0),
                        unit(p[1], yr, 1.0),
                        unit(p[2], zr, Z_HALF),
                    ]
                })
            })
            .collect();
        let fractions: Vec<f64> = z.iter().map(|&v| fraction(v, zr)).collect();
        let (lo, hi) = ([-1.0, -1.0, -Z_HALF], [1.0, 1.0, Z_HALF]);
        let frame = Frame::begin(ui, &mut self.camera, lo, hi, false);
        let visuals = ui.visuals();
        let un = |v: f64| if log { v.exp() } else { v };
        let labels = [
            format!("{} {}…{}", sh.labels[0], short(xr[0]), short(xr[1])),
            format!("{} {}…{}", sh.labels[1], short(yr[0]), short(yr[1])),
            format!("{} {}…{}", sh.labels[2], short(un(zr[0])), short(un(zr[1]))),
        ];
        let labels = [&labels[0], &labels[1], &labels[2]].map(String::as_str);
        let quads = surface_quads(w, h, &nodes, &fractions, &frame.camera);
        let screen: Vec<Pos2> = nodes
            .iter()
            .map(|p| p.map_or(Pos2::ZERO, |p| frame.to_screen(p)))
            .collect();
        if quads.len() < OUTLINE_BELOW {
            let outline = Stroke::new(0.5, visuals.extreme_bg_color.gamma_multiply(0.6));
            for q in &quads {
                let pts = q.nodes.map(|k| screen[k]).to_vec();
                frame
                    .painter
                    .add(Shape::convex_polygon(pts, q.colour, outline));
            }
        } else {
            let mut mesh = Mesh::default();
            for q in &quads {
                let base = mesh.vertices.len() as u32;
                for k in q.nodes {
                    mesh.colored_vertex(screen[k], q.colour);
                }
                mesh.add_triangle(base, base + 1, base + 2);
                mesh.add_triangle(base, base + 2, base + 3);
            }
            frame.painter.add(Shape::mesh(mesh));
        }
        frame.axes(visuals, lo, hi, labels);
        frame.hint(visuals);
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
}

/// `n` levels evenly spaced strictly inside `[lo, hi]`.
fn levels([lo, hi]: [f64; 2], n: usize) -> Vec<f64> {
    (1..=n)
        .map(|k| lo + (hi - lo) * k as f64 / (n + 1) as f64)
        .collect()
}

/// The lines where the field `z` (`z[j * x.len() + i]` at `(x[i], y[j])`)
/// equals `level`, by marching squares with linear interpolation along the
/// cell edges. Cells with a missing value are skipped; a saddle cell is
/// split by its mean value. Closed lines end where they start.
pub fn contour_lines(x: &[f64], y: &[f64], z: &[f64], level: f64) -> Vec<Vec<[f64; 2]>> {
    let (w, h) = (x.len(), y.len());
    if w < 2 || h < 2 || z.len() != w * h {
        return Vec::new();
    }
    // Edge ids: 2 × node for the edge to the right of it, + 1 for the edge up.
    let mut points: HashMap<usize, [f64; 2]> = HashMap::new();
    let mut segments: Vec<[usize; 2]> = Vec::new();
    for j in 0..h - 1 {
        for i in 0..w - 1 {
            // Corners anticlockwise from the bottom left.
            let c = [(i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1)];
            let v = c.map(|(i, j)| z[j * w + i]);
            if v.iter().any(|v| !v.is_finite()) {
                continue;
            }
            let above = v.map(|v| v > level);
            // Bottom, right, top and left edges, between which corners.
            let edges = [
                (2 * (j * w + i), 0, 1),
                (2 * (j * w + i + 1) + 1, 1, 2),
                (2 * ((j + 1) * w + i), 3, 2),
                (2 * (j * w + i) + 1, 0, 3),
            ];
            let crossed: Vec<usize> = (0..4)
                .filter(|&k| above[edges[k].1] != above[edges[k].2])
                .collect();
            for &k in &crossed {
                let (e, a, b) = edges[k];
                points.entry(e).or_insert_with(|| {
                    let t = (level - v[a]) / (v[b] - v[a]);
                    let (pa, pb) = (c[a], c[b]);
                    [
                        x[pa.0] + t * (x[pb.0] - x[pa.0]),
                        y[pa.1] + t * (y[pb.1] - y[pa.1]),
                    ]
                });
            }
            let e = edges.map(|e| e.0);
            match crossed.len() {
                2 => segments.push([e[crossed[0]], e[crossed[1]]]),
                4 => {
                    let centre = v.iter().sum::<f64>() / 4.0 > level;
                    if centre == above[0] {
                        // Corners 0 and 2 join across the middle: cut off 1 and 3.
                        segments.extend([[e[0], e[1]], [e[2], e[3]]]);
                    } else {
                        segments.extend([[e[3], e[0]], [e[1], e[2]]]);
                    }
                }
                _ => {}
            }
        }
    }
    chain(&segments)
        .into_iter()
        .map(|line| line.iter().map(|e| points[e]).collect())
        .collect()
}

/// Join segments that share an edge into lines of edge ids.
fn chain(segments: &[[usize; 2]]) -> Vec<Vec<usize>> {
    let mut by_edge: HashMap<usize, Vec<usize>> = HashMap::new();
    for (s, seg) in segments.iter().enumerate() {
        for &e in seg {
            by_edge.entry(e).or_default().push(s);
        }
    }
    let mut used = vec![false; segments.len()];
    // The far end of an unused segment at edge `end`, marking it used.
    let next = |end: usize, used: &mut Vec<bool>| -> Option<usize> {
        let &s = by_edge[&end].iter().find(|&&s| !used[s])?;
        used[s] = true;
        let [a, b] = segments[s];
        Some(if a == end { b } else { a })
    };
    let mut lines = Vec::new();
    for start in 0..segments.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let mut line = VecDeque::from(segments[start].to_vec());
        while let Some(e) = next(*line.back().expect("not empty"), &mut used) {
            line.push_back(e);
        }
        while let Some(e) = next(*line.front().expect("not empty"), &mut used) {
            line.push_front(e);
        }
        lines.push(line.into());
    }
    lines
}

/// Lines of equal value over the field's two axes, coloured from low to
/// high like the heat map.
struct Contour;

impl View for Contour {
    fn show(&mut self, ui: &mut egui::Ui, input: &ViewInput<'_>) {
        let Some(sh) = sheet(input, MAX_COLUMNS) else {
            return;
        };
        let log = input.style("log_scale") != 0.0;
        let z = scaled(&sh.z, log);
        let Some(zr) = finite_range(&z) else {
            ui.weak(format!("{}: nothing to draw", sh.labels[2]));
            return;
        };
        let n = input.style("levels").round().clamp(1.0, 100.0) as usize;
        ui.weak(format!("{n} lines; {}", range_text(&sh.labels[2], zr, log)));
        let mut plot = Plot::new("plot.contour")
            .x_axis_label(sh.labels[0].clone())
            .y_axis_label(sh.labels[1].clone())
            .allow_scroll(false);
        for (r, x) in [(finite_range(&sh.x), true), (finite_range(&sh.y), false)] {
            if let Some([lo, hi]) = r {
                plot = if x {
                    plot.include_x(lo).include_x(hi)
                } else {
                    plot.include_y(lo).include_y(hi)
                };
            }
        }
        plot.show(ui, |plot| {
            scroll_to_zoom(plot);
            for level in levels(zr, n) {
                let colour = sequential(fraction(level, zr) as f32);
                let name = fmt(if log { level.exp() } else { level });
                for line in contour_lines(&sh.x, &sh.y, &z, level) {
                    plot.line(Line::new(name.clone(), line).color(colour));
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linspace(a: f64, b: f64, n: usize) -> Vec<f64> {
        (0..n)
            .map(|k| a + (b - a) * k as f64 / (n - 1) as f64)
            .collect()
    }

    #[test]
    fn contours_of_a_plane_are_straight_and_exact() {
        let (x, y) = ([0.0, 1.0, 2.0, 3.0], [0.0, 1.0, 2.0]);
        let z: Vec<f64> = (0..12).map(|k| x[k % 4]).collect();
        let lines = contour_lines(&x, &y, &z, 1.5);
        assert_eq!(lines.len(), 1);
        let mut line = lines[0].clone();
        if line[0][1] > 0.0 {
            line.reverse();
        }
        assert_eq!(line, [[1.5, 0.0], [1.5, 1.0], [1.5, 2.0]]);
        // Outside the values: nothing.
        assert!(contour_lines(&x, &y, &z, 5.0).is_empty());
    }

    #[test]
    fn contours_of_a_cone_are_closed_circles() {
        let g = linspace(-1.0, 1.0, 41);
        let z: Vec<f64> = (0..41 * 41).map(|k| g[k % 41].hypot(g[k / 41])).collect();
        for r in [0.3, 0.55, 0.8] {
            let lines = contour_lines(&g, &g, &z, r);
            assert_eq!(lines.len(), 1, "r = {r}");
            let l = &lines[0];
            assert_eq!(l.first(), l.last(), "closed");
            assert!(l.len() > 20);
            // Linear interpolation along edges of spacing h = 0.05 is off by
            // at most about h² / r.
            for p in l {
                let err = (p[0].hypot(p[1]) - r).abs();
                assert!(err < 0.01, "r = {r}: {p:?} is {err} off");
            }
        }
    }

    #[test]
    fn saddles_and_missing_values_are_handled() {
        // z = x y on a 3 × 3 grid: the zero contour is two crossing lines,
        // which marching squares draws as lines meeting at the centre.
        let g = [-1.0, 0.0, 1.0];
        let z: Vec<f64> = (0..9).map(|k| g[k % 3] * g[k / 3]).collect();
        let lines = contour_lines(&g, &g, &z, 0.25);
        assert_eq!(lines.len(), 2);
        // A cell with a missing value is skipped; the line stops there.
        let mut gap = z.clone();
        gap[8] = f64::NAN;
        assert_eq!(contour_lines(&g, &g, &gap, 0.25).len(), 1);
        // A 1-row grid has no cells.
        assert!(contour_lines(&g, &[0.0], &[0.0, 1.0, 2.0], 0.5).is_empty());
    }

    #[test]
    fn axis_ends_are_short() {
        assert_eq!(short(3.0), "3");
        assert_eq!(short(0.02), "0.02");
        assert_eq!(short(44.87912), "44.88");
        assert_eq!(short(1234567.0), "1234567");
        assert_eq!(short(0.0), "0");
    }

    #[test]
    fn levels_lie_inside_the_range_and_logs_space_them_geometrically() {
        assert_eq!(levels([0.0, 10.0], 4), [2.0, 4.0, 6.0, 8.0]);
        let z = scaled(&[1.0, 100.0, -1.0], true);
        assert!(z[2].is_nan());
        let r = finite_range(&z).unwrap();
        let l: Vec<f64> = levels(r, 1).into_iter().map(f64::exp).collect();
        assert!((l[0] - 10.0).abs() < 1e-9);
    }

    #[test]
    fn surface_quads_skip_missing_values_and_are_sorted_far_first() {
        let camera = Camera::default();
        let node = |k: usize| Some([(k % 3) as f32 - 1.0, (k / 3) as f32 - 1.0, 0.1 * k as f32]);
        let mut nodes: Vec<Option<[f32; 3]>> = (0..9).map(node).collect();
        let fractions: Vec<f64> = (0..9).map(|k| k as f64 / 8.0).collect();
        let quads = surface_quads(3, 3, &nodes, &fractions, &camera);
        assert_eq!(quads.len(), 4);
        assert!(quads.windows(2).all(|w| w[0].depth >= w[1].depth));
        assert_eq!(quads.iter().filter(|q| q.nodes.contains(&4)).count(), 4);
        // A missing corner removes its one quad; the centre, all four.
        nodes[0] = None;
        assert_eq!(surface_quads(3, 3, &nodes, &fractions, &camera).len(), 3);
        nodes[4] = None;
        assert!(surface_quads(3, 3, &nodes, &fractions, &camera).is_empty());
    }

    #[test]
    fn large_surfaces_are_strided_to_the_quad_limit() {
        let (w, h) = (1000, 500);
        let nodes = vec![Some([0.0f32; 3]); w * h];
        let fractions = vec![0.5; w * h];
        let quads = surface_quads(w, h, &nodes, &fractions, &Camera::default());
        assert!(quads.len() <= MAX_QUADS && quads.len() > MAX_QUADS / 4);
        assert_eq!(strided(10, 4), [0, 4, 8, 9]);
    }
}
