//! The built-in views: `plot.xy` (ADR 0018) and `plot.3d`.

use crate::colour::{segments, trail_start, ColourKey, HEAD};
use crate::history::{History, SignalRef, TIME};
use crate::view::{Bindings, Preset, Slot, StyleProperty, View, ViewInput, ViewRegistry, ViewType};
use crate::view3d::{self, Camera};
use egui_plot::{Legend, Line, MarkerShape, Plot, PlotPoints, PlotUi, Points};

/// Segments a single coloured trail is split into.
const COLOUR_CHUNKS: usize = 48;

const COLOUR: Slot = Slot {
    name: "colour",
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

pub fn register(r: &mut ViewRegistry) -> Result<(), String> {
    r.register(ViewType {
        name: "plot.xy",
        label: "Plot",
        description: "Signals against time or against each other",
        slots: &[
            Slot {
                name: "x",
                many: false,
                required: true,
                description: "Horizontal axis: 't' for time, or any signal",
            },
            Slot {
                name: "y",
                many: true,
                required: true,
                description: "Vertical axis: one or more signals",
            },
            COLOUR,
        ],
        style: &[TRAIL_LENGTH],
        presets: &[
            Preset {
                label: "Plot against time",
                description: "Every signal against time",
                bindings: |s| {
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
                bindings: |s| {
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
                many: false,
                required: true,
                description: "First axis",
            },
            Slot {
                name: "y",
                many: false,
                required: true,
                description: "Second axis",
            },
            Slot {
                name: "z",
                many: false,
                required: true,
                description: "Third axis, up",
            },
            COLOUR,
        ],
        style: &[TRAIL_LENGTH],
        presets: &[Preset {
            label: "3D trajectory",
            description: "Three signals as a path in space",
            bindings: |s| {
                (s.len() >= 3).then(|| {
                    bindings(&[("x", &s[0]), ("y", &s[1]), ("z", &s[2]), ("colour", TIME)])
                })
            },
        }],
        create: || Box::new(Trajectory::default()),
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
        if ys.len() == 1 {
            plot = plot.y_axis_label(h.name(ys[0]));
        } else {
            plot = plot.legend(Legend::default());
        }
        plot.show(ui, |plot| {
            scroll_to_zoom(plot);
            let points = |y: SignalRef, from: usize, to: usize| -> PlotPoints<'_> {
                (from..to).map(|k| [h.value(x, k), h.value(y, k)]).collect()
            };
            if let ([y], Some(_)) = (ys, input.one("colour")) {
                // One curve with a colour encoding: coloured segments.
                for seg in segments(first, n, &colour_key(input), COLOUR_CHUNKS) {
                    let pts = points(*y, seg.start, seg.end + 1);
                    plot.line(Line::new("trail", pts).color(seg.colour).allow_hover(false));
                }
            } else {
                for &y in ys {
                    let name = h.name(y).to_string();
                    match y {
                        // The whole run against time: plot the recorded points directly.
                        SignalRef::Signal(i) if against_time && first == 0 => {
                            plot.line(Line::new(name, h.points(i)));
                        }
                        _ if n > first + 1 => plot.line(Line::new(name, points(y, first, n))),
                        _ => {}
                    }
                }
            }
            // Mark the current point unless time runs along x (it is the right edge).
            if !against_time && n > 0 {
                for &y in ys {
                    plot.points(
                        Points::new("now", vec![[h.value(x, n - 1), h.value(y, n - 1)]])
                            .shape(MarkerShape::Circle)
                            .radius(4.0)
                            .color(HEAD),
                    );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrolling_up_zooms_in() {
        assert!(zoom_factor(50.0) > 1.0);
        assert!(zoom_factor(-50.0) < 1.0);
        assert_eq!(zoom_factor(0.0), 1.0);
    }
}
