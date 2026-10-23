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
use simsandbox_file::{FilterEntry, ModelFile, ViewEntry};
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
    /// Save where data-layer nodes sit: filters by name, views by place.
    Placed {
        filters: Vec<(String, [f64; 2])>,
        views: Vec<(usize, [f64; 2])>,
    },
    /// Apply these filters and views to the running model, then log the
    /// notes.
    Data {
        filters: Vec<FilterEntry>,
        views: Vec<ViewEntry>,
        notes: Vec<String>,
    },
    /// Put every node where this file has it (undoing a move).
    Layout(Box<ModelFile>),
    /// Edit a filter in the Data window.
    EditData(String),
    /// Show a view's tile.
    ShowView(usize),
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
}

impl DiagramTab {
    /// The model changed (opened or rebuilt): draw it afresh, dropping any
    /// draft.
    pub fn reset(&mut self) {
        self.draft = None;
        self.cache = None;
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
                ui.strong(&s);
                let field =
                    ui.add(egui::TextEdit::singleline(&mut self.rename).desired_width(100.0));
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (ui.small_button("Rename").clicked() || enter) && self.rename != s {
                    edits.push(Edit::RenameSubsystem {
                        from: s.clone(),
                        to: self.rename.trim().to_string(),
                    });
                }
                let text = match uses.len() {
                    0 => "not used yet".to_string(),
                    _ => format!("used by {}: editing changes every use", uses.join(", ")),
                };
                ui.add(egui::Label::new(egui::RichText::new(text).weak()).truncate());
            });
        }
        if self.cache.as_ref().is_some_and(|d| d.scope != self.scope) {
            self.cache = None;
        }
        // Filters and views change outside the canvas too.
        let key = match self.hide_data {
            true => String::new(),
            false => serde_json::to_string(&(views.0, &model.filters)).unwrap_or_default(),
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
            if !hide_data {
                if draft {
                    d.observe(source, views.1);
                } else {
                    // The running model's filters and the open tiles.
                    let mut f = source.clone();
                    f.controls.filters = model.filters.clone();
                    f.controls.views = views.0.to_vec();
                    d.observe(&f, views.1);
                }
            }
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
            ui.checkbox(&mut self.canvas.show_values, "Values")
                .on_hover_text(
                    "Parameter values on the blocks; zoom in and double-click one to change it",
                );
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
                Request::SetValue { key, value } => {
                    out.push(Outcome::SetValue { key, value });
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
                    notes: vec![word.into()],
                });
            }
            Change::Model => {
                self.draft = None;
                let text = next.to_json();
                match Session::load_deferred("check", &text, registry) {
                    Ok(_) => out.push(Outcome::Apply {
                        text,
                        notes: vec![word.into()],
                    }),
                    Err(problem) => self.draft = Some((next, problem)),
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
        let mut placed = (Vec::new(), Vec::new());
        let mut model_moves = Vec::new();
        for e in moves {
            let Edit::Move { block, at } = e else {
                continue;
            };
            if let Some(f) = block.strip_prefix("filter:") {
                placed.0.push((f.to_string(), at));
            } else if let Some(k) = block.strip_prefix("view:").and_then(|k| k.parse().ok()) {
                placed.1.push((k, at));
            } else {
                model_moves.push((block, at));
            }
        }
        if !model_moves.is_empty() {
            out.push(Outcome::Moved(scope.clone(), model_moves));
        }
        if !placed.0.is_empty() || !placed.1.is_empty() {
            out.push(Outcome::Placed {
                filters: placed.0,
                views: placed.1,
            });
        }
        if !rest.is_empty() {
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
        self.draft = None;
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
        if matches!(
            self.canvas.selected,
            Some(Selection::Block(_) | Selection::Many(_))
        ) {
            self.canvas.selected = None;
        }
        self.cache = None;
        let text = file.to_json();
        match Session::load_deferred("check", &text, registry) {
            Ok(_) => out.push(Outcome::Apply { text, notes }),
            Err(problem) => self.draft = Some((file, problem)),
        }
    }
}
