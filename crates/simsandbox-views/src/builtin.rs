//! The built-in views: `plot.xy` (ADR 0018) and `plot.3d` here; the field
//! views `plot.heatmap`, `plot.profile` and `plot.scatter` in
//! [`crate::fields`].

use crate::colour::{bins, segments, trail_start, ColourKey, HEAD};
use crate::decimate::{self, Columns};
use crate::history::{label, paired, split_source, SignalRef, Stamp, TIME};
use crate::view::{
    always, over_time, Bindings, Bound, Preset, Slot, SlotKind, StyleProperty, Swap, View,
    ViewInput, ViewRegistry, ViewType,
};
use crate::view3d::{self, Camera};
use egui_plot::{Legend, Line, MarkerShape, Plot, PlotPoint, PlotPoints, PlotUi, Points};

/// Segments a single coloured trail is split into.
const COLOUR_CHUNKS: usize = 48;

const COLOUR: Slot = Slot {
    name: "colour",
    kind: SlotKind::Scalar,
    of: None,
    applies: always,
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
    applies: over_time,
};

const REACH: StyleProperty = StyleProperty {
    name: "reach",
    default: 0.0,
    suggested: [0.0, 10.0],
    unit: "",
    description: "Always show at least this far either side of 0 on both axes; 0 fits the data only. Gives a view that sends its pointer room to press in",
    applies: always,
};

const EQUAL_SCALES: StyleProperty = StyleProperty {
    name: "equal_scales",
    default: 0.0,
    suggested: [0.0, 1.0],
    unit: "",
    description: "1 draws both axes to the same scale, so shapes keep their proportions",
    applies: always,
};

pub fn register(r: &mut ViewRegistry) -> Result<(), String> {
    r.register(ViewType {
        name: "plot.xy",
        label: "Plot",
        description: "Signals against time or against each other; lines of cells against each other cell by cell (a study's runs)",
        slots: &[
            Slot {
                name: "x",
                kind: SlotKind::Values,
                of: None,
                applies: always,
                many: false,
                required: true,
                description: "Horizontal axis: 't' for time, or any signal",
            },
            Slot {
                name: "y",
                kind: SlotKind::Values,
                of: None,
                applies: always,
                many: true,
                required: true,
                description: "Vertical axis: one or more signals",
            },
            Slot {
                kind: SlotKind::Values,
                ..COLOUR
            },
        ],
        style: &[TRAIL_LENGTH, REACH, EQUAL_SCALES],
        applies_to: &[],
        swap: Some(Swap::Transpose),
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
            Preset {
                label: "Cell by cell",
                description: "One line of cells against another, a point per cell, coloured by a third: a study's runs",
                bindings: |a| {
                        let [x, y, rest @ ..] = a.lines else {
                        return None;
                    };
                    let mut b = bindings(&[("x", x), ("y", y)]);
                    if let Some(c) = rest.first() {
                        b.insert("colour".into(), vec![c.clone()]);
                    }
                    Some(b)
                },
            },
        ],
        create: || Box::new(Xy::default()),
    })?;
    r.register(ViewType {
        name: "plot.3d",
        label: "3D trajectory",
        description: "Three signals as a path in space",
        slots: &[
            Slot {
                name: "x",
                kind: SlotKind::Scalar,
                of: None,
                applies: always,
                many: false,
                required: true,
                description: "First axis",
            },
            Slot {
                name: "y",
                kind: SlotKind::Scalar,
                of: None,
                applies: always,
                many: false,
                required: true,
                description: "Second axis",
            },
            Slot {
                name: "z",
                kind: SlotKind::Scalar,
                of: None,
                applies: always,
                many: false,
                required: true,
                description: "Third axis, up",
            },
            COLOUR,
        ],
        style: &[TRAIL_LENGTH],
        applies_to: &[],
        swap: None,
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
    crate::fields::register(r)?;
    crate::surface::register(r)?;
    crate::values::register(r)?;
    crate::mesh::register(r)
}

/// A bound signal's label with its unit, as an axis shows it.
fn with_unit(b: Bound<'_>) -> String {
    match b.history.unit(b.history.name(b.signal)) {
        Some(u) if b.signal != SignalRef::Time => format!("{} [{u}]", label(b.name)),
        _ => label(b.name),
    }
}

pub(crate) fn bindings(pairs: &[(&str, &str)]) -> Bindings {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), vec![v.to_string()]))
        .collect()
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
pub(crate) fn series_colour(k: usize) -> egui::Color32 {
    // egui_plot's automatic colours: golden-ratio hues.
    let golden_ratio = (5.0_f32.sqrt() - 1.0) / 2.0;
    let h = k as f32 * golden_ratio;
    egui::epaint::Hsva::new(h, 0.85, 0.5, 1.0).into()
}

