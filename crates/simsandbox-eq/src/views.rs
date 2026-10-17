//! Views of equation-domain blocks (ADR 0016), behind the `views` feature
//! so the domain itself does not depend on a GUI toolkit.

use crate::registry::PENDULUM;
use egui_plot::{Line, Plot, Points};
use simsandbox_views::builtin::{finite_runs, scroll_to_zoom};
use simsandbox_views::colour::{trail_start, HEAD, LINE};
use simsandbox_views::view::{Slot, StyleProperty, View, ViewInput};
use simsandbox_views::{Available, Bindings, Preset, SlotKind, ViewRegistry, ViewType};

/// Register the domain's views.
pub fn register_views(r: &mut ViewRegistry) -> Result<(), String> {
    r.register(ViewType {
        name: "eq.pendulum",
        label: "Pendulum",
        description: "The pendulum itself: rod, bob and the bob's recent path",
        slots: &[
            Slot {
                name: "x",
                kind: SlotKind::Scalar,
                many: false,
                required: true,
                description: "Bob position across, pivot at 0",
            },
            Slot {
                name: "y",
                kind: SlotKind::Scalar,
                many: false,
                required: true,
                description: "Bob position up, pivot at 0",
            },
        ],
        style: &[StyleProperty {
            name: "trail_length",
            default: 1.0,
            suggested: [-1.0, 10.0],
            unit: "s",
            description: "Seconds of the bob's path shown; 0 shows none, -1 the whole run",
        }],
        applies_to: &[PENDULUM],
        presets: &[Preset {
            label: "Pendulum",
            description: "The pendulum swinging",
            bindings: pendulum_bindings,
        }],
        create: || Box::new(PendulumView),
    })
}

/// The first block offering both `.x` and `.y`.
fn pendulum_bindings(a: &Available<'_>) -> Option<Bindings> {
    let block = a.scalars.iter().find_map(|s| {
        let b = s.strip_suffix(".x")?;
        a.scalars.contains(&format!("{b}.y")).then_some(b)
    })?;
    Some(
        [
            ("x".to_string(), vec![format!("{block}.x")]),
            ("y".to_string(), vec![format!("{block}.y")]),
        ]
        .into(),
    )
}

struct PendulumView;

impl View for PendulumView {
    fn show(&mut self, ui: &mut egui::Ui, input: &ViewInput<'_>) {
        let h = input.history;
        let (Some(x), Some(y)) = (input.one("x"), input.one("y")) else {
            return;
        };
        let n = h.len();
        if n == 0 {
            return;
        }
        let bob = [h.value(x, n - 1), h.value(y, n - 1)];
        if !bob.iter().all(|v| v.is_finite()) {
            return;
        }
        // Frame the whole swing: the rod's length, with a margin.
        let reach = 1.15 * bob[0].hypot(bob[1]).max(1e-9);
        let first = trail_start(h.times(), input.style("trail_length"));
        let trail = finite_runs((first..n).map(|k| [h.value(x, k), h.value(y, k)]));
        Plot::new("eq.pendulum")
            .data_aspect(1.0)
            .allow_scroll(false)
            .include_x(-reach)
            .include_x(reach)
            .include_y(-reach)
            .include_y(reach)
            .show(ui, |plot| {
                scroll_to_zoom(plot);
                for run in trail.into_iter().filter(|r| r.len() > 1) {
                    plot.line(
                        Line::new("path", run)
                            .color(LINE.gamma_multiply(0.6))
                            .allow_hover(false),
                    );
                }
                plot.line(
                    Line::new("rod", vec![[0.0, 0.0], bob])
                        .color(egui::Color32::GRAY)
                        .width(3.0),
                );
                plot.points(
                    Points::new("pivot", vec![[0.0, 0.0]])
                        .radius(3.0)
                        .color(egui::Color32::GRAY),
                );
                plot.points(Points::new("bob", vec![bob]).radius(9.0).color(HEAD));
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pendulum_view_applies_only_to_models_with_a_pendulum() {
        let mut r = ViewRegistry::builtin();
        register_views(&mut r).unwrap();
        let t = r.get("eq.pendulum").unwrap();
        let scalars: Vec<String> = ["bob.angle", "bob.omega", "bob.x", "bob.y"]
            .map(String::from)
            .to_vec();
        let pendulum = [PENDULUM.to_string()];
        let other = ["eq.lorenz".to_string()];
        let with = Available {
            scalars: &scalars,
            block_types: &pendulum,
            ..Default::default()
        };
        let without = Available {
            block_types: &other,
            ..with
        };
        assert!(t.applies(&with));
        assert!(!t.applies(&without));
        let bind = (t.presets[0].bindings)(&with).unwrap();
        assert_eq!(bind["x"], ["bob.x"]);
        assert_eq!(bind["y"], ["bob.y"]);
        assert!(simsandbox_views::ViewState::from_preset(t, 0, &without).is_none());
    }
}
