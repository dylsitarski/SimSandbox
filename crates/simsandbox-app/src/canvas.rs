//! The node canvas (ADR 0042): a [`Diagram`] painted by
//! `simsandbox_session::diagram_paint`, and the mouse and keyboard turned
//! into [`Edit`]s. The app applies the edits; headless rendering paints the
//! same diagram without input.
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

use eframe::egui;
use egui::{FontId, Pos2, Rect, Sense, Vec2};
use simsandbox_core::block::{Causality, Shape};
use simsandbox_core::registry::Registry;
use simsandbox_session::diagram::{Diagram, Edit, Node, NodeKind, Scope, NODE_WIDTH, VALUE_ROW};
use simsandbox_session::diagram_paint::{
    format_number, open_key, open_nodes, title, unit_key, unit_setting, Marks, View, SHOW_VALUES,
};
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
    /// Edit a probe's settings (ADR 0062), by its name.
    EditProbe(String),
    /// Edit the recorder's settings (ADR 0064).
    EditRecorder,
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
    /// Choose a model file to use here, at this place (ADR 0066 step 4c).
    AddModel([f64; 2]),
    /// Write the model defined in the file that this use names to a file
    /// of its own (ADR 0068).
    MoveToFile(String),
    /// Make the model file this use reads a model defined in the file.
    PutInFile(String),
    /// Show the use at this path expanded in place (ADR 0066 step 4d).
    Expand(String),
    /// Show it as a node again.
    Collapse(String),
}

/// Values on nodes can be edited from this zoom up.
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
#[derive(Clone, Debug, Default)]
pub struct Canvas {
    view: View,
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
    /// The value being typed on a node: its key (`unit:block:from` for a
    /// conversion's unit) and the text so far.
    editing: Option<(String, String)>,
}

/// How close to a port counts as on it, in points at zoom 1.
const REACH: f32 = 9.0;

impl Canvas {
    /// Fit the diagram to the panel on the next frame.
    pub fn refit(&mut self) {
        self.fitted = None;
        self.user_moved = false;
    }