/// Any signals against any signal (ADR 0018).
#[derive(Default)]
struct Xy {
    /// Drawn with the `y` signals across and `x` up ("swap axes").
    transposed: bool,
    kept: Kept,
    /// The curve last clicked, until the app asks.
    picked: Option<String>,
    /// The pointer, when the view sends input (ADR 0058).
    pointer: crate::view::PlotPointer,
}

/// What `plot.xy` last drew, kept while nothing it was drawn from changes:
/// with the model paused, a frame draws the same lines again without
/// reading the recordings.
#[derive(Default)]
struct Kept {
    /// The recordings' stamps, the bindings and the trail length the
    /// curves were read with.
    data: Option<(Vec<Stamp>, Bindings, u64)>,
    /// Each curve's first sample shown (its trail).
    firsts: Vec<usize>,
    x_range: Option<[f64; 2]>,
    y_range: Option<[f64; 2]>,
    /// Each curve's newest point, as `(x, y)`.
    newest: Vec<Option<[f64; 2]>>,
    /// The column width, the plot's width in pixels and the orientation
    /// the lines were drawn for.
    drawn: Option<[u64; 3]>,
    /// The lines as drawn, each with its colour if it is a segment of a
    /// coloured curve, and each curve's lines among them.
    lines: Vec<(Option<egui::Color32>, Vec<PlotPoint>)>,
    spans: Vec<std::ops::Range<usize>>,
    /// Times the recordings were read and the lines drawn.
    #[cfg(test)]
    counts: [usize; 2],
}

/// A curve's id in the plot, by its signal's name.
fn curve_id(name: &str) -> egui::Id {
    egui::Id::new(("curve", name))
}

/// A recorded column of values: read where the recording keeps it, or
/// paired from several recordings (ADR 0063).
enum Column<'a> {
    Times(&'a [f64]),
    Points(&'a [PlotPoint]),
    Owned(Vec<f64>),
}

impl Column<'_> {
    fn at(&self, i: usize) -> f64 {
        match self {
            Column::Times(t) => t[i],
            Column::Points(p) => p[i].y,
            Column::Owned(v) => v[i],
        }
    }

    fn len(&self) -> usize {
        match self {
            Column::Times(t) => t.len(),
            Column::Points(p) => p.len(),
            Column::Owned(v) => v.len(),
        }
    }

    /// Times, which are never kept as points.
    fn slice(&self) -> &[f64] {
        match self {
            Column::Times(t) => t,
            Column::Owned(v) => v,
            Column::Points(_) => &[],
        }
    }

    fn to_vec(&self) -> Vec<f64> {
        match self {
            Column::Times(t) => t.to_vec(),
            Column::Points(p) => p.iter().map(|p| p.y).collect(),
            Column::Owned(v) => v.clone(),
        }
    }
}

/// One curve of `plot.xy`: its `y` against `x` (and colour), at the times
/// all of them were recorded (ADR 0063).
struct Curve<'a> {
    t: Column<'a>,
    x: Column<'a>,
    y: Column<'a>,
    n: usize,
    colour: ColourKey,
}

