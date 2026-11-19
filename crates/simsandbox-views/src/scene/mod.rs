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
//! scale stay the same.
//!
//! Grabbing (ADR 0093 point 6, ADR 0101 point 3): pressing on an instance
//! of a layer with a `grab` holds it until the button is let go. While it
//! is held the view sends that layer's `input.pointer` block the pointer
//! in the scene's own frame, `down` 1 and the instance's row id
//! (`picked`), every frame, and pan and zoom pause; on release it sends
//! `down` 0 and `picked` −1 once. The press is tested against the picture
//! on screen (the list last drawn, through the transform last shown) with
//! each primitive's exact containment. The app records what is sent as
//! input; a replay reads the recording, never the hit test.

pub mod eval;
pub mod geometry;
pub mod spec;

use crate::builtin::scroll_to_zoom;
use crate::history::Stamp;
use crate::view::{Available, Settings, View, ViewInput, ViewRegistry, ViewType};
use egui::{epaint::Mesh, Pos2, Shape, Stroke};
use egui_plot::{Plot, PlotBounds, PlotGeometry, PlotItem, PlotItemBase, PlotPoint, PlotTransform};
use eval::{Drawn, Placed};
use geometry::{Item, Pose, P};
pub use spec::{bound, grabs, reads, Scene};
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
    /// The instances the list was drawn from, in the scene's own frame,
    /// and what the picture was seen through (`view_from`): what a press
    /// is tested against.
    placed: Vec<Placed>,
    view: Option<Pose>,
    /// Times the list was built.
    #[cfg(test)]
    builds: usize,
}

/// An instance held by the pointer (ADR 0093 point 6).
#[derive(Debug, Clone, PartialEq)]
struct Held {
    /// The `input.pointer` block its layer grabs with.
    block: String,
    /// Its row id, 0 for a layer of one instance.
    picked: f64,
    /// The pointer, last seen, in the scene's own frame.
    at: P,
}

#[derive(Default)]
struct SceneView {
    /// The settings as given, saved back unchanged.
    given: Settings,
    scene: Option<Scene>,
    kept: Kept,
    /// The plot's transform as last shown: where on screen the picture
    /// the user sees lies.
    shown: Option<PlotTransform>,
    held: Option<Held>,
    /// What grabbing sent, not yet taken.
    sent: Vec<(String, [f64; 4])>,
}

impl SceneView {
    /// How many times the list was built (for tests of the cache).
    #[cfg(test)]
    fn builds(&self) -> usize {
        self.kept.builds
    }

    /// The pointer's place on screen in the scene's own frame (ADR 0093
    /// point 4: grabbing is never relative to a frame), through the
    /// picture last shown.
    fn in_scene(&self, pos: Pos2) -> Option<P> {
        let t = self.shown?;
        let v = t.value_from_position(pos);
        let p = self.kept.view?.inverse().apply([v.x, v.y]);
        (p[0].is_finite() && p[1].is_finite()).then_some(p)
    }

    /// Grabbing, before the plot is drawn (so a press on an instance never
    /// pans it): a press on an instance of a grabbing layer holds it; while
    /// held the pointer is sent each frame; release sends `down` 0 and
    /// `picked` −1.
    fn grab(&mut self, ui: &egui::Ui) {
        let (pressed, origin, down, latest) = ui.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.press_origin(),
                i.pointer.primary_down(),
                i.pointer.latest_pos(),
            )
        });
        if let Some(mut held) = self.held.take() {
            if let Some(p) = latest.and_then(|p| self.in_scene(p)) {
                held.at = p;
            }
            let [x, y] = held.at;
            if down {
                self.sent
                    .push((held.block.clone(), [x, y, 1.0, held.picked]));
                self.held = Some(held);
            } else {
                self.sent.push((held.block, [x, y, 0.0, NOTHING_PICKED]));
            }
            return;
        }
        let (true, Some(press), Some(scene), Some(t)) = (pressed, origin, &self.scene, self.shown)
        else {
            return;
        };
        // On the picture, and not under a window or menu over it.
        let on_top = ui
            .ctx()
            .layer_id_at(press)
            .is_none_or(|l| l == ui.layer_id());
        if !(t.frame().contains(press) && ui.clip_rect().contains(press) && on_top) {
            return;
        }
        let Some(at) = self.in_scene(press) else {
            return;
        };
        let scale = t.dpos_dvalue_x().abs();
        let Some(hit) = eval::picked(scene, &self.kept.placed, at, scale) else {
            return;
        };
        let block = scene.layers[hit.layer]
            .grab
            .clone()
            .expect("only grabbing layers are picked");
        // Ids are integers, exact in an f64 up to 2^53.
        let picked = hit.id.map_or(0.0, |id| id as f64);
        let held = Held { block, picked, at };
        // Pressed and let go within one frame: held for this frame, let
        // go in the next.
        self.sent
            .push((held.block.clone(), [at[0], at[1], 1.0, picked]));
        self.held = Some(held);
    }

    /// Let go of what is held, as a release would.
    fn let_go(&mut self) {
        if let Some(h) = self.held.take() {
            self.sent
                .push((h.block, [h.at[0], h.at[1], 0.0, NOTHING_PICKED]));
        }
    }
}

/// `picked` while nothing is held (as `input.pointer` defines it).
const NOTHING_PICKED: f64 = -1.0;

impl View for SceneView {
    fn show(&mut self, ui: &mut egui::Ui, input: &ViewInput<'_>) {
        self.grab(ui);
        let Some(scene) = &self.scene else {
            return;
        };
        // Pan and zoom pause while an instance is held.
        let free = self.held.is_none();
        let stamps = scene.stamps(&input.recorded);
        let label = |axis: &str| format!("{axis} [{}]", scene.unit);
        let mut plot = Plot::new(SCENE)
            .data_aspect(1.0)
            .allow_scroll(false)
            .allow_drag(free)
            .allow_zoom(free)
            .allow_boxed_zoom(free)
            .allow_double_click_reset(free)
            .allow_axis_zoom_drag(free)
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
        let response = plot.show(ui, |plot| {
            if free {
                scroll_to_zoom(plot);
            }
            let t = plot.transform();
            let scale = t.dpos_dvalue_x().abs();
            let size = t.frame().size();
            let key = (
                stamps,
                scale.to_bits(),
                [size.x.to_bits(), size.y.to_bits()],
            );
            if kept.key.as_ref() != Some(&key) {
                kept.placed = scene.place(&input.recorded);
                kept.view = scene.view(&input.recorded);
                kept.drawn = Arc::new(match kept.view {
                    Some(view) => eval::draw(&kept.placed, view, scale),
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
        self.shown = Some(response.transform);
    }

    fn grabs(&mut self) -> Vec<(String, [f64; 4])> {
        std::mem::take(&mut self.sent)
    }

    fn set_settings(
        &mut self,
        settings: &Settings,
        available: &Available<'_>,
    ) -> Result<(), String> {
        let scene = Scene::check(settings, available)?;
        // What was held may be gone: let it go, so its block is not left
        // pressed.
        self.let_go();
        self.scene = Some(scene);
        self.given = settings.clone();
        self.kept = Kept::default();
        self.shown = None;
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
