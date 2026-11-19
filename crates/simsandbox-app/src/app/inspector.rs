//! The Inspector and the Controls (ADR 0053): search, what is selected,
//! every parameter where it is set, the studies' models (ADR 0070); and
//! applying what they change to the running model.

use super::*;

/// What the inspector's rows and forms collect in a frame, carried out
/// once it is drawn.
#[derive(Default)]
struct Collected {
    changes: Vec<(ParamRow, f64)>,
    controls: Vec<(String, simsandbox_file::ParamControl)>,
    /// Edits that go through the diagram (a conversion's units, a new
    /// block's values), at the level they apply to.
    canvas: Vec<(Scope, Edit)>,
    /// Edits of the model open on the canvas inside a study.
    inside: Vec<Edit>,
    /// A new update order for an agents block (ADR 0034).
    update: Option<(String, simsandbox_agents::Update)>,
    /// A block whose equations to open in the editor, and their text.
    open_editor: Option<(String, String)>,
}

/// The `Edits` a row adds to: what it changes goes into `$c`, typed text
/// and errors stay with the app.
macro_rules! edits {
    ($app:expr, $c:expr) => {
        Edits {
            changes: &mut $c.changes,
            controls: &mut $c.controls,
            text: &mut $app.param_edits,
            pending: &mut $app.pending,
            error: &mut $app.error,
            file: &mut $c.canvas,
        }
    };
}

impl SimApp {
    /// Apply typed or dragged parameter values to the running model.
    pub(super) fn apply_changes(&self, changes: Vec<(ParamRow, f64)>) {
        let Some(live) = self.session.as_ref() else {
            return;
        };
        for (row, v) in changes {
            let key = row.key.clone();
            live.send(Some(&format!("param:{key}")), move |s, c| {
                match s.set_param_key(&row.key, v) {
                    Ok(()) => {
                        // A typed value outside the slider range widens it (ADR 0011).
                        if let Some(ctl) = s.file.controls.params.get_mut(&row.key) {
                            if ctl.slider {
                                let [lo, hi] = ctl.range.unwrap_or(row.suggested);
                                if v < lo || v > hi {
                                    ctl.range = Some([lo.min(v), hi.max(v)]);
                                }
                            }
                        }
                        if row.initial_condition && s.time() > 0.0 {
                            c.info(format!("{} changed: press Restart to apply it", row.key));
                        }
                    }
                    Err(e) => c.error(format!("{}: {e}", row.key)),
                }
            });
        }
    }

    /// New values, and pin and slider choices (saved in `controls`, ADR
    /// 0012).
    pub(super) fn apply_edits(
        &mut self,
        changes: Vec<(ParamRow, f64)>,
        controls: Vec<(String, simsandbox_file::ParamControl)>,
    ) {
        if let Some(live) = self.session.as_ref() {
            for (key, control) in controls {
                live.send(None, move |s, _| {
                    s.file.controls.params.insert(key, control);
                });
            }
        }
        self.apply_changes(changes);
    }

    /// The Controls pane (ADR 0053): the pinned parameters, each with an
    /// optional slider (ADR 0011).
    pub(super) fn controls_panel(&mut self, ui: &mut egui::Ui) {
        let Some(live) = self.session.as_ref() else {
            return;
        };
        let m = live.model();
        let mut changes: Vec<(ParamRow, f64)> = Vec::new();
        let mut controls: Vec<(String, simsandbox_file::ParamControl)> = Vec::new();
        let mut file_edits: Vec<(Scope, Edit)> = Vec::new();
        let pinned: Vec<&ParamRow> = m
            .params
            .iter()
            .filter(|r| {
                m.file
                    .controls
                    .params
                    .get(&r.key)
                    .is_some_and(|c| c.pinned())
            })
            .collect();
        if pinned.is_empty() {
            ui.weak(
                "Nothing pinned. Pin a value with 📌 in the inspector to keep it here, \
                 with a slider if you like.",
            );
        }
        ui.spacing_mut().slider_width = 90.0;
        egui::Grid::new("controls")
            .num_columns(4)
            .spacing([6.0, 4.0])
            .show(ui, |ui| {
                for row in pinned {
                    let mut r = row.clone();
                    r.name = r.key.clone();
                    let edits = Edits {
                        changes: &mut changes,
                        controls: &mut controls,
                        text: &mut self.param_edits,
                        pending: &mut self.pending,
                        error: &mut self.error,
                        file: &mut file_edits,
                    };
                    param_row(ui, &m, &r, Place::Controls, None, edits);
                }
            });
        self.apply_edits(changes, controls);
        for (scope, e) in file_edits {
            self.edit_at(scope, e);
        }
    }

