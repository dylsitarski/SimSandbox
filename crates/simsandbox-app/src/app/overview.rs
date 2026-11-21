//! The Overview tab (ADR 0108): the model's card (ADR 0107), opened first
//! for every model. Built on the worker from the session when the file
//! changes; values are read live from the published parameters, so they
//! follow sliders and wires. Each part can be inspected from here.

use super::*;
use serde_json::Value;

/// Roles shown open; the others start closed when long.
const OPEN_ROLES: [&str; 3] = ["property", "drive", "tuning"];

fn text(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

fn strings(v: &Value) -> Vec<&str> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

/// Assumptions and reasons, as the card writes them.
fn notes(ui: &mut egui::Ui, notes: &Value) {
    let assumptions = strings(&notes["assumptions"]);
    if !assumptions.is_empty() {
        ui.label(egui::RichText::new("Assumptions").strong());
        for a in assumptions {
            ui.label(format!("• {a}"));
        }
    }
    if let Some(reasons) = notes["reasons"].as_object().filter(|r| !r.is_empty()) {
        ui.label(egui::RichText::new("Why these values").strong());
        for (k, v) in reasons {
            ui.label(format!("• {k}: {}", text(v)));
        }
    }
}

impl SimApp {
    /// The open model's card, built again when its file changed.
    pub(super) fn overview_card(&mut self) -> Option<Value> {
        let live = self.session.as_ref()?;
        let m = live.model();
        if let Some((file, card)) = &self.overview {
            if *file == m.file {
                return Some(card.clone());
            }
        }
        let summaries = simsandbox_session::card::type_summaries(&self.registry);
        let card = live
            .call(move |s, _| simsandbox_session::card::card(s, &|t| summaries.get(t).cloned()))
            .ok()?;
        self.overview = Some((m.file.clone(), card.clone()));
        Some(card)
    }

    /// The Overview tab.
    pub(super) fn overview_tab(&mut self, ui: &mut egui::Ui) {
        let Some(card) = self.overview_card() else {
            return;
        };
        let Some(live) = &self.session else {
            return;
        };
        let m = live.model();
        // A parameter's value now: driven, set by a formula, or as it is.
        let value = |key: &str, unit: &str| -> String {
            match m.params.iter().find(|r| r.key == key) {
                Some(r) => {
                    let v = format_value(r.value);
                    let with_unit = if unit.is_empty() {
                        v
                    } else {
                        format!("{v} {unit}")
                    };
                    match &r.driven {
                        Some(d) => format!("{with_unit} (driven by {d})"),
                        None => with_unit,
                    }
                }
                None => String::new(),
            }
        };
        let mut edits: Vec<Edit> = Vec::new();
        let mut inspect: Option<String> = None;
        egui::ScrollArea::vertical().id_salt("overview").show(ui, |ui| {
            ui.heading(text(&card["title"]));
            if let Some(d) = card["description"].as_str() {
                ui.label(d);
            }
            if card["agent"] == true {
                if let Some(e) = inspector::agent_mark(ui, "the model's settings, views or declarations", "") {
                    edits.push(e);
                }
            }
            notes(ui, &card["notes"]);
            let t = &card["trust"];
            ui.label(format!(
                "Trust: {} speed, {} accuracy (the least of its methods); no case verified yet.",
                text(&t["speed"]),
                text(&t["accuracy"]),
            ))
            .on_hover_text("Each method's labels are below with its part (ADR 0004); verified is earned per case (ADR 0061)");
            ui.add_space(6.0);

            let keys = &card["key_parameters"];
            if let Some(list) = keys["params"].as_array().filter(|l| !l.is_empty()) {
                ui.label(egui::RichText::new(format!("Key parameters ({})", text(&keys["from"]))).strong());
                egui::Grid::new("overview keys").striped(true).show(ui, |ui| {
                    for p in list {
                        let key = text(&p["key"]);
                        if ui.link(key).on_hover_text("Inspect its block").clicked() {
                            inspect = key.split_once('.').map(|(b, _)| b.to_string());
                        }
                        ui.label(value(key, text(&p["unit"])));
                        ui.weak(text(&p["role"]));
                        ui.label(text(&p["description"]));
                        ui.end_row();
                    }
                });
                ui.add_space(6.0);
            }

            ui.label(egui::RichText::new("Parts").strong());
            for part in card["parts"].as_array().into_iter().flatten() {
                let name = text(&part["name"]);
                let header = format!("{name} — {}", text(&part["role"]));
                egui::CollapsingHeader::new(egui::RichText::new(header).strong())
                    .id_salt(("overview part", name))
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.weak(text(&part["type"]));
                            if ui.small_button("Inspect").on_hover_text("Select it: the Inspector and the diagram follow").clicked() {
                                inspect = Some(name.to_string());
                            }
                        });
                        if part["agent"] == true {
                            if let Some(e) = inspector::agent_mark(ui, "this block", name) {
                                edits.push(e);
                            }
                        }
                        match part["notes"]["purpose"].as_str() {
                            Some(p) => {
                                ui.label(format!("Purpose: {p}"));
                            }
                            None => {
                                if let Some(s) = part["summary"].as_str().filter(|s| !s.is_empty()) {
                                    let s = s.replace('\n', " ");
                                    if part["summary_of_type"] == true {
                                        // A type's summary can be long: its
                                        // first sentence, the rest on hover.
                                        let first = s.split_inclusive(". ").next().unwrap_or(&s).trim();
                                        ui.label(first).on_hover_text(&s);
                                    } else {
                                        ui.label(s);
                                    }
                                }
                            }
                        }
                        notes(ui, &part["notes"]);
                        let me = &part["method"];
                        let fused = strings(&part["fused"]["with"]);
                        let with = if fused.is_empty() {
                            String::new()
                        } else {
                            format!("; solved with {} as one system ({})", fused.join(", "), text(&part["fused"]["method"]))
                        };
                        ui.weak(format!(
                            "Method: {} ({} speed, {} accuracy){with}",
                            text(&me["name"]),
                            text(&me["speed"]),
                            text(&me["accuracy"]),
                        ));
                        let equations = strings(&part["equations"]);
                        if !equations.is_empty() {
                            egui::Frame::group(ui.style()).show(ui, |ui| {
                                for e in equations {
                                    ui.monospace(e);
                                }
                            });
                        }
                        if let Some(bodies) = part["bodies"].as_array() {
                            egui::Grid::new(("overview bodies", name)).striped(true).show(ui, |ui| {
                                for h in ["body", "joint", "kind", "on", "mass"] {
                                    ui.weak(h);
                                }
                                ui.end_row();
                                for b in bodies {
                                    ui.label(text(&b["name"]));
                                    ui.label(text(&b["joint"]));
                                    ui.label(text(&b["kind"]));
                                    ui.label(text(&b["parent"]));
                                    ui.label(format!("{} kg", b["mass"]));
                                    ui.end_row();
                                }
                            });
                        }
                        if let Some(by_role) = part["params"].as_object() {
                            for (role, list) in by_role {
                                let list: Vec<&Value> = list.as_array().into_iter().flatten().collect();
                                // Drive parameters that nothing drives and sit at 0.
                                let idle = |p: &Value| {
                                    p["role"] == "drive" && p["driven_by"].is_null() && p["value"].as_f64() == Some(0.0)
                                };
                                let shown: Vec<&&Value> = list.iter().filter(|p| !idle(p)).collect();
                                let resting = list.len() - shown.len();
                                if shown.is_empty() && resting == 0 {
                                    continue;
                                }
                                let open = OPEN_ROLES.contains(&role.as_str()) || shown.len() <= 6;
                                egui::CollapsingHeader::new(format!("{role} ({})", list.len()))
                                    .id_salt(("overview role", name, role))
                                    .default_open(open)
                                    .show(ui, |ui| {
                                        egui::Grid::new(("overview params", name, role)).striped(true).show(ui, |ui| {
                                            for p in &shown {
                                                ui.label(text(&p["name"]));
                                                ui.label(value(&format!("{name}.{}", text(&p["name"])), text(&p["unit"])));
                                                ui.label(text(&p["description"]));
                                                ui.end_row();
                                            }
                                        });
                                        if resting > 0 {
                                            ui.weak(format!("and {resting} at 0 that nothing drives"));
                                        }
                                    });
                            }
                        }
                        for (what, key, dir) in [("Inputs", "inputs", "from"), ("Outputs", "outputs", "to")] {
                            let ports: Vec<&Value> = part[key].as_array().into_iter().flatten().collect();
                            if ports.is_empty() {
                                continue;
                            }
                            egui::CollapsingHeader::new(format!("{what} ({})", ports.len()))
                                .id_salt(("overview ports", name, key))
                                .show(ui, |ui| {
                                    for p in ports {
                                        let unit = text(&p["unit"]);
                                        let wired = strings(&p[dir]);
                                        let arrow = if wired.is_empty() {
                                            String::new()
                                        } else if dir == "from" {
                                            format!("  ← {}", wired.join(", "))
                                        } else {
                                            format!("  → {}", wired.join(", "))
                                        };
                                        let unit = if unit.is_empty() { String::new() } else { format!(" [{unit}]") };
                                        let desc = p["description"].as_str().filter(|d| !d.is_empty()).map(|d| format!(": {d}")).unwrap_or_default();
                                        ui.label(format!("{}{unit}{desc}{arrow}", text(&p["name"])));
                                    }
                                });
                        }
                    });
            }

            if let Some(bundles) = card["connections"].as_array().filter(|b| !b.is_empty()) {
                ui.add_space(6.0);
                ui.label(egui::RichText::new("How the parts connect").strong());
                for b in bundles {
                    let by_agent = if b["agent"] == true { "  (wired by an agent)" } else { "" };
                    ui.label(format!(
                        "{} → {}: {}{by_agent}",
                        text(&b["from"]),
                        text(&b["to"]),
                        strings(&b["wires"]).join(", "),
                    ));
                }
            }
            if let Some(views) = card["views"].as_array().filter(|v| !v.is_empty()) {
                ui.add_space(6.0);
                ui.label(egui::RichText::new("Views").strong());
                for v in views {
                    let shows = strings(&v["shows"]);
                    let grab = strings(&v["grabbed_with"]);
                    let grab = if grab.is_empty() { String::new() } else { format!("; grabbed with {}", grab.join(", ")) };
                    ui.label(format!(
                        "{} showing {}{grab}",
                        text(&v["type"]),
                        if shows.is_empty() { "nothing bound".to_string() } else { shows.join(", ") },
                    ));
                }
            }
        });
        for e in edits {
            self.edit_at(Scope::Model, e);
        }
        if let Some(name) = inspect {
            self.select(&name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_model_opens_on_its_overview() {
        // ADR 0108: the card is the first tab shown; it is built from the
        // model and follows its file.
        let mut app = SimApp::new(None, None, None, None);
        let k = EXAMPLES
            .iter()
            .position(|e| e.name.starts_with("Cart-pole"))
            .unwrap();
        app.open_example(k);
        assert!(app.error.is_none(), "{:?}", app.error);
        let active = app
            .dock
            .state
            .find_active_focused()
            .map(|(_, pane)| pane.clone());
        assert_eq!(active, Some(Pane::Overview));
        let card = app.overview_card().unwrap();
        let roles: Vec<&str> = card["parts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, ["interaction", "system", "controller"]);
        // An edit changes the file, and the card with it.
        app.edit_at(
            Scope::Model,
            Edit::SetParam {
                block: "control".into(),
                param: "umax".into(),
                value: 25.0,
            },
        );
        let card = app.overview_card().unwrap();
        let control = &card["parts"][2];
        assert_eq!(control["name"], "control");
        // Drawn with the rest of the app, headless.
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 700.0),
            )),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| app.docked(ui));
        out.textures_delta.clear();
        // A layout saved now names it.
        let index = |id: u64| app.views.tiles.iter().position(|t| t.id == id);
        let standard = crate::dock::Dock::standard(&app.tile_ids())
            .to_layout(&index)
            .unwrap();
        assert!(serde_json::to_string(&standard)
            .unwrap()
            .contains("\"overview\""));
    }
}
