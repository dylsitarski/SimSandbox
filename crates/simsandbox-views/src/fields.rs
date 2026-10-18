//! Views of fields (ADR 0016, ADR 0024): `plot.heatmap` (a grid as colours
//! over two axes), `plot.profile` (line fields against a coordinate, or cuts
//! through 2D grids, ADR 0029) and `plot.scatter` (a point set).

use crate::builtin::{bindings, finite_runs, scroll_to_zoom, series_colour, widened};
use crate::colour::{sequential, trail_start};
use crate::history::{FieldShape, History, SignalRef, TIME};
use crate::view::{
    always, Available, Context, Preset, Slot, SlotKind, StyleProperty, View, ViewInput,
    ViewRegistry, ViewType,
};
use egui_plot::{Legend, Line, Plot, PlotImage, PlotPoint, Points};
use serde::{Deserialize, Serialize};

/// At most this many samples of a line field over time are drawn.
const MAX_COLUMNS: usize = 512;
/// Uneven coordinates are resampled onto at most this many texels per axis.
const MAX_TEXELS: usize = 1024;

/// Applies to a heat map whose field is a line of cells drawn over time.
fn line_over_time(c: &Context<'_>) -> bool {
    let line = c
        .bind
        .get("field")
        .and_then(|v| v.first())
        .and_then(|name| match c.history.resolve(name) {
            Some(SignalRef::Field(f)) => Some(c.history.fields()[f].shape),
            _ => None,
        })
        .is_some_and(|s| matches!(s, FieldShape::Grid { ny: 1, .. }));
    line && c.history.over_time()
}

const HEAT_TRAIL: StyleProperty = StyleProperty {
    name: "trail_length",
    default: -1.0,
    suggested: [-1.0, 60.0],
    unit: "s",
    description:
        "Seconds of the run shown along the time axis; -1 (any negative value) the whole run",
    applies: line_over_time,
};

const LOG_COLOUR: StyleProperty = StyleProperty {
    name: "log_colour",
    default: 0.0,
    suggested: [0.0, 1.0],
    unit: "",
    description:
        "1 colours by the logarithm of the value, for fields spanning orders of magnitude \
                  (values that are not positive are left blank); 0 colours linearly",
    applies: always,
};

/// The first grid that is not itself a coordinate of another field.
fn first_result<'a>(a: &'a Available<'_>, lines_only: bool) -> Option<&'a String> {
    a.grids.iter().find(|g| {
        !a.coordinates.iter().any(|c| &c.name == *g)
            && (!lines_only || a.lines.iter().any(|l| l == *g))
    })
}

/// The coordinate of `field` along grid axis `axis`, if it has one.
fn coordinate<'a>(a: &'a Available<'_>, field: &str, axis: usize) -> Option<&'a str> {
    a.coordinates
        .iter()
        .find(|c| c.of == field && c.axis == axis)
        .map(|c| c.name.as_str())
}

pub fn register(r: &mut ViewRegistry) -> Result<(), String> {
    r.register(ViewType {
        name: "plot.heatmap",
        label: "Heat map",
        description: "A grid field as colours over two axes: a line of cells over time, or a 2D grid now",
        slots: &[
            Slot {
                name: "field",
                kind: SlotKind::Grid,
                of: None,
                applies: always,
                many: false,
                required: true,
                description: "The grid field shown",
            },
            Slot {
                name: "x",
                kind: SlotKind::Axis,
                of: Some("field"),
                applies: always,
                many: false,
                required: false,
                description: "Across: t, or a coordinate of the field such as rod.position or a \
                              swept parameter; without it, the field's next direction by cell number",
            },
            Slot {
                name: "y",
                kind: SlotKind::Axis,
                of: Some("field"),
                applies: always,
                many: false,
                required: false,
                description: "Up: t, or a coordinate of the field; without it, the field's \
                              remaining direction by cell number",
            },
        ],
        style: &[HEAT_TRAIL, LOG_COLOUR],
        applies_to: &[],
        swap: Some(["x", "y"]),
        presets: &[Preset {
            label: "Heat map",
            description: "A grid field as colours",
            bindings: |a| {
                let field = first_result(a, false)?;
                let mut b = bindings(&[("field", field)]);
                let line = a.lines.iter().any(|l| l == field);
                let (x, y) = if line {
                    (Some(TIME), coordinate(a, field, 0))
                } else {
                    (coordinate(a, field, 0), coordinate(a, field, 1))
                };
                for (slot, name) in [("x", x), ("y", y)] {
                    if let Some(n) = name {
                        b.insert(slot.into(), vec![n.to_string()]);
                    }
                }
                Some(b)
            },
        }],
        create: || Box::new(Heatmap::default()),
    })?;
    r.register(ViewType {
        name: "plot.profile",
        label: "Profile",
        description: "Line fields as they are now against position, or cuts through 2D grids",
        slots: &[
            Slot {
                name: "x",
                kind: SlotKind::Coordinate,
                of: Some("y"),
                applies: first_y_is_line,
                many: false,
                required: false,
                description:
                    "Where each cell lies, e.g. rod.position; without it cells are numbered",
            },
            Slot {
                name: "y",
                kind: SlotKind::Grid,
                of: None,
                applies: always,
                many: true,
                required: true,
                description: "The fields drawn, one curve each",
            },
        ],
        style: &[],
        applies_to: &[],
        swap: None,
        presets: &[Preset {
            label: "Profile",
            description: "A line field now against position, or cuts through a 2D grid",
            bindings: |a| {
                let y = first_result(a, true).or_else(|| first_result(a, false))?;
                let mut b = bindings(&[("y", y)]);
                if let Some(x) = coordinate(a, y, 0) {
                    b.insert("x".into(), vec![x.to_string()]);
                }
                Some(b)
            },
        }],
        create: || Box::new(Profile::default()),
    })?;
    r.register(ViewType {
        name: "plot.scatter",
        label: "Points",
        description: "A set of points where they are now",
        slots: &[Slot {
            name: "points",
            kind: SlotKind::Points,
            of: None,
            applies: always,
            many: false,
            required: true,
            description: "The point set shown (first two coordinates)",
        }],
        style: &[],
        applies_to: &[],
        swap: None,
        presets: &[Preset {
            label: "Points",
            description: "A set of points where they are now",
            bindings: |a| a.points.first().map(|p| bindings(&[("points", p)])),
        }],
        create: || Box::new(Scatter),
    })
}