    /// What the study `name` of `file` can vary (its model's parameters).
    pub(super) fn study_keys(
        &mut self,
        file: &simsandbox_file::ModelFile,
        name: &str,
    ) -> Vec<StudyKey> {
        self.studied(file, name)
            .map(|s| s.keys.clone())
            .unwrap_or_default()
    }

    /// The model the study `name` of `file` studies: what it can vary and
    /// its blocks' values, worked out again only when it changed.
    pub(super) fn studied(
        &mut self,
        file: &simsandbox_file::ModelFile,
        name: &str,
    ) -> Option<std::sync::Arc<Studied>> {
        let b = file
            .blocks
            .iter()
            .find(|b| b.name == name && b.type_name == simsandbox_file::STUDY)?;
        let label = match (&b.model, &b.file) {
            (Some(m), _) => m.clone(),
            (None, Some(f)) => f.clone(),
            _ => return None,
        };
        let folder = self.path.as_deref().and_then(Path::parent);
        let model =
            simsandbox_session::edit_use::model_at(file, folder, &[name.to_string()]).ok()?;
        let text = model.to_json();
        if let Some((t, s)) = self.study_keys.get(name) {
            if *t == text {
                return Some(s.clone());
            }
        }
        let at = model.folder.as_deref().or(folder);
        let keys = simsandbox_session::diagram::study_parameters(&model, &self.registry, at);
        let d = Diagram::new(&model, &self.registry);
        let blocks = d
            .nodes
            .into_iter()
            .filter(|n| !n.is_observer() && !n.values.is_empty())
            .map(|n| (n.name, n.values))
            .collect();
        let s = std::sync::Arc::new(Studied {
            model: label,
            keys,
            blocks,
        });
        self.study_keys.insert(name.to_string(), (text, s.clone()));
        Some(s)
    }

