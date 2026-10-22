//! Views of fields (ADR 0016, ADR 0024): `plot.heatmap` (a grid as colours
//! over two axes), `plot.profile` (line fields against a coordinate) and
//! `plot.scatter` (a point set).

use crate::builtin::{bindings, finite_runs, scroll_to_zoom, series_colour, widened};
use crate::colour::{sequential, trail_start};
use crate::history::{FieldShape, History, SignalRef, TIME};
use crate::view::{
    always, Available, Bindings, Context, Preset, Slot, SlotKind, StyleProperty, Swap, View,
    ViewInput, ViewRegistry, ViewType,
};
use egui_plot::{Legend, Line, Plot, PlotImage, PlotPoint, Points};

/// At most this many samples of a line field over time are drawn.
pub(crate) const MAX_COLUMNS: usize = 512;
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

pub(crate) const HEAT_TRAIL: StyleProperty = StyleProperty {
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

/// The grid field a heat map, surface or contour view shows.
pub(crate) const FIELD: Slot = Slot {
    name: "field",
    kind: SlotKind::Grid,
    of: None,
    applies: always,
    many: false,
    required: true,
    description: "The grid field shown",
};

/// The field's direction across the view.
pub(crate) const X_AXIS: Slot = Slot {
    name: "x",
    kind: SlotKind::Axis,
    of: Some("field"),
    applies: always,
    many: false,
    required: false,
    description: "Across: t, or a coordinate of the field such as rod.position or a \
                  swept parameter; without it, the field's next direction by cell number",
};

/// The field's other direction.
pub(crate) const Y_AXIS: Slot = Slot {
    name: "y",
    kind: SlotKind::Axis,
    of: Some("field"),
    applies: always,
    many: false,
    required: false,
    description: "The other direction (up in a heat map, into the picture in a surface): t, \
                  or a coordinate of the field; without it, the field's remaining direction \
                  by cell number",
};

/// The first result grid over its coordinates (or a line of cells over
/// time): the starting bindings of every view of a grid field.
pub(crate) fn grid_bindings(a: &Available<'_>) -> Option<Bindings> {
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
}

/// Points drawn over a heat map, such as animals on the grass they graze.
const OVERLAY: Slot = Slot {
    name: "points",
    kind: SlotKind::Points,
    of: None,
    applies: always,
    many: false,
    required: false,
    description: "A point set or population drawn over the map, newest frame, its first two \
                  values as positions in the map's coordinates",
};

pub fn register(r: &mut ViewRegistry) -> Result<(), String> {
    r.register(ViewType {
        name: "plot.heatmap",
        label: "Heat map",
        description:
            "A grid field as colours over two axes: a line of cells over time, or a 2D grid now",
        slots: &[FIELD, X_AXIS, Y_AXIS, OVERLAY],
        style: &[HEAT_TRAIL, LOG_COLOUR],
        applies_to: &[],
        swap: Some(Swap::Slots(["x", "y"])),
        presets: &[Preset {
            label: "Heat map",
            description: "A grid field as colours",
            bindings: grid_bindings,
        }],
        create: || Box::new(Heatmap::default()),
    })?;
    r.register(ViewType {
        name: "plot.profile",
        label: "Profile",
        description: "Line fields as they are now, against position: the curve moves as the run goes on",
        slots: &[
            Slot {
                name: "x",
                kind: SlotKind::Coordinate,
                of: Some("y"),
                applies: always,
                many: false,
                required: false,
                description:
                    "Where each cell lies, e.g. rod.position; without it cells are numbered",
            },
            Slot {
                name: "y",
                kind: SlotKind::Line,
                of: None,
                applies: always,
                many: true,
                required: true,
                description: "The line fields drawn, one curve each (cuts through 2D grids come from cut filters)",
            },
        ],
        style: &[],
        applies_to: &[],
        swap: None,
        presets: &[Preset {
            label: "Profile",
            description: "Line fields now, against their coordinate",
            bindings: |a| {
                // The first line field, with every other line placed by the
                // same coordinate (e.g. all the cuts of one cut filter).
                let y = first_result(a, true)?;
                let x = coordinate(a, y, 0);
                let ys: Vec<String> = a
                    .lines
                    .iter()
                    .filter(|l| *l == y || (x.is_some() && coordinate(a, l, 0) == x))
                    .cloned()
                    .collect();
                let mut b: Bindings = [("y".to_string(), ys)].into();
                if let Some(x) = x {
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
    let (ex, mx) = axis_map(&x.centres)?;
    let (ey, my) = axis_map(&y.centres)?;
    let value = |a: usize, b: usize| cell_value(h, field, nx, samples, x, y, a, b);
    let (w, hgt) = (mx.len(), my.len());
    let mut values = Vec::with_capacity(w * hgt);
    for ty in (0..hgt).rev() {
        for &cx in &mx {
            values.push(value(cx, my[ty]));
        }
    }
    Some(([w, hgt], values, [ex[0], ex[1], ey[0], ey[1]]))
}

/// The value a pair of positions along screen axes `x` and `y` shows: a
/// sample (along a time axis, else the newest) and a cell.
#[allow(clippy::too_many_arguments)]
fn cell_value(
    h: &History,
    field: usize,
    nx: usize,
    samples: &[usize],
    x: &Axis,
    y: &Axis,
    a: usize,
    b: usize,
) -> f64 {
    let (mut k, mut i, mut j) = (h.len() - 1, 0, 0);
    for (dim, pos) in [(x.dim, a), (y.dim, b)] {
        match dim {
            Dim::Time => k = samples[pos],
            Dim::Grid(0) => i = pos,
            Dim::Grid(_) => j = pos,
        }
    }
    h.fields()[field]
        .frame(k)
        .get(j * nx + i)
        .copied()
        .unwrap_or(f64::NAN)
}

/// A grid field laid over two screen axes, as a heat map shows it, for
/// views that draw it otherwise (surfaces, contours): positions along each
/// axis in data units, the values `z[j * x.len() + i]`, and labels for x,
/// y and the values.
pub(crate) struct Sheet {
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub z: Vec<f64>,
    pub labels: [String; 3],
}

/// The sheet of the bound field: at most `max_samples` samples along a
/// time axis, within the trail length.
pub(crate) fn sheet(input: &ViewInput<'_>, max_samples: usize) -> Option<Sheet> {
    let h = input.history;
    let Some(SignalRef::Field(field)) = input.one("field") else {
        return None;
    };
    let FieldShape::Grid { nx, .. } = h.fields()[field].shape else {
        return None;
    };
    if h.is_empty() {
        return None;
    }
    let first = trail_start(h.times(), input.style("trail_length")).min(h.len() - 1);
    let stride = (h.len() - first).div_ceil(max_samples).max(1);
    let samples: Vec<usize> = (first..h.len()).step_by(stride).collect();
    let (x, y) = heat_axes(input, field, &samples)?;
    let mut z = Vec::with_capacity(x.centres.len() * y.centres.len());
    for b in 0..y.centres.len() {
        for a in 0..x.centres.len() {
            z.push(cell_value(h, field, nx, &samples, &x, &y, a, b));
        }
    }
    let labels = [
        axis_label(h, field, &x),
        axis_label(h, field, &y),
        h.label(SignalRef::Field(field)).to_string(),
    ];
    Some(Sheet {
        x: x.centres,
        y: y.centres,
        z,
        labels,
    })
}

/// The name of a screen axis: time, the coordinate's label, or cells.
fn axis_label(h: &History, field: usize, a: &Axis) -> String {
    let line = matches!(h.fields()[field].shape, FieldShape::Grid { ny: 1, .. });
    match (a.dim, a.coord) {
        (Dim::Time, _) if h.over_time() => "t [s]".to_string(),
        (Dim::Time, _) => h.index().to_string(),
        (_, Some(c)) => h.label(SignalRef::Field(c)).to_string(),
        (Dim::Grid(_), None) if line => "cell".to_string(),
        (Dim::Grid(0), None) => "column".to_string(),
        (Dim::Grid(_), None) => "row".to_string(),
    }
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
            h.label(SignalRef::Field(field)),
            fmt(lo),
            fmt(hi),
            if log { ", log scale" } else { "" }
        ));
        Plot::new("plot.heatmap")
            .x_axis_label(axis_label(h, field, &x))
            .y_axis_label(axis_label(h, field, &y))
            .allow_scroll(false)
            .show(ui, |plot| {
                scroll_to_zoom(plot);
                plot.image(PlotImage::new(
                    "field",
                    texture.id(),
                    PlotPoint::new((x0 + x1) / 2.0, (y0 + y1) / 2.0),
                    egui::vec2((x1 - x0) as f32, (y1 - y0) as f32),
                ));
                if let Some(pts) = overlay(input, &x, &y) {
                    plot.points(
                        Points::new("points", pts)
                            .radius(2.5)
                            .color(crate::colour::HEAD),
                    );
                }
            });
    }
}

/// The bound point set's newest points in the map's coordinates: only when
/// both axes run along the grid (not time), swapped if the map is.
fn overlay(input: &ViewInput<'_>, x: &Axis, y: &Axis) -> Option<Vec<[f64; 2]>> {
    let h = input.history;
    let Some(SignalRef::Field(f)) = input.one("points") else {
        return None;
    };
    let field = &h.fields()[f];
    let dim = field.shape.point_dim().filter(|&d| d >= 2)?;
    let swapped = match (x.dim, y.dim) {
        (Dim::Grid(0), Dim::Grid(_)) => false,
        (Dim::Grid(_), Dim::Grid(0)) => true,
        _ => return None,
    };
    let pts = field
        .frame(h.len().checked_sub(1)?)
        .chunks_exact(dim)
        .map(|p| if swapped { [p[1], p[0]] } else { [p[0], p[1]] })
        .filter(|p| p.iter().all(|v| v.is_finite()))
        .collect();
    Some(pts)
}

pub(crate) fn fmt(v: f64) -> String {
    if v.is_finite() {
        format!("{v:.4}")
    } else {
        "–".into()
    }
}

/// Line fields as they are now, against their coordinate (ADR 0030: cuts
/// through 2D grids come from cut filters, ADR 0031). The value axis spans
/// every value seen so far, so the curve moves instead of the axis.
#[derive(Default)]
struct Profile {
    /// Samples already folded into `range`.
    seen: usize,
    range: Option<[f64; 2]>,
    /// The bindings `range` was computed for.
    key: Vec<SignalRef>,
}

impl View for Profile {
    fn show(&mut self, ui: &mut egui::Ui, input: &ViewInput<'_>) {
        let h = input.history;
        let ys = input.many("y");
        let n = h.len();
        if n == 0 || ys.is_empty() {
            return;
        }
        // Rewind, restart or new bindings: start the range again.
        if n < self.seen || self.key != ys {
            *self = Self {
                key: ys.to_vec(),
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
            Some(x) => h.label(x).to_string(),
            None => "cell".to_string(),
        };
        let mut plot = Plot::new("plot.profile")
            .x_axis_label(x_label)
            .allow_scroll(false);
        if let Some([lo, hi]) = self.range.map(|r| widened(r).unwrap_or(r)) {
            plot = plot.include_y(lo).include_y(hi);
        }
        if ys.len() == 1 {
            plot = plot.y_axis_label(h.label(ys[0]));
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
                    plot.line(Line::new(h.label(y).to_string(), run).color(colour));
                }
            }
        });
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
        let Some(dim) = f.shape.point_dim() else {
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
}
