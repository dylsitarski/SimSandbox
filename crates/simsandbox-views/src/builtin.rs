//! The built-in views: `plot.xy` (ADR 0018), `plot.3d`, `plot.heatmap` and
//! `plot.scatter`.

use crate::colour::{segments, sequential, trail_start, ColourKey, HEAD};
use crate::history::{FieldShape, History, SignalRef, TIME};
use crate::view::{
    Available, Bindings, Preset, Slot, SlotKind, StyleProperty, View, ViewInput, ViewRegistry,
    ViewType,
};
use crate::view3d::{self, Camera};
use egui_plot::{Legend, Line, MarkerShape, Plot, PlotImage, PlotPoint, PlotUi, Points};

/// Segments a single coloured trail is split into.
const COLOUR_CHUNKS: usize = 48;

const COLOUR: Slot = Slot {
    name: "colour",
    kind: SlotKind::Scalar,
    many: false,
    required: false,
    description:
        "Colours a single curve: 't' fades it from old to new; another signal colours it by value",
};

const TRAIL_LENGTH: StyleProperty = StyleProperty {
    name: "trail_length",
    default: -1.0,
    suggested: [-1.0, 60.0],
    unit: "s",
    description: "Seconds of trail shown behind the current point; 0 shows no trail, -1 (any negative value) the whole run. With time on the x axis this is a scrolling window",
};

const TIME_AXIS: StyleProperty = StyleProperty {
    name: "time_axis",
    default: 0.0,
    suggested: [0.0, 1.0],
    unit: "",
    description: "For a line of cells over time: 0 runs time left to right, 1 runs it upwards",
};

/// A coordinate field for `field`: a grid output named `position` on the
/// same block (e.g. `rod.position` for `rod.temperature`).
fn position_of<'a>(a: &'a Available<'_>, field: &str) -> Option<&'a String> {
    let (block, _) = field.split_once('.')?;
    let name = format!("{block}.position");
    a.grids.iter().find(|g| **g == name && *g != field)
}

/// The newest frame of one or more line fields against a coordinate field.
const PROFILE_PRESET: Preset = Preset {
    label: "Profile",
    description: "A line field now, against position",
    bindings: |a| {
        let y = a.grids.first()?;
        let mut b = bindings(&[("y", y)]);
        if let Some(x) = position_of(a, y) {
            b.insert("x".into(), vec![x.clone()]);
        }
        Some(b)
    },
};

