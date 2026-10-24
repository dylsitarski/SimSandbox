//! The node canvas (ADR 0042): a [`Diagram`] drawn with egui's painter,
//! and the mouse and keyboard turned into [`Edit`]s. The app applies the
//! edits; headless rendering draws the same canvas without input.
//!
//! - Drag the background to pan; scroll to zoom around the pointer.
//! - Drag a block to move it; click to select it or a wire.
//! - Drag from an output to an input (or back) to connect; drag a wire's
//!   end off an input to move or remove it.
//! - Shift-click adds a block to the selection; Shift-drag on the
//!   background selects every block in a box; dragging a selected block
//!   moves the selection.
//! - Delete removes the selection; right-click for block and canvas menus,
//!   among them "Make subsystem" (ADR 0043).
//! - The data layer (ADR 0050), when the diagram has one: filters and views
//!   as nodes with dashed outlines, read by dashed wires. Dropping a wire
//!   on a view's slot shows that signal there; a filter dropped on a block
//!   input is refused with an offer to promote it to a block and connect.

use crate::diagram::{Diagram, Edit, Node, NodeKind, Scope, HEADER, NODE_WIDTH, ROW, VALUE_ROW};
use egui::{Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use simsandbox_core::block::{Causality, Shape};
use simsandbox_core::registry::Registry;
use std::collections::BTreeSet;

/// What is selected.
#[derive(Clone, Debug, PartialEq)]
pub enum Selection {
    Block(String),
    /// Several blocks.
    Many(Vec<String>),
    /// A wire, by the input it feeds.
    Wire(String),
}

impl Selection {
    /// The selected nodes' names.
    pub fn blocks(&self) -> Vec<String> {
        match self {
            Selection::Block(b) => vec![b.clone()],
            Selection::Many(v) => v.clone(),
            Selection::Wire(_) => Vec::new(),
        }
    }
}

/// What the user asked for besides edits.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Open the block's definition (equations), settings or, for a
    /// subsystem, its inside.
    Open(String),
    /// Edit a filter in the Data window.
    EditData(String),
    /// Show a view's tile, by its place among the views.
    ShowView(usize),
    /// A value typed on a node (ADR 0053): `"block.param"` and the value.
    SetValue { key: String, value: f64 },
    /// `= expression` typed on a node (ADR 0056): `"block.param"` and the
    /// formula.
    SetFormula { key: String, formula: String },
    /// Nodes were opened or closed: every open node now, as
    /// `controls.open` lists them (ADR 0055).
    Opened(Vec<String>),
    /// A conversion's unit typed beside its port (ADR 0045): the block,
    /// `from` or `to`, and the unit.
    SetSetting {
        block: String,
        key: String,
        text: String,
    },
}

/// Values show on nodes from this zoom up, and can be edited from this one.
const SHOW_VALUES: f32 = 0.55;
const EDIT_VALUES: f32 = 0.75;

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
    /// Nodes being dragged, each with where the pointer holds it.
    moving: Vec<(usize, Vec2)>,
    /// Where a Shift-drag selection box started, on screen.
    band: Option<Pos2>,
    wiring: Option<Dragging>,
    /// Where a context menu was opened, in canvas units, and on what.
    menu: Option<([f64; 2], Option<usize>)>,
    rename: String,
    /// A filter dropped on a model input: the filter, the input and where
    /// to offer promoting it.
    offer: Option<(String, String, Pos2)>,
    /// Nodes whose values are shown (ADR 0055): model-level names, and
    /// `subsystem::block` inside a subsystem.
    pub open: BTreeSet<String>,
    /// Show every node's values.
    open_all: bool,
    /// The value being typed on a node: its key (`unit:block:from` for a
    /// conversion's unit) and the text so far.
    editing: Option<(String, String)>,
}

impl Default for Canvas {
    fn default() -> Self {
        Self {
            pan: Vec2::ZERO,
            zoom: 1.0,
            fitted: None,
            user_moved: false,
            selected: None,
            moving: Vec::new(),
            band: None,
            wiring: None,
            menu: None,
            rename: String::new(),
            offer: None,
            open: BTreeSet::new(),
            open_all: false,
            editing: None,
        }
    }
}

/// Port dot radius and how close counts as on it, in points at zoom 1.
const DOT: f32 = 4.5;
const REACH: f32 = 9.0;

impl Canvas {
    /// A canvas showing every node's values.
    pub fn all_open() -> Self {
        Self {
            open_all: true,
            ..Self::default()
        }
    }

