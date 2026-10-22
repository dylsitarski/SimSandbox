//! The node canvas (ADR 0042): a [`Diagram`] drawn with egui's painter,
//! and the mouse and keyboard turned into [`Edit`]s. The app applies the
//! edits; headless rendering draws the same canvas without input.
//!
//! - Drag the background to pan; scroll to zoom around the pointer.
//! - Drag a block to move it; click to select it or a wire.
//! - Drag from an output to an input (or back) to connect; drag a wire's
//!   end off an input to move or remove it.
//! - Delete removes the selection; right-click for block and canvas menus.

use crate::diagram::{Diagram, Edit, Node, HEADER, NODE_WIDTH, ROW};
use egui::{Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use simsandbox_core::block::{Causality, Shape};
use simsandbox_core::registry::Registry;

/// What is selected.
#[derive(Clone, Debug, PartialEq)]
pub enum Selection {
    Block(String),
    /// A wire, by the input it feeds.
    Wire(String),
}

/// What the user asked for besides edits.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Open the block's definition (equations) or settings.
    Open(String),
}

/// A wire being drawn: from a port, to the pointer.
#[derive(Clone, Debug)]
struct Dragging {
    node: usize,
    port: usize,
    causality: Causality,
    /// The wire this drag lifted off an input, if any (`"block.port"`).
    lifted: Option<String>,
}

/// The canvas's view and interaction state, kept between frames.
#[derive(Clone, Debug)]
pub struct Canvas {
    /// Screen offset of the canvas origin from the panel's corner.
    pan: Vec2,
    zoom: f32,
    /// The panel size the view was fitted to; refitted when the panel
    /// changes size until the user pans or zooms.
    fitted: Option<Vec2>,
    user_moved: bool,
    pub selected: Option<Selection>,
    moving: Option<(usize, Vec2)>,
    wiring: Option<Dragging>,
    /// Where a context menu was opened, in canvas units, and on what.
    menu: Option<([f64; 2], Option<usize>)>,
    rename: String,
}

impl Default for Canvas {
    fn default() -> Self {
        Self {
            pan: Vec2::ZERO,
            zoom: 1.0,
            fitted: None,
            user_moved: false,
            selected: None,
            moving: None,
            wiring: None,
            menu: None,
            rename: String::new(),
        }
    }
}

/// Port dot radius and how close counts as on it, in points at zoom 1.
const DOT: f32 = 4.5;
const REACH: f32 = 9.0;

impl Canvas {
    /// Fit the diagram to the panel on the next frame.
    pub fn refit(&mut self) {
        self.fitted = None;
        self.user_moved = false;
    }

    fn to_screen(&self, origin: Pos2, p: [f64; 2]) -> Pos2 {
        origin + self.pan + Vec2::new(p[0] as f32, p[1] as f32) * self.zoom
    }

    fn to_canvas(&self, origin: Pos2, p: Pos2) -> [f64; 2] {
        let v = (p - origin - self.pan) / self.zoom;
        [v.x as f64, v.y as f64]
    }

    fn fit(&mut self, d: &Diagram, rect: Rect) {
        let Some([x0, y0, x1, y1]) = d.bounds() else {
            self.pan = Vec2::splat(40.0);
            self.zoom = 1.0;
            return;
        };
        let margin = 40.0;
        let w = (x1 - x0) as f32;
        let h = (y1 - y0) as f32;
        self.zoom = ((rect.width() - 2.0 * margin) / w)
            .min((rect.height() - 2.0 * margin) / h)
            .clamp(0.3, 1.3);
        let size = Vec2::new(w, h) * self.zoom;
        self.pan = (rect.size() - size) / 2.0 - Vec2::new(x0 as f32, y0 as f32) * self.zoom;
    }