/// A direction a heat map can run along: time, or one axis of the grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dim {
    Time,
    Grid(usize),
}

/// One screen axis: its direction, the coordinate field placing it (if
/// any), and the cell centres along it in data units.
struct Axis {
    dim: Dim,
    coord: Option<usize>,
    centres: Vec<f64>,
}

/// Where cells lie along one screen axis: the extent `[lo, hi]` and, for
/// each texel from `lo` upwards, the cell it shows. Centres must be finite
/// and strictly monotonic (either way); each cell reaches halfway to its
/// neighbours. Evenly spaced centres map one texel per cell; uneven ones are
/// resampled at the smallest spacing (at most [`MAX_TEXELS`]).
fn axis_map(centres: &[f64]) -> Option<([f64; 2], Vec<usize>)> {
    let n = centres.len();
    if n == 0 || centres.iter().any(|c| !c.is_finite()) {
        return None;
    }
    if n == 1 {
        return Some(([centres[0] - 0.5, centres[0] + 0.5], vec![0]));
    }
    let increasing = centres.windows(2).all(|w| w[1] > w[0]);
    let decreasing = centres.windows(2).all(|w| w[1] < w[0]);
    if !increasing && !decreasing {
        return None;
    }
    let order: Vec<usize> = if increasing {
        (0..n).collect()
    } else {
        (0..n).rev().collect()
    };
    let c: Vec<f64> = order.iter().map(|&i| centres[i]).collect();
    let mut edges = Vec::with_capacity(n + 1);
    edges.push(c[0] - 0.5 * (c[1] - c[0]));
    edges.extend(c.windows(2).map(|w| 0.5 * (w[0] + w[1])));
    edges.push(c[n - 1] + 0.5 * (c[n - 1] - c[n - 2]));
    let (lo, hi) = (edges[0], edges[n]);
    let gaps: Vec<f64> = edges.windows(2).map(|w| w[1] - w[0]).collect();
    let (min, max) = gaps
        .iter()
        .fold((f64::INFINITY, 0.0f64), |(a, b), &g| (a.min(g), b.max(g)));
    if max - min <= 1e-9 * max {
        return Some(([lo, hi], order));
    }
    let texels = (((hi - lo) / min).ceil() as usize).clamp(n, MAX_TEXELS.max(n));
    let map = (0..texels)
        .map(|t| {
            let u = lo + (t as f64 + 0.5) * (hi - lo) / texels as f64;
            let k = edges[1..n].partition_point(|&e| e <= u);
            order[k.min(n - 1)]
        })
        .collect();
    Some(([lo, hi], map))
}

/// A heat map picture: size in texels, values row by row from the top, and
/// the extent `[x0, x1, y0, y1]`.
type HeatImage = ([usize; 2], Vec<f64>, [f64; 4]);

/// Build the picture of field `field` over screen axes `x` and `y`.
/// `samples` are the history samples a time axis runs over.
fn heat_image(
    h: &History,
    field: usize,
    samples: &[usize],
    x: &Axis,
    y: &Axis,
) -> Option<HeatImage> {
    let f = &h.fields()[field];
    let FieldShape::Grid { nx, .. } = f.shape else {
        return None;
    };
    let n = h.len();
    let (ex, mx) = axis_map(&x.centres)?;
    let (ey, my) = axis_map(&y.centres)?;
    // Which sample and cell a pair of axis positions shows.
    let value = |a: usize, b: usize| {
        let (mut k, mut i, mut j) = (n - 1, 0, 0);
        for (dim, pos) in [(x.dim, a), (y.dim, b)] {
            match dim {
                Dim::Time => k = samples[pos],
                Dim::Grid(0) => i = pos,
                Dim::Grid(_) => j = pos,
            }
        }
        f.frame(k).get(j * nx + i).copied().unwrap_or(f64::NAN)
    };
    let (w, hgt) = (mx.len(), my.len());
    let mut values = Vec::with_capacity(w * hgt);
    for ty in (0..hgt).rev() {
        for &cx in &mx {
            values.push(value(cx, my[ty]));
        }
    }
    Some(([w, hgt], values, [ex[0], ex[1], ey[0], ey[1]]))
}