    /// The inspector (ADR 0053): search, what is selected, and every
    /// parameter where it is set.
    pub(super) fn param_panel(&mut self, ui: &mut egui::Ui) {
        let Some(live) = self.session.as_ref() else {
            return;
        };
        let m = live.model();
        let mut c = Collected::default();
        // Where a file reader's data is named from (ADR 0063).
        let folder = self
            .path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        let folder = folder.as_deref();
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 100.0).max(80.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.inspect_query)
                    .hint_text("Search parameters")
                    .desired_width(width),
            );
            ui.checkbox(&mut self.show_computed, "computed")
                .on_hover_text("Also list the values formulas set (shown, not edited)");
        });
        let query = self.inspect_query.trim().to_string();
        if !query.is_empty() {
            let found = crate::inspector::search(&m.params, &query);
            ui.weak(match found.len() {
                0 => "Nothing matches".to_string(),
                1 => "1 parameter".to_string(),
                n => format!("{n} parameters"),
            });
            egui::Grid::new("found")
                .num_columns(4)
                .spacing([6.0, 4.0])
                .show(ui, |ui| {
                    for row in found {
                        let mut r = row.clone();
                        r.name = r.key.clone();
                        param_row(ui, &m, &r, Place::Tree, None, edits!(self, c));
                    }
                });
        } else {
            let o = crate::inspector::organise(&m.params, &m.file, self.show_computed);
            // What was picked last, on the diagram or in a plot (ADR 0053).
            self.follow_canvas();
            let picked = self.picked.clone();
            // A study picked: what its model can vary, for its form.
            let keys = match picked.as_slice() {
                [(None, name)] => self.study_keys(&m.file, name),
                _ => Vec::new(),
            };
            // Every study's model, for the tree (ADR 0070).
            let studies: HashMap<String, std::sync::Arc<Studied>> = m
                .file
                .blocks
                .iter()
                .filter(|b| b.type_name == simsandbox_file::STUDY)
                .map(|b| b.name.clone())
                .collect::<Vec<_>>()
                .into_iter()
                .filter_map(|n| self.studied(&m.file, &n).map(|s| (n, s)))
                .collect();
            if let Some(tile) = self.inspected_scene() {
                self.scene_section(ui, tile);
            }
            if !picked.is_empty() {
                self.selected_section(ui, &m, &picked, &keys, folder, &mut c);
            }
            let small = m.params.len() <= 16;
            self.model_section(ui, &m, &o, &mut c);
            for t in &o.tables {
                let title = format!("{} × {}", t.subsystem, t.uses.len());
                egui::CollapsingHeader::new(egui::RichText::new(title).strong())
                    .id_salt(("uses", &t.subsystem))
                    .default_open(small || t.rows.len() <= 6)
                    .show(ui, |ui| {
                        uses_table(ui, t, &m.file.controls.params, edits!(self, c));
                    })
                    .header_response
                    .on_hover_text(format!(
                        "The subsystem {} is used {} times: a column per use",
                        t.subsystem,
                        t.uses.len()
                    ));
            }
            for g in &o.groups {
                self.group_section(ui, &m, &o, g, &studies, &mut c);
            }
            // Blocks added on the diagram that wait, as a draft, for the
            // model to load.
            self.draft_sections(ui, &m, &mut c);
            if o.computed > 0 && !self.show_computed {
                ui.add_space(4.0);
                ui.weak(format!(
                    "{} values set by formulas are not listed (tick \"computed\")",
                    o.computed
                ));
            }
        }
        self.apply_collected(c);
    }

    /// The blocks picked on the diagram, when its selection changed since
    /// last looked at.
    fn follow_canvas(&mut self) {
        let canvas = self.diagram.canvas.selected.clone();
        // A view picked alone on the canvas (`view:K`): its tile, for a
        // scene's editor; anything else picked leaves it. A canvas edit
        // of the view makes it a new tile, which is followed.
        let view = match canvas.as_ref().map(|s| s.blocks()).as_deref() {
            Some([one]) => one
                .strip_prefix("view:")
                .and_then(|k| k.parse::<usize>().ok())
                .and_then(|k| self.views.tiles.get(k))
                .map(|t| t.id),
            _ => None,
        };
        let gone = self
            .inspected_view
            .is_some_and(|id| !self.views.tiles.iter().any(|t| t.id == id));
        if gone {
            self.inspected_view = view;
        }
        if canvas != self.last_canvas {
            self.last_canvas = canvas.clone();
            self.inspected_view = view;
            // Inside a used model, picks are named inside it: its path
            // (`car1`, `vehicle/plant`) says where.
            let scope: Option<String> =
                (!self.diagram.inside.is_empty()).then(|| self.diagram.inside.join("/"));
            // A block in a model expanded in place is named by its path
            // (`shaken/mass`): inside that use, as when it is opened.
            self.picked = canvas
                .map(|s| s.blocks())
                .unwrap_or_default()
                .into_iter()
                .filter(|n| !n.contains(':'))
                .map(|n| match n.rsplit_once('/') {
                    Some((path, name)) => {
                        let full = match &scope {
                            Some(s) => format!("{s}/{path}"),
                            None => path.to_string(),
                        };
                        (Some(full), name.to_string())
                    }
                    None => (scope.clone(), n),
                })
                .collect();
        }
    }

    /// The blocks picked: their values and forms, from the running model,
    /// the diagram's draft or the used model they are inside.
    fn selected_section(
        &mut self,
        ui: &mut egui::Ui,
        m: &Model,
        picked: &[(Option<String>, String)],
        keys: &[StudyKey],
        folder: Option<&Path>,
        c: &mut Collected,
    ) {
        let title = match picked {
            [(Some(s), name)] => format!("Selected: {name} in {s}"),
            [(None, name)] => format!("Selected: {name}"),
            many => format!("Selected: {} blocks", many.len()),
        };
        let draft = self.diagram.draft_file().cloned();
        egui::CollapsingHeader::new(egui::RichText::new(title).strong())
            .id_salt("selected")
            .default_open(true)
            .show(ui, |ui| {
                for (scope, name) in picked {
                    let at = Scope::Model;
                    if picked.len() > 1 {
                        ui.strong(name);
                    }
                    let running = block_entry(&m.file, scope.as_deref(), name);
                    let drafted = draft
                        .as_ref()
                        .and_then(|f| block_entry(f, scope.as_deref(), name));
                    let mut shown = false;
                    if let Some(b) = running {
                        if let Some(t) =
                            crate::inspector::selected(&m.params, &m.file, scope.as_deref(), name)
                        {
                            uses_table(ui, &t, &m.file.controls.params, edits!(self, c));
                            shown = true;
                        }
                        for e in unit_fields(ui, b, &mut self.param_edits) {
                            c.canvas.push((at.clone(), e));
                            shown = true;
                        }
                        for e in file_fields(ui, b, &mut self.param_edits, folder) {
                            c.canvas.push((at.clone(), e));
                        }
                        for e in study_fields(ui, b, &mut self.param_edits, keys) {
                            c.canvas.push((at.clone(), e));
                        }
                        shown |= b.type_name == simsandbox_eq::convert::CONVERT
                            || b.type_name == simsandbox_eq::file_input::FILE
                            || b.type_name == simsandbox_file::STUDY;
                    } else if let (Some(b), Some(f)) = (drafted, &draft) {
                        ui.weak("Not in the running model yet: it joins when the model loads")
                            .on_hover_text(self.diagram.draft_problem().unwrap_or_default());
                        let d = Diagram::of(f, &at, &self.registry);
                        if let Some(n) = d.nodes.iter().find(|n| n.name == *name) {
                            for e in draft_rows(ui, n, &mut self.param_edits, &mut self.error) {
                                c.canvas.push((at.clone(), e));
                            }
                        }
                        for e in unit_fields(ui, b, &mut self.param_edits) {
                            c.canvas.push((at.clone(), e));
                        }
                        for e in file_fields(ui, b, &mut self.param_edits, folder) {
                            c.canvas.push((at.clone(), e));
                        }
                        for e in study_fields(ui, b, &mut self.param_edits, keys) {
                            c.canvas.push((at.clone(), e));
                        }
                        shown = true;
                    }
                    // A block inside a used model: its values by path.
                    if running.is_none() && drafted.is_none() {
                        if let Some(t) =
                            crate::inspector::selected(&m.params, &m.file, scope.as_deref(), name)
                        {
                            uses_table(ui, &t, &m.file.controls.params, edits!(self, c));
                            shown = true;
                        } else if let Some(values) = scope.as_deref().and_then(|s| {
                            let path: Vec<String> = s.split('/').map(str::to_string).collect();
                            self.diagram.inside_values(&path, name)
                        }) {
                            // Inside a study: the model's values, as
                            // written; a change edits the model.
                            ui.weak("Values of the studied model: a change edits the model");
                            for e in studied_rows(ui, name, &values, &mut self.param_edits) {
                                c.inside.push(e);
                            }
                            shown = !values.is_empty();
                        }
                    }
                    if !shown {
                        ui.weak("Nothing to set");
                    }
                }
            });
        ui.add_space(6.0);
    }

    /// The model's own parameters, a form to declare one, and the outputs
    /// it offers to models using it (ADR 0073).
    fn model_section(
        &mut self,
        ui: &mut egui::Ui,
        m: &Model,
        o: &crate::inspector::Organised,
        c: &mut Collected,
    ) {
        egui::CollapsingHeader::new(egui::RichText::new("Model").strong())
            .id_salt("model-params")
            .default_open(true)
            .show(ui, |ui| {
                egui::Grid::new("model")
                    .num_columns(4)
                    .spacing([6.0, 4.0])
                    .show(ui, |ui| {
                        for row in &o.model {
                            let used = o.used_by.get(&row.key);
                            param_row(ui, m, row, Place::Tree, used, edits!(self, c));
                        }
                    });
                if let Some(e) = declare_form(ui, &mut self.new_param, &mut self.error) {
                    c.canvas.push((Scope::Model, e));
                }
                // What models using this one see of it (ADR 0073).
                for e in offered_outputs(ui, m) {
                    c.canvas.push((Scope::Model, e));
                }
            })
            .header_response
            .on_hover_text(
                "Parameters of the model itself: block values read them with \
                 '= expression'",
            );
    }

    /// One block or use with its parameters: its update order or
    /// equations, its form, a study's model, then its values by path.
    fn group_section(
        &mut self,
        ui: &mut egui::Ui,
        m: &Model,
        o: &crate::inspector::Organised,
        g: &crate::inspector::Group,
        studies: &HashMap<String, std::sync::Arc<Studied>>,
        c: &mut Collected,
    ) {
        let small = m.params.len() <= 16;
        let entry = m.file.blocks.iter().find(|b| b.name == g.name);
        let typed = entry
            .filter(|b| b.type_name == simsandbox_eq::registry::EQUATIONS)
            .map(|b| b.equations.join("\n"));
        let agents = entry.filter(|b| b.type_name.starts_with("agents."));
        let convert = entry.filter(|b| b.type_name == simsandbox_eq::convert::CONVERT);
        let studied = studies.get(&g.name);
        let kind = match entry.filter(|b| b.is_use()).map(|b| b.file.as_deref()) {
            _ if studied.is_some() => {
                format!("study of {}", studied.map_or("", |s| s.model.as_str()))
            }
            Some(Some(file)) => format!("uses {file}"),
            Some(None) => format!(
                "uses the model {}",
                entry.and_then(|b| b.model.as_deref()).unwrap_or_default()
            ),
            None if g.instance => format!("subsystem {}", g.type_name),
            None => g.type_name.clone(),
        };
        let header = egui::RichText::new(format!("{}  ({kind})", g.name)).strong();
        egui::CollapsingHeader::new(header)
            .id_salt(("group", &g.name))
            .default_open(small || g.rows.len() <= 4)
            .show(ui, |ui| {
                if agents.is_some() || typed.is_some() {
                    ui.horizontal(|ui| {
                        if let Some(b) = agents {
                            // The update order is part of the model
                            // (ADR 0034).
                            let now = simsandbox_agents::Update::from_setting(b.update.as_deref())
                                .unwrap_or_default();
                            egui::ComboBox::from_id_salt(("update", &g.name))
                                .selected_text(now.name())
                                .show_ui(ui, |ui| {
                                    for u in simsandbox_agents::Update::ALL {
                                        if ui
                                            .selectable_label(u == now, u.name())
                                            .on_hover_text(u.description())
                                            .clicked()
                                            && u != now
                                        {
                                            c.update = Some((g.name.clone(), u));
                                        }
                                    }
                                })
                                .response
                                .on_hover_text(
                                    "The order agents act in each step; it changes \
                                     results. Changing it restarts the run.",
                                );
                        }
                        if let Some(text) = &typed {
                            if ui
                                .small_button("Edit equations…")
                                .on_hover_text("Change this block's equations")
                                .clicked()
                            {
                                c.open_editor = Some((g.name.clone(), text.clone()));
                            }
                        }
                    });
                }
                if let Some(b) = convert {
                    for e in unit_fields(ui, b, &mut self.param_edits) {
                        c.canvas.push((Scope::Model, e));
                    }
                }
                // When a used model acts (ADR 0088).
                if let Some(b) = entry.filter(|b| b.is_use() && studied.is_none()) {
                    for e in use_activation(ui, b) {
                        c.canvas.push((Scope::Model, e));
                    }
                }
                // Starting values come last, each marked ↻; a used
                // model's parameters nest by path (ADR 0066).
                // A study's model, nested: its blocks' values as
                // written (ADR 0070).
                if let Some(st) = studied {
                    studied_tree(ui, &g.name, st);
                }
                let nested = crate::inspector::nest(&g.rows);
                nested_rows(ui, &nested, &g.name, &mut |ui, name, row| {
                    let mut r = row.clone();
                    r.name = name.to_string();
                    let used = o.used_by.get(&row.key);
                    param_row(ui, m, &r, Place::Tree, used, edits!(self, c));
                });
            });
    }

    /// Blocks added on the diagram that wait, as a draft, for the model to
    /// load.
    fn draft_sections(&mut self, ui: &mut egui::Ui, m: &Model, c: &mut Collected) {
        if let Some(f) = self.diagram.draft_file().cloned() {
            let d = Diagram::of(&f, &Scope::Model, &self.registry);
            for n in d
                .nodes
                .iter()
                .filter(|n| n.is_block() && !m.file.blocks.iter().any(|b| b.name == n.name))
            {
                let header =
                    egui::RichText::new(format!("{}  ({}, not running yet)", n.name, n.type_name))
                        .strong();
                egui::CollapsingHeader::new(header)
                    .id_salt(("draft", &n.name))
                    .default_open(true)
                    .show(ui, |ui| {
                        for e in draft_rows(ui, n, &mut self.param_edits, &mut self.error) {
                            c.canvas.push((Scope::Model, e));
                        }
                        if let Some(b) = f.blocks.iter().find(|b| b.name == n.name) {
                            for e in unit_fields(ui, b, &mut self.param_edits) {
                                c.canvas.push((Scope::Model, e));
                            }
                        }
                    })
                    .header_response
                    .on_hover_text(format!(
                        "Added on the diagram; it joins the run once the model loads: {}",
                        self.diagram.draft_problem().unwrap_or_default()
                    ));
            }
        }
    }

    /// Carry out what the inspector collected this frame.
    fn apply_collected(&mut self, c: Collected) {
        if let Some((block, text)) = c.open_editor {
            self.editor = Some(Editor {
                block: Some(block),
                text,
                error: None,
            });
        }

        self.apply_edits(c.changes, c.controls);
        for (scope, e) in c.canvas {
            self.edit_at(scope, e);
        }
        if !c.inside.is_empty() {
            self.edit_inside(c.inside);
        }
        if let Some((block, u)) = c.update {
            self.rebuild(
                |file| {
                    if let Some(b) = file.blocks.iter_mut().find(|b| b.name == block) {
                        b.update = Some(u.name().into());
                    }
                },
                &format!("Update order is now {}; the run restarted", u.name()),
            );
        }
    }
}

