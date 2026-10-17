//! The built-in views: `plot.xy` (ADR 0018), `plot.3d`, `plot.heatmap` and
//! `plot.scatter`.

use crate::colour::{segments, sequential, trail_start, ColourKey, HEAD};
use crate::history::{FieldShape, History, SignalRef, TIME};
use crate::view::{
    Bindings, Preset, Slot, SlotKind, StyleProperty, View, ViewInput, ViewRegistry, ViewType,
};
use crate::view3d::{self, Camera};
use egui_plot::{
    Legend, Line, MarkerShape, Plot, PlotImage, PlotPoint, PlotPoints, PlotUi, Points,
};

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
        slots: &[Slot {
            name: "field",
            kind: SlotKind::Grid,
            many: false,
            required: true,
            description: "The grid field shown",
        }],
        style: &[TRAIL_LENGTH],
        applies_to: &[],
        presets: &[Preset {
            label: "Heat map",
            description: "A grid field as colours",
            bindings: |a| a.grids.first().map(|g| bindings(&[("field", g)])),
        }],
        create: || Box::new(Heatmap::default()),
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

/// The values of a heat map image, row by row from the top, and its extent
/// in plot coordinates `[x0, x1, y0, y1]`. A line of cells (`ny` = 1) is
/// shown over time, cell 0 at the bottom; a 2D grid is shown at the
/// newest sample, row 0 at the bottom.
fn heatmap_pixels(h: &History, field: usize, first: usize) -> Option<HeatmapImage> {
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
                values[(nx - 1 - cell) * cols + c] = v;
            }
        }
        let (t0, t1) = (h.times()[first], h.times()[n - 1]);
        let t1 = if t1 > t0 { t1 } else { t0 + 1.0 };
        Some(([cols, nx], values, [t0, t1, 0.0, nx as f64]))
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
        let Some((size, values, [x0, x1, y0, y1])) = heatmap_pixels(h, field, first) else {
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
        let over_time = matches!(h.fields()[field].shape, FieldShape::Grid { ny: 1, .. });
        ui.weak(format!(
            "{}: {} (dark) to {} (bright)",
            h.name(SignalRef::Field(field)),
            fmt(lo),
            fmt(hi)
        ));
        Plot::new("plot.heatmap")
            .x_axis_label(if over_time { "t [s]" } else { "x (cells)" })
            .y_axis_label(if over_time { "cell" } else { "y (cells)" })
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
        let (size, v, extent) = heatmap_pixels(&h, 0, 0).unwrap();
        assert_eq!(size, [4, 3]);
        // Top row is the last cell; columns are samples in time order.
        assert_eq!(&v[0..4], &[20.0, 21.0, 22.0, 23.0]);
        assert_eq!(&v[8..12], &[0.0, 1.0, 2.0, 3.0]);
        assert_eq!(extent, [0.0, 3.0, 0.0, 3.0]);
    }

    #[test]
    fn grids_show_the_newest_frame_with_row_zero_at_the_bottom() {
        let shape = FieldShape::Grid { nx: 2, ny: 2 };
        let mut h = History::with_fields("t", vec![], vec![("g".into(), shape)]);
        h.push_sample(0.0, &[], &[&[1.0, 2.0, 3.0, 4.0]]);
        let (size, v, _) = heatmap_pixels(&h, 0, 0).unwrap();
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