    /// A canvas showing the values of these nodes (`controls.open`).
    pub fn opening(open: &[String]) -> Self {
        Self {
            open: open.iter().cloned().collect(),
            ..Self::default()
        }
    }

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
        for n in &mut d.nodes {
            let key = open_key(&d.scope, &n.name);
            n.open = !n.values.is_empty() && (self.open_all || self.open.contains(&key));
        }
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
            if lifted && !w.is_data() {
                continue;
            }
            let selected = self.selected == Some(Selection::Wire(w.key()));
            let field = d.nodes[w.from.0].outputs[w.from.1].shape != Shape::Scalar;
            let width = if field { 3.5 } else { 2.0 } * self.zoom.sqrt();
            let colour = if selected {
                visuals.selection.stroke.color
            } else {
                visuals.widgets.inactive.fg_stroke.color
            };
            let path = self.wire_path(d, origin, w);
            if w.is_data() {
                // Recorded data, read after each step (ADR 0050).
                let colour = if selected {
                    colour
                } else {
                    visuals.hyperlink_color.gamma_multiply(0.8)
                };
                painter.extend(egui::Shape::dashed_line(
                    &path,
                    Stroke::new(width * 0.8, colour),
                    6.0 * self.zoom,
                    4.0 * self.zoom,
                ));
            } else {
                painter.add(egui::Shape::line(path, Stroke::new(width, colour)));
            }
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
            let selected = self.is_selected(&n.name);
            self.node(&painter, &visuals, origin, n, selected);
            if interactive {
                // Hover help: the error, or a port's unit and shape.
                let r = self.node_rect(origin, n);
                if let Some(p) = pointer.filter(|p| r.expand(REACH).contains(*p)) {
                    let value = self.value_under(d, origin, p).filter(|&(k, _)| k == i);
                    let unit = self.unit_under(d, origin, p).filter(|&(k, _)| k == i);
                    let tip = if let Some(e) = &n.error {
                        Some(e.clone())
                    } else if self.expander(origin, n).is_some_and(|r| r.contains(p)) {
                        Some(if n.open {
                            "Hide its values".to_string()
                        } else {
                            format!("Show its values ({})", n.values.len())
                        })
                    } else if let Some((_, k)) = value {
                        let v = &n.values[k];
                        let mut tip = if self.zoom >= EDIT_VALUES {
                            format!(
                                "{}: double-click to change it (a number, or '= expression' \
                                 of the parameters)",
                                v.key
                            )
                        } else {
                            format!("{}: zoom in to change it here", v.key)
                        };
                        if d.scope != Scope::Model {
                            tip.push_str(
                                "\nThe default of every use of this subsystem; changing it \
                                 restarts the run",
                            );
                            if !v.overridden.is_empty() {
                                tip.push_str(&format!(
                                    "\n* Set by its own value in {}",
                                    v.overridden.join(", ")
                                ));
                            }
                        }
                        Some(tip)
                    } else if let Some((_, c)) = unit {
                        let setting = unit_setting(n, c).unwrap_or_default();
                        Some(if self.zoom >= EDIT_VALUES {
                            format!("The unit it converts {setting}: double-click to change it")
                        } else {
                            format!("The unit it converts {setting}: zoom in to change it here")
                        })
                    } else if n.is_observer() && self.port_under(d, origin, p).is_none() {
                        Some(observer_help(n))
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
        if let (Some(a), Some(b)) = (self.band, pointer) {
            let r = Rect::from_two_pos(a, b);
            painter.rect_filled(
                r,
                CornerRadius::ZERO,
                visuals.selection.bg_fill.gamma_multiply(0.2),
            );
            painter.rect_stroke(
                r,
                CornerRadius::ZERO,
                visuals.selection.stroke,
                StrokeKind::Inside,
            );
        }
        if d.nodes.is_empty() {
            let empty = match d.scope {
                Scope::Model => "Empty model: right-click to add a block",
                Scope::Subsystem(_) => {
                    "Empty subsystem: right-click to add blocks, inputs and outputs"
                }
            };
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                empty,
                FontId::proportional(14.0),
                visuals.weak_text_color(),
            );
        }

        if interactive {
            self.value_editor(ui, d, origin, &mut requests);
            self.context_menu(&response, d, registry, &mut edits, &mut requests);
            self.promote_offer(ui, &mut edits);
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

    fn is_selected(&self, name: &str) -> bool {
        match &self.selected {
            Some(Selection::Block(b)) => b == name,
            Some(Selection::Many(v)) => v.iter().any(|b| b == name),
            _ => false,
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
        let header_fill = match n.kind {
            NodeKind::Block => visuals.widgets.inactive.bg_fill,
            NodeKind::Subsystem => visuals.selection.bg_fill.gamma_multiply(0.45),
            NodeKind::Input | NodeKind::Output => visuals.warn_fg_color.gamma_multiply(0.25),
            NodeKind::Filter => visuals.hyperlink_color.gamma_multiply(0.3),
            NodeKind::View => visuals.hyperlink_color.gamma_multiply(0.15),
        };
        painter.rect_filled(
            header,
            CornerRadius {
                nw: radius.nw,
                ne: radius.ne,
                sw: 0,
                se: 0,
            },
            header_fill,
        );
        let border = if n.error.is_some() {
            Stroke::new(2.0, visuals.error_fg_color)
        } else if selected {
            Stroke::new(2.0, visuals.selection.stroke.color)
        } else {
            visuals.window_stroke
        };
        if n.is_observer() {
            // Dashed: on the data layer, not part of the model.
            let colour = if n.error.is_some() || selected {
                border.color
            } else {
                visuals.hyperlink_color
            };
            let c = [
                r.left_top(),
                r.right_top(),
                r.right_bottom(),
                r.left_bottom(),
                r.left_top(),
            ];
            painter.extend(egui::Shape::dashed_line(
                &c,
                Stroke::new(border.width.max(1.2), colour),
                5.0 * z,
                3.0 * z,
            ));
        } else {
            painter.rect_stroke(r, radius, border, StrokeKind::Inside);
        }
        if n.kind == NodeKind::Subsystem {
            // A second outline: there is more inside.
            painter.rect_stroke(
                r.shrink(3.0 * z),
                radius,
                Stroke::new(1.0, border.color),
                StrokeKind::Inside,
            );
        }
        let text = visuals.text_color();
        let weak = visuals.weak_text_color();
        let pad = 8.0 * z;
        let title = title(n);
        painter.text(
            r.min + Vec2::new(pad, 5.0 * z),
            egui::Align2::LEFT_TOP,
            &title,
            FontId::proportional(14.0 * z),
            visuals.strong_text_color(),
        );
        let kind = if n.error.is_some() {
            "cannot be built (hover)".to_string()
        } else {
            match n.kind {
                NodeKind::Block => format!("{} · {}", n.type_name, n.method),
                NodeKind::Subsystem => format!("{} · subsystem", n.type_name),
                NodeKind::Input => "subsystem input".into(),
                NodeKind::Output => "subsystem output".into(),
                NodeKind::Filter => format!("filter · {}", n.method),
                NodeKind::View => format!("view · {}", n.method),
            }
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
                // A view's slot also names what it reads without a wire
                // (`x · t`).
                let label = match n.kind {
                    NodeKind::View => {
                        let head = format!("{}: ", p.name);
                        let more: Vec<&str> = n
                            .unreached
                            .iter()
                            .filter_map(|u| u.strip_prefix(&head))
                            .collect();
                        if more.is_empty() {
                            p.name.clone()
                        } else {
                            format!("{} · {}", p.name, more.join(", "))
                        }
                    }
                    // A conversion shows its units (ADR 0045).
                    _ => match unit_setting(n, c).filter(|_| k == 0) {
                        Some(setting) => {
                            let unit = n.settings.get(setting).unwrap_or(&p.unit);
                            if self
                                .editing
                                .as_ref()
                                .is_some_and(|(key, _)| *key == unit_key(&n.name, setting))
                            {
                                p.name.clone()
                            } else {
                                match c {
                                    Causality::Input => format!("{} [{unit}]", p.name),
                                    Causality::Output => format!("[{unit}] {}", p.name),
                                }
                            }
                        }
                        None => p.name.clone(),
                    },
                };
                painter.text(
                    at + offset,
                    align,
                    fit(&label, max),
                    FontId::proportional(12.0 * z),
                    text,
                );
            }
        }
        if let Some(e) = self.expander(origin, n) {
            // A triangle: right while closed, down while open, after how
            // many values there are.
            let c = e.center();
            let h = 4.0 * z;
            let points = if n.open {
                vec![
                    c + Vec2::new(-h, -h * 0.6),
                    c + Vec2::new(h, -h * 0.6),
                    c + Vec2::new(0.0, h * 0.8),
                ]
            } else {
                vec![
                    c + Vec2::new(-h * 0.6, -h),
                    c + Vec2::new(h * 0.8, 0.0),
                    c + Vec2::new(-h * 0.6, h),
                ]
            };
            painter.add(egui::Shape::convex_polygon(points, text, Stroke::NONE));
            if !n.open {
                painter.text(
                    Pos2::new(e.min.x, c.y),
                    egui::Align2::RIGHT_CENTER,
                    n.values.len().to_string(),
                    FontId::proportional(10.5 * z),
                    weak,
                );
            }
        }
        self.values(painter, visuals, origin, n);
    }

    /// Where a node's values are opened and closed: the top-right corner
    /// of its header, on nodes with values, when they can be shown.
    fn expander(&self, origin: Pos2, n: &Node) -> Option<Rect> {
        if n.values.is_empty() || self.zoom < SHOW_VALUES {
            return None;
        }
        let r = self.node_rect(origin, n);
        let s = 18.0 * self.zoom;
        Some(Rect::from_min_size(
            Pos2::new(r.max.x - s - 4.0 * self.zoom, r.min.y + 4.0 * self.zoom),
            Vec2::splat(s),
        ))
    }

    /// The conversion unit label under `p`: node and which port.
    fn unit_under(&self, d: &Diagram, origin: Pos2, p: Pos2) -> Option<(usize, Causality)> {
        d.nodes.iter().enumerate().rev().find_map(|(i, n)| {
            [Causality::Input, Causality::Output]
                .into_iter()
                .find(|&c| {
                    unit_setting(n, c).is_some()
                        && self.unit_rect(origin, n, c).is_some_and(|r| r.contains(p))
                })
                .map(|c| (i, c))
        })
    }

    /// Where a conversion's unit is written beside port 0.
    fn unit_rect(&self, origin: Pos2, n: &Node, c: Causality) -> Option<Rect> {
        let count = match c {
            Causality::Input => n.inputs.len(),
            Causality::Output => n.outputs.len(),
        };
        if count == 0 {
            return None;
        }
        let z = self.zoom;
        let at = self.to_screen(origin, n.port_at(c, 0));
        let half = NODE_WIDTH as f32 * z / 2.0;
        let pad = 8.0 * z;
        let h = ROW as f32 * z;
        Some(match c {
            Causality::Input => Rect::from_min_size(
                Pos2::new(at.x + pad, at.y - h / 2.0),
                Vec2::new(half - pad, h),
            ),
            Causality::Output => Rect::from_min_size(
                Pos2::new(at.x - half, at.y - h / 2.0),
                Vec2::new(half - pad, h),
            ),
        })
    }

    /// A node's parameter values, below its ports (ADR 0053).
    fn values(&self, painter: &egui::Painter, visuals: &egui::Visuals, origin: Pos2, n: &Node) {
        if !n.open || self.zoom < SHOW_VALUES {
            return;
        }
        let z = self.zoom;
        let pad = 8.0 * z;
        let top = self.to_screen(origin, n.value_at(0));
        painter.line_segment(
            [
                top + Vec2::new(pad, 0.0),
                top + Vec2::new(NODE_WIDTH as f32 * z - pad, 0.0),
            ],
            Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color),
        );
        let font = FontId::proportional(11.0 * z);
        for (k, v) in n.values.iter().enumerate() {
            if self.editing.as_ref().is_some_and(|(key, _)| *key == v.key) {
                continue;
            }
            let y = self.to_screen(origin, n.value_at(k)).y + VALUE_ROW as f32 * z / 2.0;
            let left = Pos2::new(top.x + pad, y);
            let name = if v.initial {
                format!("{} ↻", v.name)
            } else {
                v.name.clone()
            };
            painter.text(
                left,
                egui::Align2::LEFT_CENTER,
                fit(&name, 12),
                font.clone(),
                visuals.weak_text_color(),
            );
            // `*`: some uses of the subsystem set their own.
            let mark = if v.overridden.is_empty() { "" } else { " *" };
            let shown = format!("{} {}{mark}", format_number(v.value), v.unit);
            painter.text(
                Pos2::new(top.x + NODE_WIDTH as f32 * z - pad, y),
                egui::Align2::RIGHT_CENTER,
                fit(shown.trim(), 16),
                font.clone(),
                visuals.text_color(),
            );
        }
    }

    /// The value row under `p`: node and value.
    fn value_under(&self, d: &Diagram, origin: Pos2, p: Pos2) -> Option<(usize, usize)> {
        if self.zoom < SHOW_VALUES {
            return None;
        }
        d.nodes.iter().enumerate().rev().find_map(|(i, n)| {
            (0..n.value_rows()).find_map(|k| {
                let r = Rect::from_min_size(
                    self.to_screen(origin, n.value_at(k)),
                    Vec2::new(NODE_WIDTH as f32, VALUE_ROW as f32) * self.zoom,
                );
                r.contains(p).then_some((i, k))
            })
        })
    }

    /// The field a value is typed into, where it is drawn. Enter sets it;
    /// Escape or clicking away leaves it.
    fn value_editor(
        &mut self,
        ui: &mut egui::Ui,
        d: &Diagram,
        origin: Pos2,
        requests: &mut Vec<Request>,
    ) {
        let Some((key, mut text)) = self.editing.take() else {
            return;
        };
        let z = self.zoom;
        // A conversion's unit, or a value.
        let unit = key
            .strip_prefix("unit:")
            .and_then(|rest| rest.rsplit_once(':'))
            .map(|(block, setting)| (block.to_string(), setting.to_string()));
        let rect = match &unit {
            Some((block, setting)) => {
                let c = match setting.as_str() {
                    "from" => Causality::Input,
                    _ => Causality::Output,
                };
                let Some(r) = d
                    .node(block)
                    .and_then(|i| self.unit_rect(origin, &d.nodes[i], c))
                else {
                    return;
                };
                // Beside the port's name.
                match c {
                    Causality::Input => Rect::from_min_max(r.min + Vec2::new(14.0 * z, 0.0), r.max),
                    Causality::Output => {
                        Rect::from_min_max(r.min, r.max - Vec2::new(14.0 * z, 0.0))
                    }
                }
            }
            None => {
                let Some((i, k)) = d.nodes.iter().enumerate().find_map(|(i, n)| {
                    n.values
                        .iter()
                        .take(n.value_rows())
                        .position(|v| v.key == key)
                        .map(|k| (i, k))
                }) else {
                    return;
                };
                let at = self.to_screen(origin, d.nodes[i].value_at(k));
                Rect::from_min_size(
                    at + Vec2::new(NODE_WIDTH as f32 * z * 0.45, 0.0),
                    Vec2::new(NODE_WIDTH as f32 * z * 0.5, VALUE_ROW as f32 * z),
                )
            }
        };
        let r = ui.put(
            rect,
            egui::TextEdit::singleline(&mut text)
                .font(FontId::proportional(11.0 * z))
                .margin(Vec2::new(2.0, 0.0)),
        );
        if !r.has_focus() && !r.lost_focus() {
            r.request_focus();
        }
        let (enter, escape) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Enter),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if escape {
            return;
        }
        if r.lost_focus() {
            if enter {
                match (unit, text.trim().parse::<f64>()) {
                    (Some((block, setting)), _) => {
                        if !text.trim().is_empty() {
                            requests.push(Request::SetSetting {
                                block,
                                key: setting,
                                text: text.trim().to_string(),
                            });
                        }
                    }
                    (None, Ok(value)) if value.is_finite() => {
                        requests.push(Request::SetValue { key, value })
                    }
                    (None, _) if text.trim().starts_with('=') => {
                        requests.push(Request::SetFormula {
                            key,
                            formula: text.trim()[1..].trim().to_string(),
                        })
                    }
                    _ => {}
                }
            }
            return;
        }
        self.editing = Some((key, text));
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
            .map(|w| w.key())
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
                    // A data-layer input may read several signals: dragging
                    // from it starts a new wire instead.
                    let fed = (c == Causality::Input && !d.nodes[node].is_observer())
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
                    // A selected block moves the whole selection.
                    if !self.is_selected(&d.nodes[i].name) {
                        self.selected = Some(Selection::Block(d.nodes[i].name.clone()));
                    }
                    let group = self
                        .selected
                        .as_ref()
                        .map(Selection::blocks)
                        .unwrap_or_default();
                    self.moving = d
                        .nodes
                        .iter()
                        .enumerate()
                        .filter(|(_, n)| group.contains(&n.name))
                        .map(|(k, n)| (k, p - self.to_screen(origin, n.at)))
                        .collect();
                } else if ui.input(|i| i.modifiers.shift) {
                    self.band = Some(p);
                }
            }
        }
        if response.dragged_by(egui::PointerButton::Primary) {
            if let (false, Some(p)) = (self.moving.is_empty(), pointer) {
                for &(i, grab) in &self.moving {
                    d.nodes[i].at = self.to_canvas(origin, p - grab);
                }
            } else if self.wiring.is_none() && self.band.is_none() {
                self.pan += response.drag_delta();
                self.user_moved = true;
            }
        }
        if response.drag_stopped() {
            for (i, _) in std::mem::take(&mut self.moving) {
                d.nodes[i].placed = true;
                edits.push(Edit::Move {
                    block: d.nodes[i].name.clone(),
                    at: d.nodes[i].at,
                });
            }
            if let (Some(a), Some(b)) = (self.band.take(), pointer) {
                let r = Rect::from_two_pos(a, b);
                let inside: Vec<String> = d
                    .nodes
                    .iter()
                    .filter(|n| n.is_block() && r.intersects(self.node_rect(origin, n)))
                    .map(|n| n.name.clone())
                    .collect();
                self.selected = match inside.len() {
                    0 => None,
                    1 => Some(Selection::Block(inside[0].clone())),
                    _ => Some(Selection::Many(inside)),
                };
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
                    (None, Some((from, to))) => {
                        let node = |key: &str| {
                            key.split_once('.')
                                .map_or(key.to_string(), |(n, _)| n.to_string())
                        };
                        let into = d.node(&node(&to)).map(|k| &d.nodes[k]);
                        let out_of = d.node(&node(&from)).map(|k| &d.nodes[k]);
                        match (out_of.map(|n| n.kind), into.map(|n| n.kind)) {
                            // A view's slot shows the signal.
                            (_, Some(NodeKind::View)) => edits.push(Edit::Bind {
                                many: d.many.contains(&to),
                                from,
                                to,
                            }),
                            // A filter cannot feed the model: offer to
                            // promote it.
                            (Some(NodeKind::Filter), Some(k)) if !matches!(k, NodeKind::Filter) => {
                                let name = node(&from).trim_start_matches("filter:").to_string();
                                let at = pointer.unwrap_or(origin);
                                self.offer = Some((name, to, at));
                            }
                            _ => edits.push(Edit::Connect { from, to }),
                        }
                    }
                    (None, None) => {}
                }
            }
        }
        let on_expander = |p: Pos2| {
            d.nodes
                .iter()
                .rev()
                .find(|n| self.expander(origin, n).is_some_and(|r| r.contains(p)))
                .map(|n| open_key(&d.scope, &n.name))
        };
        if response.clicked() {
            // The triangle opens or closes a node's values.
            if let Some(key) = response.interact_pointer_pos().and_then(on_expander) {
                if !self.open.remove(&key) {
                    self.open.insert(key);
                }
                requests.push(Request::Opened(self.open.iter().cloned().collect()));
                return;
            }
        }
        if response.clicked() {
            if let Some(p) = response.interact_pointer_pos() {
                let shift = ui.input(|i| i.modifiers.shift);
                self.selected = match self.node_under(d, origin, p) {
                    // Shift-click adds or removes a block.
                    Some(i) if shift && d.nodes[i].is_block() => {
                        let name = d.nodes[i].name.clone();
                        let mut v = self
                            .selected
                            .as_ref()
                            .map(Selection::blocks)
                            .unwrap_or_default();
                        match v.iter().position(|b| b == &name) {
                            Some(k) => {
                                v.remove(k);
                            }
                            None => v.push(name),
                        }
                        match v.len() {
                            0 => None,
                            1 => Some(Selection::Block(v.remove(0))),
                            _ => Some(Selection::Many(v)),
                        }
                    }
                    Some(i) => Some(Selection::Block(d.nodes[i].name.clone())),
                    None => self.wire_under(d, origin, p).map(Selection::Wire),
                };
            }
        }
        if response.double_clicked() {
            let p = response.interact_pointer_pos();
            if let Some((i, k)) = p.and_then(|p| self.value_under(d, origin, p)) {
                // A value: typed in place when zoomed in (ADR 0053).
                let v = &d.nodes[i].values[k];
                if self.zoom >= EDIT_VALUES {
                    self.editing = Some((v.key.clone(), format_number(v.value)));
                }
            } else if let Some((i, c)) = p.and_then(|p| self.unit_under(d, origin, p)) {
                // A conversion's unit (ADR 0045).
                let n = &d.nodes[i];
                if let (Some(setting), true) = (unit_setting(n, c), self.zoom >= EDIT_VALUES) {
                    let now = n.settings.get(setting).cloned().unwrap_or_default();
                    self.editing = Some((unit_key(&n.name, setting), now));
                }
            } else if let Some(i) = p.and_then(|p| self.node_under(d, origin, p)) {
                requests.push(open_request(&d.nodes[i]));
            }
        }
        if response.secondary_clicked() {
            if let Some(p) = response.interact_pointer_pos() {
                let on = self.node_under(d, origin, p);
                if let Some(i) = on {
                    let n = &d.nodes[i];
                    if !self.is_selected(&n.name) {
                        self.selected = Some(Selection::Block(n.name.clone()));
                    }
                    self.rename = n.name[n.name.find(':').map_or(0, |k| k + 1)..].to_string();
                }
                self.menu = Some((self.to_canvas(origin, p), on));
            }
        }
        let delete =
            ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace));
        let typing = ui.ctx().egui_wants_keyboard_input();
        if delete && !typing && response.hovered() {
            match self.selected.take() {
                Some(Selection::Wire(key)) => edits.push(match key.split_once('<') {
                    Some((to, signal)) => Edit::Unbind {
                        to: to.into(),
                        signal: signal.into(),
                    },
                    None => Edit::Disconnect { to: key },
                }),
                Some(s) => {
                    for block in s.blocks() {
                        edits.push(Edit::Delete { block });
                    }
                }
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
        let selection = self
            .selected
            .as_ref()
            .map(Selection::blocks)
            .unwrap_or_default();
        let open = response.context_menu(|ui| match on {
            Some(i) => {
                let n = &d.nodes[i];
                let name = n.name.clone();
                let shown = name[name.find(':').map_or(0, |k| k + 1)..].to_string();
                ui.label(egui::RichText::new(&shown).strong());
                match n.kind {
                    NodeKind::Subsystem => {
                        if ui.button("Open").on_hover_text("Edit what is inside (every use changes)").clicked() {
                            requests.push(Request::Open(name.clone()));
                            ui.close();
                        }
                    }
                    NodeKind::Block => {
                        if ui.button("Edit…").on_hover_text("Its equations or settings").clicked() {
                            requests.push(Request::Open(name.clone()));
                            ui.close();
                        }
                    }
                    NodeKind::Input | NodeKind::Output => {}
                    NodeKind::Filter => {
                        if ui.button("Edit in Data…").on_hover_text("Its formula or cuts").clicked() {
                            requests.push(open_request(n));
                            ui.close();
                        }
                        if n.type_name == "expression"
                            && ui
                                .button("Promote to block")
                                .on_hover_text(
                                    "Make it a formula block that runs with the model and can \
                                     feed other blocks (ADR 0032)",
                                )
                                .clicked()
                        {
                            edits.push(Edit::Promote {
                                filter: shown.clone(),
                            });
                            ui.close();
                        }
                    }
                    NodeKind::View => {
                        if ui.button("Show tile").clicked() {
                            requests.push(open_request(n));
                            ui.close();
                        }
                    }
                }
                ui.add_enabled_ui(n.kind != NodeKind::View, |ui| ui.horizontal(|ui| {
                    let field = ui.add(egui::TextEdit::singleline(&mut self.rename).desired_width(110.0));
                    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (ui.button("Rename").clicked() || enter) && self.rename != shown {
                        edits.push(Edit::Rename {
                            from: name.clone(),
                            to: self.rename.trim().to_string(),
                        });
                        ui.close();
                    }
                }));
                if n.is_block() {
                    let group: Vec<String> = if selection.contains(&name) {
                        selection
                            .iter()
                            .filter(|b| d.node(b).is_some_and(|k| d.nodes[k].is_block()))
                            .cloned()
                            .collect()
                    } else {
                        vec![name.clone()]
                    };
                    let label = match group.len() {
                        1 => "Make subsystem".to_string(),
                        k => format!("Make subsystem of {k} blocks"),
                    };
                    if ui
                        .button(label)
                        .on_hover_text("Move into a new subsystem; wires that cross become its ports (Shift-click or Shift-drag to select several)")
                        .clicked()
                    {
                        edits.push(Edit::Group { blocks: group });
                        ui.close();
                    }
                }
                let delete = match selection.len() {
                    k if k > 1 && selection.contains(&name) => format!("Delete {k} blocks"),
                    _ => "Delete".into(),
                };
                if ui.button(delete).clicked() {
                    if selection.len() > 1 && selection.contains(&name) {
                        for block in &selection {
                            edits.push(Edit::Delete { block: block.clone() });
                        }
                    } else {
                        edits.push(Edit::Delete { block: name });
                    }
                    ui.close();
                }
            }
            None => {
                ui.label("Add block");
                if let Some(type_name) = type_menu(ui, registry, &d.subsystems) {
                    edits.push(Edit::Add { type_name, at });
                    ui.close();
                }
                if !d.view_types.is_empty() {
                    ui.separator();
                    if ui
                        .button("Add filter")
                        .on_hover_text("An expression over recorded signals; edit it in Data…")
                        .clicked()
                    {
                        edits.push(Edit::AddFilter { at });
                        ui.close();
                    }
                    ui.menu_button("Add view", |ui| {
                        for (name, label) in &d.view_types {
                            if ui.button(label).on_hover_text(name).clicked() {
                                edits.push(Edit::AddView {
                                    type_name: name.clone(),
                                    at,
                                });
                                ui.close();
                            }
                        }
                    });
                }
                if matches!(d.scope, Scope::Subsystem(_)) {
                    ui.separator();
                    if ui.button("Add input").clicked() {
                        edits.push(Edit::AddPort { input: true, at });
                        ui.close();
                    }
                    if ui.button("Add output").clicked() {
                        edits.push(Edit::AddPort { input: false, at });
                        ui.close();
                    }
                }
            }
        });
        if open.is_none() {
            self.menu = None;
        }
    }
}

