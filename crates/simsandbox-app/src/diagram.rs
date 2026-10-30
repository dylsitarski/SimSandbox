//! The Diagram tab (ADR 0042): the open model's node canvas. Moving
//! blocks only changes the layout; other edits rebuild the model and
//! restart the run as soon as it loads. Until then the edits are kept as a
//! draft, shown with what is wrong, and the run goes on with the last
//! model that worked. Subsystems open to their inside (ADR 0043), where
//! edits change every use.
//!
//! The data layer (ADR 0050) shows the model's filters and views; editing
//! them applies to the running model without a restart. Undo and redo
//! (Ctrl+Z, Ctrl+Shift+Z) step through the canvas's edits, keeping
//! parameter values changed since.

use crate::live::Model;
use eframe::egui;
use simsandbox_core::registry::Registry;
use simsandbox_file::{FilterEntry, ModelFile, ProbeEntry, ViewEntry};
use simsandbox_session::canvas::{type_menu, Canvas, Request, Selection};
use simsandbox_session::diagram::{apply_in, Diagram, Edit, NodeKind, Scope};
use simsandbox_session::undo::{canvas_state, change, Change};
use simsandbox_session::Session;
use simsandbox_views::ViewRegistry;

/// How many canvas edits can be undone.
const UNDO_DEPTH: usize = 100;

/// What the app should do after a frame of the canvas.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// Open this model file (it loads), from t = 0, then log the notes.
    Apply {
        text: String,
        notes: Vec<String>,
    },
    /// Save block positions in the running model's file, at a level.
    Moved(Scope, Vec<(String, [f64; 2])>),
    /// Save where data-layer nodes sit: filters and probes by name, views
    /// by place.
    Placed {
        filters: Vec<(String, [f64; 2])>,
        views: Vec<(usize, [f64; 2])>,
        probes: Vec<(String, [f64; 2])>,
        recorder: Option<[f64; 2]>,
    },
    /// Apply these filters, views and probes to the running model, then
    /// log the notes.
    Data {
        filters: Vec<FilterEntry>,
        views: Vec<ViewEntry>,
        probes: Vec<ProbeEntry>,
        recorder: Option<simsandbox_file::RecorderEntry>,
        notes: Vec<String>,
    },
    /// Put every node where this file has it (undoing a move).
    Layout(Box<ModelFile>),
    /// Edit a filter in the Data window.
    EditData(String),
    /// Show a view's tile.
    ShowView(usize),
    /// Which nodes show their values, to save (ADR 0055).
    Opened(Vec<String>),
    /// Set a parameter typed on a node (ADR 0053).
    SetValue {
        key: String,
        value: f64,
    },
    /// Open a block's definition: `"block"`, or inside a subsystem
    /// `"subsystem::block"`.
    Open(String),
    Error(String),
}

/// The model file before an edit, and the level it was made at.
#[derive(Clone)]
struct Snapshot {
    file: ModelFile,
    scope: Scope,
}

#[derive(Default)]
pub struct DiagramTab {
    pub canvas: Canvas,
    /// The level shown: the model, or a subsystem's inside.
    pub scope: Scope,
    /// The subsystem's name as typed for renaming.
    rename: String,
    /// Edits that do not load yet, and why.
    draft: Option<(ModelFile, String)>,
    cache: Option<Diagram>,
    /// Hide the data layer (filters and views).
    pub hide_data: bool,
    /// The filters and views the cached diagram shows, to redraw when
    /// they change elsewhere (the Data window, the Views tab).
    data_key: String,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// A probe's settings, open from its node (ADR 0062).
    probe: Option<crate::probe_editor::ProbeEditor>,
    /// The recorder's settings, open from its node (ADR 0064).
    recorder: Option<crate::recorder_editor::RecorderEditor>,
    /// The saved model's folder, where blocks read the files they name
    /// (ADR 0063); set by the app.
    pub folder: Option<std::path::PathBuf>,
}

impl DiagramTab {
    /// The model changed (opened or rebuilt): draw it afresh, dropping any
    /// draft.
    pub fn reset(&mut self) {
        self.draft = None;
        self.cache = None;
    }

    /// The edited file that does not load yet, if any.
    pub fn draft_file(&self) -> Option<&ModelFile> {
        self.draft.as_ref().map(|(f, _)| f)
    }

    /// The draft that does not load yet, if any.
    pub fn draft_problem(&self) -> Option<&str> {
        self.draft.as_ref().map(|(_, p)| p.as_str())
    }

