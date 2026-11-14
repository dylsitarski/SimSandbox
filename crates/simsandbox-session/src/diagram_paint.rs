//! The node canvas painted (ADR 0042): a [`Diagram`] drawn with egui's
//! painter at a [`View`], and where its parts are on screen. The app's
//! canvas handles the mouse and keyboard and paints through this; headless
//! rendering calls [`show`].
//!
//! The data layer (ADR 0050), when the diagram has one: filters and views
//! as nodes with dashed outlines, read by dashed wires.

use crate::diagram::{Diagram, Node, NodeKind, Scope, HEADER, NODE_WIDTH, ROW, VALUE_ROW};
use egui::{Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use simsandbox_core::block::{Causality, Shape};

/// Values show on nodes from this zoom up.
pub const SHOW_VALUES: f32 = 0.55;

/// Port dot radius, in points at zoom 1.
const DOT: f32 = 4.5;

/// Where the diagram is on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// Screen offset of the canvas origin from the panel's corner.
    pub pan: Vec2,
    pub zoom: f32,
}

impl Default for View {
    fn default() -> Self {
        Self {
            pan: Vec2::ZERO,
            zoom: 1.0,
        }
    }
}

/// What an interactive canvas has painted over the diagram; nothing when
/// headless.
#[derive(Clone, Copy, Debug, Default)]
pub struct Marks<'a> {
    /// The selected nodes, by name.
    pub nodes: &'a [String],
    /// The selected wire, by its key.
    pub wire: Option<&'a str>,
    /// The port a wire is being drawn from: node, port and which side.
    pub wiring: Option<(usize, usize, Causality)>,
    /// The wire that drag lifted off an input (`"block.port"`): not drawn.
    pub lifted: Option<&'a str>,
    /// Where a selection box started, on screen.
    pub band: Option<Pos2>,
    /// The pointer, where the wire being drawn and the box end.
    pub pointer: Option<Pos2>,
    /// The value or unit being typed, by its key ([`unit_key`] for a
    /// unit): the field covers it, so it is not drawn.
    pub editing: Option<&'a str>,
}

/// Set which nodes show their values (ADR 0055): those with values whose
/// [`open_key`] `open` accepts.
pub fn open_nodes(d: &mut Diagram, open: impl Fn(&str) -> bool) {
    for n in &mut d.nodes {
        n.open = !n.values.is_empty() && open(&open_key(&d.scope, &n.name));
    }
}