pub fn register(r: &mut ViewRegistry) -> Result<(), String> {
    r.register(ViewType {
        name: "plot.xy",
        label: "Plot",
        description: "Signals against time or against each other",
        slots: &[
            Slot {
                name: "x",
                kind: SlotKind::Scalar,
                many: false,
                required: true,
                description: "Horizontal axis: 't' for time, or any signal",
            },
            Slot {
                name: "y",
                kind: SlotKind::Scalar,
                many: true,
                required: true,
                description: "Vertical axis: one or more signals",
            },
            COLOUR,
        ],
        style: &[TRAIL_LENGTH],
        applies_to: &[],
        presets: &[
            Preset {
                label: "Plot against time",
                description: "Every signal against time",
                bindings: |a| {
                    let s = a.scalars;
                    (!s.is_empty()).then(|| {
                        [
                            ("x".to_string(), vec![TIME.to_string()]),
                            ("y".to_string(), s.to_vec()),
                        ]
                        .into()
                    })
                },
            },
            Preset {
                label: "Phase portrait",
                description: "One signal against another, fading with time",
                bindings: |a| {
                    let s = a.scalars;
                    // x–z for three or more signals (the Lorenz butterfly).
                    let y = if s.len() >= 3 { 2 } else { 1 };
                    (s.len() >= 2)
                        .then(|| bindings(&[("x", &s[0]), ("y", &s[y]), ("colour", TIME)]))
                },
            },
        ],
        create: || Box::new(Xy),
    })?;
    r.register(ViewType {
        name: "plot.3d",
        label: "3D trajectory",
        description: "Three signals as a path in space",
        slots: &[
            Slot {
                name: "x",
                kind: SlotKind::Scalar,
                many: false,
                required: true,
                description: "First axis",
            },
            Slot {
                name: "y",
                kind: SlotKind::Scalar,
                many: false,
                required: true,
                description: "Second axis",
            },
            Slot {
                name: "z",
                kind: SlotKind::Scalar,
                many: false,
                required: true,
                description: "Third axis, up",
            },
            COLOUR,
        ],
        style: &[TRAIL_LENGTH],
        applies_to: &[],
        presets: &[Preset {
            label: "3D trajectory",
            description: "Three signals as a path in space",
            bindings: |a| {
                let s = a.scalars;
                (s.len() >= 3).then(|| {
                    bindings(&[("x", &s[0]), ("y", &s[1]), ("z", &s[2]), ("colour", TIME)])
                })
            },
        }],
        create: || Box::new(Trajectory::default()),
    })?;
    r.register(ViewType {
        name: "plot.heatmap",
        label: "Heat map",
        description: "A grid field as colours: a line of cells over time, or a 2D grid now",
        slots: &[
            Slot {
                name: "field",
                kind: SlotKind::Grid,
                many: false,
                required: true,
                description: "The grid field shown",
            },
            Slot {
                name: "position",
                kind: SlotKind::Grid,
                many: false,
                required: false,
                description:
                    "For a line of cells: where each cell lies (evenly spaced, increasing), \
                              e.g. rod.position; without it cells are numbered",
            },
        ],
        style: &[TRAIL_LENGTH, TIME_AXIS],
        applies_to: &[],
        presets: &[Preset {
            label: "Heat map",
            description: "A grid field as colours",
            bindings: |a| {
                let field = a.grids.first()?;
                let mut b = bindings(&[("field", field)]);
                if let Some(p) = position_of(a, field) {
                    b.insert("position".into(), vec![p.clone()]);
                }
                Some(b)
            },
        }],
        create: || Box::new(Heatmap::default()),
    })?;
    r.register(ViewType {
        name: "plot.profile",
        label: "Profile",
        description:
            "Line fields as they are now, against position: the curve moves as the run goes on",
        slots: &[
            Slot {
                name: "x",
                kind: SlotKind::Grid,
                many: false,
                required: false,
                description:
                    "Where each cell lies, e.g. rod.position; without it cells are numbered",
            },
            Slot {
                name: "y",
                kind: SlotKind::Grid,
                many: true,
                required: true,
                description: "The fields drawn, one curve each",
            },
        ],
        style: &[],
        applies_to: &[],
        presets: &[PROFILE_PRESET],
        create: || Box::new(Profile::default()),
    })?;
    r.register(ViewType {
        name: "plot.scatter",
        label: "Points",
        description: "A set of points where they are now",
        slots: &[Slot {
            name: "points",
            kind: SlotKind::Points,
            many: false,
            required: true,
            description: "The point set shown (first two coordinates)",
        }],
        style: &[],
        applies_to: &[],
        presets: &[Preset {
            label: "Points",
            description: "A set of points where they are now",
            bindings: |a| a.points.first().map(|p| bindings(&[("points", p)])),
        }],
        create: || Box::new(Scatter),
    })
}

fn bindings(pairs: &[(&str, &str)]) -> Bindings {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), vec![v.to_string()]))
        .collect()
}

/// How the colour slot colours a line.
fn colour_key(input: &ViewInput<'_>) -> ColourKey {
    match input.one("colour") {
        None => ColourKey::Constant,
        Some(SignalRef::Time) => ColourKey::Time(input.history.times().to_vec()),
        Some(s) => ColourKey::Value(input.history.values(s)),
    }
}

/// The scroll wheel zooms around the pointer instead of panning; drag pans
/// and double-click resets, as egui_plot does by default.
pub fn scroll_to_zoom(plot: &mut PlotUi<'_>) {
    if !plot.response().hovered() {
        return;
    }
    let dy = plot.ctx().input(|i| i.smooth_scroll_delta.y);
    if dy != 0.0 {
        plot.zoom_bounds_around_hovered(egui::Vec2::splat(zoom_factor(dy)));
    }
}

