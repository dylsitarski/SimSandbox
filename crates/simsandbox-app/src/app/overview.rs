//! The Overview tab (ADR 0108): the model's card (ADR 0107), opened first
//! for every model. Built on the worker from the session when the file
//! changes; values are read live from the published parameters, so they
//! follow sliders and wires. Each part can be inspected from here. It
//! shows one level (ADR 0110): each model used is one part with its
//! interface, opened in place; what reads inside a used model is listed,
//! with Offer as output.

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
        if let Some((file, part, card)) = &self.overview {
            if *file == m.file && *part == self.overview_part {
                return Some(card.clone());
            }
        }
        let summaries = simsandbox_session::card::type_summaries(&self.registry);
        let part = self.overview_part.clone();
        let asked = part.clone();
        let built = live
            .call(move |s, _| {
                simsandbox_session::card::card_at(s, &|t| summaries.get(t).cloned(), &asked, 1)
            })
            .ok()?;
        let card = match built {
            Ok(card) => card,
            // The part is gone (renamed, removed): the model itself.
            Err(_) if !part.is_empty() => {
                self.overview_part.clear();
                return self.overview_card();
            }
            Err(_) => return None,
        };
        self.overview = Some((m.file.clone(), part, card.clone()));
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
        let mut open: Option<String> = None;
        let level = text(&card["part"]).to_string();
        // Inspected from the model's own level, where the diagram is.
        let top = level.is_empty();
        egui::ScrollArea::vertical().id_salt("overview").show(ui, |ui| {
            if !top {
                // Where this level is: each step back a link.
                ui.horizontal_wrapped(|ui| {
                    if ui.link("Model").clicked() {
                        open = Some(String::new());
                    }
                    let mut path = String::new();
                    let steps: Vec<&str> = level.split('/').collect();
                    for (k, seg) in steps.iter().enumerate() {
                        ui.weak(">");
                        if !path.is_empty() {
                            path.push('/');
                        }
                        path.push_str(seg);
                        if k + 1 == steps.len() {
                            ui.strong(*seg);
                        } else if ui.link(*seg).clicked() {
                            open = Some(path.clone());
                        }
                    }
                });
            }
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
                        if top && key.contains('.') {
                            if ui.link(key).on_hover_text("Inspect its block").clicked() {
                                inspect = key.split_once('.').map(|(b, _)| b.to_string());
                            }
                        } else {
                            ui.label(key);
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
                if part["kind"] == "use" {
                    use_part(ui, part, top, &value, &mut open, &mut inspect, &mut edits);
                    continue;
                }
                let name = text(&part["name"]);
                let path = text(&part["path"]);
                let header = format!("{name} — {}", text(&part["role"]));
                egui::CollapsingHeader::new(egui::RichText::new(header).strong())
                    .id_salt(("overview part", name))
                    .default_open(true)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.weak(text(&part["type"]));
                            if top && ui.small_button("Inspect").on_hover_text("Select it: the Inspector and the diagram follow").clicked() {
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
                                                ui.label(value(&format!("{path}.{}", text(&p["name"])), text(&p["unit"])));
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
                                            format!(", from {}", wired.join(", "))
                                        } else {
                                            format!(", to {}", wired.join(", "))
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
                        "{} to {}: {}{by_agent}",
                        text(&b["from"]),
                        text(&b["to"]),
                        strings(&b["wires"]).join(", "),
                    ));
                }
            }
            if let Some(list) = card["reaching_inside"].as_array().filter(|l| !l.is_empty()) {
                ui.add_space(6.0);
                ui.label(egui::RichText::new("Reaching inside").strong()).on_hover_text(
                    "These read inside a used model by path, not through its interface (ADR 0110): they break if that model changes inside",
                );
                for r in list {
                    ui.horizontal_wrapped(|ui| {
                        if r["start"] == true {
                            ui.label(format!("{} starts {} from a state of its own", text(&r["use"]), text(&r["inside"])));
                            return;
                        }
                        ui.label(format!("{} reads {}", text(&r["where"]), text(&r["signal"])));
                        let signal = text(&r["signal"]).to_string();
                        match r["offered_as"].as_str() {
                            Some(o) => {
                                let name = format!("{}/{o}", text(&r["use"]));
                                if ui.small_button(format!("Read {name}")).on_hover_text("Its model offers it already: read it by that name").clicked() {
                                    edits.push(Edit::OfferAsOutput { signal });
                                }
                            }
                            None if r["can_offer"] == true => {
                                if ui
                                    .small_button("Offer as output")
                                    .on_hover_text(format!("{} offers it as an output, read by that name", text(&r["model"])))
                                    .clicked()
                                {
                                    edits.push(Edit::OfferAsOutput { signal });
                                }
                            }
                            None => {
                                ui.weak(format!("offer it in {}", text(&r["file"])));
                            }
                        }
                    });
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
        if let Some(part) = open {
            self.overview_part = part;
        }
    }
}