/// Draw the diagram filling `ui`'s space, fitted to it, without input: as
/// headless rendering shows it.
pub fn show(ui: &mut egui::Ui, d: &Diagram) {
    let (rect, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
    let mut view = View::default();
    if rect.width() > 1.0 {
        view.fit(d, rect);
    }
    let painter = ui.painter_at(rect);
    let visuals = ui.visuals().clone();
    view.background(&painter, rect, &visuals);
    view.paint(&painter, &visuals, rect, d, &Marks::default());
}

impl View {
    pub fn to_screen(&self, origin: Pos2, p: [f64; 2]) -> Pos2 {
        origin + self.pan + Vec2::new(p[0] as f32, p[1] as f32) * self.zoom
    }

    pub fn to_canvas(&self, origin: Pos2, p: Pos2) -> [f64; 2] {
        let v = (p - origin - self.pan) / self.zoom;
        [v.x as f64, v.y as f64]
    }

    /// Fit the diagram to `rect`, centred.
    pub fn fit(&mut self, d: &Diagram, rect: Rect) {
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

    /// The canvas under the diagram: its fill and grid.
    pub fn background(&self, painter: &egui::Painter, rect: Rect, visuals: &egui::Visuals) {
        painter.rect_filled(rect, CornerRadius::ZERO, visuals.extreme_bg_color);
        self.grid(painter, rect, visuals.faint_bg_color);
    }

    /// Draw the diagram in `rect`, over the background, with `marks`.
    pub fn paint(
        &self,
        painter: &egui::Painter,
        visuals: &egui::Visuals,
        rect: Rect,
        d: &Diagram,
        marks: &Marks,
    ) {
        let origin = rect.min;
        // Expanded uses under everything (ADR 0066 step 4d): an outline
        // coloured by its clock, titled with its path, model and clock.
        let z = self.zoom;
        for o in &d.outlines {
            let r = Rect::from_min_max(
                self.to_screen(origin, [o.rect[0], o.rect[1]]),
                self.to_screen(origin, [o.rect[2], o.rect[3]]),
            );
            let tint = clock_colour(&o.clock, visuals);
            painter.rect_filled(r, 6.0 * z, tint.gamma_multiply(0.08));
            painter.rect_stroke(
                r,
                6.0 * z,
                Stroke::new(1.5, tint.gamma_multiply(0.8)),
                StrokeKind::Inside,
            );
            painter.text(
                r.min + Vec2::new(8.0 * z, 6.0 * z),
                egui::Align2::LEFT_TOP,
                format!("{} · {} · {}", o.path, o.label, o.clock),
                egui::FontId::proportional(12.0 * z),
                visuals.text_color(),
            );
        }

        // Wires under nodes.
        for w in &d.wires {
            let lifted = marks.lifted == Some(w.to_name.as_str());
            if lifted && !w.is_data() {
                continue;
            }
            let selected = marks.wire == Some(w.key().as_str());
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
        if let (Some((node, port, causality)), Some(p)) = (marks.wiring, marks.pointer) {
            let at = self.to_screen(origin, d.nodes[node].port_at(causality, port));
            let (a, b) = match causality {
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
            let selected = marks.nodes.contains(&n.name);
            self.node(painter, visuals, origin, d, i, selected, marks);
        }
        if let (Some(a), Some(b)) = (marks.band, marks.pointer) {
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
            let empty = "Empty model: right-click to add a block";
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                empty,
                FontId::proportional(14.0),
                visuals.weak_text_color(),
            );
        }
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

    /// A node's rectangle on screen.
    pub fn node_rect(&self, origin: Pos2, n: &Node) -> Rect {
        Rect::from_min_size(
            self.to_screen(origin, n.at),
            Vec2::new(NODE_WIDTH as f32, n.height() as f32) * self.zoom,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn node(
        &self,
        painter: &egui::Painter,
        visuals: &egui::Visuals,
        origin: Pos2,
        d: &Diagram,
        i: usize,
        selected: bool,
        marks: &Marks,
    ) {
        let n = &d.nodes[i];
        let z = self.zoom;
        let r = self.node_rect(origin, n);
        let radius = CornerRadius::same((6.0 * z).round().clamp(0.0, 255.0) as u8);
        // A study runs its model many times: drawn as a stack of cards
        // (ADR 0070).
        let study = n.kind == NodeKind::Block && n.type_name == simsandbox_file::STUDY;
        if study {
            // Below the node and narrower, clear of the ports at its sides.
            for k in [2.0, 1.0] {
                let card = r
                    .translate(Vec2::new(0.0, 4.0 * k * z))
                    .shrink2(Vec2::new(8.0 * k * z, 0.0));
                painter.rect_filled(card, radius, visuals.window_fill);
                painter.rect_stroke(card, radius, visuals.window_stroke, StrokeKind::Inside);
            }
        }
        painter.rect_filled(r, radius, visuals.window_fill);
        let header = Rect::from_min_size(r.min, Vec2::new(r.width(), HEADER as f32 * z));
        let header_fill = match n.kind {
            NodeKind::Block if study => {
                STUDY_HEADER.gamma_multiply(if visuals.dark_mode { 0.55 } else { 0.3 })
            }
            NodeKind::Block => visuals.widgets.inactive.bg_fill,
            NodeKind::Use => visuals.selection.bg_fill.gamma_multiply(0.7),
            NodeKind::Filter => visuals.hyperlink_color.gamma_multiply(0.3),
            NodeKind::View => visuals.hyperlink_color.gamma_multiply(0.15),
            NodeKind::Probe => visuals.warn_fg_color.gamma_multiply(0.18),
            NodeKind::Recorder => visuals.error_fg_color.gamma_multiply(0.18),
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
        // A probe feeding a block is part of the model: drawn solid.
        if n.is_observer() && !n.settings.contains_key("feeds") {
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
        if n.kind == NodeKind::Use || study {
            // A second outline: there is a model inside, to open.
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
                NodeKind::Block if study => format!("study {}", n.method),
                NodeKind::Block => format!("{} · {}", n.type_name, n.method),
                // A model file used here: its file and its clock.
                NodeKind::Use => format!("{} · {}", n.method, n.type_name),
                NodeKind::Filter => format!("filter · {}", n.method),
                NodeKind::View => format!("view · {}", n.method),
                NodeKind::Probe => n.method.clone(),
                NodeKind::Recorder => format!("rewind · {}", n.method),
            }
        };
        // What the run says of it now first: held, started again (ADR 0088).
        let kind = match &n.status {
            Some(status) => format!("{status} · {kind}"),
            None => kind,
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
                // Exposed to the models using this one: ringed (ADR 0074),
                // wherever the model is drawn.
                let kind = match c {
                    Causality::Input => "in",
                    Causality::Output => "out",
                };
                let ringed = d.exposed_as(i, kind, &p.name).is_some();
                if ringed {
                    painter.circle_stroke(
                        at,
                        (DOT + 3.5) * z,
                        Stroke::new(1.5 * z, visuals.selection.stroke.color),
                    );
                }
                // Clear of the ring.
                let pad = if ringed { pad + 4.0 * z } else { pad };
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
                    // An activated part's mode beside its port (ADR 0088).
                    _ if c == Causality::Input && p.name == "activate" => {
                        match n.settings.get("activate") {
                            Some(mode) => format!("activate · {mode}"),
                            None => p.name.clone(),
                        }
                    }
                    // A probe names its file beside its input.
                    NodeKind::Probe => match (c, n.settings.get("to")) {
                        (Causality::Input, Some(to)) => format!("{} · {to}", p.name),
                        _ => p.name.clone(),
                    },
                    // A conversion shows its units (ADR 0045).
                    _ => match unit_setting(n, c).filter(|_| k == 0) {
                        Some(setting) => {
                            let unit = n.settings.get(setting).unwrap_or(&p.unit);
                            if marks.editing == Some(unit_key(&n.name, setting).as_str()) {
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
        self.values(painter, visuals, origin, d, i, marks);
    }

    /// Where a node's values are opened and closed: the top-right corner
    /// of its header, on nodes with values, when they can be shown.
    pub fn expander(&self, origin: Pos2, n: &Node) -> Option<Rect> {
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

    /// Where a conversion's unit is written beside port 0.
    pub fn unit_rect(&self, origin: Pos2, n: &Node, c: Causality) -> Option<Rect> {
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
    fn values(
        &self,
        painter: &egui::Painter,
        visuals: &egui::Visuals,
        origin: Pos2,
        d: &Diagram,
        i: usize,
        marks: &Marks,
    ) {
        let n = &d.nodes[i];
        let exposable = d.exposable(i);
        if n.value_rows() == 0 || self.zoom < SHOW_VALUES {
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
        // While a wire is drawn from an output, every value is a place to
        // drop it (ADR 0075).
        let wiring = marks
            .wiring
            .is_some_and(|(_, _, c)| c == Causality::Output && marks.lifted.is_none());
        for (k, v) in n.values.iter().enumerate().take(n.value_rows()) {
            let port = self.to_screen(origin, n.value_port(k));
            if v.driven.is_some() {
                painter.circle_filled(port, DOT * z * 0.8, visuals.widgets.active.fg_stroke.color);
            } else if wiring && n.is_block() {
                painter.circle_stroke(
                    port,
                    DOT * z * 0.8,
                    Stroke::new(1.0 * z, visuals.weak_text_color()),
                );
            }
            if marks.editing == Some(v.key.as_str()) {
                continue;
            }
            let y = self.to_screen(origin, n.value_at(k)).y + VALUE_ROW as f32 * z / 2.0;
            let mut left = Pos2::new(top.x + pad, y);
            // Its toggle (ADR 0074): a ring, filled while it is exposed.
            if exposable {
                let at = self.toggle_at(origin, n, k);
                let colour = visuals.selection.stroke.color;
                if v.exposed.is_some() {
                    painter.circle_filled(at, 2.5 * z, colour);
                    painter.circle_stroke(at, 4.5 * z, Stroke::new(1.5 * z, colour));
                } else {
                    painter.circle_stroke(
                        at,
                        4.5 * z,
                        Stroke::new(1.0 * z, visuals.weak_text_color()),
                    );
                }
                left.x = at.x + 8.0 * z;
            }
            let mut name = if v.initial {
                format!("{} ↻", v.name)
            } else {
                v.name.clone()
            };
            if let Some(e) = v.exposed.as_ref().filter(|e| **e != v.name) {
                name = format!("{name} as {e}");
            }
            painter.text(
                left,
                egui::Align2::LEFT_CENTER,
                fit(&name, 12),
                font.clone(),
                visuals.weak_text_color(),
            );
            // `*`: some uses of the subsystem set their own.
            let mark = if v.overridden.is_empty() { "" } else { " *" };
            // Driven: its source, not a number to change (ADR 0075).
            let shown = match &v.driven {
                Some(from) => format!("from {from}"),
                None => format!("{} {}{mark}", format_number(v.value), v.unit),
            };
            painter.text(
                Pos2::new(top.x + NODE_WIDTH as f32 * z - pad, y),
                egui::Align2::RIGHT_CENTER,
                fit(shown.trim(), 16),
                font.clone(),
                visuals.text_color(),
            );
        }
    }

    /// The centre of the toggle exposing value `k` of `n` (ADR 0074): at
    /// the left of its row.
    pub fn toggle_at(&self, origin: Pos2, n: &Node, k: usize) -> Pos2 {
        let z = self.zoom;
        let row = self.to_screen(origin, n.value_at(k));
        row + Vec2::new(14.0 * z, VALUE_ROW as f32 * z / 2.0)
    }

    /// A wire's path on screen: a curve when it runs forwards, else
    /// around underneath both blocks.
    pub fn wire_path(&self, d: &Diagram, origin: Pos2, w: &crate::diagram::Wire) -> Vec<Pos2> {
        let a = self.to_screen(
            origin,
            d.nodes[w.from.0].port_at(Causality::Output, w.from.1),
        );
        let b = self.to_screen(origin, self.wire_end(d, w));
        let below = self
            .node_rect(origin, &d.nodes[w.from.0])
            .max
            .y
            .max(self.node_rect(origin, &d.nodes[w.to.0]).max.y);
        route(a, b, below, self.zoom)
    }

    /// Where a wire ends, in canvas units: its input, or the value it
    /// drives (ADR 0075), at the node's side when values are not drawn.
    fn wire_end(&self, d: &Diagram, w: &crate::diagram::Wire) -> [f64; 2] {
        let n = &d.nodes[w.to.0];
        if !w.param {
            return n.port_at(Causality::Input, w.to.1);
        }
        if self.zoom >= SHOW_VALUES && w.to.1 < n.value_rows() {
            n.value_port(w.to.1)
        } else {
            [n.at[0], n.at[1] + HEADER / 2.0]
        }
    }
}

/// The header colour of a study's node (ADR 0070): apart from blocks,
/// used models and the data layer.
const STUDY_HEADER: Color32 = Color32::from_rgb(150, 90, 200);

/// A colour for a clock: the same clock, the same colour; a shared clock
/// in the selection colour.
fn clock_colour(clock: &str, visuals: &egui::Visuals) -> Color32 {
    if clock.contains("shared") {
        return visuals.selection.bg_fill;
    }
    let h = clock.bytes().fold(2166136261u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(16777619)
    });
    let hue = (h % 360) as f32 / 360.0;
    egui::ecolor::Hsva::new(hue, 0.55, 0.85, 1.0).into()
}

/// What a node is called on the canvas.
pub fn title(n: &Node) -> String {
    match n.kind {
        NodeKind::Recorder => "Recorder".into(),
        NodeKind::Filter | NodeKind::Probe => {
            n.name[n.name.find(':').map_or(0, |i| i + 1)..].to_string()
        }
        NodeKind::View => {
            let k = n.name["view:".len()..].parse::<usize>().unwrap_or(0);
            format!("View {}", k + 1)
        }
        // Inside an expanded use its own name; the outline gives the path.
        _ => n.name.rsplit('/').next().unwrap_or(&n.name).to_string(),
    }
}

/// The setting a conversion block's port shows its unit from (ADR 0045):
/// `from` beside its input, `to` beside its output.
pub fn unit_setting(n: &Node, c: Causality) -> Option<&'static str> {
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
    }
}

/// The editing key of a conversion's unit.
pub fn unit_key(block: &str, setting: &str) -> String {
    format!("unit:{block}:{setting}")
}

/// A compact display of a value: up to 6 significant digits.
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

/// Port rows of a node, for tests and layout.
pub fn rows(n: &Node) -> usize {
    n.inputs.len().max(n.outputs.len()).max(1)
}