/// A menu of block types by domain, with their summaries on hover, and
/// the file's subsystems. Returns the chosen type.
pub fn type_menu(ui: &mut egui::Ui, registry: &Registry, subsystems: &[String]) -> Option<String> {
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
    if !subsystems.is_empty() {
        ui.menu_button("Subsystems", |ui| {
            for s in subsystems {
                if ui.button(s).clicked() {
                    chosen = Some(s.clone());
                }
            }
        });
    }
    chosen
}

impl Canvas {
    /// After a filter was dropped on a model input: say why that cannot
    /// be, and offer to promote the filter and connect its block.
    fn promote_offer(&mut self, ui: &egui::Ui, edits: &mut Vec<Edit>) {
        let Some((filter, to, at)) = self.offer.clone() else {
            return;
        };
        let mut done = false;
        egui::Area::new(egui::Id::new("promote-offer"))
            .fixed_pos(at + Vec2::new(8.0, 8.0))
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_max_width(320.0);
                    ui.label(crate::observers::feeds_model(&filter));
                    ui.horizontal(|ui| {
                        if ui.button("Promote and connect").clicked() {
                            edits.push(Edit::PromoteConnect {
                                filter: filter.clone(),
                                to: to.clone(),
                            });
                            done = true;
                        }
                        if ui.button("Cancel").clicked() {
                            done = true;
                        }
                    });
                });
            });
        if done || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.offer = None;
        }
    }

    /// The offer to promote a filter, if one is open: filter and input.
    pub fn offered(&self) -> Option<(&str, &str)> {
        self.offer
            .as_ref()
            .map(|(f, t, _)| (f.as_str(), t.as_str()))
    }
}