/// Colour each value on the sequential scale between the smallest and
/// largest finite value; non-finite values are transparent. With `log`,
/// colours follow the logarithm, so fields spanning orders of magnitude stay
/// readable; values that are not positive are then transparent. The range
/// returned is in the values' own units.
fn colour_image(size: [usize; 2], values: &[f64], log: bool) -> (egui::ColorImage, f64, f64) {
    let scale = |v: f64| if log { v.ln() } else { v };
    let (lo, hi) = values
        .iter()
        .map(|&v| scale(v))
        .filter(|v| v.is_finite())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| {
            (a.min(v), b.max(v))
        });
    let span = (hi - lo).max(1e-300);
    let pixels = values
        .iter()
        .map(|&v| {
            let v = scale(v);
            if v.is_finite() {
                sequential(((v - lo) / span) as f32)
            } else {
                egui::Color32::TRANSPARENT
            }
        })
        .collect();
    let (lo, hi) = if log { (lo.exp(), hi.exp()) } else { (lo, hi) };
    (
        egui::ColorImage {
            size,
            source_size: egui::Vec2::new(size[0] as f32, size[1] as f32),
            pixels,
        },
        lo,
        hi,
    )
}

/// The screen axes of a heat map from its bindings (ADR 0024): each of `x`
/// and `y` claims time or a grid axis; unbound ones take the directions
/// left, in order, numbered by cell.
fn heat_axes(input: &ViewInput<'_>, field: usize, samples: &[usize]) -> Option<(Axis, Axis)> {
    let h = input.history;
    let f = &h.fields()[field];
    let FieldShape::Grid { nx, ny } = f.shape else {
        return None;
    };
    let line = ny == 1;
    let claim = |slot: &str| -> Option<(Dim, Option<usize>)> {
        let name = input.bound(slot)?;
        if name == TIME {
            return line.then_some((Dim::Time, None));
        }
        let axis = h.coordinate_axis(name, &f.name)?;
        match input.one(slot) {
            Some(SignalRef::Field(c)) if axis < 2 && !(line && axis > 0) => {
                Some((Dim::Grid(axis), Some(c)))
            }
            _ => None,
        }
    };
    let x = claim("x");
    let mut y = claim("y");
    if x.is_some() && x.map(|c| c.0) == y.map(|c| c.0) {
        y = None;
    }
    let needed = if line {
        [Dim::Time, Dim::Grid(0)]
    } else {
        [Dim::Grid(0), Dim::Grid(1)]
    };
    let (cx, cy) = (x.map(|c| c.0), y.map(|c| c.0));
    let mut free = needed
        .into_iter()
        .filter(|d| Some(*d) != cx && Some(*d) != cy);
    let x = x.or_else(|| free.next().map(|d| (d, None)));
    let y = y.or_else(|| free.next().map(|d| (d, None)));
    let axis = |(dim, coord): (Dim, Option<usize>)| {
        let cells = match dim {
            Dim::Time => 0,
            Dim::Grid(0) => nx,
            Dim::Grid(_) => ny,
        };
        let cell_centres = || (0..cells).map(|i| i as f64 + 0.5).collect::<Vec<_>>();
        let centres = match (dim, coord) {
            (Dim::Time, _) => samples.iter().map(|&k| h.times()[k]).collect(),
            (Dim::Grid(a), Some(c)) => {
                let frame = h.fields()[c].frame(h.len() - 1);
                let along: Vec<f64> = (0..cells)
                    .map(|i| {
                        frame
                            .get(if a == 0 { i } else { i * nx })
                            .copied()
                            .unwrap_or(f64::NAN)
                    })
                    .collect();
                // A coordinate that cannot place the cells falls back to cells.
                if axis_map(&along).is_some() {
                    along
                } else {
                    return Axis {
                        dim,
                        coord: None,
                        centres: cell_centres(),
                    };
                }
            }
            (Dim::Grid(_), None) => cell_centres(),
        };
        Axis {
            dim,
            coord,
            centres,
        }
    };
    Some((axis(x?), axis(y?)))
}

#[derive(Default)]
struct Heatmap {
    texture: Option<egui::TextureHandle>,
}

impl View for Heatmap {
    fn show(&mut self, ui: &mut egui::Ui, input: &ViewInput<'_>) {
        let h = input.history;
        let Some(SignalRef::Field(field)) = input.one("field") else {
            return;
        };
        if h.is_empty() {
            return;
        }
        let first = trail_start(h.times(), input.style("trail_length")).min(h.len() - 1);
        let stride = (h.len() - first).div_ceil(MAX_COLUMNS).max(1);
        let samples: Vec<usize> = (first..h.len()).step_by(stride).collect();
        let Some((x, y)) = heat_axes(input, field, &samples) else {
            return;
        };
        let Some((size, values, [x0, x1, y0, y1])) = heat_image(h, field, &samples, &x, &y) else {
            return;
        };
        let log = input.style("log_colour") != 0.0;
        let (image, lo, hi) = colour_image(size, &values, log);
        let options = egui::TextureOptions::NEAREST;
        let texture = match &mut self.texture {
            Some(t) => {
                t.set(image, options);
                t.clone()
            }
            None => {
                let t = ui.ctx().load_texture("plot.heatmap", image, options);
                self.texture = Some(t.clone());
                t
            }
        };
        ui.weak(format!(
            "{}: {} (dark) to {} (bright){}",
            h.name(SignalRef::Field(field)),
            fmt(lo),
            fmt(hi),
            if log { ", log scale" } else { "" }
        ));
        let line = matches!(h.fields()[field].shape, FieldShape::Grid { ny: 1, .. });
        let label = |a: &Axis| match (a.dim, a.coord) {
            (Dim::Time, _) if h.over_time() => "t [s]".to_string(),
            (Dim::Time, _) => h.index().to_string(),
            (_, Some(c)) => h.name(SignalRef::Field(c)).to_string(),
            (Dim::Grid(_), None) if line => "cell".to_string(),
            (Dim::Grid(0), None) => "column".to_string(),
            (Dim::Grid(_), None) => "row".to_string(),
        };
        Plot::new("plot.heatmap")
            .x_axis_label(label(&x))
            .y_axis_label(label(&y))
            .allow_scroll(false)
            .show(ui, |plot| {
                scroll_to_zoom(plot);
                plot.image(PlotImage::new(
                    "field",
                    texture.id(),
                    PlotPoint::new((x0 + x1) / 2.0, (y0 + y1) / 2.0),
                    egui::vec2((x1 - x0) as f32, (y1 - y0) as f32),
                ));
            });
    }
}

