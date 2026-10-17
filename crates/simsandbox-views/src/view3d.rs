//! A rotatable 3D trajectory view, projected on the CPU and drawn with the
//! egui painter. Enough for lines; full 3D scenes will use wgpu paint
//! callbacks (ADR 0008).

use crate::colour::{Segment, HEAD};
use egui::{Align2, FontId, Pos2, Sense, Stroke, Vec2};

/// Distance of the eye from the centre, in normalised units; smaller is a
/// stronger perspective.
const EYE_DISTANCE: f32 = 4.0;
/// At most this many points are drawn; longer trajectories are strided.
const MAX_DRAWN: usize = 20_000;

/// View orientation. The third axis points up on screen at zero pitch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// Rotation about the vertical axis, radians.
    pub yaw: f32,
    /// Tilt; positive looks down from above, radians.
    pub pitch: f32,
    pub zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            yaw: -0.6,
            pitch: 0.35,
            zoom: 1.0,
        }
    }
}

impl Camera {
    /// Project a point in normalised coordinates (about [-1, 1] per axis) to
    /// screen units: right, up (both about [-1, 1]) and depth (into the
    /// screen).
    pub fn project(&self, p: [f32; 3]) -> [f32; 3] {
        let [x, y, z] = p;
        let (sy, cy) = self.yaw.sin_cos();
        let right = cy * x - sy * y;
        let ahead = sy * x + cy * y;
        let (sp, cp) = self.pitch.sin_cos();
        let up = cp * z + sp * ahead;
        let depth = cp * ahead - sp * z;
        let f = EYE_DISTANCE / (EYE_DISTANCE + depth);
        [right * f * self.zoom, up * f * self.zoom, depth]
    }

    /// Rotate by a mouse drag in points.
    pub fn drag(&mut self, delta: Vec2) {
        self.yaw -= delta.x * 0.01;
        self.pitch = (self.pitch + delta.y * 0.01).clamp(-1.5, 1.5);
    }
}

/// Maps data coordinates into a cube of half-width 1 around the origin,
/// keeping the axes' proportions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Normaliser {
    pub lo: [f64; 3],
    pub hi: [f64; 3],
    center: [f64; 3],
    half: f64,
}

impl Normaliser {
    pub fn fit(points: impl Iterator<Item = [f64; 3]>) -> Option<Self> {
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for p in points.filter(|p| p.iter().all(|v| v.is_finite())) {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        if lo[0] > hi[0] {
            return None;
        }
        let center = [0, 1, 2].map(|k| 0.5 * (lo[k] + hi[k]));
        let half = (0..3)
            .map(|k| 0.5 * (hi[k] - lo[k]))
            .fold(0.0, f64::max)
            .max(1e-12);
        Some(Self {
            lo,
            hi,
            center,
            half,
        })
    }

    pub fn apply(&self, p: [f64; 3]) -> [f32; 3] {
        [0, 1, 2].map(|k| ((p[k] - self.center[k]) / self.half) as f32)
    }
}

/// Screen scale (points per normalised unit) that keeps the whole bounding
/// box inside a square of side `side` at any rotation, at zoom 1: the box's
/// bounding sphere, enlarged by perspective when it is nearest the eye.
fn fit_radius(norm: &Normaliser, side: f32) -> f32 {
    let lo = norm.apply(norm.lo);
    let hi = norm.apply(norm.hi);
    let r = (0..3)
        .map(|k| lo[k].abs().max(hi[k].abs()).powi(2))
        .sum::<f32>()
        .sqrt()
        .max(1e-6);
    let nearest = EYE_DISTANCE / (EYE_DISTANCE - r.min(0.9 * EYE_DISTANCE));
    0.49 * side / (r * nearest)
}

/// Draw the trajectory `points` in coloured `segments` (see
/// [`crate::colour::segments`]) with axis names `labels`, filling the
/// available space. The box fits the whole run, so it stays still while the
/// trail moves. Drag rotates, scroll zooms, double-click resets.
pub fn show(
    ui: &mut egui::Ui,
    camera: &mut Camera,
    points: &[[f64; 3]],
    segments: &[Segment],
    labels: [&str; 3],
) {
    let size = ui.available_size();
    let (response, painter) = ui.allocate_painter(size, Sense::click_and_drag());
    if response.dragged() {
        camera.drag(response.drag_delta());
    }
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        camera.zoom = (camera.zoom * (scroll * 0.002).exp()).clamp(0.1, 20.0);
    }
    if response.double_clicked() {
        *camera = Camera::default();
    }

    let rect = response.rect;
    let visuals = ui.visuals();
    painter.rect_filled(rect, 0.0, visuals.extreme_bg_color);
    let Some(norm) = Normaliser::fit(points.iter().copied()) else {
        return;
    };
    let center = rect.center();
    let radius = fit_radius(&norm, rect.width().min(rect.height()));
    let to_screen = |p: [f32; 3]| {
        let [x, y, _] = camera.project(p);
        Pos2::new(center.x + x * radius, center.y - y * radius)
    };