    /// Draw the diagram filling `ui`'s space. With `interactive`, handle
    /// input and return the edits and requests the user made; `diagram`
    /// node positions follow drags as they happen.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        d: &mut Diagram,
        registry: &Registry,
        interactive: bool,
    ) -> (Vec<Edit>, Vec<Request>) {
        let mut edits = Vec::new();
        let mut requests = Vec::new();
        let sense = if interactive {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        };
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), sense);
        let origin = rect.min;
        let resized = self.fitted.is_some_and(|s| s != rect.size()) && !self.user_moved;
        if (self.fitted.is_none() || resized) && rect.width() > 1.0 {
            self.fit(d, rect);
            self.fitted = Some(rect.size());
        }
        let painter = ui.painter_at(rect);
        let visuals = ui.visuals().clone();
        painter.rect_filled(rect, CornerRadius::ZERO, visuals.extreme_bg_color);
        self.grid(&painter, rect, visuals.faint_bg_color);

        let pointer = response.hover_pos().or(response.interact_pointer_pos());
        if interactive {
            self.input(ui, &response, d, origin, pointer, &mut edits, &mut requests);
        }

        // Wires under nodes.
        for w in &d.wires {
            let lifted = self
                .wiring
                .as_ref()
                .is_some_and(|g| g.lifted.as_deref() == Some(w.to_name.as_str()));
            if lifted {
                continue;
            }
            let selected = self.selected == Some(Selection::Wire(w.to_name.clone()));
            let field = d.nodes[w.from.0].outputs[w.from.1].shape != Shape::Scalar;
            let width = if field { 3.5 } else { 2.0 } * self.zoom.sqrt();
            let colour = if selected {
                visuals.selection.stroke.color
            } else {
                visuals.widgets.inactive.fg_stroke.color
            };
            let path = self.wire_path(d, origin, w);
            painter.add(egui::Shape::line(path, Stroke::new(width, colour)));
        }
        if let (Some(g), Some(p)) = (&self.wiring, pointer) {
            let at = self.to_screen(origin, d.nodes[g.node].port_at(g.causality, g.port));
            let (a, b) = match g.causality {
                Causality::Output => (at, p),
                Causality::Input => (p, at),
            };
            let below = a.y.max(b.y);
            painter.add(egui::Shape::line(
                route(a, b, below, self.zoom),
                visuals.selection.stroke,
            ));
        }
        for (i, n) in d.nodes.iter().enumerate() {
            let selected = self.selected == Some(Selection::Block(n.name.clone()));
            self.node(&painter, &visuals, origin, n, selected);
            if interactive {
                // Hover help: the error, or a port's unit and shape.
                let r = self.node_rect(origin, n);
                if let Some(p) = pointer.filter(|p| r.expand(REACH).contains(*p)) {
                    let tip = if let Some(e) = &n.error {
                        Some(e.clone())
                    } else {
                        self.port_under(d, origin, p)
                            .filter(|&(k, _, _)| k == i)
                            .map(|(k, port, c)| {
                                let q = match c {
                                    Causality::Input => &d.nodes[k].inputs[port],
                                    Causality::Output => &d.nodes[k].outputs[port],
                                };
                                port_help(&n.name, q, c)
                            })
                    };
                    if let Some(tip) = tip {
                        response.clone().on_hover_text_at_pointer(tip);
                    }
                }
            }
        }
        if d.nodes.is_empty() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Empty model: right-click to add a block",
                FontId::proportional(14.0),
                visuals.weak_text_color(),
            );
        }

        if interactive {
            self.context_menu(&response, d, registry, &mut edits, &mut requests);
        }
        (edits, requests)
    }

    fn grid(&self, painter: &egui::Painter, rect: Rect, colour: Color32) {
        let step = 40.0 * self.zoom;
        if step < 8.0 {
            return;
        }
        let start = rect.min + Vec2::new(self.pan.x.rem_euclid(step), self.pan.y.rem_euclid(step));
        let mut x = start.x;
        while x < rect.max.x {
            let mut y = start.y;
            while y < rect.max.y {
                painter.circle_filled(Pos2::new(x, y), 1.2, colour);
                y += step;
            }
            x += step;
        }
    }

    fn node_rect(&self, origin: Pos2, n: &Node) -> Rect {
        Rect::from_min_size(
            self.to_screen(origin, n.at),
            Vec2::new(NODE_WIDTH as f32, n.height() as f32) * self.zoom,
        )
    }

    fn node(
        &self,
        painter: &egui::Painter,
        visuals: &egui::Visuals,
        origin: Pos2,
        n: &Node,
        selected: bool,
    ) {
        let z = self.zoom;
        let r = self.node_rect(origin, n);
        let radius = CornerRadius::same((6.0 * z).round().clamp(0.0, 255.0) as u8);
        painter.rect_filled(r, radius, visuals.window_fill);
        let header = Rect::from_min_size(r.min, Vec2::new(r.width(), HEADER as f32 * z));
        painter.rect_filled(
            header,
            CornerRadius {
                nw: radius.nw,
                ne: radius.ne,
                sw: 0,
                se: 0,
            },
            visuals.widgets.inactive.bg_fill,
        );
        let border = if n.error.is_some() {
            Stroke::new(2.0, visuals.error_fg_color)
        } else if selected {
            Stroke::new(2.0, visuals.selection.stroke.color)
        } else {
            visuals.window_stroke
        };
        painter.rect_stroke(r, radius, border, StrokeKind::Inside);
        let text = visuals.text_color();
        let weak = visuals.weak_text_color();
        let pad = 8.0 * z;
        painter.text(
            r.min + Vec2::new(pad, 5.0 * z),
            egui::Align2::LEFT_TOP,
            &n.name,
            FontId::proportional(14.0 * z),
            visuals.strong_text_color(),
        );
        let kind = if n.error.is_some() {
            "cannot be built (hover)".to_string()
        } else {
            format!("{} · {}", n.type_name, n.method)
        };
        painter.text(
            r.min + Vec2::new(pad, 22.0 * z),
            egui::Align2::LEFT_TOP,
            fit(&kind, ((NODE_WIDTH as f32 - 16.0) / 6.0) as usize),
            FontId::proportional(10.5 * z),
            weak,
        );
        for (c, ports) in [
            (Causality::Input, &n.inputs),
            (Causality::Output, &n.outputs),
        ] {
            for (k, p) in ports.iter().enumerate() {
                let at = self.to_screen(origin, n.port_at(c, k));
                let field = p.shape != Shape::Scalar;
                let colour = if field {
                    visuals.hyperlink_color
                } else {
                    visuals.widgets.active.fg_stroke.color
                };
                if field {
                    let s = Vec2::splat(DOT * z);
                    painter.rect_filled(
                        Rect::from_center_size(at, s * 2.0),
                        CornerRadius::ZERO,
                        colour,
                    );
                } else {
                    painter.circle_filled(at, DOT * z, colour);
                }
                let (offset, align) = match c {
                    Causality::Input => (Vec2::new(pad, 0.0), egui::Align2::LEFT_CENTER),
                    Causality::Output => (Vec2::new(-pad, 0.0), egui::Align2::RIGHT_CENTER),
                };
                let max = if n.inputs.is_empty() || n.outputs.is_empty() {
                    24
                } else {
                    12
                };
                painter.text(
                    at + offset,
                    align,
                    fit(&p.name, max),
                    FontId::proportional(12.0 * z),
                    text,
                );
            }
        }
    }

    /// The port under `p`: (node, port, causality).
    fn port_under(&self, d: &Diagram, origin: Pos2, p: Pos2) -> Option<(usize, usize, Causality)> {
        let reach = REACH * self.zoom.max(0.7);
        let mut best = None;
        let mut best_dist = reach;
        for (i, n) in d.nodes.iter().enumerate() {
            for (c, count) in [
                (Causality::Input, n.inputs.len()),
                (Causality::Output, n.outputs.len()),
            ] {
                for k in 0..count {
                    let dist = self.to_screen(origin, n.port_at(c, k)).distance(p);
                    if dist < best_dist {
                        best_dist = dist;
                        best = Some((i, k, c));
                    }
                }
            }
        }
        best
    }

    fn node_under(&self, d: &Diagram, origin: Pos2, p: Pos2) -> Option<usize> {
        // The topmost (last drawn) first.
        (0..d.nodes.len())
            .rev()
            .find(|&i| self.node_rect(origin, &d.nodes[i]).contains(p))
    }

    /// A wire's path on screen: a curve when it runs forwards, else
    /// around underneath both blocks.
    fn wire_path(&self, d: &Diagram, origin: Pos2, w: &crate::diagram::Wire) -> Vec<Pos2> {
        let a = self.to_screen(
            origin,
            d.nodes[w.from.0].port_at(Causality::Output, w.from.1),
        );
        let b = self.to_screen(origin, d.nodes[w.to.0].port_at(Causality::Input, w.to.1));
        let below = self
            .node_rect(origin, &d.nodes[w.from.0])
            .max
            .y
            .max(self.node_rect(origin, &d.nodes[w.to.0]).max.y);
        route(a, b, below, self.zoom)
    }

    fn wire_under(&self, d: &Diagram, origin: Pos2, p: Pos2) -> Option<String> {
        d.wires
            .iter()
            .find(|w| {
                self.wire_path(d, origin, w)
                    .windows(2)
                    .any(|s| segment_distance(p, s[0], s[1]) < 5.0)
            })
            .map(|w| w.to_name.clone())
    }

    #[allow(clippy::too_many_arguments)]
    fn input(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        d: &mut Diagram,
        origin: Pos2,
        pointer: Option<Pos2>,
        edits: &mut Vec<Edit>,
        requests: &mut Vec<Request>,
    ) {
        // Zoom around the pointer.
        if let Some(p) = response.hover_pos() {
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = pinch * (scroll / 200.0).exp();
            if (factor - 1.0).abs() > 1e-4 {
                self.user_moved = true;
                let before = self.to_canvas(origin, p);
                self.zoom = (self.zoom * factor).clamp(0.2, 3.0);
                let after = self.to_screen(origin, before);
                self.pan += p - after;
            }
        }
        if response.drag_started_by(egui::PointerButton::Primary) {
            // Where the press began: a drag is reported only once the
            // pointer has moved.
            let start = ui.input(|i| i.pointer.press_origin());
            if let Some(p) = start.or(response.interact_pointer_pos()) {
                if let Some((node, port, c)) = self.port_under(d, origin, p) {
                    let key = |n: usize, q: &str| format!("{}.{}", d.nodes[n].name, q);
                    // Dragging a connected input lifts its wire from the
                    // source end.
                    let fed = (c == Causality::Input)
                        .then(|| d.wires.iter().find(|w| w.to == (node, port)))
                        .flatten();
                    self.wiring = Some(match fed {
                        Some(w) => Dragging {
                            node: w.from.0,
                            port: w.from.1,
                            causality: Causality::Output,
                            lifted: Some(key(node, &d.nodes[node].inputs[port].name)),
                        },
                        None => Dragging {
                            node,
                            port,
                            causality: c,
                            lifted: None,
                        },
                    });
                } else if let Some(i) = self.node_under(d, origin, p) {
                    let grab = p - self.to_screen(origin, d.nodes[i].at);
                    self.moving = Some((i, grab));
                    self.selected = Some(Selection::Block(d.nodes[i].name.clone()));
                }
            }
        }
        if response.dragged_by(egui::PointerButton::Primary) {
            if let (Some((i, grab)), Some(p)) = (self.moving, pointer) {
                d.nodes[i].at = self.to_canvas(origin, p - grab);
            } else if self.wiring.is_none() {
                self.pan += response.drag_delta();
                self.user_moved = true;
            }
        }
        if response.drag_stopped() {
            if let Some((i, _)) = self.moving.take() {
                d.nodes[i].placed = true;
                edits.push(Edit::Move {
                    block: d.nodes[i].name.clone(),
                    at: d.nodes[i].at,
                });
            }
            if let Some(g) = self.wiring.take() {
                let target = pointer.and_then(|p| self.port_under(d, origin, p));
                let key = |n: usize, c: Causality, k: usize| {
                    let ports = match c {
                        Causality::Input => &d.nodes[n].inputs,
                        Causality::Output => &d.nodes[n].outputs,
                    };
                    format!("{}.{}", d.nodes[n].name, ports[k].name)
                };
                let here = key(g.node, g.causality, g.port);
                let new = match target {
                    Some((n, k, c)) if c != g.causality => {
                        let there = key(n, c, k);
                        Some(match c {
                            Causality::Input => (here.clone(), there),
                            Causality::Output => (there, here.clone()),
                        })
                    }
                    _ => None,
                };
                match (g.lifted, new) {
                    // Dropped back where it was.
                    (Some(old), Some((_, to))) if old == to => {}
                    (Some(old), new) => {
                        edits.push(Edit::Disconnect { to: old });
                        if let Some((from, to)) = new {
                            edits.push(Edit::Connect { from, to });
                        }
                    }
                    (None, Some((from, to))) => edits.push(Edit::Connect { from, to }),
                    (None, None) => {}
                }
            }
        }
        if response.clicked() {
            if let Some(p) = response.interact_pointer_pos() {
                self.selected = if let Some(i) = self.node_under(d, origin, p) {
                    Some(Selection::Block(d.nodes[i].name.clone()))
                } else {
                    self.wire_under(d, origin, p).map(Selection::Wire)
                };
            }
        }
        if response.double_clicked() {
            if let Some(i) = response
                .interact_pointer_pos()
                .and_then(|p| self.node_under(d, origin, p))
            {
                requests.push(Request::Open(d.nodes[i].name.clone()));
            }
        }
        if response.secondary_clicked() {
            if let Some(p) = response.interact_pointer_pos() {
                let on = self.node_under(d, origin, p);
                if let Some(i) = on {
                    self.selected = Some(Selection::Block(d.nodes[i].name.clone()));
                    self.rename = d.nodes[i].name.clone();
                }
                self.menu = Some((self.to_canvas(origin, p), on));
            }
        }
        let delete =
            ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace));
        let typing = ui.ctx().egui_wants_keyboard_input();
        if delete && !typing && response.hovered() {
            match self.selected.take() {
                Some(Selection::Block(block)) => edits.push(Edit::Delete { block }),
                Some(Selection::Wire(to)) => edits.push(Edit::Disconnect { to }),
                None => {}
            }
        }
    }

    fn context_menu(
        &mut self,
        response: &egui::Response,
        d: &Diagram,
        registry: &Registry,
        edits: &mut Vec<Edit>,
        requests: &mut Vec<Request>,
    ) {
        let Some((at, on)) = self.menu else {
            return;
        };
        let open = response.context_menu(|ui| match on {
            Some(i) => {
                let name = d.nodes[i].name.clone();
                ui.label(egui::RichText::new(&name).strong());
                if ui
                    .button("Edit…")
                    .on_hover_text("Its equations or settings")
                    .clicked()
                {
                    requests.push(Request::Open(name.clone()));
                    ui.close();
                }
                ui.horizontal(|ui| {
                    let field =
                        ui.add(egui::TextEdit::singleline(&mut self.rename).desired_width(110.0));
                    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (ui.button("Rename").clicked() || enter) && self.rename != name {
                        edits.push(Edit::Rename {
                            from: name.clone(),
                            to: self.rename.trim().to_string(),
                        });
                        ui.close();
                    }
                });
                if ui.button("Delete").clicked() {
                    edits.push(Edit::Delete { block: name });
                    ui.close();
                }
            }
            None => {
                ui.label("Add block");
                if let Some(type_name) = type_menu(ui, registry) {
                    edits.push(Edit::Add { type_name, at });
                    ui.close();
                }
            }
        });
        if open.is_none() {
            self.menu = None;
        }
    }
}

