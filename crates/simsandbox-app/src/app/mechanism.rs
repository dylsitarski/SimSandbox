//! The Inspector's table of a mechanism's bodies (ADR 0105): one row per
//! body and its joint (name, joint, kind, parent); a row opened (`…`)
//! shows its numbers, its inputs and buttons to move or remove it below
//! the table; `+ body` adds one. Each change is one [`Edit::SetMechanism`], so a
//! rename carries the wires, sliders and views along, and a change that
//! does not load waits as a draft with its error, as other edits do.

use super::*;
use serde_json::{Map, Value};

/// One change the table makes.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum RowsEdit {
    /// A column of a row: a number, a pair, a kind or a parent.
    Set {
        row: usize,
        key: String,
        value: Value,
    },
    /// A body's (`name`) or joint's (`joint`) new name.
    Rename { row: usize, key: String, to: String },
    /// A new body hanging from the last one.
    Add,
    /// Remove a row; its children hang from its parent instead.
    Remove(usize),
    /// Move a row up (or down) one place.
    Move { row: usize, up: bool },
    /// List or unlist a joint in `actuated` or a body in `loaded`.
    Toggle {
        setting: String,
        name: String,
        on: bool,
    },
}

/// The columns opened below the table for a row: each with its label and
/// whether it is a pair; units by the joint's kind.
const MORE: [(&str, &str, bool); 9] = [
    ("mass", "mass, kg", false),
    ("at", "joint at (x, y), m", true),
    ("turn", "joint turned, rad", false),
    ("centre", "centre of mass (x, y), m", true),
    ("inertia", "inertia, kg·m²", false),
    ("damping", "damping", false),
    ("stiffness", "stiffness", false),
    ("rest", "spring's rest", false),
    ("start", "start (coordinate, rate)", true),
];

const KINDS: [&str; 3] = ["revolute", "prismatic", "fixed"];

