//! The equation editor (ADR 0017) and the Data window (ADR 0030): the
//! windows that edit the model's equations and its filters, and what they
//! apply.

use super::*;

impl SimApp {
    /// The equation editor window (ADR 0017).
    pub(super) fn equation_editor(&mut self, ctx: &egui::Context) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        let title = match &editor.block {
            Some(b) => format!("Equations of '{b}'"),
            None => "New model".into(),
        };
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new(title)
            .open(&mut open)
            .default_width(520.0)
            .show(ctx, |ui| {
                egui::CollapsingHeader::new("How to write equations").show(ui, |ui| {
                    ui.label(SYNTAX_HELP);
                });
                ui.add(
                    egui::TextEdit::multiline(&mut editor.text)
                        .code_editor()
                        .desired_rows(12)
                        .desired_width(f32::INFINITY),
                );
                if let Some(e) = &editor.error {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
                ui.horizontal(|ui| {
                    apply = ui.button("Apply").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        if cancel || !open {
            self.editor = None;
        } else if apply {
            self.apply_equations();
        }
    }

    /// The derived-signals window (ADR 0015).
    /// The Data window: the model's filters (ADR 0030, ADR 0032), derived
    /// expressions and cuts, edited together and applied at once.
    pub(super) fn signals_window(&mut self, ctx: &egui::Context) {
        let (Some(ed), Some(live)) = (self.signals_editor.as_mut(), self.session.as_ref()) else {
            return;
        };
        let mut open = true;
        let (mut apply, mut cancel) = (false, false);
        {
            let history = live.history();
            let m = live.model();
            egui::Window::new("Data")
                .open(&mut open)
                .default_width(640.0)
                .min_width(480.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(520.0)
                        .show(ui, |ui| {
                            data_editor(ui, ed, &history, &m);
                        });
                    for p in &ed.problems {
                        ui.colored_label(ui.visuals().error_fg_color, p);
                    }
                    ui.horizontal(|ui| {
                        apply = ui.button("Apply").clicked();
                        cancel = ui.button("Cancel").clicked();
                    });
                });
        }
        if cancel || !open {
            self.signals_editor = None;
        } else if apply {
            let before: Vec<String> = live
                .model()
                .filters
                .iter()
                .map(|f| f.name.clone())
                .collect();
            let mut entries = ed.entries();
            // Filters keep their place on the canvas (ADR 0050) and the
            // probe they run on (ADR 0063).
            for e in &mut entries {
                let was = live.model();
                let was = was.filters.iter().find(|f| f.name == e.name);
                e.canvas = was.and_then(|f| f.canvas);
                e.probe = was.and_then(|f| f.probe.clone());
                e.from = was.map(|f| f.from.clone()).unwrap_or_default();
                e.when = was.and_then(|f| f.when.clone());
            }
            match live.call(move |s, _| s.set_filters(entries)) {
                Ok(Ok(())) => {
                    // A new cut filter opens a profile of its cuts.
                    let m = live.model();
                    let history = live.history();
                    let available = m.available(&history);
                    for c in &ed.cuts {
                        if before.contains(&c.name) {
                            continue;
                        }
                        let entry = c.entry();
                        let outputs = entry.outputs();
                        let (lines, coordinate) = outputs.split_at(outputs.len() - 1);
                        let bind = [
                            ("y".to_string(), lines.to_vec()),
                            ("x".to_string(), coordinate.to_vec()),
                        ]
                        .into();
                        self.views.add_bound("plot.profile", bind, &available);
                    }
                    self.signals_editor = None;
                    self.log.info("Data filters applied");
                }
                Ok(Err(p)) => ed.problems = p,
                Err(e) => ed.problems = vec![e],
            }
        }
    }

    /// Run the editor's equations: as a new model, or as the new equations
    /// of a block of the open model, keeping the values of parameters that
    /// still exist.
    pub(super) fn apply_equations(&mut self) {
        let Some(editor) = self.editor.as_ref() else {
            return;
        };
        let lines: Vec<String> = editor.text.lines().map(str::to_string).collect();
        let block = editor.block.clone();
        // A new model replaces the open one.
        if block.is_none() && self.ask_first(Discard::NewModel) {
            return;
        }
        // Inside a used model: an edit there, which may ask where it goes
        // (ADR 0068).
        if let Some((path, inner)) = block.as_deref().and_then(|b| b.split_once("::")) {
            let Some(live) = &self.session else {
                return;
            };
            let m = live.model();
            let (speed, views, layout) = (self.speed, self.views.entries(), self.layout());
            let current = || {
                let (views, layout) = (views.clone(), layout.clone());
                live.call(move |s, _| s.save_json_with(speed, Some(views), Some(layout)))?
            };
            self.diagram.inside = path.split('/').map(str::to_string).collect();
            let edit = Edit::SetEquations {
                block: inner.to_string(),
                equations: lines,
            };
            let outcomes = self
                .diagram
                .begin_edit(vec![edit], &self.registry, &current);
            let previous_error = self.error.take();
            self.diagram_outcomes(&m, outcomes);
            match self.error.take() {
                Some(e) => {
                    self.error = previous_error;
                    if let Some(ed) = self.editor.as_mut() {
                        ed.error = Some(e);
                    }
                }
                None => {
                    self.editor = None;
                    self.log.info("Equations applied");
                }
            }
            return;
        }
        let result = match &block {
            None => {
                let mut file = simsandbox_file::ModelFile::new();
                file.blocks.push(simsandbox_file::BlockEntry {
                    name: "model".into(),
                    type_name: simsandbox_eq::registry::EQUATIONS.into(),
                    method: simsandbox_eq::EquationSet::parse("model", &lines)
                        .map_or(simsandbox_eq::Kind::Continuous, |s| s.kind())
                        .default_method()
                        .into(),
                    update: None,
                    equations: lines,
                    params: Default::default(),
                    ..Default::default()
                });
                Ok(("Untitled".to_string(), file.to_json()))
            }
            Some(name) => self.edited_model(name, lines),
        };
        let (title, text) = match result {
            Ok(v) => v,
            Err(e) => {
                if let Some(ed) = self.editor.as_mut() {
                    ed.error = Some(e);
                }
                return;
            }
        };
        let previous_error = self.error.take();
        self.open_text(&title, &text);
        match self.error.take() {
            Some(e) => {
                self.error = previous_error;
                if let Some(ed) = self.editor.as_mut() {
                    ed.error = Some(e);
                }
            }
            None => {
                if block.is_none() {
                    self.path = None;
                    self.example_folder = None;
                    self.mark_saved();
                }
                self.editor = None;
                self.log.info("Equations applied");
            }
        }
    }

    /// The open model as a file, with the equations of `block` replaced and
    /// parameters that no longer exist dropped.
    pub(super) fn edited_model(
        &mut self,
        block: &str,
        lines: Vec<String>,
    ) -> Result<(String, String), String> {
        let title = match &self.session {
            Some(live) => live.model().title.clone(),
            None => return Err("no model is open".into()),
        };
        let inner = block;
        let set = simsandbox_eq::EquationSet::parse(inner, &lines).map_err(|e| e.to_string())?;
        let keep: Vec<String> = simsandbox_eq::OdeSystem::spec(&set)
            .params
            .iter()
            .map(|p| p.name.clone())
            .collect();
        let text = self.saved_text()?;
        let (mut file, _) =
            simsandbox_file::ModelFile::from_json(&text).map_err(|e| e.to_string())?;
        let entry = file
            .blocks
            .iter_mut()
            .find(|b| b.name == inner)
            .ok_or_else(|| format!("no block '{block}'"))?;
        entry.equations = lines;
        entry.params.retain(|name, _| keep.contains(name));
        // Switching between continuous equations, a map and formulas changes the method.
        let method = entry.method.name.as_str();
        let compatible = match set.kind() {
            simsandbox_eq::Kind::Continuous => !matches!(method, "iterate" | "evaluate"),
            simsandbox_eq::Kind::Map => method == "iterate",
            simsandbox_eq::Kind::Formulas => method == "evaluate",
        };
        if !compatible {
            entry.method = set.kind().default_method().into();
        }
        Ok((title, file.to_json()))
    }
}

/// The Data window's working copy of the filters.
pub(super) struct SignalsEditor {
    pub(super) exprs: Vec<(String, String)>,
    pub(super) cuts: Vec<CutEdit>,
    pub(super) problems: Vec<String>,
}

/// One cut filter being edited.
pub(super) struct CutEdit {
    pub(super) name: String,
    pub(super) input: String,
    pub(super) settings: simsandbox_views::cut::CutSettings,
}

impl CutEdit {
    pub(super) fn entry(&self) -> simsandbox_file::FilterEntry {
        let settings = match serde_json::to_value(&self.settings) {
            Ok(serde_json::Value::Object(m)) => m.into_iter().collect(),
            _ => Default::default(),
        };
        simsandbox_file::FilterEntry {
            name: self.name.trim().into(),
            type_name: "cut".into(),
            inputs: vec![self.input.clone()],
            canvas: None,
            probe: None,
            from: Default::default(),
            when: None,
            settings,
        }
    }
}

impl SignalsEditor {
    /// The session's filters, ready to edit.
    pub(super) fn new(session: &Model) -> Self {
        let mut ed = Self {
            exprs: Vec::new(),
            cuts: Vec::new(),
            problems: Vec::new(),
        };
        for f in &session.filters {
            match f.type_name.as_str() {
                "expression" => ed
                    .exprs
                    .push((f.name.clone(), f.expr().unwrap_or_default().to_string())),
                "cut" => {
                    let object =
                        serde_json::Value::Object(f.settings.clone().into_iter().collect());
                    if let Ok(settings) = serde_json::from_value(object) {
                        ed.cuts.push(CutEdit {
                            name: f.name.clone(),
                            input: f.inputs.first().cloned().unwrap_or_default(),
                            settings,
                        });
                    }
                }
                _ => {}
            }
        }
        ed
    }

    /// The filters to apply.
    pub(super) fn entries(&self) -> Vec<simsandbox_file::FilterEntry> {
        self.exprs
            .iter()
            .map(|(name, text)| {
                simsandbox_file::FilterEntry::expression(name.trim(), text.trim(), Vec::new())
            })
            .chain(self.cuts.iter().map(CutEdit::entry))
            .collect()
    }
}

/// The equation editor: for a new model (`block` is `None`) or for a block
/// of the open model.
pub(super) struct Editor {
    pub(super) block: Option<String>,
    pub(super) text: String,
    pub(super) error: Option<String>,
}

/// What "New model…" starts with: a damped oscillator that moves at once.
pub(super) const NEW_MODEL: &str = "\
# A spring with friction. Change anything, then press Apply.
# x and v are states: where they start, then how they change.
state x = 1
state v = 0
dx/dt = v
dv/dt = -k*x - c*v
# a quantity computed from the states, plotted like them
energy = 0.5*v^2 + 0.5*k*x^2
";

pub(super) const SYNTAX_HELP: &str = "\
One equation per line:
  dx/dt = ...    (or x' = ...)   x is a state that changes over time
  E = ...                         a quantity computed from the states
  input u, F                      u and F are inputs, set by connected blocks
  param k = 2                     a parameter with its default value
  state x = 0                     x is a state starting at 0 (optional)
Every other name is a parameter (default 1) you can change in the side panel.
Each state x also gets an initial value x0 (default 1).
Units go in brackets where a name appears: input F [N], param m [kg] = 2,
state x [m] = 0; at the end of a line they are the unit of its left side:
E = m*v^2/2 [J], dv/dt = F/m [m/s^2]. Units take SI prefixes (kN, MPa, µs, mL) and include
min, h, L, bar, deg, %, ft, in, mi, lb, lbf, psi, mph and more; temperatures
use K. Then every line is checked to balance; names without a unit
are dimensionless. Connections need the same unit on both ends: put a
units.convert block between different ones (e.g. mm to m).
t is time. Operators: + - * / ^ and parentheses; write 2*x, not 2x.
Functions: sin cos tan asin acos atan atan2 sinh cosh tanh exp ln log10
sqrt abs sign floor ceil min max pow. pi is 3.14159…
Lines starting with # are comments.";