/// What a node is called on the canvas.
fn title(n: &Node) -> String {
    match n.kind {
        NodeKind::Input | NodeKind::Output | NodeKind::Filter => {
            n.name[n.name.find(':').map_or(0, |i| i + 1)..].to_string()
        }
        NodeKind::View => {
            let k = n.name["view:".len()..].parse::<usize>().unwrap_or(0);
            format!("View {}", k + 1)
        }
        _ => n.name.clone(),
    }
}

/// What double-clicking a node opens.
fn open_request(n: &Node) -> Request {
    match n.kind {
        NodeKind::Filter => Request::EditData(title(n)),
        NodeKind::View => Request::ShowView(n.name["view:".len()..].parse().unwrap_or(0)),
        _ => Request::Open(n.name.clone()),
    }
}

/// Hover help for a data-layer node: what it is and what it reads that
/// has no wire.
fn observer_help(n: &Node) -> String {
    let what = match n.kind {
        NodeKind::Filter => format!(
            "Filter {} ({}): computed from recorded data after each step; it cannot feed the \
             model. Double-click to edit it in Data…",
            title(n),
            n.method
        ),
        _ => format!(
            "{} ({}): drop a signal on a slot to show it. Double-click to show the tile.",
            title(n),
            n.type_name
        ),
    };
    if n.unreached.is_empty() {
        what
    } else {
        format!("{what}\nAlso reads: {}", n.unreached.join(", "))
    }
}