/// A menu of block types by domain, with their summaries on hover.
/// Returns the chosen type.
pub fn type_menu(ui: &mut egui::Ui, registry: &Registry) -> Option<String> {
    let mut domains: std::collections::BTreeMap<&str, Vec<&str>> = Default::default();
    for t in registry.type_names() {
        let domain = t.split_once('.').map_or("other", |(d, _)| d);
        domains.entry(domain).or_default().push(t);
    }
    let mut chosen = None;
    for (domain, types) in domains {
        let label = match domain {
            "eq" => "Equations",
            "agents" => "Agents",
            d => d,
        };
        ui.menu_button(label, |ui| {
            for t in types {
                let summary = registry.info(t).map_or("", |i| i.summary.as_str());
                if ui.button(t).on_hover_text(summary).clicked() {
                    chosen = Some(t.to_string());
                }
            }
        });
    }
    chosen
}

fn port_help(block: &str, p: &crate::diagram::Port, c: Causality) -> String {
    let dir = match c {
        Causality::Input => "input",
        Causality::Output => "output",
    };
    let shape = match p.shape {
        Shape::Scalar => "a number".to_string(),
        Shape::Vector(n) => format!("{n} numbers"),
        Shape::Grid { nx, ny } => format!("a {nx} × {ny} grid"),
        Shape::Points { count, dim } => format!("{count} points in {dim}D"),
        Shape::Population { dim } => format!("a population of points in {dim}D"),
    };
    let unit = if p.unit.is_empty() || p.unit == "1" {
        "no unit".to_string()
    } else {
        p.unit.clone()
    };
    format!("{block}.{} ({dir}): {shape}, {unit}", p.name)
}