/// The block named `name` in a file, on the model level or inside a
/// subsystem.
pub(super) fn block_entry<'a>(
    file: &'a simsandbox_file::ModelFile,
    scope: Option<&str>,
    name: &str,
) -> Option<&'a simsandbox_file::BlockEntry> {
    // Inside a used model it is not one of the file's own blocks.
    scope
        .is_none()
        .then(|| file.blocks.iter().find(|b| b.name == name))
        .flatten()
}

/// The outputs the model offers to models using it (ADR 0073): each by
/// name with the signal it shows, removable, and a picker to offer another
/// block output.
pub(super) fn offered_outputs(ui: &mut egui::Ui, m: &crate::live::Model) -> Vec<Edit> {
    let mut out = Vec::new();
    let offered = &m.file.outputs;
    ui.add_space(4.0);
    ui.label("Offered to models using this one").on_hover_text(
        "Ports of this model where another model uses it, by name: they stay valid when \
             its insides change. A model nothing uses needs none.",
    );
    for o in offered {
        ui.horizontal(|ui| {
            ui.monospace(&o.name);
            ui.weak(format!("shows {}", o.from));
            if ui
                .small_button("×")
                .on_hover_text("Stop offering it")
                .clicked()
            {
                out.push(Edit::RemoveOutput {
                    name: o.name.clone(),
                });
            }
        });
    }
    // Whole outputs of the model's own blocks, not yet offered.
    let blocks: Vec<&str> = m
        .file
        .blocks
        .iter()
        .filter(|b| !b.is_use())
        .map(|b| b.name.as_str())
        .collect();
    let candidates: Vec<&String> = m
        .scalars
        .iter()
        .chain(m.grids())
        .filter(|s| !s.contains('[') && !s.contains('/'))
        .filter(|s| s.split_once('.').is_some_and(|(b, _)| blocks.contains(&b)))
        .filter(|s| !offered.iter().any(|o| &o.from == *s))
        .collect();
    if candidates.is_empty() {
        return out;
    }
    egui::ComboBox::from_id_salt("offer-output")
        .selected_text("+ offer an output")
        .show_ui(ui, |ui| {
            for c in candidates {
                if ui.selectable_label(false, c.as_str()).clicked() {
                    let port = c.split_once('.').map_or(c.as_str(), |(_, p)| p);
                    let taken = |x: &str| offered.iter().any(|o| o.name == x);
                    let name = if taken(port) {
                        simsandbox_session::diagram::free_name(&c.replace('.', "_"), taken)
                    } else {
                        port.to_string()
                    };
                    out.push(Edit::DeclareOutput {
                        name,
                        from: c.clone(),
                    });
                }
            }
        });
    out
}

