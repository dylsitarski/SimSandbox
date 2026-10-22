//! The Diagram tab (ADR 0042): the open model's node canvas. Moving
//! blocks only changes the layout; other edits rebuild the model and
//! restart the run as soon as it loads. Until then the edits are kept as a
//! draft, shown with what is wrong, and the run goes on with the last
//! model that worked.

use crate::live::Model;
use eframe::egui;
use simsandbox_core::registry::Registry;
use simsandbox_file::ModelFile;
use simsandbox_session::canvas::{type_menu, Canvas, Request, Selection};
use simsandbox_session::diagram::{apply, Diagram, Edit};
use simsandbox_session::Session;

/// What the app should do after a frame of the canvas.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// Open this model file (it loads), from t = 0, then log the notes.
    Apply {
        text: String,
        notes: Vec<String>,
    },
    /// Save block positions in the running model's file.
    Moved(Vec<(String, [f64; 2])>),
    /// Open a block's definition.
    Open(String),
    Error(String),
}

#[derive(Default)]
pub struct DiagramTab {
    pub canvas: Canvas,
    /// Edits that do not load yet, and why.
    draft: Option<(ModelFile, String)>,
    cache: Option<Diagram>,
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

    /// Draw the tab. `current` gives the running model as a file with its
    /// current parameter values, read only when an edit needs it.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        model: &Model,
        registry: &Registry,
        current: impl FnOnce() -> Result<String, String>,
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
        let d = self
            .cache
            .get_or_insert_with(|| Diagram::new(source, registry));
        ui.horizontal(|ui| {
            ui.menu_button("Add block", |ui| {
                if let Some(type_name) = type_menu(ui, registry) {
                    // To the right of what is there.
                    let at = d.bounds().map_or([0.0, 0.0], |b| [b[2] + 60.0, b[1]]);
                    edits.push(Edit::Add { type_name, at });
                    ui.close();
                }
            });
            if ui.button("Fit").on_hover_text("Show every block").clicked() {
                self.canvas.refit();
            }
            let hint = "Drag from an output to an input to connect · drag a wire off an \
                        input to remove it · Delete removes the selection · right-click for \
                        more · edits restart the run";
            ui.add(egui::Label::new(egui::RichText::new(hint).weak()).truncate())
                .on_hover_text(hint);
        });
        let (made, requests) = self.canvas.show(ui, d, registry, true);
        edits.extend(made);
        for r in requests {
            match r {
                Request::Open(name) => out.push(Outcome::Open(name)),
            }
        }
        out.extend(self.handle(edits, registry, current));
        out
    }

    /// Turn the canvas's edits into outcomes: moves are saved as they are;
    /// other edits apply if the result loads, else they become the draft.
    pub fn handle(
        &mut self,
        edits: Vec<Edit>,
        registry: &Registry,
        current: impl FnOnce() -> Result<String, String>,
    ) -> Vec<Outcome> {
        let mut out = Vec::new();
        let moves: Vec<(String, [f64; 2])> = edits
            .iter()
            .filter_map(|e| match e {
                Edit::Move { block, at } => Some((block.clone(), *at)),
                _ => None,
            })
            .collect();
        let changes: Vec<Edit> = edits.into_iter().filter(Edit::changes_model).collect();
        if let Some((draft, _)) = &mut self.draft {
            for (block, at) in &moves {
                let _ = apply(
                    draft,
                    registry,
                    &Edit::Move {
                        block: block.clone(),
                        at: *at,
                    },
                );
            }
        } else if !moves.is_empty() {
            out.push(Outcome::Moved(moves));
        }
        if changes.is_empty() {
            return out;
        }
        // Edit the draft, or the running model with its current values.
        let mut file = match self.draft.take() {
            Some((f, _)) => f,
            None => {
                match current().and_then(|t| ModelFile::from_json(&t).map_err(|e| e.to_string())) {
                    Ok((f, _)) => f,
                    Err(e) => {
                        out.push(Outcome::Error(e));
                        return out;
                    }
                }
            }
        };
        // Fix where every block is, so the layout does not jump.
        for n in Diagram::new(&file, registry).nodes {
            if let Some(b) = file.blocks.iter_mut().find(|b| b.name == n.name) {
                b.at.get_or_insert(n.at);
            }
        }
        let mut notes = Vec::new();
        for e in &changes {
            match apply(&mut file, registry, e) {
                Ok(note) => notes.push(note),
                Err(err) => out.push(Outcome::Error(err)),
            }
        }
        if let Some(Selection::Block(name)) = &self.canvas.selected {
            if !file.blocks.iter().any(|b| &b.name == name) {
                self.canvas.selected = None;
            }
        }
        self.cache = None;
        let text = file.to_json();
        match Session::load_deferred("check", &text, registry) {
            Ok(_) => out.push(Outcome::Apply { text, notes }),
            Err(problem) => self.draft = Some((file, problem)),
        }
        out
    }
}