fn text(row: &Map<String, Value>, key: &str) -> String {
    row.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn list(entry: &simsandbox_file::BlockEntry, setting: &str) -> Vec<String> {
    entry
        .settings
        .get(setting)
        .map(|s| {
            s.split([',', ' '])
                .filter(|w| !w.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// A name not yet used by a body or joint: `base1`, `base2`, ….
fn free(rows: &[Map<String, Value>], base: &str) -> String {
    let used = |n: &str| {
        rows.iter()
            .any(|r| text(r, "name") == n || text(r, "joint") == n)
    };
    (1..)
        .map(|k| format!("{base}{k}"))
        .find(|n| !used(n))
        .expect("names are endless")
}

/// The [`Edit::SetMechanism`] a table change makes of `entry`.
pub(super) fn mechanism_edit(entry: &simsandbox_file::BlockEntry, e: &RowsEdit) -> Option<Edit> {
    let mut rows = entry.bodies.clone();
    let mut settings = entry.settings.clone();
    let mut renames = Vec::new();
    match e {
        RowsEdit::Set { row, key, value } => {
            let r = rows.get_mut(*row)?;
            if value.is_null() {
                r.remove(key);
            } else {
                r.insert(key.clone(), value.clone());
            }
        }
        RowsEdit::Rename { row, key, to } => {
            let to = to.trim();
            let from = text(rows.get(*row)?, key);
            if to.is_empty() || to == from {
                return None;
            }
            rows[*row].insert(key.clone(), to.into());
            if key == "name" {
                for r in &mut rows {
                    if text(r, "parent") == from {
                        r.insert("parent".into(), to.into());
                    }
                }
            }
            renames.push((from, to.to_string()));
        }
        RowsEdit::Add => {
            let parent = rows
                .last()
                .map_or("ground".to_string(), |r| text(r, "name"));
            let (name, joint) = (free(&rows, "body"), free(&rows, "joint"));
            let at = if parent == "ground" {
                [0.0, 0.0]
            } else {
                [0.0, -1.0]
            };
            rows.push(
                serde_json::json!({
                    "name": name, "parent": parent, "joint": joint, "kind": "revolute",
                    "at": at, "mass": 1.0, "centre": [0.0, -0.5], "inertia": 0.08
                })
                .as_object()
                .expect("an object")
                .clone(),
            );
        }
        RowsEdit::Remove(row) => {
            let gone = rows.get(*row)?.clone();
            rows.remove(*row);
            let (name, parent) = (text(&gone, "name"), text(&gone, "parent"));
            for r in &mut rows {
                if text(r, "parent") == name {
                    r.insert("parent".into(), parent.clone().into());
                }
            }
            for (setting, who) in [("actuated", text(&gone, "joint")), ("loaded", name)] {
                if let Some(s) = settings.get_mut(setting) {
                    *s = s
                        .split([',', ' '])
                        .filter(|w| !w.is_empty() && *w != who)
                        .collect::<Vec<_>>()
                        .join(", ");
                }
            }
        }
        RowsEdit::Move { row, up } => {
            let other = if *up { row.checked_sub(1)? } else { row + 1 };
            if other >= rows.len() {
                return None;
            }
            rows.swap(*row, other);
        }
        RowsEdit::Toggle { setting, name, on } => {
            let mut now = list(entry, setting);
            now.retain(|w| w != name);
            if *on {
                now.push(name.clone());
            }
            settings.insert(setting.clone(), now.join(", "));
        }
    }
    settings.retain(|_, v| !v.is_empty());
    Some(Edit::SetMechanism {
        block: entry.name.clone(),
        bodies: rows,
        settings,
        renames,
    })
}

impl SimApp {
    /// The table of `entry`'s bodies; the edits it makes.
    pub(super) fn mechanism_section(
        &mut self,
        ui: &mut egui::Ui,
        entry: &simsandbox_file::BlockEntry,
    ) -> Vec<Edit> {
        let mut out: Vec<RowsEdit> = Vec::new();
        let block = entry.name.clone();
        let rows = &entry.bodies;
        let names: Vec<String> = rows.iter().map(|r| text(r, "name")).collect();
        egui::CollapsingHeader::new("Bodies")
            .id_salt(("bodies", &block))
            .default_open(true)
            .show(ui, |ui| {
                // A change waiting as a draft: why it does not load.
                if let Some(p) = self.diagram.draft_problem().filter(|_| {
                    self.diagram
                        .draft_file()
                        .is_some_and(|f| f.blocks.iter().any(|b| b.name == block))
                }) {
                    ui.colored_label(ui.visuals().error_fg_color, p);
                }
                egui::ScrollArea::horizontal()
                    .id_salt(("bodies scroll", &block))
                    .show(ui, |ui| {
                        egui::Grid::new(("bodies grid", &block))
                            .num_columns(5)
                            .striped(true)
                            .show(ui, |ui| {
                                self.bodies_grid(ui, &block, rows, &names, &mut out)
                            });
                    });
                if let Some((_, k)) = self
                    .mech_open
                    .clone()
                    .filter(|(b, k)| *b == block && *k < rows.len())
                {
                    out.extend(self.row_details(ui, entry, k));
                }
                if ui
                    .small_button("+ body")
                    .on_hover_text("A body on a turning joint, hanging from the last one")
                    .clicked()
                {
                    out.push(RowsEdit::Add);
                }
            });
        out.iter()
            .filter_map(|e| mechanism_edit(entry, e))
            .collect()
    }

    /// The table's rows: name, joint, kind, parent, and `…` to open one.
    fn bodies_grid(
        &mut self,
        ui: &mut egui::Ui,
        block: &str,
        rows: &[Map<String, Value>],
        names: &[String],
        out: &mut Vec<RowsEdit>,
    ) {
        for h in ["body", "joint", "kind", "on", ""] {
            ui.label(egui::RichText::new(h).small().weak());
        }
        ui.end_row();
        for (k, r) in rows.iter().enumerate() {
            for key in ["name", "joint"] {
                let now = text(r, key);
                let id = format!("mech:{block}:{k}:{key}");
                if let Some(to) = self.mech_text(ui, &id, &now, 52.0) {
                    out.push(RowsEdit::Rename {
                        row: k,
                        key: key.into(),
                        to,
                    });
                }
            }
            let kind = text(r, "kind");
            egui::ComboBox::from_id_salt(("kind", block, k))
                .selected_text(&kind)
                .width(72.0)
                .show_ui(ui, |ui| {
                    for c in KINDS {
                        if ui.selectable_label(c == kind, c).clicked() && c != kind {
                            out.push(RowsEdit::Set {
                                row: k,
                                key: "kind".into(),
                                value: c.into(),
                            });
                        }
                    }
                });
            let parent = text(r, "parent");
            egui::ComboBox::from_id_salt(("parent", block, k))
                .selected_text(&parent)
                .width(56.0)
                .show_ui(ui, |ui| {
                    let earlier = names[..k].iter().map(String::as_str);
                    for p in std::iter::once("ground").chain(earlier) {
                        if ui.selectable_label(p == parent, p).clicked() && p != parent {
                            out.push(RowsEdit::Set {
                                row: k,
                                key: "parent".into(),
                                value: p.into(),
                            });
                        }
                    }
                });
            let open = self
                .mech_open
                .as_ref()
                .is_some_and(|(b, r)| b == block && *r == k);
            if ui
                .selectable_label(open, "…")
                .on_hover_text("This body's numbers, its inputs, and moving or removing it")
                .clicked()
            {
                self.mech_open = (!open).then(|| (block.to_string(), k));
            }
            ui.end_row();
        }
    }

    /// Row `k`'s other numbers and its inputs.
    fn row_details(
        &mut self,
        ui: &mut egui::Ui,
        entry: &simsandbox_file::BlockEntry,
        k: usize,
    ) -> Vec<RowsEdit> {
        let mut out = Vec::new();
        let block = &entry.name;
        let r = &entry.bodies[k];
        let (name, joint) = (text(r, "name"), text(r, "joint"));
        let moving = text(r, "kind") != "fixed";
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("{name} on {joint}")).strong());
                if ui.small_button("↑").on_hover_text("Move up").clicked() {
                    out.push(RowsEdit::Move { row: k, up: true });
                }
                if ui.small_button("↓").on_hover_text("Move down").clicked() {
                    out.push(RowsEdit::Move { row: k, up: false });
                }
                if ui
                    .small_button("✕")
                    .on_hover_text("Remove this body; its children hang from its parent")
                    .clicked()
                {
                    out.push(RowsEdit::Remove(k));
                }
            });
            egui::Grid::new(("body details", block, k))
                .num_columns(2)
                .show(ui, |ui| {
                    for (key, label, pair) in MORE {
                        let joint_only = matches!(key, "damping" | "stiffness" | "rest" | "start");
                        if joint_only && !moving {
                            continue;
                        }
                        ui.label(label);
                        ui.horizontal(|ui| {
                            let at = |i: usize| match r.get(key) {
                                Some(Value::Array(a)) => {
                                    a.get(i).and_then(Value::as_f64).unwrap_or(0.0)
                                }
                                Some(v) if !pair => v.as_f64().unwrap_or(0.0),
                                _ => 0.0,
                            };
                            let parts = if pair { 2 } else { 1 };
                            for i in 0..parts {
                                let id = format!("mech:{block}:{k}:{key}:{i}");
                                if let Some(v) = self.mech_number(ui, &id, at(i)) {
                                    let value = if pair {
                                        let mut both = [at(0), at(1)];
                                        both[i] = v;
                                        serde_json::json!(both)
                                    } else {
                                        v.into()
                                    };
                                    out.push(RowsEdit::Set {
                                        row: k,
                                        key: key.into(),
                                        value,
                                    });
                                }
                            }
                        });
                        ui.end_row();
                    }
                });
            if moving {
                let on = list(entry, "actuated").contains(&joint);
                let mut now = on;
                ui.checkbox(
                    &mut now,
                    format!(
                        "driven: an input {joint}_{}",
                        if text(r, "kind") == "revolute" {
                            "torque"
                        } else {
                            "force"
                        }
                    ),
                )
                .on_hover_text("A force or torque on the joint from a wire (actuated)");
                if now != on {
                    out.push(RowsEdit::Toggle {
                        setting: "actuated".into(),
                        name: joint.clone(),
                        on: now,
                    });
                }
            }
            let on = list(entry, "loaded").contains(&name);
            let mut now = on;
            ui.checkbox(
                &mut now,
                format!("loaded: inputs {name}_fx, {name}_fy, {name}_torque"),
            )
            .on_hover_text("A force at the centre of mass and a torque from wires (loaded)");
            if now != on {
                out.push(RowsEdit::Toggle {
                    setting: "loaded".into(),
                    name,
                    on: now,
                });
            }
        });
        out
    }

    /// A text field that gives its text when it loses focus changed.
    fn mech_text(&mut self, ui: &mut egui::Ui, id: &str, now: &str, width: f32) -> Option<String> {
        let mut t = self
            .param_edits
            .get(id)
            .cloned()
            .unwrap_or_else(|| now.to_string());
        let r = ui.add(egui::TextEdit::singleline(&mut t).desired_width(width));
        if r.has_focus() {
            self.param_edits.insert(id.to_string(), t.clone());
        }
        if r.lost_focus() {
            self.param_edits.remove(id);
            if t.trim() != now {
                return Some(t);
            }
        }
        None
    }

    /// A number field that gives its number when it loses focus changed
    /// (text that is not a finite number is dropped).
    fn mech_number(&mut self, ui: &mut egui::Ui, id: &str, now: f64) -> Option<f64> {
        let shown = format!("{now}");
        let t = self.mech_text(ui, id, &shown, 52.0)?;
        t.trim()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && *v != now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cartpole() -> SimApp {
        let mut app = SimApp::new(None, None, None, None);
        let k = EXAMPLES
            .iter()
            .position(|e| e.name.starts_with("Cart-pole"))
            .unwrap();
        app.open_example(k);
        assert!(app.error.is_none(), "{:?}", app.error);
        app
    }

    fn entry(app: &SimApp) -> simsandbox_file::BlockEntry {
        let m = app.session.as_ref().unwrap().model();
        m.file
            .blocks
            .iter()
            .find(|b| b.name == "cartpole")
            .unwrap()
            .clone()
    }

    fn edit(app: &mut SimApp, e: RowsEdit) {
        let edit = mechanism_edit(&entry(app), &e).unwrap();
        app.edit_at(Scope::Model, edit);
    }

    fn params(app: &SimApp) -> Vec<String> {
        let m = app.session.as_ref().unwrap().model();
        m.params.iter().map(|r| r.key.clone()).collect()
    }

    #[test]
    fn the_bodies_table_renames_sets_adds_and_removes() {
        let mut app = cartpole();
        // A joint renamed: its ports, parameters and wires follow.
        edit(
            &mut app,
            RowsEdit::Rename {
                row: 0,
                key: "joint".into(),
                to: "rail".into(),
            },
        );
        assert!(
            app.diagram.draft_problem().is_none(),
            "{:?}",
            app.diagram.draft_problem()
        );
        assert!(params(&app).contains(&"cartpole.rail_damping".to_string()));
        let m = app.session.as_ref().unwrap().model();
        assert!(m
            .file
            .connections
            .iter()
            .any(|c| c.to == "cartpole.rail_force"));
        // A body renamed: its children follow it.
        edit(
            &mut app,
            RowsEdit::Rename {
                row: 0,
                key: "name".into(),
                to: "trolley".into(),
            },
        );
        assert_eq!(text(&entry(&app).bodies[1], "parent"), "trolley");
        // A number.
        edit(
            &mut app,
            RowsEdit::Set {
                row: 1,
                key: "mass".into(),
                value: 0.2.into(),
            },
        );
        assert_eq!(entry(&app).bodies[1]["mass"], 0.2);
        // A body added, hanging from the last; then removed.
        edit(&mut app, RowsEdit::Add);
        let e = entry(&app);
        assert_eq!(e.bodies.len(), 3);
        assert_eq!(text(&e.bodies[2], "parent"), "pole");
        assert!(params(&app).contains(&"cartpole.joint1_angle0".to_string()));
        edit(&mut app, RowsEdit::Remove(2));
        assert_eq!(entry(&app).bodies.len(), 2);
        assert!(
            app.diagram.draft_problem().is_none(),
            "{:?}",
            app.diagram.draft_problem()
        );
    }

    #[test]
    fn a_change_that_does_not_check_waits_as_a_draft() {
        let mut app = cartpole();
        // The cart's joint welded while it still has damping: refused at
        // its row and column; the running model stays.
        edit(
            &mut app,
            RowsEdit::Set {
                row: 0,
                key: "kind".into(),
                value: "fixed".into(),
            },
        );
        let problem = app.diagram.draft_problem().unwrap();
        assert!(problem.contains("bodies[0].damping"), "{problem}");
        assert_eq!(text(&entry(&app).bodies[0], "kind"), "prismatic");
        // The pole no longer loaded: its inputs and their wires go.
        let mut app = cartpole();
        edit(
            &mut app,
            RowsEdit::Toggle {
                setting: "loaded".into(),
                name: "pole".into(),
                on: false,
            },
        );
        assert!(
            app.diagram.draft_problem().is_none(),
            "{:?}",
            app.diagram.draft_problem()
        );
        let e = entry(&app);
        assert!(!e.settings.contains_key("loaded"));
        let m = app.session.as_ref().unwrap().model();
        assert!(!m
            .file
            .connections
            .iter()
            .any(|c| c.to.starts_with("cartpole.pole_")));
        // Moving a body above its parent is refused at the row.
        edit(&mut app, RowsEdit::Move { row: 1, up: true });
        let problem = app.diagram.draft_problem().unwrap();
        assert!(problem.contains("bodies[0].parent"), "{problem}");
    }
}
