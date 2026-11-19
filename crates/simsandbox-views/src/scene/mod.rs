//! `plot.scene` (ADR 0093, ADR 0101): layers of primitives drawn from
//! signals, constants and rows of tables, in frames that chain.
//!
//! - [`spec`]: the primitives and their attributes, declared; a scene's
//!   settings read and checked, each problem at its path.
//! - [`geometry`]: poses, and each primitive as a parametric mesh (its
//!   triangles and lines, bounds and containment).
//! - [`eval`]: the newest sample placed through the frames and drawn as one
//!   list.
//!
//! The view draws that list with egui's painter inside a plot (axes, pan
//! and zoom as the plots have them, equal scales), in the window and
//! headless alike, and keeps it while the recordings, the bindings and the
//! scale stay the same. Grabbing (ADR 0093 point 6) is a later stage: a
//! layer's `grab` is kept and not acted on.

pub mod eval;
pub mod geometry;
pub mod spec;

use crate::builtin::scroll_to_zoom;
use crate::history::Stamp;
use crate::view::{Available, Settings, View, ViewInput, ViewRegistry, ViewType};
use egui::{epaint::Mesh, Pos2, Shape, Stroke};
use egui_plot::{Plot, PlotBounds, PlotGeometry, PlotItem, PlotItemBase, PlotPoint, PlotTransform};
use eval::Drawn;
use geometry::Item;
pub use spec::{reads, Scene};
use std::sync::Arc;

/// The scene view's type name.
pub const SCENE: &str = "plot.scene";

/// Register `plot.scene`.
pub fn register(r: &mut ViewRegistry) -> Result<(), String> {
    r.register(ViewType {
        name: SCENE,
        label: "Scene",
        description: "Layers of primitives (points, segments, boxes, discs) drawn from signals, constants and rows of tables, in frames that chain (ADR 0093)",
        slots: &[],
        style: &[],
        applies_to: &[],
        swap: None,
        // Made from layers, not from slots: never offered from presets.
        presets: &[],
        create: || Box::<SceneView>::default(),
        settings: Some(spec::describe),
    })
}

/// What the scene last drew, kept while nothing it was built from changes
/// (ADR 0101 point 2): with the model paused, a frame draws the same list
/// again without reading the recordings.
#[derive(Default)]
struct Kept {
    /// The recordings' stamps, and the scale (screen points per unit) and
    /// plot size in points the list was built for. The bindings and the
    /// extent are the scene's own and reset this when they change.
    key: Option<(Vec<Stamp>, u64, [u32; 2])>,
    drawn: Arc<Drawn>,
    /// Times the list was built.
    #[cfg(test)]
    builds: usize,
}

#[derive(Default)]
struct SceneView {
    /// The settings as given, saved back unchanged.
    given: Settings,
    scene: Option<Scene>,
    kept: Kept,
}

impl SceneView {
    /// How many times the list was built (for tests of the cache).
    #[cfg(test)]
    fn builds(&self) -> usize {
        self.kept.builds
    }
}

impl View for SceneView {
    fn show(&mut self, ui: &mut egui::Ui, input: &ViewInput<'_>) {
        let Some(scene) = &self.scene else {
            return;
        };
        let stamps = scene.stamps(&input.recorded);
        let label = |axis: &str| format!("{axis} [{}]", scene.unit);
        let mut plot = Plot::new(SCENE)
            .data_aspect(1.0)
            .allow_scroll(false)
            .x_axis_label(label("x"))
            .y_axis_label(label("y"));
        // A given extent is shown whole, widened along one axis to keep
        // the scales equal; else the bounds follow what is drawn.
        if let Some(e) = &scene.extent {
            plot = plot
                .include_x(e.x[0])
                .include_x(e.x[1])
                .include_y(e.y[0])
                .include_y(e.y[1])
                .set_margin_fraction(egui::Vec2::ZERO);
        }
        let follow = scene.extent.is_none();
        let kept = &mut self.kept;
        plot.show(ui, |plot| {
            scroll_to_zoom(plot);
            let t = plot.transform();
            let scale = t.dpos_dvalue_x().abs();
            let size = t.frame().size();
            let key = (
                stamps,
                scale.to_bits(),
                [size.x.to_bits(), size.y.to_bits()],
            );
            if kept.key.as_ref() != Some(&key) {
                let placed = scene.place(&input.recorded);
                kept.drawn = Arc::new(match scene.view(&input.recorded) {
                    Some(view) => eval::draw(&placed, view, scale),
                    None => Drawn::default(),
                });
                kept.key = Some(key);
                #[cfg(test)]
                {
                    kept.builds += 1;
                }
            }
            plot.add(SceneItem {
                drawn: kept.drawn.clone(),
                follow,
                base: PlotItemBase::new(SCENE.into()),
            });
        });
    }

