//! The built-in views: `plot.xy` (ADR 0018) and `plot.3d` here; the field
//! views `plot.heatmap`, `plot.profile` and `plot.scatter` in
//! [`crate::fields`].

use crate::colour::{bins, segments, trail_start, ColourKey, HEAD};
use crate::history::{History, SignalRef, TIME};
use crate::view::{
    always, over_time, Available, Bindings, Preset, Slot, SlotKind, StyleProperty, Swap, View,
    ViewInput, ViewRegistry, ViewType,
};
use crate::view3d::{self, Camera};
use egui_plot::{Legend, Line, MarkerShape, Plot, PlotUi, Points};

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

pub fn register(r: &mut ViewRegistry) -> Result<(), String> {
    r.register(ViewType {
        name: "plot.xy",
        label: "Plot",
        description: "Signals against time or against each other",
        slots: &[
            Slot {
                name: "x",
                kind: SlotKind::Scalar,
                of: None,
                applies: always,
                many: false,
                required: true,
                description: "Horizontal axis: 't' for time, or any signal",
            },
            Slot {
                name: "y",
                kind: SlotKind::Scalar,
                of: None,
                applies: always,
                many: true,
                required: true,
                description: "Vertical axis: one or more signals",
            },
            COLOUR,
        ],
        style: &[TRAIL_LENGTH],
        applies_to: &[],
        swap: Some(Swap::Transpose),
        presets: &[
            Preset {
                label: "Plot against time",
                description: "Every signal against time",
                bindings: |a| {
                    let s = a.scalars;
                    // A sweep's index is only the sample number.
                    (!s.is_empty() && a.swept.is_empty()).then(|| {
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
                    (s.len() >= 2 && a.swept.is_empty())
                        .then(|| bindings(&[("x", &s[0]), ("y", &s[y]), ("colour", TIME)]))
                },
            },
            Preset {
                label: "Samples",
                description:
                    "Where two swept parameters were sampled, coloured by the first result",
                bindings: |a| {
                    let [x, y, ..] = a.swept else {
                        return None;
                    };
                    let colour = results(a).into_iter().next()?;
                    Some(bindings(&[("x", x), ("y", y), ("colour", &colour)]))
                },
            },
            Preset {
                label: "Against the swept parameter",
                description: "Every result against the first swept parameter",
                bindings: |a| {
                    let x = a.swept.first()?;
                    let ys = results(a);
                    (!ys.is_empty())
                        .then(|| [("x".to_string(), vec![x.clone()]), ("y".to_string(), ys)].into())
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
    crate::surface::register(r)
}

/// The scalar signals of a sweep that are not swept parameters.
fn results(a: &Available<'_>) -> Vec<String> {
    a.scalars
        .iter()
        .filter(|s| !a.swept.contains(s))
        .cloned()
        .collect()
}

pub(crate) fn bindings(pairs: &[(&str, &str)]) -> Bindings {
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
}

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
        let y_label = (ys.len() == 1).then(|| h.name(ys[0]).to_string());
        // A point of the curve of y against x, as drawn.
        let tr = self.transposed;
        let at = move |a: f64, b: f64| if tr { [b, a] } else { [a, b] };
        let (across, up) = if tr {
            (y_label, Some(x_label))
        } else {
            (Some(x_label), y_label)
        };
        let mut plot = Plot::new("plot.xy").allow_scroll(false);
        if let Some(l) = across {
            plot = plot.x_axis_label(l);
        }
        if let Some(l) = up {
            plot = plot.y_axis_label(l);
        }
        if ys.len() > 1 {
            plot = plot.legend(Legend::default());
        }
        // Keep near-constant signals (e.g. a conserved energy) from being
        // magnified until rounding noise looks like drift.
        let range = |s: SignalRef| value_range((first..n).map(|k| h.value(s, k)));
        let y_range = ys
            .iter()
            .filter_map(|&y| range(y))
            .reduce(|a, b| [a[0].min(b[0]), a[1].max(b[1])]);
        let (across_range, up_range) = if tr {
            (y_range, range(x))
        } else {
            (range(x), y_range)
        };
        if let Some([lo, hi]) = across_range.and_then(widened) {
            plot = plot.include_x(lo).include_x(hi);
        }
        if let Some([lo, hi]) = up_range.and_then(widened) {
            plot = plot.include_y(lo).include_y(hi);
        }
        plot.show(ui, |plot| {
            scroll_to_zoom(plot);
            let points = |y: SignalRef, from: usize, to: usize| {
                finite_runs((from..to).map(|k| at(h.value(x, k), h.value(y, k))))
            };
            if h.unordered() {
                // Samples with no order between them: markers, not lines
                // (ADR 0028).
                let markers =
                    |y: SignalRef, ks: &mut dyn Iterator<Item = usize>| -> Vec<[f64; 2]> {
                        ks.map(|k| at(h.value(x, k), h.value(y, k)))
                            .filter(|p| p.iter().all(|v| v.is_finite()))
                            .collect()
                    };
                if let ([y], Some(_)) = (ys, input.one("colour")) {
                    for (colour, ks) in bins(first, n, &colour_key(input), COLOUR_CHUNKS) {
                        let pts = markers(*y, &mut ks.into_iter());
                        plot.points(Points::new("samples", pts).radius(2.0).color(colour));
                    }
                } else {
                    for (k, &y) in ys.iter().enumerate() {
                        let pts = markers(y, &mut (first..n));
                        plot.points(
                            Points::new(h.name(y).to_string(), pts)
                                .radius(2.0)
                                .color(series_colour(k)),
                        );
                    }
                }
            } else if let ([y], Some(_)) = (ys, input.one("colour")) {
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
                                && !tr
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
            // Mark the current point unless time is an axis (it is the edge).
            if !against_time && !h.unordered() && n > 0 {
                for &y in ys {
                    let now = at(h.value(x, n - 1), h.value(y, n - 1));
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
    fn scrolling_up_zooms_in() {
        assert!(zoom_factor(50.0) > 1.0);
        assert!(zoom_factor(-50.0) < 1.0);
        assert_eq!(zoom_factor(0.0), 1.0);
    }
}