/// A model used at this level, as one part (ADR 0110): what it is, its
/// interface for this use with values live, Open to show its own level.
fn use_part(
    ui: &mut egui::Ui,
    part: &Value,
    top: bool,
    value: &dyn Fn(&str, &str) -> String,
    open: &mut Option<String>,
    inspect: &mut Option<String>,
    edits: &mut Vec<Edit>,
) {
    let name = text(&part["name"]);
    let path = text(&part["path"]);
    let model = part["model"].as_str().map_or_else(
        || format!("file {}", text(&part["file"])),
        |m| format!("model {m}"),
    );
    egui::CollapsingHeader::new(egui::RichText::new(format!("{name} — {model}")).strong())
        .id_salt(("overview use", path))
        .default_open(true)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .small_button("Open")
                    .on_hover_text("Show what it holds: its own parts, one level down")
                    .clicked()
                {
                    *open = Some(path.to_string());
                }
                if top
                    && ui
                        .small_button("Inspect")
                        .on_hover_text("Select it: the Inspector and the diagram follow")
                        .clicked()
                {
                    *inspect = Some(name.to_string());
                }
                let inside = strings(&part["inside"]);
                if !inside.is_empty() {
                    ui.weak(format!("holds {}", inside.join(", ")));
                }
            });
            if part["agent"] == true {
                if let Some(e) = inspector::agent_mark(ui, "this use or its model", name) {
                    edits.push(e);
                }
            }
            if let Some(s) = part["summary"].as_str().filter(|s| !s.is_empty()) {
                ui.label(s);
            }
            notes(ui, &part["notes"]);
            let t = &part["trust"];
            ui.weak(format!(
                "{} speed, {} accuracy (the least of its methods)",
                text(&t["speed"]),
                text(&t["accuracy"]),
            ));
            let params: Vec<&Value> = part["params"]["property"]
                .as_array()
                .into_iter()
                .flatten()
                .collect();
            if !params.is_empty() {
                egui::Grid::new(("overview use params", path))
                    .striped(true)
                    .show(ui, |ui| {
                        for p in params {
                            let n = text(&p["name"]);
                            ui.label(n);
                            let shown = value(&format!("{path}.{n}"), text(&p["unit"]));
                            match p["formula"].as_str() {
                                Some(f) => ui.label(format!("{shown}  (= {f})")),
                                None => ui.label(shown),
                            };
                            ui.label(text(&p["description"]));
                            ui.end_row();
                        }
                    });
            }
            // In words: the app's font has no arrows.
            for (what, key, dir) in [("in", "inputs", "from"), ("out", "outputs", "to")] {
                for p in part[key].as_array().into_iter().flatten() {
                    let wired = strings(&p[dir]);
                    let wired = if wired.is_empty() {
                        String::new()
                    } else {
                        format!(", {dir} {}", wired.join(", "))
                    };
                    ui.label(format!("{what}: {}{wired}", text(&p["name"])));
                }
            }
        });
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

    #[test]
    fn the_overview_opens_a_used_model_in_place() {
        // ADR 0110: one level at a time; Open shows a use's own parts, and
        // a level that is gone falls back to the model.
        let mut app = SimApp::new(None, None, None, None);
        let k = EXAMPLES
            .iter()
            .position(|e| e.name.starts_with("Platoon"))
            .unwrap();
        app.open_example(k);
        assert!(app.error.is_none(), "{:?}", app.error);
        let card = app.overview_card().unwrap();
        let kinds: Vec<&str> = card["parts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["kind"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, ["use", "use", "use", "block"]);
        app.overview_part = "car2".into();
        let card = app.overview_card().unwrap();
        assert_eq!(card["part"], "car2");
        assert_eq!(card["parts"][0]["path"], "car2/car");
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 700.0),
            )),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| app.overview_tab(ui));
        out.textures_delta.clear();
        app.overview_part = "car9".into();
        let card = app.overview_card().unwrap();
        assert_eq!(card["part"], "");
        assert!(app.overview_part.is_empty());
    }
}