/// Shorten text to `max` characters with an ellipsis.
fn fit(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

/// The points of a wire from output `a` to input `b`: an S-curve when `b`
/// is to the right of `a`; otherwise out to the right, down below
/// `below` (the lower block's bottom), across and back in from the left.
fn route(a: Pos2, b: Pos2, below: f32, zoom: f32) -> Vec<Pos2> {
    let out = 18.0 * zoom;
    if b.x >= a.x + 2.0 * out {
        let dx = ((b.x - a.x) / 2.0).max(40.0 * zoom);
        let curve = [a, a + Vec2::new(dx, 0.0), b - Vec2::new(dx, 0.0), b];
        return egui::epaint::CubicBezierShape::from_points_stroke(
            curve,
            false,
            Color32::TRANSPARENT,
            Stroke::NONE,
        )
        .flatten(Some(0.5));
    }
    let y = below + out;
    let corners = [
        a,
        Pos2::new(a.x + out, a.y),
        Pos2::new(a.x + out, y),
        Pos2::new(b.x - out, y),
        Pos2::new(b.x - out, b.y),
        b,
    ];
    rounded(&corners, 8.0 * zoom)
}

/// A polyline with its inner corners rounded by quadratic arcs of radius
/// about `r`.
fn rounded(p: &[Pos2], r: f32) -> Vec<Pos2> {
    let mut out = vec![p[0]];
    for k in 1..p.len() - 1 {
        let (prev, at, next) = (p[k - 1], p[k], p[k + 1]);
        let r = r.min(prev.distance(at) / 2.0).min(next.distance(at) / 2.0);
        let a = at + (prev - at).normalized() * r;
        let b = at + (next - at).normalized() * r;
        for i in 0..=6 {
            let t = i as f32 / 6.0;
            let q = a.lerp(at, t).lerp(at.lerp(b, t), t);
            out.push(q);
        }
    }
    out.push(p[p.len() - 1]);
    out
}

fn segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t).distance(p)
}

