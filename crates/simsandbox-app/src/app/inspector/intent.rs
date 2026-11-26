//! The Inspector by intent (ADR 0108 point 5's fourth step, ADR 0109
//! point 5): for a block, what it does first (its purpose or summary, its
//! method and labels, what it is solved with), then its key parameters,
//! then the rest grouped by role, starting values last. Drive parameters
//! at 0 that nothing drives are counted, not listed.

use super::*;
use serde_json::Value;
use simsandbox_core::block::ParamRole;

/// Roles shown open; the others start closed when long.
const OPEN_ROLES: [ParamRole; 3] = [ParamRole::Property, ParamRole::Drive, ParamRole::Tuning];

/// Parameter rows by their names in the block.
type Rows = Vec<(String, ParamRow)>;

/// A drive parameter at 0 that nothing drives: there, but idle.
fn idle(r: &ParamRow) -> bool {
    r.role == ParamRole::Drive && r.driven.is_none() && r.value == 0.0
}

/// The rows of a block in the order the Inspector tells them: key
/// parameters, then each role's (starting values last), and how many
/// drive parameters sit idle.
pub(super) fn by_intent(
    rows: &[(String, ParamRow)],
    pins: &BTreeMap<String, simsandbox_file::ParamControl>,
) -> (Rows, Vec<(ParamRole, Rows)>, usize) {
    let pinned = |r: &ParamRow| pins.get(&r.key).is_some_and(|c| c.pinned());
    let any_pinned = rows.iter().any(|(_, r)| pinned(r));
    let key = |r: &ParamRow| {
        if any_pinned {
            pinned(r)
        } else {
            r.suggested_key
        }
    };
    let keys: Vec<(String, ParamRow)> = rows.iter().filter(|(_, r)| key(r)).cloned().collect();
    let mut groups = Vec::new();
    let mut resting = 0;
    let mut order = ParamRole::ALL.to_vec();
    // Starting values last, as they apply on Restart.
    order.retain(|r| *r != ParamRole::Start);
    order.push(ParamRole::Start);
    for role in order {
        let list: Vec<(String, ParamRow)> = rows
            .iter()
            .filter(|(_, r)| r.role == role && !key(r))
            .filter(|(_, r)| {
                let i = idle(r);
                resting += usize::from(i);
                !i
            })
            .cloned()
            .collect();
        if !list.is_empty() {
            groups.push((role, list));
        }
    }
    (keys, groups, resting)
}

impl SimApp {
    /// The model's own card, for what each of its blocks does: built when
    /// the file changes.
    pub(super) fn top_card(&mut self) -> Option<Value> {
        let live = self.session.as_ref()?;
        let m = live.model();
        if let Some((file, card)) = &self.top_card {
            if *file == m.file {
                return Some(card.clone());
            }
        }
        let summaries = simsandbox_session::card::type_summaries(&self.registry);
        let card = live
            .call(move |s, _| simsandbox_session::card::card(s, &|t| summaries.get(t).cloned()))
            .ok()?;
        self.top_card = Some((m.file.clone(), card.clone()));
        Some(card)
    }

