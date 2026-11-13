//! The recorder's settings (ADR 0064), opened from its node on the
//! canvas: how often it saves the model's state, and how much memory it
//! may use.

use eframe::egui;
use simsandbox_file::RecorderEntry;

/// The settings as typed.
pub struct RecorderEditor {
    entry: RecorderEntry,
    every: String,
    mb: String,
    pub problem: Option<String>,
}

/// What the window asks for.
pub enum Done {
    Apply(RecorderEntry),
    Close,
}

impl RecorderEditor {
    pub fn new(entry: &RecorderEntry) -> Self {
        Self {
            entry: entry.clone(),
            every: entry.every().to_string(),
            mb: entry.mb().to_string(),
            problem: None,
        }
    }

    /// The recorder as set, or what is wrong; defaults are left out.
    pub fn entry(&self) -> Result<RecorderEntry, String> {
        let every: u64 = self
            .every
            .trim()
            .parse()
            .map_err(|_| format!("every: '{}' is not a whole number", self.every.trim()))?;
        let mb: f64 = self
            .mb
            .trim()
            .parse()
            .map_err(|_| format!("memory: '{}' is not a number", self.mb.trim()))?;
        let e = RecorderEntry {
            every: (every != RecorderEntry::EVERY).then_some(every),
            mb: (mb != RecorderEntry::MB).then_some(mb),
            canvas: self.entry.canvas,
        };
        e.check()?;
        Ok(e)
    }

    /// Draw the window; `Some` when it is applied or closed.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<Done> {
        let mut done = None;
        let mut open = true;
        egui::Window::new("Recorder")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.weak("Saves the whole model's state as it runs, so it can be rewound.");
                egui::Grid::new("recorder settings")
                    .num_columns(3)
                    .show(ui, |ui| {
                        ui.label("Every");
                        ui.add(egui::TextEdit::singleline(&mut self.every).desired_width(80.0))
                            .on_hover_text(
                                "Steps between saved states: fewer is finer rewind, more memory",
                            );
                        ui.label("steps");
                        ui.end_row();
                        ui.label("Memory");
                        ui.add(egui::TextEdit::singleline(&mut self.mb).desired_width(80.0))
                            .on_hover_text(
                                "Past this, every other saved state is dropped: the whole run \\
                                 stays reachable, coarser further back",
                            );
                        ui.label("MB");
                        ui.end_row();
                    });
                if let Some(p) = &self.problem {
                    ui.colored_label(ui.visuals().error_fg_color, p);
                }
                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        match self.entry() {
                            Ok(e) => done = Some(Done::Apply(e)),
                            Err(p) => self.problem = Some(p),
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        done = Some(Done::Close);
                    }
                });
            });
        if !open {
            done = Some(Done::Close);
        }
        done
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_settings_become_the_recorder() {
        let mut ed = RecorderEditor::new(&RecorderEntry::default());
        assert_eq!(ed.entry().unwrap(), RecorderEntry::default());
        ed.every = "5".into();
        ed.mb = "64".into();
        let e = ed.entry().unwrap();
        assert_eq!((e.every, e.mb), (Some(5), Some(64.0)));
        ed.every = "0".into();
        assert!(ed.entry().is_err());
    }
}
