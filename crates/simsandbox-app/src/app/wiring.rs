//! Connecting by intent (the Structure lens, ADR 0108 step 4; the
//! interface report, section 6). Three ways, one edit (`Edit::Connect`):
//! the Inspector's "Wire into it…" lists what fits each input and
//! parameter; a block dragged onto another (from the Overview, or from a
//! view on the Stage drawing the selected block) offers the connections
//! that fit, in a small window; the diagram takes the same drop.

use super::SimApp;
use eframe::egui;
use simsandbox_session::diagram::{Diagram, Edit, Scope};
use simsandbox_session::wiring;

/// A block being dragged onto another to wire them (ADR 0108 step 4).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DraggedBlock(pub(crate) String);

/// "wire": a handle that drags `block` onto another block.
pub(super) fn handle(ui: &mut egui::Ui, block: &str) {
    ui.dnd_drag_source(
        // Apart wherever it is shown (a view's header, the Inspector).
        ui.make_persistent_id(("drag block", block)),
        DraggedBlock(block.to_string()),
        |ui| ui.label(egui::RichText::new("wire").small().underline()),
    )
    .response
    .on_hover_text(format!(
        "Drag {block} onto another block (here, on the Overview or on the diagram) to wire its outputs into it"
    ));
}

/// The block dropped on `response` this frame, if one was.
pub(super) fn dropped(response: &egui::Response) -> Option<String> {
    response
        .dnd_release_payload::<DraggedBlock>()
        .map(|p| p.0.clone())
}

impl SimApp {
    /// The model's diagram at its own level, for what fits where.
    fn wiring_diagram(&self) -> Option<Diagram> {
        let live = self.session.as_ref()?;
        Some(Diagram::new(&live.model().file, &self.registry))
    }

    /// `from` dropped on `to`: the connections that fit, to choose from.
    pub(super) fn offer_wiring(&mut self, from: &str, to: &str) {
        if from == to {
            return;
        }
        self.wire_offer = Some((from.to_string(), to.to_string()));
    }

    /// The window of connections from one block to another, while one is
    /// offered.
    pub(super) fn wiring_window(&mut self, ctx: &egui::Context) {
        let Some((from, to)) = self.wire_offer.clone() else {
            return;
        };
        let Some(d) = self.wiring_diagram() else {
            self.wire_offer = None;
            return;
        };
        let pairs = wiring::pairs(&d, &from, &to);
        let mut open = true;
        let mut chosen = None;
        egui::Window::new(format!("Wire {from} into {to}"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                if pairs.is_empty() {
                    ui.label(format!(
                        "No output of {from} fits an input or parameter of {to}: units and shapes must be the same"
                    ));
                    return;
                }
                ui.weak("Same units and shapes; same names first");
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for p in &pairs {
                        let mut text = format!("{} to {}", p.from, p.to);
                        if p.drives {
                            text.push_str(" (drives it)");
                        }
                        if p.replaces {
                            text.push_str(" (replaces what sets it now)");
                        }
                        if ui.button(text).clicked() {
                            chosen = Some(p.clone());
                        }
                    }
                });
            });
        if let Some(p) = chosen {
            self.wire_offer = None;
            self.edit_at(
                Scope::Model,
                Edit::Connect {
                    from: p.from,
                    to: p.to,
                },
            );
        } else if !open {
            self.wire_offer = None;
        }
    }

    /// "Wire into it…" for block `block`: each input and parameter, with
    /// the outputs that fit it. Built only while open.
    pub(super) fn wire_into_menu(&mut self, ui: &mut egui::Ui, block: &str) {
        let mut chosen = None;
        ui.menu_button("Wire into it…", |ui| {
            let Some(d) = self.wiring_diagram() else {
                return;
            };
            let Some(n) = d.node(block).map(|i| d.nodes[i].clone()) else {
                return;
            };
            let targets: Vec<String> = n
                .inputs
                .iter()
                .map(|p| format!("{block}.{}", p.name))
                .chain(
                    n.values
                        .iter()
                        .filter(|v| v.formula.is_none() && !v.initial)
                        .map(|v| v.key.clone()),
                )
                .collect();
            let mut any = false;
            for t in targets {
                let Ok(sources) = wiring::sources(&d, &t) else {
                    continue;
                };
                if sources.is_empty() {
                    continue;
                }
                any = true;
                let now = d.wires.iter().find(|w| w.to_name == t).map(|w| w.from_name.clone());
                let label = match &now {
                    Some(f) => format!("{t} (from {f})"),
                    None => t.clone(),
                };
                ui.menu_button(label, |ui| {
                    for s in sources {
                        let on = now.as_deref() == Some(s.signal.as_str());
                        if ui.selectable_label(on, &s.signal).clicked() && !on {
                            chosen = Some((s.signal.clone(), t.clone()));
                            ui.close();
                        }
                    }
                });
            }
            if !any {
                ui.weak("Nothing in the model fits its inputs or parameters");
            }
        })
        .response
        .on_hover_text(format!(
            "Feed an input or drive a parameter of {block} from an output that fits (same unit and shape)"
        ));
        if let Some((from, to)) = chosen {
            self.edit_at(Scope::Model, Edit::Connect { from, to });
        }
    }
}
