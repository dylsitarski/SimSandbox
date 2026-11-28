//! The Definitions pane (ADR 0108 point 1): the selected block where it is
//! defined, edited there. Typed equations as an equation sheet, each line
//! with what it declares, its unit and its value now, the text one click
//! away; a mechanism's bodies as their table beside the picture it offers;
//! a study's design; a used model's own level on the Overview; any other
//! block's parameters. The JSON is `simsandbox_session::definition`, as
//! `explain --definition` prints it.

use super::*;
use serde_json::Value;

impl SimApp {
    /// The block the pane shows: the one selected at the model's level.
    fn defined_block(&self) -> Option<String> {
        match (
            &self.diagram.canvas.selected,
            self.diagram.inside.is_empty(),
        ) {
            (Some(crate::canvas::Selection::Block(b)), true) => Some(b.clone()),
            _ => None,
        }
    }

    /// The definition of `block`, read again when the file changes.
    fn definition_of(&mut self, block: &str) -> Option<Value> {
        let live = self.session.as_ref()?;
        let m = live.model();
        if let Some((file, b, d)) = &self.definition {
            if *file == m.file && b == block {
                return Some(d.clone());
            }
        }
        let name = block.to_string();
        let d = live
            .call(move |s, _| simsandbox_session::definition::definition(s, &name))
            .ok()?
            .ok()?;
        self.definition = Some((m.file.clone(), block.to_string(), d.clone()));
        Some(d)
    }

    pub(super) fn definitions_tab(&mut self, ui: &mut egui::Ui) {
        let Some(block) = self.defined_block() else {
            ui.weak(
                "Select a block (on the diagram, the Overview, or by pressing on a picture) to see \
                 where it is defined",
            );
            return;
        };
        let Some(d) = self.definition_of(&block) else {
            ui.weak(format!("{block} is not in the running model"));
            return;
        };
        egui::ScrollArea::vertical()
            .id_salt("definitions")
            .show(ui, |ui| match d["kind"].as_str() {
                Some("equations") => self.equation_sheet(ui, &block, &d),
                Some("bodies") => self.bodies_definition(ui, &block, &d),
                Some("study") => self.study_definition(ui, &block),
                Some("model") => {
                    ui.heading(&block);
                    ui.label(format!(
                        "A use of {}: its parts are one level down, on the Overview.",
                        d["model"].as_str().unwrap_or("a model")
                    ));
                    if ui.button("Open on the Overview").clicked() {
                        self.overview_part = block.clone();
                        self.focus = Some(Focus::Pane(Pane::Overview));
                    }
                }
                _ => self.parameters_definition(ui, &block, &d),
            });
    }

    /// A value now, as the sheet shows it: a parameter's, else the signal's
    /// newest sample, else what the definition was read with.
    fn value_now(&self, block: &str, line: &Value) -> Option<f64> {
        let name = line["name"].as_str()?;
        let key = format!("{block}.{name}");
        let live = self.session.as_ref()?;
        let m = live.model();
        if let Some(r) = m.params.iter().find(|r| r.key == key) {
            return Some(r.value);
        }
        // An input: what feeds it.
        let signal = m
            .file
            .connections
            .iter()
            .find(|c| c.to == key)
            .map_or(key.clone(), |c| c.from.clone());
        let h = live.history();
        let recorded = simsandbox_views::Recorded::single(&h);
        let newest = recorded
            .resolve(&signal)
            .and_then(|(h, s)| h.values(s).last().copied());
        newest.or_else(|| line["value"].as_f64())
    }

    /// Typed equations as an equation sheet.
    fn equation_sheet(&mut self, ui: &mut egui::Ui, block: &str, d: &Value) {
        ui.horizontal(|ui| {
            ui.heading(block);
            ui.weak("typed equations");
            if ui
                .button("Edit text")
                .on_hover_text("Change the equations as text, in the equation editor")
                .clicked()
            {
                if let Some(live) = &self.session {
                    let m = live.model();
                    if let Some(b) = m.file.blocks.iter().find(|b| b.name == block) {
                        self.editor = Some(super::editors::Editor {
                            block: Some(block.to_string()),
                            text: b.equations.join("\n"),
                            error: None,
                        });
                    }
                }
            }
        });
        let lines: Vec<Value> = d["lines"].as_array().cloned().unwrap_or_default();
        egui::Grid::new(("equation sheet", block))
            .striped(true)
            .num_columns(3)
            .show(ui, |ui| {
                for l in &lines {
                    let text = l["text"].as_str().unwrap_or("");
                    let code = text.split('#').next().unwrap_or("").trim_end();
                    if l["what"] == "note" {
                        ui.label(
                            egui::RichText::new(text.trim_start_matches('#').trim()).italics(),
                        );
                        ui.label("");
                        ui.label("");
                        ui.end_row();
                        continue;
                    }
                    let shown = ui.monospace(code);
                    if let Some(note) = l["note"].as_str() {
                        shown.on_hover_text(note);
                    }
                    let unit = l["unit"].as_str().unwrap_or("");
                    match self.value_now(block, l) {
                        Some(v) => {
                            let v = simsandbox_session::diagram_paint::format_number(v);
                            ui.label(if unit.is_empty() {
                                v
                            } else {
                                format!("{v} {unit}")
                            });
                        }
                        None => {
                            ui.label("");
                        }
                    }
                    ui.weak(l["note"].as_str().unwrap_or(""));
                    ui.end_row();
                }
            });
    }