    // Bounding box and labelled axes along three edges from the low corner.
    let lo = norm.apply(norm.lo);
    let hi = norm.apply(norm.hi);
    let corner = |m: [bool; 3]| [0, 1, 2].map(|k| if m[k] { hi[k] } else { lo[k] });
    let box_stroke = Stroke::new(1.0, visuals.weak_text_color().gamma_multiply(0.4));
    let axis_stroke = Stroke::new(1.5, visuals.text_color().gamma_multiply(0.7));
    for k in 0..3 {
        for a in [false, true] {
            for b in [false, true] {
                let mut m0 = [false; 3];
                let others: Vec<usize> = (0..3).filter(|&j| j != k).collect();
                m0[others[0]] = a;
                m0[others[1]] = b;
                let mut m1 = m0;
                m1[k] = true;
                let stroke = if !a && !b { axis_stroke } else { box_stroke };
                painter.line_segment([to_screen(corner(m0)), to_screen(corner(m1))], stroke);
            }
        }
        let mut end = [false; 3];
        end[k] = true;
        painter.text(
            to_screen(corner(end)),
            Align2::LEFT_BOTTOM,
            format!(" {}", labels[k]),
            FontId::proportional(13.0),
            visuals.text_color(),
        );
    }

    // The trail, in coloured segments; long runs are strided.
    let drawn = segments.last().map_or(0, |l| l.end + 1 - segments[0].start);
    let stride = drawn.div_ceil(MAX_DRAWN).max(1);
    let finite = |p: &[f64; 3]| p.iter().all(|v| v.is_finite());
    for seg in segments {
        let samples = (seg.start..seg.end).step_by(stride).chain([seg.end]);
        // Non-finite points leave a gap: the painter cannot draw them.
        let mut line: Vec<Pos2> = Vec::new();
        for k in samples {
            if finite(&points[k]) {
                line.push(to_screen(norm.apply(points[k])));
            } else if !line.is_empty() {
                painter.line(std::mem::take(&mut line), Stroke::new(1.5, seg.colour));
            }
        }
        if !line.is_empty() {
            painter.line(line, Stroke::new(1.5, seg.colour));
        }
    }
    if let Some(head) = points.last().filter(|p| finite(p)) {
        painter.circle_filled(to_screen(norm.apply(*head)), 4.0, HEAD);
    }
    painter.text(
        rect.right_bottom() + Vec2::new(-8.0, -6.0),
        Align2::RIGHT_BOTTOM,
        "drag to rotate · scroll to zoom · double-click to reset",
        FontId::proportional(12.0),
        visuals.weak_text_color(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    #[test]
    fn projection_without_rotation_maps_axes_to_screen() {
        let c = Camera {
            yaw: 0.0,
            pitch: 0.0,
            zoom: 1.0,
        };
        assert!(close(c.project([0.0, 0.0, 0.0]), [0.0, 0.0, 0.0]));
        assert!(close(c.project([1.0, 0.0, 0.0]), [1.0, 0.0, 0.0]));
        assert!(close(c.project([0.0, 0.0, 1.0]), [0.0, 1.0, 0.0]));
        // The second axis points into the screen: centred, further away.
        let p = c.project([0.0, 1.0, 0.0]);
        assert!(close(p, [0.0, 0.0, 1.0]));
    }

    #[test]
    fn perspective_shrinks_distant_points_and_yaw_turns_the_scene() {
        let c = Camera {
            yaw: 0.0,
            pitch: 0.0,
            zoom: 1.0,
        };
        let near = c.project([1.0, -1.0, 0.0])[0];
        let far = c.project([1.0, 1.0, 0.0])[0];
        assert!(near > 1.0 && far < 1.0 && far > 0.0);

        let turned = Camera {
            yaw: std::f32::consts::FRAC_PI_2,
            ..c
        };
        // A quarter turn about the vertical moves the first axis into depth.
        assert!(close(turned.project([1.0, 0.0, 0.0]), [0.0, 0.0, 1.0]));
    }

    #[test]
    fn the_box_fits_at_every_rotation() {
        let n = Normaliser::fit([[0.0, 0.0, 0.0], [10.0, 7.0, 9.0]].into_iter()).unwrap();
        let side = 400.0;
        let radius = fit_radius(&n, side);
        let (lo, hi) = (n.apply(n.lo), n.apply(n.hi));
        for yaw in 0..24 {
            for pitch in -6..=6 {
                let c = Camera {
                    yaw: yaw as f32 * 0.26,
                    pitch: pitch as f32 * 0.25,
                    zoom: 1.0,
                };
                for m in 0..8 {
                    let p = [0, 1, 2].map(|k| if m >> k & 1 == 1 { hi[k] } else { lo[k] });
                    let [x, y, _] = c.project(p);
                    assert!(x.abs() * radius <= 0.5 * side && y.abs() * radius <= 0.5 * side);
                }
            }
        }
    }

    #[test]
    fn normaliser_fits_the_longest_axis_into_the_unit_cube() {
        let n = Normaliser::fit([[0.0, 0.0, 0.0], [10.0, 2.0, 4.0]].into_iter()).unwrap();
        assert!(close(n.apply([10.0, 2.0, 4.0]), [1.0, 0.2, 0.4]));
        assert!(close(n.apply([0.0, 0.0, 0.0]), [-1.0, -0.2, -0.4]));
        assert!(Normaliser::fit(std::iter::empty()).is_none());
        // A single point does not divide by zero.
        let one = Normaliser::fit([[1.0, 1.0, 1.0]].into_iter()).unwrap();
        assert!(close(one.apply([1.0, 1.0, 1.0]), [0.0, 0.0, 0.0]));
    }
}