    fn set_settings(
        &mut self,
        settings: &Settings,
        available: &Available<'_>,
    ) -> Result<(), String> {
        self.scene = Some(Scene::check(settings, available)?);
        self.given = settings.clone();
        self.kept = Kept::default();
        Ok(())
    }

    fn settings(&self) -> Settings {
        self.given.clone()
    }
}

/// The drawn list as an item of the plot, painted in the plot's
/// coordinates.
struct SceneItem {
    drawn: Arc<Drawn>,
    /// The plot's bounds follow what is drawn (no extent given).
    follow: bool,
    base: PlotItemBase,
}

impl PlotItem for SceneItem {
    fn shapes(&self, _ui: &egui::Ui, transform: &PlotTransform, shapes: &mut Vec<Shape>) {
        let at = |p: &[f64; 2]| transform.position_from_point(&PlotPoint::new(p[0], p[1]));
        let finite = |p: &Pos2| p.x.is_finite() && p.y.is_finite();
        // Triangles of neighbouring items share one mesh; anything else
        // drawn between them keeps the order.
        let mut mesh = Mesh::default();
        let flush = |mesh: &mut Mesh, shapes: &mut Vec<Shape>| {
            if !mesh.is_empty() {
                shapes.push(Shape::mesh(std::mem::take(mesh)));
            }
        };
        for item in &self.drawn.items {
            match item {
                Item::Triangles { points, colour } => {
                    for t in points.chunks_exact(3) {
                        let corners = [at(&t[0]), at(&t[1]), at(&t[2])];
                        if !corners.iter().all(finite) {
                            continue;
                        }
                        let first = mesh.vertices.len() as u32;
                        for c in corners {
                            mesh.colored_vertex(c, *colour);
                        }
                        mesh.add_triangle(first, first + 1, first + 2);
                    }
                }
                Item::Line {
                    points,
                    width,
                    colour,
                } => {
                    flush(&mut mesh, shapes);
                    let points: Vec<Pos2> = points.iter().map(at).collect();
                    if points.iter().all(finite) {
                        shapes.push(Shape::line(points, Stroke::new(*width, *colour)));
                    }
                }
                Item::Dot {
                    at: p,
                    radius,
                    colour,
                } => {
                    flush(&mut mesh, shapes);
                    let c = at(p);
                    if finite(&c) {
                        shapes.push(Shape::circle_filled(c, *radius, *colour));
                    }
                }
            }
        }
        flush(&mut mesh, shapes);
    }

    fn initialize(&mut self, _x_range: std::ops::RangeInclusive<f64>) {}

    fn color(&self) -> egui::Color32 {
        egui::Color32::TRANSPARENT
    }

    fn allow_hover(&self) -> bool {
        false
    }

    fn geometry(&self) -> PlotGeometry<'_> {
        PlotGeometry::None
    }

    /// What is drawn, for an extent that follows it.
    fn bounds(&self) -> PlotBounds {
        match self.drawn.bounds {
            Some([lo, hi]) if self.follow => PlotBounds::from_min_max(lo, hi),
            _ => PlotBounds::NOTHING,
        }
    }

    fn base(&self) -> &PlotItemBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut PlotItemBase {
        &mut self.base
    }
}

#[cfg(test)]
mod tests;
