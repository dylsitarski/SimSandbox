//! The built-in views: `plot.xy` (ADR 0018) and `plot.3d` here; the field
//! views `plot.heatmap`, `plot.profile` and `plot.scatter` in
//! [`crate::fields`].

use crate::colour::{bins, segments, trail_start, ColourKey, HEAD};
use crate::history::{label, paired, split_source, SignalRef, TIME};
use crate::view::{
    always, over_time, Available, Bindings, Bound, Preset, Slot, SlotKind, StyleProperty, Swap,
    View, ViewInput, ViewRegistry, ViewType,
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
        style: &[TRAIL_LENGTH, REACH, EQUAL_SCALES],
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
    /// The curve last clicked, until the app asks.
    picked: Option<String>,
    /// The pointer, when the view sends input (ADR 0058).
    pointer: crate::view::PlotPointer,
}

/// A curve's id in the plot, by its signal's name.
fn curve_id(name: &str) -> egui::Id {
    egui::Id::new(("curve", name))
}

/// One curve of `plot.xy`: its `y` against `x` (and colour), at the times
/// all of them were recorded (ADR 0063).
struct Curve {
    /// The binding as written, and as shown.
    name: String,
    label: String,
    t: Vec<f64>,
    x: Vec<f64>,
    y: Vec<f64>,
    colour: ColourKey,
    unordered: bool,
}

impl Curve {
    fn new(x: Bound<'_>, y: Bound<'_>, colour: Option<Bound<'_>>) -> Self {
        let mut signals = vec![(x.history, x.signal), (y.history, y.signal)];
        signals.extend(colour.map(|c| (c.history, c.signal)));
        let mut p = paired(&signals);
        let colour = match colour {
            None => ColourKey::Constant,
            Some(c) if c.signal == SignalRef::Time => ColourKey::Time(p.t.clone()),
            Some(_) => ColourKey::Value(p.columns.pop().unwrap_or_default()),
        };
        let ys = p.columns.pop().unwrap_or_default();
        let xs = p.columns.pop().unwrap_or_default();
        Self {
            name: y.name.to_string(),
            label: label(y.name),
            t: p.t,
            x: xs,
            y: ys,
            colour,
            unordered: y.history.unordered(),
        }
    }
}

impl View for Xy {
    fn show(&mut self, ui: &mut egui::Ui, input: &ViewInput<'_>) {
        let Some(x) = input.one_any("x") else {
            return;
        };
        let colour = input.one_any("colour");
        // Each curve from its own recording, paired with x (and colour) at
        // the times both recorded.
        let curves: Vec<Curve> = input
            .many_any("y")
            .iter()
            .map(|&y| Curve::new(x, y, colour))
            .collect();
        let trail = input.style("trail_length");
        let firsts: Vec<usize> = curves.iter().map(|c| trail_start(&c.t, trail)).collect();
        let against_time = x.signal == SignalRef::Time;
        let x_label = if against_time && input.history.index() == TIME {
            "t [s]".to_string()
        } else if against_time {
            input.history.index().to_string()
        } else {
            label(x.name)
        };
        let y_label = (curves.len() == 1).then(|| curves[0].label.clone());
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
        if curves.len() > 1 {
            plot = plot.legend(Legend::default());
        }
        // Keep near-constant signals (e.g. a conserved energy) from being
        // magnified until rounding noise looks like drift.
        let union = |a: [f64; 2], b: [f64; 2]| [a[0].min(b[0]), a[1].max(b[1])];
        let y_range = curves
            .iter()
            .zip(&firsts)
            .filter_map(|(c, &f)| value_range(c.y[f..].iter().copied()))
            .reduce(union);
        let x_range = curves
            .iter()
            .zip(&firsts)
            .filter_map(|(c, &f)| value_range(c.x[f..].iter().copied()))
            .reduce(union);
        let (across_range, up_range) = if tr {
            (y_range, x_range)
        } else {
            (x_range, y_range)
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
        let shown: Vec<String> = curves.iter().map(|c| c.name.clone()).collect();
        let one_coloured = curves.len() == 1 && colour.is_some();
        let response = plot.show(ui, |plot| {
            scroll_to_zoom(plot);
            for (k, (c, &first)) in curves.iter().zip(&firsts).enumerate() {
                let n = c.t.len();
                let points =
                    |from: usize, to: usize| finite_runs((from..to).map(|i| at(c.x[i], c.y[i])));
                if c.unordered {
                    // Samples with no order between them: markers, not lines
                    // (ADR 0028).
                    let markers = |ks: &mut dyn Iterator<Item = usize>| -> Vec<[f64; 2]> {
                        ks.map(|i| at(c.x[i], c.y[i]))
                            .filter(|p| p.iter().all(|v| v.is_finite()))
                            .collect()
                    };
                    if one_coloured {
                        for (colour, ks) in bins(first, n, &c.colour, COLOUR_CHUNKS) {
                            let pts = markers(&mut ks.into_iter());
                            plot.points(Points::new("samples", pts).radius(2.0).color(colour));
                        }
                    } else {
                        plot.points(
                            Points::new(c.label.clone(), markers(&mut (first..n)))
                                .id(curve_id(&c.name))
                                .radius(2.0)
                                .color(series_colour(k)),
                        );
                    }
                } else if one_coloured {
                    // One curve with a colour encoding: coloured segments.
                    for seg in segments(first, n, &c.colour, COLOUR_CHUNKS) {
                        for run in points(seg.start, seg.end + 1) {
                            plot.line(Line::new("trail", run).color(seg.colour).allow_hover(false));
                        }
                    }
                } else if n > first + 1 {
                    // Pieces of one curve (split where it is not finite)
                    // share its name and colour.
                    for run in points(first, n) {
                        plot.line(
                            Line::new(c.label.clone(), run)
                                .id(curve_id(&c.name))
                                .color(series_colour(k)),
                        );
                    }
                }
                // Mark the current point unless time is an axis (it is the
                // edge).
                if !against_time && !c.unordered && n > 0 {
                    let now = at(c.x[n - 1], c.y[n - 1]);
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