/// The values of block `block` of a studied model, as written: a field
/// each, a changed value an edit of the model; values formulas set are
/// shown, not edited.
pub(super) fn studied_rows(
    ui: &mut egui::Ui,
    block: &str,
    values: &[simsandbox_session::diagram::NodeValue],
    edits: &mut HashMap<String, String>,
) -> Vec<Edit> {
    let mut out = Vec::new();
    egui::Grid::new(("studied-rows", block))
        .num_columns(3)
        .spacing([6.0, 4.0])
        .show(ui, |ui| {
            for v in values {
                ui.label(&v.name);
                match (&v.driven, &v.formula) {
                    (Some(from), _) => {
                        ui.weak(format!("from {from}"));
                    }
                    (None, Some(f)) => {
                        ui.weak(format!("= {f}"));
                    }
                    (None, None) => {
                        let id = format!("studied:{block}.{}", v.name);
                        let now = simsandbox_views::values::number(v.value);
                        let mut text = edits.get(&id).cloned().unwrap_or_else(|| now.clone());
                        let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(70.0));
                        if r.has_focus() {
                            edits.insert(id.clone(), text.clone());
                        }
                        if r.lost_focus() {
                            edits.remove(&id);
                            match text.trim().parse::<f64>() {
                                Ok(x) if x.is_finite() && text.trim() != now => {
                                    out.push(Edit::SetParam {
                                        block: block.to_string(),
                                        param: v.name.clone(),
                                        value: x,
                                    })
                                }
                                _ => {}
                            }
                        }
                    }
                }
                ui.label(if v.initial {
                    format!("{} ↻", v.unit)
                } else {
                    v.unit.clone()
                });
                ui.end_row();
            }
        });
    out
}