impl<'a> Curve<'a> {
    fn new(x: Bound<'a>, y: Bound<'a>, colour: Option<Bound<'a>>) -> Self {
        let mut bound = vec![x, y];
        bound.extend(colour);
        // One recording: its samples, read in place.
        let mut sources = bound
            .iter()
            .filter(|b| b.signal != SignalRef::Time)
            .map(|b| b.history);
        let first = sources.next();
        let one = match first {
            Some(h) if sources.all(|g| std::ptr::eq(g, h)) => Some(h),
            Some(_) => None,
            None => Some(x.history),
        };
        let (t, mut columns) = match one {
            Some(h) => {
                let column = |b: &Bound<'a>| match b.signal {
                    SignalRef::Time => Column::Times(h.times()),
                    SignalRef::Signal(i) => Column::Points(h.points(i)),
                    SignalRef::Field(_) => Column::Owned(vec![f64::NAN; h.len()]),
                };
                let columns: Vec<Column<'a>> = bound.iter().map(column).collect();
                (Column::Times(h.times()), columns)
            }
            None => {
                let signals: Vec<_> = bound.iter().map(|b| (b.history, b.signal)).collect();
                let p = paired(&signals);
                let columns = p.columns.into_iter().map(Column::Owned).collect();
                (Column::Owned(p.t), columns)
            }
        };
        let n = t.len();
        // The colour's column is the last; taken off before `y` and `x`
        // whatever it is bound to (time included).
        let colour = match colour {
            None => ColourKey::Constant,
            Some(c) => {
                let column = columns.pop();
                match c.signal {
                    SignalRef::Time => ColourKey::Time(t.to_vec()),
                    _ => ColourKey::Value(column.map_or_else(Vec::new, |c| c.to_vec())),
                }
            }
        };
        let ys = columns.pop().unwrap_or(Column::Owned(Vec::new()));
        let xs = columns.pop().unwrap_or(Column::Owned(Vec::new()));
        Self {
            t,
            x: xs,
            y: ys,
            n,
            colour,
        }
    }

    /// The first sample shown with `trail` seconds of trail.
    fn first(&self, trail: f64) -> usize {
        trail_start(self.t.slice(), trail)
    }
}

/// Points `from..to` of a curve as drawn (`at`), in runs of finite points
/// (plots cannot draw infinities or NaN), each thinned to columns `width`
/// wide ([`Columns`]; not thinned if `width` is 0).
fn drawn(
    c: &Curve<'_>,
    from: usize,
    to: usize,
    at: impl Fn(f64, f64) -> [f64; 2],
    width: f64,
) -> Vec<Vec<PlotPoint>> {
    let mut runs = Vec::new();
    let mut run: Option<Columns> = None;
    for i in from..to {
        let p = at(c.x.at(i), c.y.at(i));
        if p.iter().all(|v| v.is_finite()) {
            run.get_or_insert_with(|| Columns::new(width)).push(p);
        } else if let Some(r) = run.take() {
            runs.push(r.finish());
        }
    }
    runs.extend(run.map(Columns::finish));
    runs
}