    /// Draw the diagram filling `ui`'s space, handle input and return the
    /// edits and requests the user made; `diagram` node positions follow
    /// drags as they happen.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        d: &mut Diagram,
        registry: &Registry,
    ) -> (Vec<Edit>, Vec<Request>) {
        let mut edits = Vec::new();
        let mut requests = Vec::new();
        open_nodes(d, |key| self.open.contains(key));
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let origin = rect.min;
        let resized = self.fitted.is_some_and(|s| s != rect.size()) && !self.user_moved;
        if (self.fitted.is_none() || resized) && rect.width() > 1.0 {
            self.view.fit(d, rect);
            self.fitted = Some(rect.size());
        }
        let painter = ui.painter_at(rect);
        let visuals = ui.visuals().clone();
        self.view.background(&painter, rect, &visuals);

        let pointer = response.hover_pos().or(response.interact_pointer_pos());
        self.input(ui, &response, d, origin, pointer, &mut edits, &mut requests);

        let nodes: &[String] = match &self.selected {
            Some(Selection::Block(b)) => std::slice::from_ref(b),
            Some(Selection::Many(v)) => v,
            _ => &[],
        };
        let marks = Marks {
            nodes,
            wire: match &self.selected {
                Some(Selection::Wire(key)) => Some(key),
                _ => None,
            },
            wiring: self.wiring.as_ref().map(|g| (g.node, g.port, g.causality)),
            lifted: self.wiring.as_ref().and_then(|g| g.lifted.as_deref()),
            band: self.band,
            pointer,
            editing: self.editing.as_ref().map(|(key, _)| key.as_str()),
        };
        self.view.paint(&painter, &visuals, rect, d, &marks);

        // Hover help: the error, or a port's unit and shape.
        for (i, n) in d.nodes.iter().enumerate() {
            let r = self.view.node_rect(origin, n);
            if let Some(p) = pointer.filter(|p| r.expand(REACH).contains(*p)) {
                let value = self.value_under(d, origin, p).filter(|&(k, _)| k == i);
                let unit = self.unit_under(d, origin, p).filter(|&(k, _)| k == i);
                let tip = if let Some(e) = &n.error {
                    Some(e.clone())
                } else if self.view.expander(origin, n).is_some_and(|r| r.contains(p)) {
                    Some(if n.open {
                        "Hide its values".to_string()
                    } else {
                        format!("Show its values ({})", n.values.len())
                    })
                } else if let Some((_, k)) = value {
                    let v = &n.values[k];
                    let mut tip = if let Some(from) = &v.driven {
                        format!(
                            "{}: driven by {from}, whose value it takes (ADR 0075); drag \
                                 the wire off its dot to set it again",
                            v.key
                        )
                    } else if let Some(e) = &v.exposed {
                        format!(
                            "{}: the model's parameter {e}, set by each model or study \
                                 using this one; its own value is changed in the Inspector",
                            v.key
                        )
                    } else if self.view.zoom >= EDIT_VALUES {
                        format!(
                            "{}: double-click to change it (a number, or '= expression' \
                                 of the parameters)",
                            v.key
                        )
                    } else {
                        format!("{}: zoom in to change it here", v.key)
                    };
                    if d.exposable(i) {
                        tip.push_str(if v.exposed.is_some() {
                            "\nClick the ring to make it the block's own again"
                        } else {
                            "\nClick the ring to expose it: models using this one set it"
                        });
                    }
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
                    Some(if self.view.zoom >= EDIT_VALUES {
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
                            let mut tip = port_help(&n.name, q, c);
                            if d.exposable(k) {
                                let kind = match c {
                                    Causality::Input => "in",
                                    Causality::Output => "out",
                                };
                                tip.push_str(&match d.exposed_as(k, kind, &q.name) {
                                    Some(e) => format!(
                                        "\nExposed as {e} to models using this one: \
                                             click to hide it"
                                    ),
                                    None => "\nClick to expose it to models using this \
                                                 one; drag to wire it"
                                        .to_string(),
                                });
                            }
                            tip
                        })
                };
                if let Some(tip) = tip {
                    response.clone().on_hover_text_at_pointer(tip);
                }
            }
        }

        self.value_editor(ui, d, origin, &mut requests);
        self.context_menu(&response, d, registry, &mut edits, &mut requests);
        self.promote_offer(ui, &mut edits);
        (edits, requests)
    }

    fn is_selected(&self, name: &str) -> bool {
        match &self.selected {
            Some(Selection::Block(b)) => b == name,
            Some(Selection::Many(v)) => v.iter().any(|b| b == name),
            _ => false,
        }
    }

    /// The conversion unit label under `p`: node and which port.
    fn unit_under(&self, d: &Diagram, origin: Pos2, p: Pos2) -> Option<(usize, Causality)> {
        d.nodes.iter().enumerate().rev().find_map(|(i, n)| {
            [Causality::Input, Causality::Output]
                .into_iter()
                .find(|&c| {
                    unit_setting(n, c).is_some()
                        && self
                            .view
                            .unit_rect(origin, n, c)
                            .is_some_and(|r| r.contains(p))
                })
                .map(|c| (i, c))
        })
    }

    /// The exposure toggle under `p` (ADR 0074): the port itself, or a
    /// value's ring, on a node inside a model. Node, kind and item.
    fn toggle_under(&self, d: &Diagram, origin: Pos2, p: Pos2) -> Option<(usize, &str, String)> {
        if let Some((i, k, c)) = self.port_under(d, origin, p) {
            if !d.exposable(i) {
                return None;
            }
            return Some(match c {
                Causality::Input => (i, "in", d.nodes[i].inputs[k].name.clone()),
                Causality::Output => (i, "out", d.nodes[i].outputs[k].name.clone()),
            });
        }
        let (i, k) = self.value_under(d, origin, p)?;
        let n = &d.nodes[i];
        (d.exposable(i) && self.view.toggle_at(origin, n, k).distance(p) <= 7.0 * self.view.zoom)
            .then(|| (i, "par", n.values[k].name.clone()))
    }

    /// The value row under `p`: node and value.
    fn value_under(&self, d: &Diagram, origin: Pos2, p: Pos2) -> Option<(usize, usize)> {
        if self.view.zoom < SHOW_VALUES {
            return None;
        }
        d.nodes.iter().enumerate().rev().find_map(|(i, n)| {
            (0..n.value_rows()).find_map(|k| {
                let r = Rect::from_min_size(
                    self.view.to_screen(origin, n.value_at(k)),
                    Vec2::new(NODE_WIDTH as f32, VALUE_ROW as f32) * self.view.zoom,
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
        let z = self.view.zoom;
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
                    .and_then(|i| self.view.unit_rect(origin, &d.nodes[i], c))
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
                let at = self.view.to_screen(origin, d.nodes[i].value_at(k));
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
        let reach = REACH * self.view.zoom.max(0.7);
        let mut best = None;
        let mut best_dist = reach;
        for (i, n) in d.nodes.iter().enumerate() {
            for (c, count) in [
                (Causality::Input, n.inputs.len()),
                (Causality::Output, n.outputs.len()),
            ] {
                for k in 0..count {
                    let dist = self.view.to_screen(origin, n.port_at(c, k)).distance(p);
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
            .find(|&i| self.view.node_rect(origin, &d.nodes[i]).contains(p))
    }

    /// The name a wire into value `k` of node `i` ends at (ADR 0075),
    /// `node.param` as the canvas names ports (a used model's becomes
    /// `use/param` in the file).
    fn value_key(d: &Diagram, i: usize, k: usize) -> String {
        let n = &d.nodes[i];
        format!("{}.{}", n.name, n.values[k].name)
    }

    /// The value row whose port is under `p` (ADR 0075): node and value.
    fn value_port_under(&self, d: &Diagram, origin: Pos2, p: Pos2) -> Option<(usize, usize)> {
        if self.view.zoom < SHOW_VALUES {
            return None;
        }
        let reach = REACH * self.view.zoom.max(0.7);
        d.nodes.iter().enumerate().rev().find_map(|(i, n)| {
            (n.is_block() && !n.is_observer())
                .then(|| {
                    (0..n.value_rows()).find(|&k| {
                        self.view.to_screen(origin, n.value_port(k)).distance(p) <= reach
                    })
                })
                .flatten()
                .map(|k| (i, k))
        })
    }

    fn wire_under(&self, d: &Diagram, origin: Pos2, p: Pos2) -> Option<String> {
        d.wires
            .iter()
            .find(|w| {
                self.view
                    .wire_path(d, origin, w)
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
                let before = self.view.to_canvas(origin, p);
                self.view.zoom = (self.view.zoom * factor).clamp(0.2, 3.0);
                let after = self.view.to_screen(origin, before);
                self.view.pan += p - after;
            }
        }
        if response.drag_started_by(egui::PointerButton::Primary) {
            // Where the press began: a drag is reported only once the
            // pointer has moved.
            let start = ui.input(|i| i.pointer.press_origin());
            if let Some(p) = start.or(response.interact_pointer_pos()) {
                // A driven value's wire is lifted from its source end
                // (ADR 0075).
                let driven = self.value_port_under(d, origin, p).and_then(|(i, k)| {
                    d.wires
                        .iter()
                        .find(|w| w.param && w.to == (i, k))
                        .map(|w| (w, Self::value_key(d, i, k)))
                });
                if let Some((w, key)) = driven {
                    self.wiring = Some(Dragging {
                        node: w.from.0,
                        port: w.from.1,
                        causality: Causality::Output,
                        lifted: Some(key),
                    });
                } else if let Some((node, port, c)) = self.port_under(d, origin, p) {
                    let key = |n: usize, q: &str| format!("{}.{}", d.nodes[n].name, q);
                    // Dragging a connected input lifts its wire from the
                    // source end.
                    // A data-layer input may read several signals: dragging
                    // from it starts a new wire instead.
                    let fed = (c == Causality::Input && !d.nodes[node].is_observer())
                        .then(|| d.wires.iter().find(|w| !w.param && w.to == (node, port)))
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
                        .map(|(k, n)| (k, p - self.view.to_screen(origin, n.at)))
                        .collect();
                } else if ui.input(|i| i.modifiers.shift) {
                    self.band = Some(p);
                }
            }
        }
        if response.dragged_by(egui::PointerButton::Primary) {
            if let (false, Some(p)) = (self.moving.is_empty(), pointer) {
                for &(i, grab) in &self.moving {
                    d.nodes[i].at = self.view.to_canvas(origin, p - grab);
                }
            } else if self.wiring.is_none() && self.band.is_none() {
                self.view.pan += response.drag_delta();
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
                    .filter(|n| n.is_block() && r.intersects(self.view.node_rect(origin, n)))
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
                // From an output onto a value: the wire drives it (ADR 0075).
                let onto_value = (g.causality == Causality::Output)
                    .then(|| pointer.and_then(|p| self.value_port_under(d, origin, p)))
                    .flatten();
                let new = match (target, onto_value) {
                    (Some((n, k, c)), _) if c != g.causality => {
                        let there = key(n, c, k);
                        Some(match c {
                            Causality::Input => (here.clone(), there),
                            Causality::Output => (there, here.clone()),
                        })
                    }
                    (_, Some((i, k))) => Some((here.clone(), Self::value_key(d, i, k))),
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
                            // A probe records it too.
                            (_, Some(NodeKind::Probe)) => edits.push(Edit::Bind {
                                many: true,
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
                .find(|n| self.view.expander(origin, n).is_some_and(|r| r.contains(p)))
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
            // Exposing a port or parameter inside a model (ADR 0074).
            let toggle = response
                .interact_pointer_pos()
                .and_then(|p| self.toggle_under(d, origin, p));
            if let Some((i, kind, item)) = toggle {
                edits.push(d.toggle_exposed(i, kind, &item));
                return;
            }
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
                // An exposed value is the model's parameter: changed in the
                // Inspector, not over the block's own (ADR 0074).
                if self.view.zoom >= EDIT_VALUES && v.exposed.is_none() && v.driven.is_none() {
                    self.editing = Some((v.key.clone(), format_number(v.value)));
                }
            } else if let Some((i, c)) = p.and_then(|p| self.unit_under(d, origin, p)) {
                // A conversion's unit (ADR 0045).
                let n = &d.nodes[i];
                if let (Some(setting), true) = (unit_setting(n, c), self.view.zoom >= EDIT_VALUES) {
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
                self.menu = Some((self.view.to_canvas(origin, p), on));
            }
        }
        let delete =
            ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace));
        let typing = ui.ctx().egui_wants_keyboard_input();
        // Ctrl+D duplicates the selected blocks (ADR 0069).
        let duplicate = ui.input_mut(|i| {
            i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::D,
            ))
        });
        if duplicate && !typing && response.hovered() {
            if let Some(s) = &self.selected {
                let blocks: Vec<String> = s
                    .blocks()
                    .into_iter()
                    .filter(|b| d.node(b).is_some_and(|k| d.nodes[k].is_block()))
                    .collect();
                if !blocks.is_empty() {
                    edits.push(Edit::Duplicate { blocks });
                }
            }
        }
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
                    NodeKind::Block => {
                        if ui.button("Edit…").on_hover_text("Its equations or settings").clicked() {
                            requests.push(Request::Open(name.clone()));
                            ui.close();
                        }
                    }
                    NodeKind::Use => {
                        ui.weak(format!("Uses {}, {}", n.type_name, n.method));
                        if ui.button("Open").on_hover_text("See inside the model it uses").clicked() {
                            requests.push(Request::Open(name.clone()));
                            ui.close();
                        }
                        if ui
                            .button("Expand in place")
                            .on_hover_text("Show its blocks here, in an outline; edits there change the model it uses")
                            .clicked()
                        {
                            requests.push(Request::Expand(name.clone()));
                            ui.close();
                        }
                        // A study of the same model beside it (ADR 0070).
                        if ui
                            .button("Study this model")
                            .on_hover_text("Add a study block that runs this model at 20 values of its first parameter; edit its sample in the Inspector")
                            .clicked()
                        {
                            edits.push(Edit::Study { block: name.clone() });
                            ui.close();
                        }
                        // A definition goes to a file and back (ADR 0068).
                        if n.type_name.starts_with("model ") {
                            if ui
                                .button("Move to its own file…")
                                .on_hover_text("Write this model to a model file; every use here uses the file")
                                .clicked()
                            {
                                requests.push(Request::MoveToFile(name.clone()));
                                ui.close();
                            }
                        } else if ui
                            .button("Put in this file")
                            .on_hover_text("Copy the model file into this model as a model defined here; the file stays as it is")
                            .clicked()
                        {
                            requests.push(Request::PutInFile(name.clone()));
                            ui.close();
                        }
                    }
                    NodeKind::Filter => {
                        if ui.button("Edit in Data…").on_hover_text("Its formula or cuts").clicked() {
                            requests.push(open_request(n));
                            ui.close();
                        }
                        if n.settings.contains_key("clock") {
                            let mut any = n.method.ends_with(" · on any");
                            if ui
                                .checkbox(&mut any, "Run when any input records")
                                .on_hover_text(
                                    "Off: it runs where all its inputs' sources recorded at the \
                                     same time. On: whenever one of them records, with the \
                                     others' latest values (ADR 0063)",
                                )
                                .changed()
                            {
                                edits.push(Edit::SetFilterWhen {
                                    filter: name.clone(),
                                    any,
                                });
                                ui.close();
                            }
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
                    NodeKind::Probe => {
                        if ui
                            .button("Settings…")
                            .on_hover_text("When it records, how long it keeps samples, its file")
                            .clicked()
                        {
                            requests.push(open_request(n));
                            ui.close();
                        }
                    }
                    NodeKind::Recorder => {
                        if ui
                            .button("Settings…")
                            .on_hover_text("How often it saves the model's state, and how much memory it may use")
                            .clicked()
                        {
                            requests.push(open_request(n));
                            ui.close();
                        }
                    }
                }
                ui.add_enabled_ui(!matches!(n.kind, NodeKind::View | NodeKind::Recorder), |ui| ui.horizontal(|ui| {
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
                    // A model defined in the file (ADR 0068).
                    let label = match group.len() {
                        1 => "Make model".to_string(),
                        k => format!("Make model of {k} blocks"),
                    };
                    if ui
                        .button(label)
                        .on_hover_text("Move into a new model defined in this file; wires that cross become its input blocks and outputs (Shift-click or Shift-drag to select several)")
                        .clicked()
                    {
                        edits.push(Edit::GroupModel { blocks: group });
                        ui.close();
                    }
                }
                // A study opens to the model it studies (ADR 0070).
                if n.type_name == simsandbox_file::STUDY
                    && ui
                        .button("Open")
                        .on_hover_text("See inside the model it studies, and edit it")
                        .clicked()
                {
                    requests.push(Request::Open(name.clone()));
                    ui.close();
                }
                if n.is_block() {
                    let many = selection.len() > 1 && selection.contains(&name);
                    let label = if many {
                        format!("Duplicate {} blocks", selection.len())
                    } else {
                        "Duplicate".to_string()
                    };
                    if ui.button(label).on_hover_text("Copies beside it, with its values (Ctrl+D)").clicked() {
                        let blocks = if many { selection.clone() } else { vec![name.clone()] };
                        edits.push(Edit::Duplicate { blocks });
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
                // Inside an expanded use: collapse it (ADR 0066 step 4d).
                for o in d.outlines.iter().filter(|o| {
                    at[0] >= o.rect[0] && at[0] <= o.rect[2] && at[1] >= o.rect[1] && at[1] <= o.rect[3]
                }) {
                    if ui.button(format!("Collapse {}", o.path)).clicked() {
                        requests.push(Request::Collapse(o.path.clone()));
                        ui.close();
                    }
                }
                ui.label("Add block");
                if let Some(type_name) = type_menu(ui, registry, &d.subsystems) {
                    edits.push(Edit::Add { type_name, at });
                    ui.close();
                }
                if ui
                    .button("Add model…")
                    .on_hover_text("Use a model file here, on its own clock (ADR 0066)")
                    .clicked()
                {
                    requests.push(Request::AddModel(at));
                    ui.close();
                }
                // Run the whole model at many points (ADR 0071).
                if !d.nodes.iter().all(|n| n.is_observer())
                    && ui
                        .button("Study this model")
                        .on_hover_text("Move every block into a model defined in this file, used in their place, with a study of it beside: it runs the model at many values of a parameter")
                        .clicked()
                {
                    edits.push(Edit::StudyModel);
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
                    if ui
                        .button("Add probe")
                        .on_hover_text(
                            "Records the signals and fields wired into it, when and for as long \
                             as you choose, and can save them to a file (ADR 0062)",
                        )
                        .clicked()
                    {
                        edits.push(Edit::AddProbe { at });
                        ui.close();
                    }
                    if d.node(simsandbox_session::observers::RECORDER).is_none()
                        && ui
                            .button("Add recorder")
                            .on_hover_text(
                                "Saves the model's state as it runs, so it can be rewound \
                                 (ADR 0064); without one the rewind bar is hidden",
                            )
                            .clicked()
                    {
                        edits.push(Edit::AddRecorder { at });
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
                    ui.label(simsandbox_session::observers::feeds_model(&filter));
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
    #[cfg(test)]
    fn offered(&self) -> Option<(&str, &str)> {
        self.offer
            .as_ref()
            .map(|(f, t, _)| (f.as_str(), t.as_str()))
    }
}

/// What double-clicking a node opens.
fn open_request(n: &Node) -> Request {
    match n.kind {
        NodeKind::Filter => Request::EditData(title(n)),
        NodeKind::View => Request::ShowView(n.name["view:".len()..].parse().unwrap_or(0)),
        NodeKind::Probe => Request::EditProbe(title(n)),
        NodeKind::Recorder => Request::EditRecorder,
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
        NodeKind::Probe => format!(
            "Probe {} ({}): records the signals and fields wired into it, without changing \
             results. Drop more on it; double-click for its settings.",
            title(n),
            n.method
        ),
        NodeKind::Recorder => format!(
            "Recorder ({}): saves the whole model's state as it runs, so it can be rewound. \
             Double-click for its settings; delete it to stop.",
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

fn port_help(block: &str, p: &simsandbox_session::diagram::Port, c: Causality) -> String {
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
        Shape::Rows { dim: 1 } => "a value per row of a table".to_string(),
        Shape::Rows { dim } => format!("{dim} values per row of a table"),
        Shape::Tensor { rank, dims } => {
            let d: Vec<String> = dims[..rank].iter().map(|n| n.to_string()).collect();
            format!("values over {rank} axes, {}", d.join(" × "))
        }
    };
    let unit = if p.unit.is_empty() || p.unit == "1" {
        "no unit".to_string()
    } else {
        p.unit.clone()
    };
    format!("{block}.{} ({dir}): {shape}, {unit}", p.name)
}

fn segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t).distance(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use simsandbox_file::ModelFile;
    use simsandbox_session::diagram_paint::rows;
    use simsandbox_session::examples::{registry, EXAMPLES};

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
            out = canvas.show(ui, d, &r);
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
        c.view.to_screen(Pos2::ZERO, n.port_at(causality, k))
    }

    #[test]
    fn a_wire_dropped_on_a_value_drives_it_and_is_lifted_off_it() {
        let e = EXAMPLES
            .iter()
            .find(|e| e.name.contains("Forced spring"))
            .unwrap();
        let (f, _) = ModelFile::from_json(e.text).unwrap();
        let mut d = Diagram::new(&f, &registry());
        let ctx = egui::Context::default();
        let mut c = Canvas {
            open: d.nodes.iter().map(|n| n.name.clone()).collect(),
            ..Canvas::default()
        };
        frame(&ctx, &mut c, &mut d.clone(), vec![]);
        // Zoomed in enough to show values (ADR 0053).
        c.view.zoom = c.view.zoom.max(SHOW_VALUES);
        frame(&ctx, &mut c, &mut d.clone(), vec![]);
        let value = |c: &Canvas, d: &Diagram, node: &str, name: &str| {
            let n = &d.nodes[d.node(node).unwrap()];
            let k = n.values.iter().position(|v| v.name == name).unwrap();
            c.view.to_screen(Pos2::ZERO, n.value_port(k))
        };
        // push.F onto held's value F: a wire that drives it (ADR 0075).
        let a = screen(&c, &d, "push", Causality::Output, "F");
        let b = value(&c, &d, "held", "F");
        let edits = drag(&ctx, &mut c, &mut d, a, b);
        assert_eq!(
            edits,
            [Edit::Connect {
                from: "push.F".into(),
                to: "held.F".into()
            }]
        );
        // Off shaken's driven F onto empty canvas: the wire goes.
        let mut d = Diagram::new(&f, &registry());
        let from = value(&c, &d, "shaken", "F");
        let edits = drag(&ctx, &mut c, &mut d, from, Pos2::new(5.0, 590.0));
        assert_eq!(
            edits,
            [Edit::Disconnect {
                to: "shaken.F".into()
            }]
        );
    }

    #[test]
    fn dragging_a_block_moves_it_and_saves_where_it_ended() {
        let (mut d, mut c, ctx) = cruise();
        let n = d.node("car").unwrap();
        let start = d.nodes[n].at;
        let grab = c
            .view
            .to_screen(Pos2::ZERO, [start[0] + 60.0, start[1] + 10.0]);
        let edits = drag(&ctx, &mut c, &mut d, grab, grab + Vec2::new(100.0, 50.0));
        let [Edit::Move { block, at }] = &edits[..] else {
            panic!("{edits:?}");
        };
        assert_eq!(block, "car");
        let dx = (at[0] - start[0]) * c.view.zoom as f64;
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
        d.observe(&f, &simsandbox_session::examples::view_registry());
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
        let p = c.view.to_screen(Pos2::ZERO, [0.0, 0.0]);
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
        c.view.zoom = c.view.zoom.max(1.0);
        frame(&ctx, &mut c, &mut d, vec![]);
        // Closed: no values until its triangle is clicked.
        assert_eq!(d.nodes[n].value_rows(), 0);
        let e = c.view.expander(Pos2::ZERO, &d.nodes[n]).unwrap().center();
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
        let p = c.view.to_screen(Pos2::ZERO, [at[0] + 100.0, at[1] + 8.0]);
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
        c.view.zoom = 0.4;
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
        c.view.zoom = 1.0;
        let at = d.nodes[n].at;
        c.view.pan = Vec2::new(100.0 - at[0] as f32, 100.0 - at[1] as f32);
        frame(&ctx, &mut c, &mut d, vec![]);
        let p = c
            .view
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
        let p = c
            .view
            .to_screen(Pos2::ZERO, [n.at[0] + 50.0, n.at[1] + 5.0]);
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
        let before = c.view.to_canvas(Pos2::ZERO, p);
        frame(&ctx, &mut c, &mut d, vec![egui::Event::PointerMoved(p)]);
        let zoom = c.view.zoom;
        frame(
            &ctx,
            &mut c,
            &mut d,
            vec![egui::Event::Zoom(1.5), egui::Event::PointerMoved(p)],
        );
        assert!(
            (c.view.zoom / zoom - 1.5).abs() < 1e-3,
            "{} {}",
            c.view.zoom,
            zoom
        );
        let after = c.view.to_canvas(Pos2::ZERO, p);
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
        let grab = c
            .view
            .to_screen(Pos2::ZERO, [n.at[0] + 60.0, n.at[1] + 10.0]);
        let edits = drag(&ctx, &mut c, &mut d, grab, grab + Vec2::new(30.0, 0.0));
        assert_eq!(edits.len(), 2, "{edits:?}");
        for (k, n) in d.nodes.iter().enumerate() {
            assert!(n.at[0] > before[k][0]);
        }
        // Delete removes both.
        let p = c.view.to_screen(
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
            let p = c
                .view
                .to_screen(Pos2::ZERO, [n.at[0] + 50.0, n.at[1] + 5.0]);
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