fn port_help(block: &str, p: &crate::diagram::Port, c: Causality) -> String {
    let dir = match c {
        Causality::Input => "input",
        Causality::Output => "output",
    };
    let shape = match p.shape {
        Shape::Scalar => "a number".to_string(),
        Shape::Vector(n) => format!("{n} numbers"),
        Shape::Grid { nx: 0, .. } => "a line field".to_string(),
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

/// A compact display of a value: up to 6 significant digits.
/// The setting a conversion block's port shows its unit from (ADR 0045):
/// `from` beside its input, `to` beside its output.
fn unit_setting(n: &Node, c: Causality) -> Option<&'static str> {
    if n.type_name != simsandbox_eq::convert::CONVERT || n.error.is_some() {
        return None;
    }
    Some(match c {
        Causality::Input => "from",
        Causality::Output => "to",
    })
}

/// How `controls.open` names a node: by name on the model level,
/// `subsystem::block` inside one.
pub fn open_key(scope: &Scope, name: &str) -> String {
    match scope {
        Scope::Model => name.to_string(),
        Scope::Subsystem(s) => format!("{s}::{name}"),
    }
}

/// The editing key of a conversion's unit.
fn unit_key(block: &str, setting: &str) -> String {
    format!("unit:{block}:{setting}")
}

pub fn format_number(v: f64) -> String {
    if v == 0.0 {
        "0".into()
    } else if (1e-4..1e6).contains(&v.abs()) {
        let digits = (5 - v.abs().log10().floor() as i32).clamp(0, 9) as usize;
        let s = format!("{v:.digits$}");
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s
        }
    } else {
        format!("{v:e}")
    }
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

/// Port rows of a node, for tests and layout.
pub fn rows(n: &Node) -> usize {
    n.inputs.len().max(n.outputs.len()).max(1)
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
        frame_with(ctx, canvas, d, events, egui::Modifiers::NONE)
    }

    fn frame_with(
        ctx: &egui::Context,
        canvas: &mut Canvas,
        d: &mut Diagram,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) -> (Vec<Edit>, Vec<Request>) {
        let r = registry();
        let mut all = vec![egui::Event::ModifiersChanged(modifiers)];
        all.extend(events);
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
            events: all,
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
    fn data_wires_show_signals_in_views_and_filters_offer_promotion() {
        let mut f = ModelFile::from_json(
            EXAMPLES
                .iter()
                .find(|e| e.name.contains("Cruise"))
                .unwrap()
                .text,
        )
        .unwrap()
        .0;
        f.controls
            .filters
            .push(simsandbox_file::FilterEntry::expression(
                "push",
                "cruise.F",
                Vec::new(),
            ));
        let mut d = Diagram::new(&f, &registry());
        d.observe(&f, &crate::examples::view_registry());
        let ctx = egui::Context::default();
        let mut c = Canvas::default();
        frame(&ctx, &mut c, &mut d, vec![]);
        // An output dropped on a view's slot that takes several adds it.
        let a = screen(&c, &d, "car", Causality::Output, "v");
        let b = screen(&c, &d, "view:1", Causality::Input, "y");
        let edits = drag(&ctx, &mut c, &mut d, a, b);
        assert_eq!(
            edits,
            [Edit::Bind {
                from: "car.v".into(),
                to: "view:1.y".into(),
                many: true
            }]
        );
        // A filter dropped on a block input is not connected; promoting is
        // offered instead.
        let a = screen(&c, &d, "filter:push", Causality::Output, "push");
        let b = screen(&c, &d, "car", Causality::Input, "F");
        let edits = drag(&ctx, &mut c, &mut d, a, b);
        assert!(edits.is_empty(), "{edits:?}");
        assert_eq!(c.offered(), Some(("push", "car.F")));
        // Data wires are selected by input and signal, and Delete unbinds.
        let w = d
            .wires
            .iter()
            .find(|w| w.signal == "cruise.F")
            .unwrap()
            .clone();
        c.selected = Some(Selection::Wire(w.key()));
        let p = c.to_screen(Pos2::ZERO, [0.0, 0.0]);
        let (edits, _) = frame(
            &ctx,
            &mut c,
            &mut d,
            vec![
                egui::Event::PointerMoved(p + Vec2::splat(5.0)),
                egui::Event::Key {
                    key: egui::Key::Delete,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                },
            ],
        );
        assert!(
            edits
                .iter()
                .any(|e| matches!(e, Edit::Unbind { signal, .. } if signal == "cruise.F")),
            "{edits:?}"
        );
    }

    #[test]
    fn values_on_nodes_are_typed_in_place_when_zoomed_in() {
        let (mut d, mut c, ctx) = cruise();
        let n = d.node("car").unwrap();
        // Fitted at this size the zoom is high enough to edit; make sure.
        c.zoom = c.zoom.max(1.0);
        frame(&ctx, &mut c, &mut d, vec![]);
        // Closed: no values until its triangle is clicked.
        assert_eq!(d.nodes[n].value_rows(), 0);
        let e = c.expander(Pos2::ZERO, &d.nodes[n]).unwrap().center();
        let mut asked = Vec::new();
        for events in [
            vec![egui::Event::PointerMoved(e)],
            vec![press(e, true), press(e, false)],
            vec![],
        ] {
            asked.extend(frame(&ctx, &mut c, &mut d, events).1);
        }
        // Saved in the file as `controls.open`.
        assert_eq!(asked, [Request::Opened(vec!["car".into()])]);
        assert!(c.open.contains("car"));
        assert_eq!(d.nodes[n].value_rows(), 3);
        // Clicking it selected nothing.
        assert_eq!(c.selected, None);
        // (A new context, so that click does not count towards a double.)
        let ctx = egui::Context::default();
        let at = d.nodes[n].value_at(0);
        let p = c.to_screen(Pos2::ZERO, [at[0] + 100.0, at[1] + 8.0]);
        let click = |pressed| press(p, pressed);
        for events in [
            vec![egui::Event::PointerMoved(p)],
            vec![click(true)],
            vec![click(false)],
            vec![click(true)],
            vec![click(false)],
            vec![],
        ] {
            frame(&ctx, &mut c, &mut d, events);
        }
        assert_eq!(c.editing.as_ref().map(|(k, _)| k.as_str()), Some("car.m"));
        // Replace the text and press Enter.
        c.editing = Some(("car.m".into(), "1500".into()));
        frame(&ctx, &mut c, &mut d, vec![]);
        let enter = egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        };
        let (_, requests) = frame(&ctx, &mut c, &mut d, vec![enter.clone()]);
        assert_eq!(
            requests,
            [Request::SetValue {
                key: "car.m".into(),
                value: 1500.0
            }]
        );
        assert!(c.editing.is_none());
        // '= expression' asks for a formula.
        frame(&ctx, &mut c, &mut d, vec![]);
        c.editing = Some(("car.m".into(), "= 2*600".into()));
        frame(&ctx, &mut c, &mut d, vec![]);
        let (_, requests) = frame(&ctx, &mut c, &mut d, vec![enter]);
        assert_eq!(
            requests,
            [Request::SetFormula {
                key: "car.m".into(),
                formula: "2*600".into()
            }]
        );
        // Zoomed out, values are not shown or edited.
        c.zoom = 0.4;
        assert!(c.value_under(&d, Pos2::ZERO, p).is_none());
    }

    #[test]
    fn a_conversions_units_are_typed_beside_its_ports() {
        let r = registry();
        let e = EXAMPLES.iter().find(|e| e.name.contains("nits")).unwrap();
        let f = simsandbox_file::ModelFile::from_json(e.text).unwrap().0;
        let mut d = Diagram::new(&f, &r);
        let mut c = Canvas::default();
        let ctx = egui::Context::default();
        frame(&ctx, &mut c, &mut d, vec![]);
        let n = d.node("b_cm").unwrap();
        // Zoomed in, with the conversion in view.
        c.zoom = 1.0;
        let at = d.nodes[n].at;
        c.pan = Vec2::new(100.0 - at[0] as f32, 100.0 - at[1] as f32);
        frame(&ctx, &mut c, &mut d, vec![]);
        let p = c
            .unit_rect(Pos2::ZERO, &d.nodes[n], Causality::Output)
            .unwrap()
            .center();
        for events in [
            vec![egui::Event::PointerMoved(p)],
            vec![press(p, true)],
            vec![press(p, false)],
            vec![press(p, true)],
            vec![press(p, false)],
            vec![],
        ] {
            frame(&ctx, &mut c, &mut d, events);
        }
        let now = d.nodes[n].settings["to"].clone();
        assert_eq!(c.editing, Some(("unit:b_cm:to".into(), now)));
        c.editing = Some(("unit:b_cm:to".into(), "mm".into()));
        frame(&ctx, &mut c, &mut d, vec![]);
        let enter = egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        };
        let (_, requests) = frame(&ctx, &mut c, &mut d, vec![enter]);
        assert_eq!(
            requests,
            [Request::SetSetting {
                block: "b_cm".into(),
                key: "to".into(),
                text: "mm".into()
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

    #[test]
    fn a_box_selects_several_blocks_which_move_and_delete_together() {
        let (mut d, mut c, ctx) = cruise();
        let shift = egui::Modifiers::SHIFT;
        // A box around everything, Shift held.
        let (a, b) = (Pos2::new(2.0, 2.0), Pos2::new(798.0, 598.0));
        for events in [
            vec![egui::Event::PointerMoved(a)],
            vec![press(a, true)],
            vec![egui::Event::PointerMoved(Pos2::new(400.0, 300.0))],
            vec![egui::Event::PointerMoved(b)],
            vec![press(b, false)],
        ] {
            frame_with(&ctx, &mut c, &mut d, events, shift);
        }
        let mut chosen = c.selected.clone().unwrap().blocks();
        chosen.sort();
        assert_eq!(chosen, ["car", "cruise"]);
        // Dragging one moves both.
        let before: Vec<[f64; 2]> = d.nodes.iter().map(|n| n.at).collect();
        let n = &d.nodes[0];
        let grab = c.to_screen(Pos2::ZERO, [n.at[0] + 60.0, n.at[1] + 10.0]);
        let edits = drag(&ctx, &mut c, &mut d, grab, grab + Vec2::new(30.0, 0.0));
        assert_eq!(edits.len(), 2, "{edits:?}");
        for (k, n) in d.nodes.iter().enumerate() {
            assert!(n.at[0] > before[k][0]);
        }
        // Delete removes both.
        let p = c.to_screen(
            Pos2::ZERO,
            [d.nodes[0].at[0] + 60.0, d.nodes[0].at[1] + 10.0],
        );
        frame(&ctx, &mut c, &mut d, vec![egui::Event::PointerMoved(p)]);
        let key = egui::Event::Key {
            key: egui::Key::Delete,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        };
        let (edits, _) = frame(&ctx, &mut c, &mut d, vec![key]);
        assert_eq!(edits.len(), 2);
        assert!(edits.iter().all(|e| matches!(e, Edit::Delete { .. })));
    }

    #[test]
    fn shift_click_adds_to_the_selection() {
        let (mut d, mut c, ctx) = cruise();
        for name in ["car", "cruise"] {
            let n = &d.nodes[d.node(name).unwrap()];
            let p = c.to_screen(Pos2::ZERO, [n.at[0] + 50.0, n.at[1] + 5.0]);
            frame(&ctx, &mut c, &mut d, vec![egui::Event::PointerMoved(p)]);
            let pr = |pressed| egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::SHIFT,
            };
            frame_with(
                &ctx,
                &mut c,
                &mut d,
                vec![pr(true), pr(false)],
                egui::Modifiers::SHIFT,
            );
        }
        assert_eq!(
            c.selected,
            Some(Selection::Many(vec!["car".into(), "cruise".into()]))
        );
    }
}