    /// What a block of the model does, in a line or two: its purpose or
    /// summary (the first sentence, the rest on hover), its method and
    /// labels, what it is solved with.
    pub(super) fn what_it_does(&mut self, ui: &mut egui::Ui, block: &str) {
        let Some(card) = self.top_card() else {
            return;
        };
        let Some(part) = card["parts"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|p| p["name"] == block && p["kind"] == "block")
        else {
            return;
        };
        let said = part["notes"]["purpose"]
            .as_str()
            .or_else(|| part["summary"].as_str())
            .unwrap_or("")
            .replace('\n', " ");
        if !said.is_empty() {
            // Its first sentence, at most a couple of lines; all on hover.
            let first = said.split_inclusive(". ").next().unwrap_or(&said).trim();
            let short = if first.chars().count() > 140 {
                let cut: String = first.chars().take(140).collect();
                let cut = cut
                    .rsplit_once(' ')
                    .map_or(cut.as_str(), |(a, _)| a)
                    .to_string();
                format!("{cut} …")
            } else {
                first.to_string()
            };
            ui.label(short).on_hover_text(&said);
        }
        let m = &part["method"];
        let fused: Vec<&str> = part["fused"]["with"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let with = if fused.is_empty() {
            String::new()
        } else if fused.len() <= 3 {
            format!("; solved with {}", fused.join(", "))
        } else {
            format!("; solved with {} others as one system", fused.len())
        };
        ui.weak(format!(
            "{}: {} speed, {} accuracy{with}",
            m["name"].as_str().unwrap_or(""),
            m["speed"].as_str().unwrap_or(""),
            m["accuracy"].as_str().unwrap_or(""),
        ))
        .on_hover_text(format!(
            "Its method and labels (ADR 0004){}",
            if fused.is_empty() {
                String::new()
            } else {
                format!("; solved as one system with {}", fused.join(", "))
            }
        ));
    }

    /// A block's parameters by intent (see the module): `rows` by name.
    pub(super) fn intent_rows(
        &mut self,
        ui: &mut egui::Ui,
        m: &Model,
        salt: &str,
        rows: &[(String, ParamRow)],
        used_by: Option<&BTreeMap<String, Vec<String>>>,
        c: &mut Collected,
    ) {
        let (keys, groups, resting) = by_intent(rows, &m.file.controls.params);
        let row = |ui: &mut egui::Ui,
                   app: &mut SimApp,
                   c: &mut Collected,
                   (name, r): &(String, ParamRow)| {
            let mut r = r.clone();
            r.name.clone_from(name);
            let used = used_by.and_then(|u| u.get(&r.key));
            param_row(ui, m, &r, Place::Tree, used, edits!(app, c));
        };
        if !keys.is_empty() {
            if !groups.is_empty() {
                ui.weak("key parameters").on_hover_text(
                    "Those the model pins to the Controls, else those its type suggests (ADR 0109)",
                );
            }
            egui::Grid::new(("intent keys", salt))
                .num_columns(4)
                .spacing([6.0, 4.0])
                .show(ui, |ui| {
                    for k in &keys {
                        row(ui, self, c, k);
                    }
                });
        }
        // With no key parameters and one role, no headers are needed.
        let plain = keys.is_empty() && groups.len() == 1;
        for (role, list) in &groups {
            let draw = |ui: &mut egui::Ui, app: &mut SimApp, c: &mut Collected| {
                egui::Grid::new(("intent", salt, role.name()))
                    .num_columns(4)
                    .spacing([6.0, 4.0])
                    .show(ui, |ui| {
                        for r in list {
                            row(ui, app, c, r);
                        }
                    });
            };
            if plain {
                draw(ui, self, c);
                continue;
            }
            let title = if *role == ParamRole::Start {
                format!("start values ({})", list.len())
            } else {
                format!("{} ({})", role.name(), list.len())
            };
            egui::CollapsingHeader::new(title)
                .id_salt(("intent role", salt, role.name()))
                .default_open(OPEN_ROLES.contains(role) || list.len() <= 4)
                .show(ui, |ui| draw(ui, self, c));
        }
        if resting > 0 {
            ui.weak(format!(
                "and {resting} drive parameters at 0 that nothing drives"
            ))
            .on_hover_text(
                "Forces and loads nothing sets: wire one, or tick \"computed\" to list them",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cart_poles_mechanism_tells_its_key_parameters_first() {
        // ADR 0109 point 5: key parameters first, then by role, drives at
        // 0 counted.
        let mut app = SimApp::new(None, None, None, None);
        let k = EXAMPLES
            .iter()
            .position(|e| e.name.starts_with("Cart-pole"))
            .unwrap();
        app.open_example(k);
        let m = app.session.as_ref().unwrap().model();
        let rows: Vec<(String, ParamRow)> = m
            .params
            .iter()
            .filter(|r| r.owner == "cartpole")
            .map(|r| (r.name.clone(), r.clone()))
            .collect();
        let (keys, groups, resting) = by_intent(&rows, &m.file.controls.params);
        // The model pins pole_mass, so that is its key parameter.
        let keys: Vec<&str> = keys.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(keys, ["pole_mass"]);
        let roles: Vec<ParamRole> = groups.iter().map(|(r, _)| *r).collect();
        assert_eq!(roles.last(), Some(&ParamRole::Start));
        assert!(roles.contains(&ParamRole::Geometry), "{roles:?}");
        // Forces and loads nothing drives are counted, not listed.
        assert!(resting > 0);
        assert!(groups.iter().flat_map(|(_, l)| l).all(|(_, r)| !idle(r)));
        // Drawn, with what it does, headless.
        app.picked = vec![(None, "cartpole".into())];
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| app.docked(ui));
        out.textures_delta.clear();
        let card = app.top_card().unwrap();
        assert!(card["parts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == "cartpole"));
    }
}