    /// Forget the edit history (a different model was opened).
    pub fn forget_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    /// Draw the tab. `views` are the open tiles as saved; `current` gives
    /// the running model as a file with its current parameter values and
    /// views, read only when an edit needs it.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        model: &Model,
        registry: &Registry,
        views: (&[ViewEntry], &ViewRegistry),
        current: &dyn Fn() -> Result<String, String>,
    ) -> Vec<Outcome> {
        let mut out = Vec::new();
        let mut edits = Vec::new();
        if let Some((_, problem)) = &self.draft {
            let mut revert = false;
            egui::Frame::new()
                .fill(ui.visuals().warn_fg_color.gamma_multiply(0.15))
                .inner_margin(6.0)
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(ui.visuals().warn_fg_color, "Not applied yet:");
                        ui.label(problem);
                        ui.weak("The run goes on with the last model that worked.");
                        revert = ui
                            .button("Revert")
                            .on_hover_text("Drop the edits since then")
                            .clicked();
                    });
                });
            if revert {
                self.reset();
            }
        }
        let source = match &self.draft {
            Some((f, _)) => f,
            None => &model.file,
        };
        // A subsystem that is gone (renamed, or not in a new model).
        if let Scope::Subsystem(s) = &self.scope {
            if !source.subsystems.contains_key(s) {
                self.scope = Scope::Model;
                self.cache = None;
                self.canvas.refit();
            }
        }
        if let Scope::Subsystem(s) = self.scope.clone() {
            if self.rename.is_empty() {
                self.rename = s.clone();
            }
            let uses = simsandbox_session::diagram::uses(source, &s);
            ui.horizontal(|ui| {
                if ui
                    .link("Model")
                    .on_hover_text("Back to the whole model")
                    .clicked()
                {
                    self.scope = Scope::Model;
                    self.cache = None;
                    self.canvas.refit();
                }
                ui.label("›");
                ui.strong(&s).on_hover_text(if uses.is_empty() {
                    "Blocks and wires drawn here apply to every use of this subsystem".to_string()
                } else {
                    format!(
                        "Blocks and wires drawn here apply to every use ({}); each use keeps \
                         its own values, set in the inspector",
                        uses.join(", ")
                    )
                });
                let field =
                    ui.add(egui::TextEdit::singleline(&mut self.rename).desired_width(100.0));
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (ui.small_button("Rename").clicked() || enter) && self.rename != s {
                    edits.push(Edit::RenameSubsystem {
                        from: s.clone(),
                        to: self.rename.trim().to_string(),
                    });
                }
                if uses.is_empty() {
                    ui.weak("not used yet");
                }
            });
        }
        if self.cache.as_ref().is_some_and(|d| d.scope != self.scope) {
            self.cache = None;
        }
        // Filters and views change outside the canvas too.
        let key = match self.hide_data {
            true => String::new(),
            false => {
                serde_json::to_string(&(views.0, &model.filters, &model.probes, &model.recorder))
                    .unwrap_or_default()
            }
        };
        if key != self.data_key {
            self.data_key = key;
            self.cache = None;
        }
        let scope = self.scope.clone();
        let hide_data = self.hide_data;
        let draft = self.draft.is_some();
        let built = self.cache.get_or_insert_with(|| {
            let mut d = Diagram::of(source, &scope, registry);
            let mut f = source.clone();
            if !draft {
                // The running model's filters and the open tiles.
                f.controls.filters = model.filters.clone();
                f.controls.views = views.0.to_vec();
                f.controls.probes = model.probes.clone();
                f.controls.recorder = model.recorder.clone();
            }
            // Hidden, the data layer keeps the probes feeding blocks: they
            // are part of the model (ADR 0063).
            if hide_data {
                f = simsandbox_session::observers::model_part(&f);
            }
            d.observe(&f, views.1);
            d
        });
        // The running values on the nodes (ADR 0053).
        if !draft {
            built.live_values(&model.params);
        }
        // What the toolbar needs, so it can borrow the tab.
        let (subsystems, bounds) = (built.subsystems.clone(), built.bounds());
        let mut step = None;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.undo.is_empty(), egui::Button::new("Undo"))
                .on_hover_text("Undo the last canvas edit (Ctrl+Z)")
                .clicked()
            {
                step = Some(true);
            }
            if ui
                .add_enabled(!self.redo.is_empty(), egui::Button::new("Redo"))
                .on_hover_text("Redo (Ctrl+Shift+Z)")
                .clicked()
            {
                step = Some(false);
            }
            ui.menu_button("Add block", |ui| {
                if let Some(type_name) = type_menu(ui, registry, &subsystems) {
                    // To the right of what is there.
                    let at = bounds.map_or([0.0, 0.0], |b| [b[2] + 60.0, b[1]]);
                    edits.push(Edit::Add { type_name, at });
                    ui.close();
                }
            });
            if ui.button("Fit").on_hover_text("Show every block").clicked() {
                self.canvas.refit();
            }
            if ui
                .checkbox(&mut self.hide_data, "Hide data")
                .on_hover_text(
                    "Hide filters and views: they read recorded data and never feed the model \
                     (dashed)",
                )
                .changed()
            {
                // Redrawn from the next frame on.
                self.data_key = "\u{0}".into();
            }
            let hint = "Drag from an output to an input to connect · drag a wire off an \
                        input to remove it · Delete removes the selection · right-click for \
                        more · model edits restart the run, data edits do not";
            ui.add(egui::Label::new(egui::RichText::new(hint).weak()).truncate())
                .on_hover_text(hint);
        });
        let typing = ui.ctx().egui_wants_keyboard_input();
        if !typing {
            use egui::{Key, KeyboardShortcut, Modifiers};
            let redo = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
            let redo_y = KeyboardShortcut::new(Modifiers::COMMAND, Key::Y);
            let undo = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
            ui.input_mut(|i| {
                if i.consume_shortcut(&redo) || i.consume_shortcut(&redo_y) {
                    step = Some(false);
                } else if i.consume_shortcut(&undo) {
                    step = Some(true);
                }
            });
        }
        let d = self.cache.as_mut().expect("built above");
        let (made, requests) = self.canvas.show(ui, d, registry, true);
        edits.extend(made);
        let d = self.cache.as_ref().expect("built above");
        for r in requests {
            let name = match r {
                Request::Open(name) => name,
                Request::EditData(name) => {
                    out.push(Outcome::EditData(name));
                    continue;
                }
                Request::ShowView(k) => {
                    out.push(Outcome::ShowView(k));
                    continue;
                }
                Request::EditRecorder => {
                    let recorder = match &self.draft {
                        Some((f, _)) => f.controls.recorder.clone(),
                        None => model.recorder.clone(),
                    };
                    if let Some(r) = recorder {
                        self.recorder = Some(crate::recorder_editor::RecorderEditor::new(&r));
                    }
                    continue;
                }
                Request::EditProbe(name) => {
                    let probes = match &self.draft {
                        Some((f, _)) => &f.controls.probes,
                        None => &model.probes,
                    };
                    if let Some(p) = probes.iter().find(|p| p.name == name) {
                        self.probe = Some(crate::probe_editor::ProbeEditor::new(p));
                    }
                    continue;
                }
                // While edits wait as a draft, values go into it: the
                // running model may not have the block yet. Inside a
                // subsystem a value is every use's default, in the file.
                Request::SetValue { key, value } => {
                    let in_file = self.draft.is_some() || self.scope != Scope::Model;
                    match (in_file, key.split_once('.')) {
                        (true, Some((block, param))) => edits.push(Edit::SetParam {
                            block: block.into(),
                            param: param.into(),
                            value,
                        }),
                        _ => out.push(Outcome::SetValue { key, value }),
                    }
                    continue;
                }
                Request::SetFormula { key, formula } => {
                    if let Some((block, param)) = key.split_once('.') {
                        edits.push(Edit::SetFormula {
                            block: block.into(),
                            param: param.into(),
                            formula,
                        });
                    }
                    continue;
                }
                Request::Opened(open) => {
                    out.push(Outcome::Opened(open));
                    continue;
                }
                Request::SetSetting { block, key, text } => {
                    edits.push(Edit::SetSetting { block, key, text });
                    continue;
                }
            };
            let node = d.node(&name).map(|k| d.nodes[k].clone());
            match node {
                // A subsystem opens to its inside.
                Some(n) if n.kind == NodeKind::Subsystem => {
                    self.scope = Scope::Subsystem(n.type_name.clone());
                    self.rename = n.type_name.clone();
                    self.cache = None;
                    self.canvas.selected = None;
                    self.canvas.refit();
                    return out;
                }
                Some(n) if n.kind == NodeKind::Block => {
                    out.push(Outcome::Open(match &self.scope {
                        Scope::Model => name,
                        Scope::Subsystem(s) => format!("{s}::{name}"),
                    }))
                }
                _ => {}
            }
        }
        if let Some(ed) = &mut self.probe {
            use crate::probe_editor::Done;
            match ed.show(ui.ctx()) {
                Some(Done::Apply(probe)) => {
                    edits.push(Edit::SetProbe { probe: *probe });
                    self.probe = None;
                }
                Some(Done::Close) => self.probe = None,
                None => {}
            }
        }
        if let Some(ed) = &mut self.recorder {
            use crate::recorder_editor::Done;
            match ed.show(ui.ctx()) {
                Some(Done::Apply(recorder)) => {
                    edits.push(Edit::SetRecorder { recorder });
                    self.recorder = None;
                }
                Some(Done::Close) => self.recorder = None,
                None => {}
            }
        }
        if let Some(back) = step {
            out.extend(self.step(back, registry, current));
        } else {
            out.extend(self.handle(edits, registry, current));
        }
        out
    }

    /// The file the canvas edits: the draft, or the running model.
    fn base(&self, current: &dyn Fn() -> Result<String, String>) -> Result<ModelFile, String> {
        match &self.draft {
            Some((f, _)) => Ok(f.clone()),
            None => current().and_then(|t| {
                ModelFile::from_json(&t)
                    .map(|(f, _)| f)
                    .map_err(|e| e.to_string())
            }),
        }
    }

    /// Keep `file` as it was before an edit, for undo.
    fn remember(&mut self, file: ModelFile) {
        self.undo.push(Snapshot {
            file,
            scope: self.scope.clone(),
        });
        if self.undo.len() > UNDO_DEPTH {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Undo (`back`) or redo one canvas edit.
    pub fn step(
        &mut self,
        back: bool,
        registry: &Registry,
        current: &dyn Fn() -> Result<String, String>,
    ) -> Vec<Outcome> {
        let mut out = Vec::new();
        let popped = if back {
            self.undo.pop()
        } else {
            self.redo.pop()
        };
        let Some(snap) = popped else {
            return out;
        };
        let now = match self.base(current) {
            Ok(f) => f,
            Err(e) => {
                out.push(Outcome::Error(e));
                return out;
            }
        };
        let here = Snapshot {
            file: now.clone(),
            scope: self.scope.clone(),
        };
        if back {
            self.redo.push(here);
        } else {
            self.undo.push(here);
        }
        let next = canvas_state(&now, &snap.file);
        self.scope = match snap.scope {
            Scope::Subsystem(s) if !next.subsystems.contains_key(&s) => Scope::Model,
            s => s,
        };
        self.cache = None;
        self.canvas.selected = None;
        let word = if back { "Undone" } else { "Redone" };
        let kind = if self.draft.is_some() {
            Change::Model
        } else {
            change(&now, &next)
        };
        match kind {
            Change::Layout => out.push(Outcome::Layout(Box::new(next))),
            Change::Data => {
                out.push(Outcome::Layout(Box::new(next.clone())));
                out.push(Outcome::Data {
                    filters: next.controls.filters,
                    views: next.controls.views,
                    probes: next.controls.probes,
                    recorder: next.controls.recorder,
                    notes: vec![word.into()],
                });
            }
            Change::Model => {
                self.draft = None;
                let text = next.to_json();
                match Session::load_in("check", &text, registry, self.folder.as_deref(), true) {
                    Ok(_) => out.push(Outcome::Apply {
                        text,
                        notes: vec![word.into()],
                    }),
                    Err(problem) => {
                        let mut next = next;
                        next.folder = self.folder.clone();
                        self.draft = Some((next, problem));
                    }
                }
            }
        }
        out
    }

    /// Turn the canvas's edits into outcomes: moves are saved as they are;
    /// filter and view edits apply to the running model; other edits apply
    /// if the result loads, else they become the draft. Each batch can be
    /// undone.
    pub fn handle(
        &mut self,
        edits: Vec<Edit>,
        registry: &Registry,
        current: &dyn Fn() -> Result<String, String>,
    ) -> Vec<Outcome> {
        let mut out = Vec::new();
        if edits.is_empty() {
            return out;
        }
        let base = match self.base(current) {
            Ok(f) => f,
            Err(e) => {
                out.push(Outcome::Error(e));
                return out;
            }
        };
        self.remember(base.clone());
        let scope = self.scope.clone();
        let (moves, rest): (Vec<Edit>, Vec<Edit>) = edits
            .into_iter()
            .partition(|e| matches!(e, Edit::Move { .. }));
        if self.draft.is_some() || rest.iter().any(Edit::changes_model) {
            self.rebuild(
                base,
                moves.into_iter().chain(rest).collect(),
                registry,
                &mut out,
            );
            return out;
        }
        let mut placed = (Vec::new(), Vec::new(), Vec::new());
        let mut recorder_at = None;
        let mut model_moves = Vec::new();
        for e in moves {
            let Edit::Move { block, at } = e else {
                continue;
            };
            if let Some(f) = block.strip_prefix("filter:") {
                placed.0.push((f.to_string(), at));
            } else if let Some(p) = block.strip_prefix("probe:") {
                placed.2.push((p.to_string(), at));
            } else if block == simsandbox_session::observers::RECORDER {
                recorder_at = Some(at);
            } else if let Some(k) = block.strip_prefix("view:").and_then(|k| k.parse().ok()) {
                placed.1.push((k, at));
            } else {
                model_moves.push((block, at));
            }
        }
        if !model_moves.is_empty() {
            out.push(Outcome::Moved(scope.clone(), model_moves));
        }
        if !placed.0.is_empty()
            || !placed.1.is_empty()
            || !placed.2.is_empty()
            || recorder_at.is_some()
        {
            out.push(Outcome::Placed {
                filters: placed.0,
                views: placed.1,
                probes: placed.2,
                recorder: recorder_at,
            });
        }
        if !rest.is_empty() {
            // A data edit that reaches the model (a probe feeding a block,
            // ADR 0063) rebuilds it like any model edit.
            let mut trial = base.clone();
            for e in &rest {
                let _ = apply_in(&mut trial, &scope, registry, e);
            }
            if change(&base, &trial) == Change::Model {
                self.rebuild(base, rest, registry, &mut out);
                return out;
            }
            let mut file = base;
            let mut notes = Vec::new();
            for e in &rest {
                match apply_in(&mut file, &scope, registry, e) {
                    Ok(note) => notes.push(note),
                    Err(err) => out.push(Outcome::Error(err)),
                }
            }
            self.cache = None;
            out.push(Outcome::Data {
                filters: file.controls.filters,
                views: file.controls.views,
                probes: file.controls.probes,
                recorder: file.controls.recorder,
                notes,
            });
        }
        out
    }

    /// Apply edits that change the model to `file`: open it if it loads,
    /// else keep it as the draft.
    fn rebuild(
        &mut self,
        mut file: ModelFile,
        changes: Vec<Edit>,
        registry: &Registry,
        out: &mut Vec<Outcome>,
    ) {
        let scope = self.scope.clone();
        let had_draft = self.draft.take().is_some();
        // Fix where every node of this level is, so the layout does not
        // jump.
        for n in Diagram::of(&file, &scope, registry).nodes {
            if !n.placed {
                let m = Edit::Move {
                    block: n.name.clone(),
                    at: n.at,
                };
                let _ = apply_in(&mut file, &scope, registry, &m);
            }
        }
        let mut notes = Vec::new();
        for e in &changes {
            match apply_in(&mut file, &scope, registry, e) {
                Ok(note) => {
                    notes.push(note);
                    if let Edit::RenameSubsystem { to, .. } = e {
                        self.scope = Scope::Subsystem(to.clone());
                    }
                }
                Err(err) => out.push(Outcome::Error(err)),
            }
        }
        // Nothing applied (and nothing waiting): no restart, no undo step.
        if notes.is_empty() && !had_draft {
            self.undo.pop();
            return;
        }
        // A value or setting changed: the blocks are still there.
        let kept = changes
            .iter()
            .all(|e| matches!(e, Edit::SetParam { .. } | Edit::SetSetting { .. }));
        if !kept
            && matches!(
                self.canvas.selected,
                Some(Selection::Block(_) | Selection::Many(_))
            )
        {
            self.canvas.selected = None;
        }
        self.cache = None;
        let text = file.to_json();
        // A formula or parameter that does not work is refused, not kept
        // as a draft (ADR 0056): it is a typo to fix where it was typed.
        let refused = !had_draft
            && changes.iter().all(|e| {
                matches!(
                    e,
                    Edit::SetFormula { .. } | Edit::DeclareParam { .. } | Edit::RemoveParam { .. }
                )
            });
        match Session::load_in("check", &text, registry, self.folder.as_deref(), true) {
            Ok(_) => out.push(Outcome::Apply { text, notes }),
            Err(problem) if refused => {
                self.undo.pop();
                out.push(Outcome::Error(format!("Not applied: {problem}")));
            }
            Err(problem) => {
                let mut file = file;
                file.folder = self.folder.clone();
                self.draft = Some((file, problem));
            }
        }
    }
}