impl Xy {
    /// `plot.xy` over lines of cells: each `y` against `x` at their latest
    /// frame, a marker per cell, coloured by `colour`'s cells if bound.
    fn cells(
        &mut self,
        ui: &mut egui::Ui,
        input: &ViewInput<'_>,
        x: Bound<'_>,
        fx: usize,
        colour: Option<Bound<'_>>,
    ) {
        // Over a table's rows (ADR 0086), each value paired with x's row
        // of the same id; NaN where it has none.
        let x_ids: Option<Vec<u64>> = (!x.history.is_empty())
            .then(|| x.history.fields()[fx].rows(x.history.len() - 1))
            .flatten()
            .map(|(_, ids, _)| ids.iter().map(|&i| i as u64).collect());
        let latest = |b: Bound<'_>, f: usize| -> Vec<f64> {
            let h = b.history;
            if h.is_empty() {
                return Vec::new();
            }
            let field = &h.fields()[f];
            match (field.rows(h.len() - 1), &x_ids) {
                (Some((_, ids, values)), Some(order)) => {
                    let at: std::collections::HashMap<u64, f64> = ids
                        .iter()
                        .zip(values)
                        .map(|(&i, &v)| (i as u64, v))
                        .collect();
                    order
                        .iter()
                        .map(|i| at.get(i).copied().unwrap_or(f64::NAN))
                        .collect()
                }
                (Some((_, _, values)), None) => values.to_vec(),
                _ => field.frame(h.len() - 1).to_vec(),
            }
        };
        let xs = latest(x, fx);
        let key = match colour {
            Some(c) => match c.signal {
                SignalRef::Field(fc) => ColourKey::Value(latest(c, fc)),
                _ => ColourKey::Constant,
            },
            None => ColourKey::Constant,
        };
        let tr = self.transposed;
        let at = move |a: f64, b: f64| if tr { [b, a] } else { [a, b] };
        let ys: Vec<(String, Vec<f64>)> = input
            .many_any("y")
            .iter()
            .filter_map(|&y| match y.signal {
                SignalRef::Field(fy) => Some((with_unit(y), latest(y, fy))),
                _ => None,
            })
            .collect();
        let mut plot = Plot::new("plot.xy.cells").allow_scroll(false);
        let (across, up) = (with_unit(x), (ys.len() == 1).then(|| ys[0].0.clone()));
        let (across, up) = if tr {
            (up, Some(across))
        } else {
            (Some(across), up)
        };
        if let Some(l) = across {
            plot = plot.x_axis_label(l);
        }
        if let Some(l) = up {
            plot = plot.y_axis_label(l);
        }
        if ys.len() > 1 {
            plot = plot.legend(Legend::default());
        }
        plot.show(ui, |plot| {
            scroll_to_zoom(plot);
            for (k, (name, y)) in ys.iter().enumerate() {
                let n = xs.len().min(y.len());
                let point = |i: usize| at(xs[i], y[i]);
                let finite = |p: &[f64; 2]| p.iter().all(|v| v.is_finite());
                if ys.len() == 1 && !matches!(key, ColourKey::Constant) {
                    for (c, cells) in bins(0, n, &key, COLOUR_CHUNKS) {
                        let pts: Vec<[f64; 2]> =
                            cells.into_iter().map(point).filter(finite).collect();
                        plot.points(Points::new(name.clone(), pts).radius(2.0).color(c));
                    }
                } else {
                    let pts: Vec<[f64; 2]> = (0..n).map(point).filter(finite).collect();
                    plot.points(
                        Points::new(name.clone(), pts)
                            .radius(2.0)
                            .color(series_colour(k)),
                    );
                }
            }
        });
    }
}