fn fmt(v: f64) -> String {
    if v.is_finite() {
        format!("{v:.4}")
    } else {
        "–".into()
    }
}

/// What a profile's cuts are plotted against (ADR 0029).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Against {
    /// 0 to 1 along each cut; works for any cuts.
    #[default]
    Fraction,
    /// The grid's x coordinate; when every cut runs across.
    X,
    /// The grid's y coordinate; when every cut runs up or down.
    Y,
    /// Distance along the cut; when both directions share a unit.
    Distance,
}

/// How values between cell centres are found: an approximation, so it is a
/// method choice (ADR 0020).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Interpolation {
    Nearest,
    #[default]
    Bilinear,
}

/// A straight cut between two points in a grid's coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Cut {
    pub from: [f64; 2],
    pub to: [f64; 2],
}

/// A profile's selection: its cuts and how they are drawn (ADR 0029).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cuts {
    pub cuts: Vec<Cut>,
    #[serde(default)]
    pub against: Against,
    #[serde(default)]
    pub interpolation: Interpolation,
}

/// Where a 2D grid's cells lie: their centres along each axis, in the
/// coordinates' units (or cell numbers, centres at i + 0.5, as on a heat
/// map), and the coordinates' names.
#[derive(Debug, Clone, PartialEq)]
struct Plane {
    nx: usize,
    ny: usize,
    centres: [Vec<f64>; 2],
    names: [Option<String>; 2],
}

impl Plane {
    fn of(h: &History, field: usize) -> Option<Self> {
        let f = &h.fields()[field];
        let FieldShape::Grid { nx, ny } = f.shape else {
            return None;
        };
        if nx < 2 || ny < 2 || h.is_empty() {
            return None;
        }
        let along = |axis: usize| -> (Vec<f64>, Option<String>) {
            let cells = if axis == 0 { nx } else { ny };
            let coord = h
                .coordinates()
                .iter()
                .find(|c| c.of == f.name && c.axis == axis)
                .and_then(|c| match h.resolve(&c.name) {
                    Some(SignalRef::Field(i)) => Some((i, c.name.clone())),
                    _ => None,
                });
            if let Some((i, name)) = coord {
                let frame = h.fields()[i].frame(h.len() - 1);
                let values: Vec<f64> = (0..cells)
                    .map(|k| {
                        frame
                            .get(if axis == 0 { k } else { k * nx })
                            .copied()
                            .unwrap_or(f64::NAN)
                    })
                    .collect();
                if axis_map(&values).is_some() {
                    return (values, Some(name));
                }
            }
            ((0..cells).map(|k| k as f64 + 0.5).collect(), None)
        };
        let (cx, nx_name) = along(0);
        let (cy, ny_name) = along(1);
        Some(Self {
            nx,
            ny,
            centres: [cx, cy],
            names: [nx_name, ny_name],
        })
    }

    fn label(&self, axis: usize) -> String {
        self.names[axis]
            .clone()
            .unwrap_or_else(|| if axis == 0 { "column" } else { "row" }.into())
    }

    /// Four cuts across the grid along rows spread over y, each on a row so
    /// it reads it exactly, plotted against x.
    fn default_cuts(&self) -> Cuts {
        let [cx, cy] = &self.centres;
        let mut rows: Vec<usize> = (0..4)
            .map(|k| (((k as f64 + 0.5) / 4.0 * self.ny as f64) as usize).min(self.ny - 1))
            .collect();
        rows.dedup();
        Cuts {
            cuts: rows
                .into_iter()
                .map(|j| Cut {
                    from: [cx[0], cy[j]],
                    to: [cx[self.nx - 1], cy[j]],
                })
                .collect(),
            against: Against::X,
            interpolation: Interpolation::Bilinear,
        }
    }

    /// What the cuts can be plotted against.
    fn against_choices(&self, cuts: &[Cut]) -> Vec<Against> {
        let mut out = vec![Against::Fraction];
        if cuts.iter().all(|c| c.from[0] != c.to[0]) {
            out.push(Against::X);
        }
        if cuts.iter().all(|c| c.from[1] != c.to[1]) {
            out.push(Against::Y);
        }
        // Units are not recorded yet, so distance is offered only where
        // both directions count cells.
        if self.names.iter().all(Option::is_none) {
            out.push(Against::Distance);
        }
        out
    }
}

/// The fractional cell index of `p` among monotonic `centres`, or `None`
/// outside them.
fn index_of(centres: &[f64], p: f64) -> Option<f64> {
    let n = centres.len();
    if n < 2 || !p.is_finite() {
        return (n == 1 && centres[0] == p).then_some(0.0);
    }
    let sign = if centres[1] > centres[0] { 1.0 } else { -1.0 };
    let q = sign * p;
    if q < sign * centres[0] || q > sign * centres[n - 1] {
        return None;
    }
    let k = centres.partition_point(|&c| sign * c <= q).clamp(1, n - 1) - 1;
    Some(k as f64 + (p - centres[k]) / (centres[k + 1] - centres[k]))
}