/// A study's model in the model tree: each block's values as written, to
/// read; changed by opening the study's node.
pub(super) fn studied_tree(ui: &mut egui::Ui, study: &str, s: &Studied) {
    egui::CollapsingHeader::new(format!("model {}", s.model))
        .id_salt(("studied", study))
        .default_open(false)
        .show(ui, |ui| {
            ui.weak("Its values as written; open the study's node and pick a block to change them");
            for (block, values) in &s.blocks {
                ui.strong(block);
                egui::Grid::new(("studied-tree", study, block))
                    .num_columns(2)
                    .spacing([8.0, 2.0])
                    .show(ui, |ui| {
                        for v in values {
                            ui.label(&v.name);
                            let text = match (&v.driven, &v.formula) {
                                (Some(from), _) => format!("from {from}"),
                                (None, Some(f)) => format!("= {f}"),
                                (None, None) => format!(
                                    "{} {}",
                                    simsandbox_views::values::number(v.value),
                                    v.unit
                                ),
                            };
                            ui.monospace(text);
                            ui.end_row();
                        }
                    });
            }
        })
        .header_response
        .on_hover_text("The model this study runs at each of its points");
}

/// The model a study studies, as the Inspector shows it: its name, what
/// it can vary, and each block's values as its file writes them.
pub(super) struct Studied {
    pub(super) model: String,
    pub(super) keys: Vec<StudyKey>,
    pub(super) blocks: Vec<(String, Vec<simsandbox_session::diagram::NodeValue>)>,
}

/// A parameter a study can vary: its key, a range for it and whether it
/// is an initial condition.
pub(super) type StudyKey = (String, [f64; 2], bool);