impl View for Xy {
    fn show(&mut self, ui: &mut egui::Ui, input: &ViewInput<'_>) {
        let Some(x) = input.one_any("x") else {
            return;
        };
        let colour = input.one_any("colour");
        // Lines of cells: their latest values, cell by cell (ADR 0071).
        if let SignalRef::Field(fx) = x.signal {
            self.cells(ui, input, x, fx, colour);
            return;
        }
        let ys = input.many_any("y");
        let trail = input.style("trail_length");
        // Read the recordings only when they or the bindings changed: with
        // the model paused, a frame draws what the last one did.
        let stamps: Vec<Stamp> = std::iter::once(x)
            .chain(ys.iter().copied())
            .chain(colour)
            .map(|b| b.history.stamp())
            .collect();
        let kept = &mut self.kept;
        let same = kept
            .data
            .as_ref()
            .is_some_and(|(s, b, t)| *s == stamps && b == input.bind && *t == trail.to_bits());
        // Each curve from its own recording, paired with x (and colour) at
        // the times both recorded.
        let mut curves: Option<Vec<Curve<'_>>> = None;
        if !same {
            let read: Vec<Curve<'_>> = ys.iter().map(|&y| Curve::new(x, y, colour)).collect();
            kept.firsts = read.iter().map(|c| c.first(trail)).collect();
            // Keep near-constant signals (e.g. a conserved energy) from
            // being magnified until rounding noise looks like drift.
            let union = |a: [f64; 2], b: [f64; 2]| [a[0].min(b[0]), a[1].max(b[1])];
            kept.y_range = read
                .iter()
                .zip(&kept.firsts)
                .filter_map(|(c, &f)| value_range((f..c.n).map(|i| c.y.at(i))))
                .reduce(union);
            kept.x_range = read
                .iter()
                .zip(&kept.firsts)
                .filter_map(|(c, &f)| value_range((f..c.n).map(|i| c.x.at(i))))
                .reduce(union);
            kept.newest = read
                .iter()
                .map(|c| (c.n > 0).then(|| [c.x.at(c.n - 1), c.y.at(c.n - 1)]))
                .collect();
            kept.data = Some((stamps, input.bind.clone(), trail.to_bits()));
            kept.drawn = None;
            #[cfg(test)]
            {
                kept.counts[0] += 1;
            }
            curves = Some(read);
        }
        let against_time = x.signal == SignalRef::Time;
        let x_label = if against_time && input.history.index() == TIME {
            "t [s]".to_string()
        } else if against_time {
            input.history.index().to_string()
        } else {
            with_unit(x)
        };
        let y_label = (ys.len() == 1).then(|| with_unit(ys[0]));
        // A point of the curve of y against x, as drawn.
        let tr = self.transposed;
        let at = move |a: f64, b: f64| if tr { [b, a] } else { [a, b] };
        let (across, up) = if tr {
            (y_label, Some(x_label))
        } else {
            (Some(x_label), y_label)
        };
        // Sending the pointer, a drag moves it instead of the plot.
        let mut plot = Plot::new("plot.xy")
            .allow_scroll(false)
            .allow_drag(!input.sends_pointer)
            .allow_boxed_zoom(!input.sends_pointer);
        if let Some(l) = across {
            plot = plot.x_axis_label(l);
        }
        if let Some(l) = up {
            plot = plot.y_axis_label(l);
        }
        if ys.len() > 1 {
            plot = plot.legend(Legend::default());
        }
        let (across_range, up_range) = if tr {
            (kept.y_range, kept.x_range)
        } else {
            (kept.x_range, kept.y_range)
        };
        if let Some([lo, hi]) = across_range.and_then(widened) {
            plot = plot.include_x(lo).include_x(hi);
        }
        if let Some([lo, hi]) = up_range.and_then(widened) {
            plot = plot.include_y(lo).include_y(hi);
        }
        let reach = input.style("reach");
        if reach > 0.0 && reach.is_finite() {
            plot = plot
                .include_x(-reach)
                .include_x(reach)
                .include_y(-reach)
                .include_y(reach);
        }
        if input.style("equal_scales") >= 0.5 {
            plot = plot.data_aspect(1.0);
        }
        let shown: Vec<String> = ys.iter().map(|y| y.name.to_string()).collect();
        let labels: Vec<String> = ys.iter().map(|y| label(y.name)).collect();
        let one_coloured = ys.len() == 1 && colour.is_some();
        let (names, labels) = (&shown, &labels);
        let response = plot.show(ui, move |plot| {
            scroll_to_zoom(plot);
            // Long curves are thinned to the pixel columns they are drawn
            // on, keeping each column's extremes.
            // This frame's width; the bounds as last drawn.
            let pixels = f64::from(plot.response().rect.width() * plot.ctx().pixels_per_point());
            let data = across_range.map_or(0.0, |[lo, hi]| hi - lo);
            let column = decimate::pixel_width(plot.plot_bounds().width(), data, pixels)
                / decimate::COLUMNS_PER_PIXEL;
            // Drawn again only for other recordings, another width or the
            // other orientation.
            let key = [column.to_bits(), pixels.to_bits(), u64::from(tr)];
            if kept.drawn != Some(key) {
                let curves = curves
                    .get_or_insert_with(|| ys.iter().map(|&y| Curve::new(x, y, colour)).collect());
                kept.lines.clear();
                kept.spans.clear();
                for (c, &first) in curves.iter().zip(&kept.firsts) {
                    let n = c.n;
                    let start = kept.lines.len();
                    // Curves with fewer points than the plot is pixels
                    // wide (twice over) are drawn whole.
                    let width = if (n - first.min(n)) as f64 > 2.0 * pixels {
                        column
                    } else {
                        0.0
                    };
                    if one_coloured {
                        // One curve with a colour encoding: coloured
                        // segments.
                        for seg in segments(first, n, &c.colour, COLOUR_CHUNKS) {
                            for run in drawn(c, seg.start, seg.end + 1, at, width) {
                                kept.lines.push((Some(seg.colour), run));
                            }
                        }
                    } else if n > first + 1 {
                        for run in drawn(c, first, n, at, width) {
                            kept.lines.push((None, run));
                        }
                    }
                    kept.spans.push(start..kept.lines.len());
                }
                kept.drawn = Some(key);
                #[cfg(test)]
                {
                    kept.counts[1] += 1;
                }
            }
            let kept: &Kept = kept;
            for (k, span) in kept.spans.iter().enumerate() {
                for (colour, run) in &kept.lines[span.clone()] {
                    let points = PlotPoints::Borrowed(run);
                    match colour {
                        Some(c) => {
                            plot.line(Line::new("trail", points).color(*c).allow_hover(false));
                        }
                        // Pieces of one curve (split where it is not
                        // finite) share its name and colour.
                        None => plot.line(
                            Line::new(labels[k].clone(), points)
                                .id(curve_id(&names[k]))
                                .color(series_colour(k)),
                        ),
                    }
                }
                // Mark the current point unless time is an axis (it is the
                // edge).
                if let Some([a, b]) = kept.newest[k].filter(|_| !against_time) {
                    let now = at(a, b);
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
        if input.sends_pointer {
            // The plot is drawn as (x, y) or transposed; the pointer is sent
            // in the plot's own axes.
            self.pointer.read(&response);
        }
        // A click on a curve picks its signal; with one curve (drawn in
        // colour segments that cannot be hovered), a click anywhere does.
        if response.response.clicked() && !input.sends_pointer {
            let hit = response
                .hovered_plot_item
                .and_then(|id| shown.iter().find(|n| curve_id(n) == id).cloned());
            self.picked = hit
                .or_else(|| (shown.len() == 1).then(|| shown[0].clone()))
                .map(|n| split_source(&n).1.to_string());
        }
    }

    fn picked(&mut self) -> Option<String> {
        self.picked.take()
    }

    fn pointer(&mut self) -> Option<[f64; 3]> {
        self.pointer.take()
    }

    fn transpose(&mut self) {
        self.transposed = !self.transposed;
    }

    fn selection(&self) -> Option<serde_json::Value> {
        self.transposed
            .then(|| serde_json::json!({ "transposed": true }))
    }

    fn set_selection(&mut self, selection: &serde_json::Value) -> Result<(), String> {
        match selection
            .get("transposed")
            .and_then(serde_json::Value::as_bool)
        {
            Some(t) if selection.as_object().is_some_and(|o| o.len() == 1) => {
                self.transposed = t;
                Ok(())
            }
            _ => Err("expected {\"transposed\": true or false}".into()),
        }
    }
}

#[derive(Default)]
struct Trajectory {
    camera: Camera,
}

impl View for Trajectory {
    fn show(&mut self, ui: &mut egui::Ui, input: &ViewInput<'_>) {
        let (Some(x), Some(y), Some(z)) =
            (input.one_any("x"), input.one_any("y"), input.one_any("z"))
        else {
            return;
        };
        // Paired at the times all of them were recorded (ADR 0063).
        let colour = input.one_any("colour");
        let mut signals: Vec<_> = [x, y, z].iter().map(|b| (b.history, b.signal)).collect();
        signals.extend(colour.map(|c| (c.history, c.signal)));
        let p = paired(&signals);
        let points: Vec<[f64; 3]> = (0..p.t.len())
            .map(|k| [p.columns[0][k], p.columns[1][k], p.columns[2][k]])
            .collect();
        let key = match colour {
            None => ColourKey::Constant,
            Some(c) if c.signal == SignalRef::Time => ColourKey::Time(p.t.clone()),
            Some(_) => ColourKey::Value(p.columns[3].clone()),
        };
        let first = trail_start(&p.t, input.style("trail_length"));
        let segs = segments(first, p.t.len(), &key, COLOUR_CHUNKS);
        let names = [x, y, z].map(|b| label(b.name));
        view3d::show(
            ui,
            &mut self.camera,
            &points,
            &segs,
            [&names[0], &names[1], &names[2]].map(String::as_str),
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn signals_from_different_recordings_pair_at_common_times() {
        use crate::history::{paired, Recorded};
        // Every step (0.1 s) and a probe every 0.3 s.
        let mut all = crate::History::new(vec!["a.x".into()]);
        let mut probe = crate::History::new(vec!["a.x".into()]);
        for k in 0..10 {
            let t = k as f64 * 0.1;
            all.push(t, &[k as f64]);
            if k % 3 == 0 {
                probe.push(t, &[10.0 * k as f64]);
            }
        }
        let probes = [("p", &probe)];
        let r = Recorded {
            all: &all,
            probes: &probes,
            derived: &[],
        };
        let (h, x) = r.resolve("a.x").unwrap();
        let (hp, xp) = r.resolve("probe:p/a.x").unwrap();
        assert!(r.resolve("probe:gone/a.x").is_none());
        let p = paired(&[(h, x), (hp, xp)]);
        assert_eq!(p.columns[0], [0.0, 3.0, 6.0, 9.0]);
        assert_eq!(p.columns[1], [0.0, 30.0, 60.0, 90.0]);
        // One recording alone: every sample; t binds to its times.
        let one = paired(&[(hp, xp), (hp, SignalRef::Time)]);
        assert_eq!(one.t.len(), 4);
        assert_eq!(one.columns[1], one.t);
        assert_eq!(crate::history::label("probe:p/a.x"), "a.x (p)");
    }

    /// Frames of a `plot.xy` of `y` against time, `size` points large.
    fn xy_frames(
        ctx: &egui::Context,
        view: &mut Xy,
        h: &crate::History,
        size: egui::Vec2,
        frames: usize,
    ) {
        xy_frames_bound(ctx, view, h, &[("x", "t"), ("y", "y")], size, frames);
    }

    /// Frames of a `plot.xy` with the given bindings.
    fn xy_frames_bound(
        ctx: &egui::Context,
        view: &mut Xy,
        h: &crate::History,
        bound: &[(&str, &str)],
        size: egui::Vec2,
        frames: usize,
    ) {
        let reg = crate::ViewRegistry::builtin();
        let t = reg.get("plot.xy").unwrap();
        let bind = bindings(bound);
        let style = Default::default();
        let input = crate::view::view_input(&bind, &style, t, h.into());
        for _ in 0..frames {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            };
            let mut out = ctx.run_ui(raw, |ui| view.show(ui, &input));
            out.textures_delta.clear();
        }
    }

    #[test]
    fn a_curve_coloured_by_time_is_still_y_against_x() {
        // A phase portrait: y = 2x, coloured by t. Before 2026-11-18 the
        // time column was taken as y and y as x.
        let mut h = crate::History::new(vec!["x".into(), "y".into()]);
        for k in 0..50 {
            let x = (k as f64 * 0.3).sin();
            h.push(k as f64 * 0.1, &[x, 2.0 * x]);
        }
        let ctx = egui::Context::default();
        let size = egui::vec2(400.0, 300.0);
        for bound in [
            &[("x", "x"), ("y", "y"), ("colour", "t")][..],
            &[("x", "x"), ("y", "y"), ("colour", "x")],
            &[("x", "x"), ("y", "y")],
        ] {
            let mut view = Xy::default();
            xy_frames_bound(&ctx, &mut view, &h, bound, size, 2);
            let points: Vec<&PlotPoint> = view.kept.lines.iter().flat_map(|(_, r)| r).collect();
            assert!(!points.is_empty(), "{bound:?}");
            for p in points {
                assert_eq!(p.y, 2.0 * p.x, "{bound:?}");
            }
        }
    }

    #[test]
    fn long_curves_are_drawn_thinned_with_every_peak() {
        let mut h = crate::History::new(vec!["y".into()]);
        for k in 0..100_000 {
            let y = if k == 61_234 {
                50.0
            } else {
                (k as f64 * 0.01).sin()
            };
            h.push(k as f64 * 1e-3, &[y]);
        }
        let mut view = Xy::default();
        let ctx = egui::Context::default();
        xy_frames(&ctx, &mut view, &h, egui::vec2(400.0, 300.0), 3);
        let points: Vec<&PlotPoint> = view.kept.lines.iter().flat_map(|(_, r)| r).collect();
        // Four columns to each of fewer than 400 pixels, at most six
        // points to a column.
        assert!(points.len() < 6 * 4 * 400, "{}", points.len());
        assert!(points.iter().any(|p| p.y == 50.0));
        assert_eq!(points.first().map(|p| p.x), Some(0.0));
        assert_eq!(points.last().map(|p| p.x), Some(99.999));
        // A short recording is drawn whole.
        let mut h = crate::History::new(vec!["y".into()]);
        for k in 0..300 {
            h.push(k as f64, &[(k % 7) as f64]);
        }
        let mut view = Xy::default();
        xy_frames(&ctx, &mut view, &h, egui::vec2(400.0, 300.0), 3);
        assert_eq!(view.kept.lines[0].1.len(), 300);
    }

    #[test]
    fn a_still_plot_is_drawn_from_what_it_kept_until_data_or_size_change() {
        let mut h = crate::History::new(vec!["y".into()]);
        for k in 0..20_000 {
            h.push(k as f64 * 1e-2, &[(k as f64 * 0.1).sin()]);
        }
        let size = egui::vec2(400.0, 300.0);
        let mut view = Xy::default();
        let ctx = egui::Context::default();
        // The plot settles its bounds over its first frames.
        xy_frames(&ctx, &mut view, &h, size, 3);
        let [read, drawn] = view.kept.counts;
        // Paused: nothing is read or drawn again.
        xy_frames(&ctx, &mut view, &h, size, 5);
        assert_eq!(view.kept.counts, [read, drawn]);
        // A new sample is read and drawn.
        h.push(200.0, &[7.0]);
        xy_frames(&ctx, &mut view, &h, size, 1);
        assert_eq!(view.kept.counts, [read + 1, drawn + 1]);
        let last = view.kept.lines.last().unwrap().1.last().unwrap();
        assert_eq!((last.x, last.y), (200.0, 7.0));
        // Another size draws the lines again from the same data.
        xy_frames(&ctx, &mut view, &h, egui::vec2(800.0, 300.0), 1);
        assert_eq!(view.kept.counts[0], read + 1);
        assert!(view.kept.counts[1] > drawn + 1);
        // So does a rewind.
        h.truncate_after(100.0);
        xy_frames(&ctx, &mut view, &h, egui::vec2(800.0, 300.0), 1);
        assert_eq!(view.kept.counts[0], read + 2);
        let last = view.kept.lines.last().unwrap().1.last().unwrap();
        assert_eq!(last.x, 100.0);
    }

    #[test]
    fn scrolling_up_zooms_in() {
        assert!(zoom_factor(50.0) > 1.0);
        assert!(zoom_factor(-50.0) < 1.0);
        assert_eq!(zoom_factor(0.0), 1.0);
    }

    #[test]
    fn clicking_a_curve_picks_its_signal() {
        let reg = crate::ViewRegistry::builtin();
        let t = reg.get("plot.xy").unwrap();
        let mut h = crate::History::new(vec!["low".into(), "high".into()]);
        for k in 0..=20 {
            h.push(k as f64 * 0.05, &[0.0, 1.0]);
        }
        let bind: crate::Bindings = [
            ("x".to_string(), vec!["t".to_string()]),
            ("y".to_string(), vec!["low".to_string(), "high".to_string()]),
        ]
        .into();
        let style = Default::default();
        let input = crate::view::view_input(&bind, &style, t, (&h).into());
        let ctx = egui::Context::default();
        let mut view = Xy::default();
        let mut frame = |events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 300.0),
                )),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(raw, |ui| view.show(ui, &input));
            out.textures_delta.clear();
            view.picked()
        };
        let button = |p, pressed| egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(Vec::new());
        // Click down the middle, top to bottom: the high curve, then the
        // low one.
        let mut seen = Vec::new();
        for y in (0..300).step_by(3) {
            let p = egui::pos2(200.0, y as f32);
            frame(vec![egui::Event::PointerMoved(p)]);
            frame(vec![button(p, true)]);
            if let Some(s) = frame(vec![button(p, false)]) {
                if seen.last() != Some(&s) {
                    seen.push(s);
                }
            }
        }
        assert_eq!(seen, ["high", "low"]);
    }
}