/// Zoom per scrolled point; scrolling up (positive) zooms in.
pub fn zoom_factor(scroll: f32) -> f32 {
    (scroll * 0.002).exp()
}

/// Plots span at least this fraction of the largest magnitude shown, so a
/// signal constant to rounding error draws as a flat line instead of being
/// magnified until its noise looks like drift. Zooming in still works.
pub const MIN_RELATIVE_SPAN: f64 = 1e-6;

/// Smallest and largest finite value, if any.
pub fn value_range(values: impl Iterator<Item = f64>) -> Option<[f64; 2]> {
    values.filter(|v| v.is_finite()).fold(None, |r, v| match r {
        None => Some([v, v]),
        Some([lo, hi]) => Some([lo.min(v), hi.max(v)]),
    })
}

/// A range widened about its middle to the minimum span, or `None` if it is
/// already wide enough (or zero, which plots handle themselves).
pub fn widened([lo, hi]: [f64; 2]) -> Option<[f64; 2]> {
    let span = MIN_RELATIVE_SPAN * lo.abs().max(hi.abs());
    if hi - lo >= span || span == 0.0 {
        return None;
    }
    let mid = 0.5 * (lo + hi);
    Some([mid - 0.5 * span, mid + 0.5 * span])
}

/// Split points into runs of finite points: plots cannot draw infinities
/// or NaN (egui panics on them in debug builds), so a curve that passes
/// through one is drawn with a gap there.
pub fn finite_runs(points: impl Iterator<Item = [f64; 2]>) -> Vec<Vec<[f64; 2]>> {
    let mut runs = vec![Vec::new()];
    for p in points {
        if p.iter().all(|v| v.is_finite()) {
            runs.last_mut().expect("never empty").push(p);
        } else if !runs.last().expect("never empty").is_empty() {
            runs.push(Vec::new());
        }
    }
    runs.retain(|r| !r.is_empty());
    runs
}

/// The colour of the `k`th curve in a plot, as egui_plot would pick it, so
/// a curve split into pieces keeps one colour.
fn series_colour(k: usize) -> egui::Color32 {
    // egui_plot's automatic colours: golden-ratio hues.
    let golden_ratio = (5.0_f32.sqrt() - 1.0) / 2.0;
    let h = k as f32 * golden_ratio;
    egui::epaint::Hsva::new(h, 0.85, 0.5, 1.0).into()
}

/// Any signals against any signal (ADR 0018).
struct Xy;