/// The value at fractional cell `(u, v)` of an `nx`-wide frame.
fn sample(frame: &[f64], nx: usize, ny: usize, u: f64, v: f64, how: Interpolation) -> f64 {
    let at = |i: usize, j: usize| frame.get(j * nx + i).copied().unwrap_or(f64::NAN);
    match how {
        Interpolation::Nearest => at(
            (u.round() as usize).min(nx - 1),
            (v.round() as usize).min(ny - 1),
        ),
        Interpolation::Bilinear => {
            let split = |x: f64, n: usize| {
                let i = (x.floor() as usize).min(n - 2);
                (i, x - i as f64)
            };
            let (i, a) = split(u, nx);
            let (j, b) = split(v, ny);
            let row = |j| at(i, j) * (1.0 - a) + at(i + 1, j) * a;
            row(j) * (1.0 - b) + row(j + 1) * b
        }
    }
}

/// One cut through a frame: `[horizontal, value]` points, NaN outside the
/// grid.
fn cut_curve(plane: &Plane, frame: &[f64], cut: &Cut, cuts: &Cuts) -> Vec<[f64; 2]> {
    let [cx, cy] = &plane.centres;
    let ends = [cut.from, cut.to].map(|p| (index_of(cx, p[0]), index_of(cy, p[1])));
    let span = match ends {
        [(Some(u0), Some(v0)), (Some(u1), Some(v1))] => (u1 - u0).abs().max((v1 - v0).abs()),
        _ => cx.len().max(cy.len()) as f64,
    };
    let m = ((span.ceil() as usize) + 1).clamp(2, 2000);
    let d = [cut.to[0] - cut.from[0], cut.to[1] - cut.from[1]];
    let length = d[0].hypot(d[1]);
    (0..m)
        .map(|s| {
            let f = s as f64 / (m - 1) as f64;
            let p = [cut.from[0] + f * d[0], cut.from[1] + f * d[1]];
            let value = match (index_of(cx, p[0]), index_of(cy, p[1])) {
                (Some(u), Some(v)) => sample(frame, plane.nx, plane.ny, u, v, cuts.interpolation),
                _ => f64::NAN,
            };
            let x = match cuts.against {
                Against::Fraction => f,
                Against::X => p[0],
                Against::Y => p[1],
                Against::Distance => f * length,
            };
            [x, value]
        })
        .collect()
}

/// Applies when the profile's first field is a line of cells.
fn first_y_is_line(c: &Context<'_>) -> bool {
    c.bind
        .get("y")
        .and_then(|v| v.first())
        .and_then(|name| match c.history.resolve(name) {
            Some(SignalRef::Field(f)) => Some(c.history.fields()[f].shape),
            _ => None,
        })
        .is_some_and(|s| matches!(s, FieldShape::Grid { ny: 1, .. }))
}

/// Line fields against a coordinate as they are now, or cuts through 2D
/// grids (ADR 0029). The value axis spans every value seen so far, so the
/// curve moves instead of the axis.
#[derive(Default)]
struct Profile {
    /// Samples already folded into `range`.
    seen: usize,
    range: Option<[f64; 2]>,
    /// The bindings `range` was computed for.
    key: Vec<SignalRef>,
    /// Cuts through 2D grids; made from the grid when first drawn.
    cuts: Option<Cuts>,
}

impl Profile {
    /// The 2D grid the cuts run through: the first bound field, if 2D.
    fn plane(input: &ViewInput<'_>) -> Option<Plane> {
        match input.many("y").first() {
            Some(&SignalRef::Field(f)) => Plane::of(input.history, f),
            _ => None,
        }
    }

    fn show_cuts(&mut self, ui: &mut egui::Ui, input: &ViewInput<'_>, plane: &Plane) {
        let h = input.history;
        let cuts = self
            .cuts
            .get_or_insert_with(|| plane.default_cuts())
            .clone();
        let mut curves = Vec::new();
        for &y in input.many("y") {
            let SignalRef::Field(f) = y else { continue };
            if h.fields()[f].shape
                != (FieldShape::Grid {
                    nx: plane.nx,
                    ny: plane.ny,
                })
            {
                continue;
            }
            let frame = h.fields()[f].frame(h.len() - 1);
            for cut in &cuts.cuts {
                let label = if cut.from[1] == cut.to[1] {
                    format!("{} = {}", plane.label(1), short(cut.from[1]))
                } else if cut.from[0] == cut.to[0] {
                    format!("{} = {}", plane.label(0), short(cut.from[0]))
                } else {
                    format!(
                        "({}, {}) to ({}, {})",
                        short(cut.from[0]),
                        short(cut.from[1]),
                        short(cut.to[0]),
                        short(cut.to[1])
                    )
                };
                curves.push((
                    format!("{} at {label}", h.name(y)),
                    cut_curve(plane, frame, cut, &cuts),
                ));
            }
        }
        let interp = match cuts.interpolation {
            Interpolation::Nearest => "nearest cell",
            Interpolation::Bilinear => "bilinear interpolation",
        };
        ui.weak(format!("{} cut(s), {interp}", cuts.cuts.len()));
        let x_label = match cuts.against {
            Against::Fraction => "fraction along the cut".to_string(),
            Against::X => plane.label(0),
            Against::Y => plane.label(1),
            Against::Distance => "distance (cells)".to_string(),
        };
        let mut plot = Plot::new("plot.profile")
            .x_axis_label(x_label)
            .allow_scroll(false)
            .legend(Legend::default());
        let all = curves.iter().flat_map(|(_, c)| c.iter().map(|p| p[1]));
        if let Some([lo, hi]) = value_range_of(all) {
            plot = plot.include_y(lo).include_y(hi);
        }
        plot.show(ui, |plot| {
            scroll_to_zoom(plot);
            for (k, (name, curve)) in curves.into_iter().enumerate() {
                let colour = series_colour(k);
                for run in finite_runs(curve.into_iter()) {
                    plot.line(Line::new(name.clone(), run).color(colour));
                }
            }
        });
    }
}

