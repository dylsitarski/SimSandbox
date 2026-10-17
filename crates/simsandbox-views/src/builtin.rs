//! The built-in views: `plot.time`, `plot.phase` and `plot.3d`.

use crate::colour::{segments, trail_start, ColourKey, HEAD};
use crate::history::{History, SignalRef, TIME};
use crate::view::{Bindings, Slot, StyleProperty, View, ViewInput, ViewRegistry, ViewType};
use crate::view3d::{self, Camera};
use egui_plot::{Legend, Line, MarkerShape, Plot, PlotPoints, PlotUi, Points};

/// Segments a 2D or 3D trail is split into for colouring.
const COLOUR_CHUNKS: usize = 48;

const COLOUR: Slot = Slot {
    name: "colour",
    many: false,
    required: false,
    description:
        "Colours the line: 't' fades it from old to new; another signal colours it by value",
};

const TRAIL_LENGTH: StyleProperty = StyleProperty {
    name: "trail_length",
    default: -1.0,
    suggested: [-1.0, 60.0],
    unit: "s",
    description: "Seconds of trail shown behind the current point; 0 shows no trail, -1 (any negative value) the whole run",
};

pub fn register(r: &mut ViewRegistry) -> Result<(), String> {
    r.register(ViewType {
        name: "plot.time",
        label: "Time series",
        description: "Signals against time",
        slots: &[Slot {
            name: "y",
            many: true,
            required: true,
            description: "Signals plotted against time",
        }],
        style: &[],
        default_bindings: |s| (!s.is_empty()).then(|| [("y".to_string(), s.to_vec())].into()),
        create: || Box::new(TimeSeries),
    })?;
    r.register(ViewType {
        name: "plot.phase",
        label: "Phase portrait",
        description: "One signal against another",
        slots: &[
            Slot {
                name: "x",
                many: false,
                required: true,
                description: "Horizontal axis",
            },
            Slot {
                name: "y",
                many: false,
                required: true,
                description: "Vertical axis",
            },
            COLOUR,
        ],
        style: &[TRAIL_LENGTH],
        default_bindings: |s| {
            // x–z for three or more signals (the Lorenz butterfly).
            let y = if s.len() >= 3 { 2 } else { 1 };
            (s.len() >= 2).then(|| bindings(&[("x", &s[0]), ("y", &s[y]), ("colour", TIME)]))
        },
        create: || Box::new(Phase),
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
        default_bindings: |s| {
            (s.len() >= 3)
                .then(|| bindings(&[("x", &s[0]), ("y", &s[1]), ("z", &s[2]), ("colour", TIME)]))
        },
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

struct TimeSeries;

impl View for TimeSeries {
    fn show(&mut self, ui: &mut egui::Ui, input: &ViewInput<'_>) {
        let h = input.history;
        Plot::new("plot.time")
            .legend(Legend::default())
            .x_axis_label("t [s]")
            .allow_scroll(false)
            .show(ui, |plot| {
                scroll_to_zoom(plot);
                for &s in input.many("y") {
                    if let SignalRef::Signal(i) = s {
                        plot.line(Line::new(h.name(s).to_string(), h.points(i)));
                    }
                }
            });
    }
}

struct Phase;

impl View for Phase {
    fn show(&mut self, ui: &mut egui::Ui, input: &ViewInput<'_>) {
        let h = input.history;
        let (Some(x), Some(y)) = (input.one("x"), input.one("y")) else {
            return;
        };
        let first = trail_start(h.times(), input.style("trail_length"));
        let segs = segments(first, h.len(), &colour_key(input), COLOUR_CHUNKS);
        let head = (!h.is_empty()).then(|| [h.value(x, h.len() - 1), h.value(y, h.len() - 1)]);
        Plot::new("plot.phase")
            .x_axis_label(h.name(x))
            .y_axis_label(h.name(y))
            .allow_scroll(false)
            .show(ui, |plot| {
                scroll_to_zoom(plot);
                for seg in &segs {
                    let pts: PlotPoints<'_> = (seg.start..=seg.end)
                        .map(|k| [h.value(x, k), h.value(y, k)])
                        .collect();
                    plot.line(Line::new("trail", pts).color(seg.colour).allow_hover(false));
                }
                if let Some(p) = head {
                    plot.points(
                        Points::new("now", vec![p])
                            .shape(MarkerShape::Circle)
                            .radius(4.0)
                            .color(HEAD),
                    );
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