impl View for Xy {
    fn show(&mut self, ui: &mut egui::Ui, input: &ViewInput<'_>) {
        let h = input.history;
        let Some(x) = input.one("x") else {
            return;
        };
        let ys = input.many("y");
        let n = h.len();
        let first = trail_start(h.times(), input.style("trail_length"));
        let against_time = x == SignalRef::Time;
        let x_label = if against_time && h.index() == TIME {
            "t [s]".to_string()
        } else {
            h.name(x).to_string()
        };
        let mut plot = Plot::new("plot.xy")
            .x_axis_label(x_label)
            .allow_scroll(false);
        // Keep near-constant signals (e.g. a conserved energy) from being
        // magnified until rounding noise looks like drift.
        let range = |s: SignalRef| value_range((first..n).map(|k| h.value(s, k)));
        if let Some([lo, hi]) = range(x).and_then(widened) {
            plot = plot.include_x(lo).include_x(hi);
        }
        let y_range = ys
            .iter()
            .filter_map(|&y| range(y))
            .reduce(|a, b| [a[0].min(b[0]), a[1].max(b[1])]);
        if let Some([lo, hi]) = y_range.and_then(widened) {
            plot = plot.include_y(lo).include_y(hi);
        }
        if ys.len() == 1 {
            plot = plot.y_axis_label(h.name(ys[0]));
        } else {
            plot = plot.legend(Legend::default());
        }
        plot.show(ui, |plot| {
            scroll_to_zoom(plot);
            let points = |y: SignalRef, from: usize, to: usize| {
                finite_runs((from..to).map(|k| [h.value(x, k), h.value(y, k)]))
            };
            if let ([y], Some(_)) = (ys, input.one("colour")) {
                // One curve with a colour encoding: coloured segments.
                for seg in segments(first, n, &colour_key(input), COLOUR_CHUNKS) {
                    for run in points(*y, seg.start, seg.end + 1) {
                        plot.line(Line::new("trail", run).color(seg.colour).allow_hover(false));
                    }
                }
            } else {
                for (k, &y) in ys.iter().enumerate() {
                    let name = h.name(y).to_string();
                    // Pieces of one curve (split where it is not finite)
                    // share its name and colour.
                    let colour = series_colour(k);
                    match y {
                        // The whole run against time: plot the recorded points
                        // directly when they are all finite.
                        SignalRef::Signal(i)
                            if against_time
                                && first == 0
                                && h.points(i).iter().all(|p| p.y.is_finite()) =>
                        {
                            plot.line(Line::new(name, h.points(i)).color(colour));
                        }
                        _ if n > first + 1 => {
                            for run in points(y, first, n) {
                                plot.line(Line::new(name.clone(), run).color(colour));
                            }
                        }
                        _ => {}
                    }
                }
            }
            // Mark the current point unless time runs along x (it is the right edge).
            if !against_time && n > 0 {
                for &y in ys {
                    let now = [h.value(x, n - 1), h.value(y, n - 1)];
                    if now.iter().all(|v| v.is_finite()) {
                        plot.points(
                            Points::new("now", vec![now])
                                .shape(MarkerShape::Circle)
                                .radius(4.0)
                                .color(HEAD),
                        );
                    }
                }
            }
        });
    }
}

#[derive(Default)]
struct Trajectory {
    camera: Camera,
}