/// The finite range of values, widened to the minimum span.
fn value_range_of(values: impl Iterator<Item = f64>) -> Option<[f64; 2]> {
    crate::builtin::value_range(values).map(|r| widened(r).unwrap_or(r))
}

/// A number short enough for a legend.
fn short(v: f64) -> String {
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

impl View for Profile {
    fn show(&mut self, ui: &mut egui::Ui, input: &ViewInput<'_>) {
        let h = input.history;
        let ys = input.many("y");
        let n = h.len();
        if n == 0 || ys.is_empty() {
            return;
        }
        if let Some(plane) = Self::plane(input) {
            self.show_cuts(ui, input, &plane);
            return;
        }
        // Rewind, restart or new bindings: start the range again.
        if n < self.seen || self.key != ys {
            *self = Self {
                key: ys.to_vec(),
                cuts: self.cuts.take(),
                ..Self::default()
            };
        }
        for &y in ys {
            let SignalRef::Field(f) = y else { continue };
            for k in self.seen..n {
                for &v in h.fields()[f].frame(k).iter().filter(|v| v.is_finite()) {
                    let [lo, hi] = self.range.get_or_insert([v, v]);
                    *lo = lo.min(v);
                    *hi = hi.max(v);
                }
            }
        }
        self.seen = n;
        let coords = match input.one("x") {
            Some(SignalRef::Field(x)) => Some(h.fields()[x].frame(n - 1)),
            _ => None,
        };
        let x_label = match input.one("x") {
            Some(x) => h.name(x).to_string(),
            None => "cell".to_string(),
        };
        let mut plot = Plot::new("plot.profile")
            .x_axis_label(x_label)
            .allow_scroll(false);
        if let Some([lo, hi]) = self.range.map(|r| widened(r).unwrap_or(r)) {
            plot = plot.include_y(lo).include_y(hi);
        }
        if ys.len() == 1 {
            plot = plot.y_axis_label(h.name(ys[0]));
        } else {
            plot = plot.legend(Legend::default());
        }
        plot.show(ui, |plot| {
            scroll_to_zoom(plot);
            for (k, &y) in ys.iter().enumerate() {
                let SignalRef::Field(f) = y else { continue };
                let values = h.fields()[f].frame(n - 1);
                let x_of = |i: usize| match coords {
                    Some(c) if c.len() == values.len() => c[i],
                    _ => i as f64,
                };
                let colour = series_colour(k);
                for run in finite_runs((0..values.len()).map(|i| [x_of(i), values[i]])) {
                    plot.line(Line::new(h.name(y).to_string(), run).color(colour));
                }
            }
        });
    }

    fn selection(&self) -> Option<serde_json::Value> {
        serde_json::to_value(self.cuts.as_ref()?).ok()
    }

    fn set_selection(&mut self, selection: &serde_json::Value) -> Result<(), String> {
        let cuts: Cuts = serde_json::from_value(selection.clone()).map_err(|e| e.to_string())?;
        let finite = cuts
            .cuts
            .iter()
            .all(|c| c.from.iter().chain(&c.to).all(|v| v.is_finite()));
        if !finite {
            return Err("cut ends must be finite".into());
        }
        self.cuts = Some(cuts);
        Ok(())
    }

    fn edit_selection(&mut self, ui: &mut egui::Ui, input: &ViewInput<'_>) {
        let Some(plane) = Self::plane(input) else {
            return;
        };
        let cuts = self.cuts.get_or_insert_with(|| plane.default_cuts());
        ui.label("Cuts").on_hover_text(
            "Straight lines through the grid, each drawn as one curve; ends in the grid's \
             coordinates",
        );
        let name = |a: Against| match a {
            Against::Fraction => "fraction along the cut".to_string(),
            Against::X => plane.label(0),
            Against::Y => plane.label(1),
            Against::Distance => "distance".to_string(),
        };
        let choices = plane.against_choices(&cuts.cuts);
        if !choices.contains(&cuts.against) {
            cuts.against = Against::Fraction;
        }
        ui.horizontal(|ui| {
            ui.label("against");
            egui::ComboBox::from_id_salt("profile_against")
                .selected_text(name(cuts.against))
                .show_ui(ui, |ui| {
                    for a in choices {
                        ui.selectable_value(&mut cuts.against, a, name(a));
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label("between cells");
            ui.selectable_value(&mut cuts.interpolation, Interpolation::Bilinear, "bilinear");
            ui.selectable_value(&mut cuts.interpolation, Interpolation::Nearest, "nearest");
        });
        let mut remove = None;
        for (k, cut) in cuts.cuts.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label("from");
                ui.add(egui::DragValue::new(&mut cut.from[0]).speed(0.01));
                ui.add(egui::DragValue::new(&mut cut.from[1]).speed(0.01));
                ui.label("to");
                ui.add(egui::DragValue::new(&mut cut.to[0]).speed(0.01));
                ui.add(egui::DragValue::new(&mut cut.to[1]).speed(0.01));
                if ui
                    .small_button("×")
                    .on_hover_text("Remove this cut")
                    .clicked()
                {
                    remove = Some(k);
                }
            });
        }
        if let Some(k) = remove {
            cuts.cuts.remove(k);
        }
        if ui.button("Add cut").clicked() {
            let [cx, cy] = &plane.centres;
            let mid = cy[plane.ny / 2];
            cuts.cuts.push(Cut {
                from: [cx[0], mid],
                to: [cx[plane.nx - 1], mid],
            });
        }
    }
}

struct Scatter;

impl View for Scatter {
    fn show(&mut self, ui: &mut egui::Ui, input: &ViewInput<'_>) {
        let h = input.history;
        let Some(SignalRef::Field(field)) = input.one("points") else {
            return;
        };
        let f = &h.fields()[field];
        let FieldShape::Points { dim, .. } = f.shape else {
            return;
        };
        if h.is_empty() || dim < 2 {
            return;
        }
        let pts: Vec<[f64; 2]> = f
            .frame(h.len() - 1)
            .chunks(dim)
            .map(|c| [c[0], c[1]])
            .filter(|p| p.iter().all(|v| v.is_finite()))
            .collect();
        Plot::new("plot.scatter")
            .data_aspect(1.0)
            .allow_scroll(false)
            .show(ui, |plot| {
                scroll_to_zoom(plot);
                plot.points(
                    Points::new(h.name(SignalRef::Field(field)).to_string(), pts)
                        .radius(2.0)
                        .color(crate::colour::LINE),
                );
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn axis(dim: Dim, centres: &[f64]) -> Axis {
        Axis {
            dim,
            coord: None,
            centres: centres.to_vec(),
        }
    }

    #[test]
    fn axes_place_cells_halfway_to_their_neighbours() {
        // Even: one texel per cell.
        assert_eq!(
            axis_map(&[0.5, 1.5, 2.5]),
            Some(([0.0, 3.0], vec![0, 1, 2]))
        );
        // Decreasing coordinates run the other way.
        assert_eq!(
            axis_map(&[3.0, 2.0, 1.0]),
            Some(([0.5, 3.5], vec![2, 1, 0]))
        );
        // Uneven: resampled at the smallest spacing, each texel showing the
        // cell its centre falls in (edges at -0.5, 0.5, 2 and 4).
        assert_eq!(
            axis_map(&[0.0, 1.0, 3.0]),
            Some(([-0.5, 4.0], vec![0, 1, 1, 2, 2]))
        );
        // Not monotonic, or not finite: cannot place cells.
        assert_eq!(axis_map(&[0.0, 2.0, 1.0]), None);
        assert_eq!(axis_map(&[0.0, f64::NAN]), None);
    }

    fn line_history() -> History {
        let shape = FieldShape::Grid { nx: 3, ny: 1 };
        let mut h = History::with_fields("t", vec![], vec![("rod.T".into(), shape)]);
        for k in 0..4 {
            let t = k as f64;
            h.push_sample(t, &[], &[&[t, 10.0 + t, 20.0 + t]]);
        }
        h
    }

    #[test]
    fn a_line_over_time_runs_either_way() {
        let h = line_history();
        let samples = [0, 1, 2, 3];
        let time = axis(Dim::Time, &[0.0, 1.0, 2.0, 3.0]);
        let cells = axis(Dim::Grid(0), &[0.5, 1.5, 2.5]);
        // Time across, cell 0 at the bottom.
        let (size, v, extent) = heat_image(&h, 0, &samples, &time, &cells).unwrap();
        assert_eq!(size, [4, 3]);
        assert_eq!(&v[0..4], &[20.0, 21.0, 22.0, 23.0]);
        assert_eq!(&v[8..12], &[0.0, 1.0, 2.0, 3.0]);
        assert_eq!(extent, [-0.5, 3.5, 0.0, 3.0]);
        // Swapped: time upwards, newest sample on top, cell 0 on the left.
        let (size, v, _) = heat_image(&h, 0, &samples, &cells, &time).unwrap();
        assert_eq!(size, [3, 4]);
        assert_eq!(&v[0..3], &[3.0, 13.0, 23.0]);
        assert_eq!(&v[9..12], &[0.0, 10.0, 20.0]);
    }

    #[test]
    fn grids_over_two_coordinates_can_be_transposed() {
        let shape = FieldShape::Grid { nx: 3, ny: 2 };
        let mut h = History::with_fields("sweep", vec![], vec![("z".into(), shape)]);
        h.push_sample(0.0, &[], &[&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]]);
        // a = 0, 1, 2 along x; b = 10, 20 along y.
        let a = axis(Dim::Grid(0), &[0.0, 1.0, 2.0]);
        let b = axis(Dim::Grid(1), &[10.0, 20.0]);
        let (size, v, extent) = heat_image(&h, 0, &[0], &a, &b).unwrap();
        assert_eq!(size, [3, 2]);
        assert_eq!(v, [4.0, 5.0, 6.0, 1.0, 2.0, 3.0]);
        assert_eq!(extent, [-0.5, 2.5, 5.0, 25.0]);
        let (size, v, extent) = heat_image(&h, 0, &[0], &b, &a).unwrap();
        assert_eq!(size, [2, 3]);
        assert_eq!(v, [3.0, 6.0, 2.0, 5.0, 1.0, 4.0]);
        assert_eq!(extent, [5.0, 25.0, -0.5, 2.5]);
    }

    #[test]
    fn colours_can_follow_the_logarithm() {
        let (img, lo, hi) = colour_image([2, 2], &[3.0, 4.0, 1.0, 2.0], false);
        assert_eq!((lo, hi), (1.0, 4.0));
        assert_eq!(img.pixels[2], sequential(0.0));
        // On a log scale the middle colour sits at the geometric mean, and
        // the range is reported in the values' units.
        let (img, lo, hi) = colour_image([3, 1], &[1.0, 10.0, 100.0], true);
        assert!((lo - 1.0).abs() < 1e-12 && (hi - 100.0).abs() < 1e-9);
        assert_eq!(img.pixels[1], sequential(0.5));
        let (img, _, _) = colour_image([2, 1], &[-1.0, 1.0], true);
        assert_eq!(img.pixels[0], egui::Color32::TRANSPARENT);
    }

    fn plane(cx: &[f64], cy: &[f64], named: bool) -> Plane {
        Plane {
            nx: cx.len(),
            ny: cy.len(),
            centres: [cx.to_vec(), cy.to_vec()],
            names: if named {
                [Some("p.a".into()), Some("p.b".into())]
            } else {
                [None, None]
            },
        }
    }

    #[test]
    fn positions_map_to_fractional_cells() {
        assert_eq!(index_of(&[0.0, 1.0, 3.0], 2.0), Some(1.5));
        assert_eq!(index_of(&[3.0, 1.0, 0.0], 2.0), Some(0.5));
        assert_eq!(index_of(&[0.0, 1.0, 3.0], 3.5), None);
        assert_eq!(index_of(&[0.0, 1.0], f64::NAN), None);
    }

    #[test]
    fn bilinear_cuts_reproduce_a_linear_field_exactly() {
        // z = 2x + 3y on unevenly spaced coordinates; a diagonal cut.
        let (cx, cy) = ([0.0, 1.0, 3.0, 3.5], [10.0, 20.0, 25.0]);
        let p = plane(&cx, &cy, true);
        let frame: Vec<f64> = cy
            .iter()
            .flat_map(|y| cx.iter().map(move |x| 2.0 * x + 3.0 * y))
            .collect();
        let cut = Cut {
            from: [0.0, 10.0],
            to: [3.5, 25.0],
        };
        for against in [Against::Fraction, Against::X, Against::Y] {
            let cuts = Cuts {
                cuts: vec![cut],
                against,
                interpolation: Interpolation::Bilinear,
            };
            let curve = cut_curve(&p, &frame, &cut, &cuts);
            assert!(curve.len() >= 4);
            for (s, [h, z]) in curve.iter().enumerate() {
                let f = s as f64 / (curve.len() - 1) as f64;
                let (x, y) = (3.5 * f, 10.0 + 15.0 * f);
                assert!((z - (2.0 * x + 3.0 * y)).abs() < 1e-12, "{z} at ({x}, {y})");
                let expected = match against {
                    Against::Fraction => f,
                    Against::X => x,
                    _ => y,
                };
                assert!((h - expected).abs() < 1e-12);
            }
        }
        // Nearest takes the closest cell instead.
        let nearest = Cuts {
            cuts: vec![cut],
            against: Against::Fraction,
            interpolation: Interpolation::Nearest,
        };
        let curve = cut_curve(
            &p,
            &frame,
            &Cut {
                from: [0.9, 12.0],
                to: [0.9, 12.0],
            },
            &nearest,
        );
        assert_eq!(curve[0][1], 2.0 * 1.0 + 3.0 * 10.0);
        // Outside the grid there is no value.
        let outside = Cut {
            from: [5.0, 10.0],
            to: [6.0, 10.0],
        };
        assert!(cut_curve(&p, &frame, &outside, &nearest)
            .iter()
            .all(|p| p[1].is_nan()));
    }

    #[test]
    fn default_cuts_read_rows_exactly_against_x() {
        let p = plane(
            &[0.0, 1.0, 2.0],
            &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0],
            true,
        );
        let frame: Vec<f64> = (0..24).map(|k| (k * k) as f64).collect();
        let cuts = p.default_cuts();
        assert_eq!(cuts.against, Against::X);
        assert_eq!(cuts.cuts.len(), 4);
        for cut in &cuts.cuts {
            let row = cut.from[1] as usize;
            let curve = cut_curve(&p, &frame, cut, &cuts);
            let ends = [curve[0], curve[curve.len() - 1]];
            assert_eq!(ends, [[0.0, frame[row * 3]], [2.0, frame[row * 3 + 2]]]);
        }
        // What cuts can be plotted against.
        let slanted = [Cut {
            from: [0.0, 0.0],
            to: [0.0, 5.0],
        }];
        assert_eq!(p.against_choices(&slanted), [Against::Fraction, Against::Y]);
        let cells = plane(&[0.5, 1.5], &[0.5, 1.5], false);
        assert!(cells.against_choices(&slanted).contains(&Against::Distance));
    }

    #[test]
    fn cuts_are_saved_as_json_and_checked() {
        let mut v = Profile::default();
        assert!(v.selection().is_none());
        let json = serde_json::json!({
            "cuts": [{"from": [0.0, 0.1], "to": [3.0, 0.8]}],
            "against": "fraction",
            "interpolation": "bilinear"
        });
        v.set_selection(&json).unwrap();
        assert_eq!(v.selection(), Some(json));
        assert!(v
            .set_selection(&serde_json::json!({"cuts": [], "against": "sideways"}))
            .is_err());
        assert!(v
            .set_selection(&serde_json::json!({"cuts": [], "extra": 1}))
            .is_err());
    }
}