    /// A mechanism: its bodies as their table, beside the picture it offers.
    fn bodies_definition(&mut self, ui: &mut egui::Ui, block: &str, d: &Value) {
        ui.heading(block);
        let picture = d["pictures"]
            .as_array()
            .and_then(|p| p.first())
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(p) = &picture {
            self.definition_picture(ui, p);
        }
        let entry = self.session.as_ref().and_then(|l| {
            l.model()
                .file
                .blocks
                .iter()
                .find(|b| b.name == block)
                .cloned()
        });
        if let Some(b) = entry {
            for e in self.mechanism_section(ui, &b) {
                self.edit_at(Scope::Model, e);
            }
        }
    }

    /// The picture a block offers, drawn in a scene of its own.
    fn definition_picture(&mut self, ui: &mut egui::Ui, template: &str) {
        let Some(live) = &self.session else {
            return;
        };
        let history = live.history();
        let m = live.model();
        let available = m.available(&history);
        if self
            .definition_picture
            .as_ref()
            .is_none_or(|(t, _, _)| t != template)
        {
            let mut scene = simsandbox_file::SceneSettings {
                unit: Some(self.views.template_unit(template)),
                ..Default::default()
            };
            scene
                .layers
                .push(crate::scene_edit::picture_layer(&scene, template));
            let entry = simsandbox_file::ViewEntry {
                type_name: simsandbox_views::scene::SCENE.to_string(),
                scene,
                ..Default::default()
            };
            let drawn = simsandbox_file::templates::expand(&entry, &self.views.templates);
            match drawn.and_then(|drawn| Ok((self.views.state_for(&entry, &available)?, drawn))) {
                Ok((state, drawn)) => {
                    self.definition_picture = Some((template.to_string(), drawn, state));
                }
                Err(e) => {
                    ui.weak(format!("{template} cannot be drawn: {e}"));
                    return;
                }
            }
        }
        let Some(t) = self.views.registry.get(simsandbox_views::scene::SCENE) else {
            return;
        };
        let Some((_, _, state)) = self.definition_picture.as_mut() else {
            return;
        };
        let recorded = simsandbox_views::Recorded::single(&history);
        ui.allocate_ui(egui::vec2(ui.available_width(), 240.0), |ui| {
            ui.set_min_height(240.0);
            state.show(ui, t, recorded);
        });
    }

    /// A study: its design, as the Inspector edits it.
    fn study_definition(&mut self, ui: &mut egui::Ui, block: &str) {
        ui.heading(block);
        let Some(live) = &self.session else {
            return;
        };
        let m = live.model();
        let Some(b) = m.file.blocks.iter().find(|b| b.name == block).cloned() else {
            return;
        };
        let keys = self.study_keys(&m.file, block);
        drop(m);
        for e in study_fields(ui, &b, &mut self.param_edits, &keys) {
            self.edit_at(Scope::Model, e);
        }
    }

    /// Any other block: its parameters, which the Inspector changes.
    fn parameters_definition(&mut self, ui: &mut egui::Ui, block: &str, d: &Value) {
        ui.heading(block);
        ui.weak(format!(
            "A {} block: defined by its parameters, changed in the Inspector",
            d["type"].as_str().unwrap_or("")
        ));
        egui::Grid::new(("definition params", block))
            .striped(true)
            .show(ui, |ui| {
                for p in d["params"].as_array().into_iter().flatten() {
                    ui.label(p["name"].as_str().unwrap_or(""));
                    let v = p["value"]
                        .as_f64()
                        .map(simsandbox_session::diagram_paint::format_number);
                    ui.label(format!(
                        "{} {}",
                        v.unwrap_or_default(),
                        p["unit"].as_str().unwrap_or("")
                    ));
                    ui.weak(p["role"].as_str().unwrap_or(""));
                    ui.end_row();
                }
            });
    }
}