impl View for Trajectory {
    fn show(&mut self, ui: &mut egui::Ui, input: &ViewInput<'_>) {
        let h: &History = input.history;
        let (Some(x), Some(y), Some(z)) = (input.one("x"), input.one("y"), input.one("z")) else {
            return;
        };
        let points: Vec<[f64; 3]> = (0..h.len())
            .map(|k| [h.value(x, k), h.value(y, k), h.value(z, k)])
            .collect();
        let first = trail_start(h.times(), input.style("trail_length"));
        let segs = segments(first, h.len(), &colour_key(input), COLOUR_CHUNKS);
        view3d::show(
            ui,
            &mut self.camera,
            &points,
            &segs,
            [x, y, z].map(|s| h.name(s)),
        );
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

/// At most this many columns of a space-time heat map are drawn.
const MAX_COLUMNS: usize = 512;

/// Colour each value on the sequential scale between the smallest and
/// largest finite value; non-finite values are transparent.
fn colour_image(size: [usize; 2], values: &[f64]) -> (egui::ColorImage, f64, f64) {
    let (lo, hi) = values
        .iter()
        .filter(|v| v.is_finite())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &v| {
            (a.min(v), b.max(v))
        });
    let span = (hi - lo).max(1e-300);
    let pixels = values
        .iter()
        .map(|&v| {
            if v.is_finite() {
                sequential(((v - lo) / span) as f32)
            } else {
                egui::Color32::TRANSPARENT
            }
        })
        .collect();
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

/// Pixels and extent of a heat map.
type HeatmapImage = ([usize; 2], Vec<f64>, [f64; 4]);

/// Where the cells of a line lie along it: from the newest frame of a
/// coordinate field with one value per cell, evenly spaced and increasing,
/// as `[start, end]` of the whole line (half a cell beyond the first and
/// last cell centres). `None` if the field does not fit.
fn line_extent(h: &History, position: usize, nx: usize) -> Option<[f64; 2]> {
    let p = h.fields().get(position)?;
    if p.shape.len() != nx || nx < 2 || h.is_empty() {
        return None;
    }
    let frame = p.frame(h.len() - 1);
    let (a, b) = (frame[0], frame[nx - 1]);
    let half = 0.5 * (b - a) / (nx - 1) as f64;
    (a.is_finite() && b.is_finite() && b > a).then_some([a - half, b + half])
}

/// The values of a heat map image, row by row from the top, and its extent
/// in plot coordinates `[x0, x1, y0, y1]`. A line of cells (`ny` = 1) is
/// shown over time: by default time runs left to right and cell 0 is at the
/// bottom; with `time_vertical` time runs upwards and cell 0 is on the left.
/// `along` places the line's cells (see [`line_extent`]); without it they
/// are numbered 0 to `nx`. A 2D grid is shown at the newest sample, row 0
/// at the bottom.
fn heatmap_pixels(
    h: &History,
    field: usize,
    first: usize,
    along: Option<[f64; 2]>,
    time_vertical: bool,
) -> Option<HeatmapImage> {
    let f = &h.fields()[field];
    let n = h.len();
    let FieldShape::Grid { nx, ny } = f.shape else {
        return None;
    };
    if n == 0 || nx == 0 || ny == 0 {
        return None;
    }
    if ny == 1 {
        let first = first.min(n - 1);
        let stride = (n - first).div_ceil(MAX_COLUMNS).max(1);
        let samples: Vec<usize> = (first..n).step_by(stride).collect();
        let cols = samples.len();
        let mut values = vec![0.0; cols * nx];
        for (c, &k) in samples.iter().enumerate() {
            for (cell, &v) in f.frame(k).iter().enumerate() {
                if time_vertical {
                    // Newest sample in the top row.
                    values[(cols - 1 - c) * nx + cell] = v;
                } else {
                    values[(nx - 1 - cell) * cols + c] = v;
                }
            }
        }
        let (t0, t1) = (h.times()[first], h.times()[n - 1]);
        let t1 = if t1 > t0 { t1 } else { t0 + 1.0 };
        let [s0, s1] = along.unwrap_or([0.0, nx as f64]);
        if time_vertical {
            Some(([nx, cols], values, [s0, s1, t0, t1]))
        } else {
            Some(([cols, nx], values, [t0, t1, s0, s1]))
        }
    } else {
        let frame = f.frame(n - 1);
        let mut values = vec![0.0; nx * ny];
        for j in 0..ny {
            for i in 0..nx {
                values[(ny - 1 - j) * nx + i] = frame[j * nx + i];
            }
        }
        Some(([nx, ny], values, [0.0, nx as f64, 0.0, ny as f64]))
    }
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
        let first = trail_start(h.times(), input.style("trail_length"));
        let FieldShape::Grid { nx, ny } = h.fields()[field].shape else {
            return;
        };
        let position = match input.one("position") {
            Some(SignalRef::Field(p)) => Some(p),
            _ => None,
        };
        let along = position.and_then(|p| line_extent(h, p, nx));
        let time_vertical = input.style("time_axis") != 0.0;
        let Some((size, values, [x0, x1, y0, y1])) =
            heatmap_pixels(h, field, first, along, time_vertical)
        else {
            return;
        };
        let (image, lo, hi) = colour_image(size, &values);
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
        let over_time = ny == 1;
        ui.weak(format!(
            "{}: {} (dark) to {} (bright)",
            h.name(SignalRef::Field(field)),
            fmt(lo),
            fmt(hi)
        ));
        let time = if h.index() == TIME {
            "t [s]".to_string()
        } else {
            h.index().to_string()
        };
        let space = match (along, position) {
            (Some(_), Some(p)) => h.name(SignalRef::Field(p)).to_string(),
            _ => "cell".to_string(),
        };
        let (x_label, y_label) = match (over_time, time_vertical) {
            (true, false) => (time, space),
            (true, true) => (space, time),
            (false, _) => ("x (cells)".to_string(), "y (cells)".to_string()),
        };
        Plot::new("plot.heatmap")
            .x_axis_label(x_label)
            .y_axis_label(y_label)
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

/// Newest frames of line fields against a coordinate field. The value axis
/// spans every value seen so far, so the curve moves instead of the axis.
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

    #[test]
    fn line_fields_show_over_time_with_cell_zero_at_the_bottom() {
        let shape = FieldShape::Grid { nx: 3, ny: 1 };
        let mut h = History::with_fields("t", vec![], vec![("rod.T".into(), shape)]);
        for k in 0..4 {
            let t = k as f64;
            h.push_sample(t, &[], &[&[t, 10.0 + t, 20.0 + t]]);
        }
        let (size, v, extent) = heatmap_pixels(&h, 0, 0, None, false).unwrap();
        assert_eq!(size, [4, 3]);
        // Top row is the last cell; columns are samples in time order.
        assert_eq!(&v[0..4], &[20.0, 21.0, 22.0, 23.0]);
        assert_eq!(&v[8..12], &[0.0, 1.0, 2.0, 3.0]);
        assert_eq!(extent, [0.0, 3.0, 0.0, 3.0]);
    }

    #[test]
    fn line_fields_can_run_time_upwards_and_be_placed_by_position() {
        let line = FieldShape::Grid { nx: 3, ny: 1 };
        let mut h = History::with_fields(
            "t",
            vec![],
            vec![("rod.T".into(), line), ("rod.position".into(), line)],
        );
        for k in 0..2 {
            let t = k as f64;
            h.push_sample(t, &[], &[&[t, 10.0 + t, 20.0 + t], &[0.25, 0.5, 0.75]]);
        }
        let along = line_extent(&h, 1, 3);
        assert_eq!(along, Some([0.125, 0.875]));
        let (size, v, extent) = heatmap_pixels(&h, 0, 0, along, true).unwrap();
        assert_eq!(size, [3, 2]);
        // Newest sample on top; cell 0 on the left.
        assert_eq!(v, [1.0, 11.0, 21.0, 0.0, 10.0, 20.0]);
        assert_eq!(extent, [0.125, 0.875, 0.0, 1.0]);
        // A coordinate that is not increasing or not one per cell is ignored.
        assert_eq!(line_extent(&h, 0, 2), None);
    }

    #[test]
    fn near_constant_ranges_are_widened_to_the_minimum_span() {
        // A conserved energy wobbling by rounding error.
        let r = value_range([0.75, 0.75 - 4e-14, f64::NAN, 0.75 + 1e-15].into_iter()).unwrap();
        let [lo, hi] = widened(r).unwrap();
        assert!((hi - lo - 0.75e-6).abs() < 1e-15, "{}", hi - lo);
        assert!(lo < r[0] && hi > r[1]);
        // Real variation is left alone, and so is an all-zero signal.
        assert_eq!(widened([0.5, 1.0]), None);
        assert_eq!(widened([0.0, 0.0]), None);
        assert_eq!(value_range([f64::NAN].into_iter()), None);
    }

    #[test]
    fn finite_runs_leave_gaps_at_infinities_and_nans() {
        let pts = [
            [0.0, 1.0],
            [1.0, f64::INFINITY],
            [2.0, 2.0],
            [3.0, 3.0],
            [f64::NAN, 0.0],
        ];
        assert_eq!(
            finite_runs(pts.into_iter()),
            [vec![[0.0, 1.0]], vec![[2.0, 2.0], [3.0, 3.0]]]
        );
    }

    #[test]
    fn grids_show_the_newest_frame_with_row_zero_at_the_bottom() {
        let shape = FieldShape::Grid { nx: 2, ny: 2 };
        let mut h = History::with_fields("t", vec![], vec![("g".into(), shape)]);
        h.push_sample(0.0, &[], &[&[1.0, 2.0, 3.0, 4.0]]);
        let (size, v, _) = heatmap_pixels(&h, 0, 0, None, false).unwrap();
        assert_eq!(size, [2, 2]);
        assert_eq!(v, [3.0, 4.0, 1.0, 2.0]);
        let (img, lo, hi) = colour_image(size, &v);
        assert_eq!((lo, hi), (1.0, 4.0));
        assert_eq!(img.pixels[2], sequential(0.0));
    }

    #[test]
    fn scrolling_up_zooms_in() {
        assert!(zoom_factor(50.0) > 1.0);
        assert!(zoom_factor(-50.0) < 1.0);
        assert_eq!(zoom_factor(0.0), 1.0);
    }
}