/// Rows of a node, for tests and layout: header plus ports.
pub fn rows(n: &Node) -> usize {
    ((n.height() - HEADER) / ROW).round() as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::examples::{registry, EXAMPLES};
    use simsandbox_file::ModelFile;

    /// Run the canvas for a frame with `events`, at 800 × 600.
    fn frame(
        ctx: &egui::Context,
        canvas: &mut Canvas,
        d: &mut Diagram,
        events: Vec<egui::Event>,
    ) -> (Vec<Edit>, Vec<Request>) {
        let r = registry();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
            events,
            ..Default::default()
        };
        let mut out = (Vec::new(), Vec::new());
        let mut output = ctx.run_ui(input, |ui| {
            out = canvas.show(ui, d, &r, true);
        });
        output.textures_delta.clear();
        out
    }

    fn press(p: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }
    }

    /// Drag from `a` to `b` over a few frames; returns the edits made.
    fn drag(ctx: &egui::Context, c: &mut Canvas, d: &mut Diagram, a: Pos2, b: Pos2) -> Vec<Edit> {
        let mut edits = Vec::new();
        let mid = a + (b - a) / 2.0;
        for events in [
            vec![egui::Event::PointerMoved(a)],
            vec![press(a, true)],
            vec![egui::Event::PointerMoved(mid)],
            vec![egui::Event::PointerMoved(b)],
            vec![press(b, false)],
            vec![],
        ] {
            edits.extend(frame(ctx, c, d, events).0);
        }
        edits
    }

    fn cruise() -> (Diagram, Canvas, egui::Context) {
        let e = EXAMPLES.iter().find(|e| e.name.contains("Cruise")).unwrap();
        let (f, _) = ModelFile::from_json(e.text).unwrap();
        let d = Diagram::new(&f, &registry());
        let ctx = egui::Context::default();
        let mut c = Canvas::default();
        let mut d2 = d.clone();
        frame(&ctx, &mut c, &mut d2, vec![]);
        (d, c, ctx)
    }

    fn screen(c: &Canvas, d: &Diagram, block: &str, causality: Causality, port: &str) -> Pos2 {
        let n = &d.nodes[d.node(block).unwrap()];
        let k = match causality {
            Causality::Input => n.inputs.iter().position(|p| p.name == port),
            Causality::Output => n.outputs.iter().position(|p| p.name == port),
        }
        .unwrap();
        c.to_screen(Pos2::ZERO, n.port_at(causality, k))
    }

    #[test]
    fn dragging_a_block_moves_it_and_saves_where_it_ended() {
        let (mut d, mut c, ctx) = cruise();
        let n = d.node("car").unwrap();
        let start = d.nodes[n].at;
        let grab = c.to_screen(Pos2::ZERO, [start[0] + 60.0, start[1] + 10.0]);
        let edits = drag(&ctx, &mut c, &mut d, grab, grab + Vec2::new(100.0, 50.0));
        let [Edit::Move { block, at }] = &edits[..] else {
            panic!("{edits:?}");
        };
        assert_eq!(block, "car");
        let dx = (at[0] - start[0]) * c.zoom as f64;
        assert!((dx - 100.0).abs() < 1.0, "{dx}");
        assert_eq!(d.nodes[n].at, *at);
    }

    #[test]
    fn wires_are_drawn_from_outputs_to_inputs_and_lifted_off_inputs() {
        let (mut d, mut c, ctx) = cruise();
        // A new wire: cruise.e → car.F (replacing cruise.F → car.F).
        let a = screen(&c, &d, "cruise", Causality::Output, "e");
        let b = screen(&c, &d, "car", Causality::Input, "F");
        let edits = drag(&ctx, &mut c, &mut d, a, b);
        assert_eq!(
            edits,
            [Edit::Connect {
                from: "cruise.e".into(),
                to: "car.F".into()
            }]
        );
        // Backwards, input to output, works too.
        let edits = drag(&ctx, &mut c, &mut d, b, a);
        // car.F is fed, so this lifts its wire and drops it on an output:
        // not an input, so the wire is removed.
        assert_eq!(edits, [Edit::Disconnect { to: "car.F".into() }]);
        // Lifting a wire and dropping it on empty canvas removes it.
        let v = screen(&c, &d, "cruise", Causality::Input, "v");
        let edits = drag(&ctx, &mut c, &mut d, v, Pos2::new(5.0, 5.0));
        assert_eq!(
            edits,
            [Edit::Disconnect {
                to: "cruise.v".into()
            }]
        );
    }

    #[test]
    fn clicking_selects_and_delete_removes() {
        let (mut d, mut c, ctx) = cruise();
        let n = &d.nodes[d.node("cruise").unwrap()];
        let p = c.to_screen(Pos2::ZERO, [n.at[0] + 50.0, n.at[1] + 5.0]);
        frame(&ctx, &mut c, &mut d, vec![egui::Event::PointerMoved(p)]);
        frame(&ctx, &mut c, &mut d, vec![press(p, true), press(p, false)]);
        assert_eq!(c.selected, Some(Selection::Block("cruise".into())));
        let key = egui::Event::Key {
            key: egui::Key::Delete,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        };
        let (edits, _) = frame(&ctx, &mut c, &mut d, vec![key]);
        assert_eq!(
            edits,
            [Edit::Delete {
                block: "cruise".into()
            }]
        );
    }

    #[test]
    fn zooming_keeps_the_point_under_the_pointer() {
        let (mut d, mut c, ctx) = cruise();
        let p = Pos2::new(300.0, 200.0);
        let before = c.to_canvas(Pos2::ZERO, p);
        frame(&ctx, &mut c, &mut d, vec![egui::Event::PointerMoved(p)]);
        let zoom = c.zoom;
        frame(
            &ctx,
            &mut c,
            &mut d,
            vec![egui::Event::Zoom(1.5), egui::Event::PointerMoved(p)],
        );
        assert!((c.zoom / zoom - 1.5).abs() < 1e-3, "{} {}", c.zoom, zoom);
        let after = c.to_canvas(Pos2::ZERO, p);
        assert!((before[0] - after[0]).abs() < 0.5 && (before[1] - after[1]).abs() < 0.5);
        assert_eq!(rows(&d.nodes[0]), 1);
    }
}
