//! A probe's settings (ADR 0062), opened from its node on the canvas:
//! when it records, how long it keeps samples and the file it writes.
//! What it records is wired on the canvas.

use eframe::egui;
use simsandbox_file::{ProbeEntry, ProbeEvery, ProbeKeep};

const EVERY: [&str; 3] = ["steps", "seconds", "times"];
/// Samples a probe keeps before thinning when nothing says otherwise.
const THIN_SAMPLES: usize = 100_000;
const KEEP: [&str; 4] = [
    "thinned past samples",
    "thinned past MB",
    "last samples",
    "all",
];

/// The settings as typed.
pub struct ProbeEditor {
    entry: ProbeEntry,
    every: usize,
    every_text: String,
    keep: usize,
    keep_text: String,
    to: String,
    stream: bool,
    pub problem: Option<String>,
}

/// What the window asks for.
pub enum Done {
    Apply(Box<ProbeEntry>),
    Close,
}

impl ProbeEditor {
    pub fn new(entry: &ProbeEntry) -> Self {
        let num = |v: f64| format!("{v}");
        let (every, every_text) = match &entry.every {
            Some(ProbeEvery {
                seconds: Some(s), ..
            }) => (1, num(*s)),
            Some(ProbeEvery {
                times: Some(ts), ..
            }) => (2, ts.iter().map(|t| num(*t)).collect::<Vec<_>>().join(", ")),
            Some(ProbeEvery { steps: Some(n), .. }) => (0, n.to_string()),
            _ => (0, "1".into()),
        };
        let (keep, keep_text) = match &entry.keep {
            Some(ProbeKeep { mb: Some(mb), .. }) => (1, num(*mb)),
            Some(ProbeKeep { last: Some(n), .. }) => (2, n.to_string()),
            Some(ProbeKeep { all: true, .. }) => (3, String::new()),
            Some(ProbeKeep {
                samples: Some(n), ..
            }) => (0, n.to_string()),
            _ => (0, THIN_SAMPLES.to_string()),
        };
        Self {
            entry: entry.clone(),
            every,
            every_text,
            keep,
            keep_text,
            to: entry.to.clone().unwrap_or_default(),
            stream: entry.stream,
            problem: None,
        }
    }

    /// The probe as set, or what is wrong.
    pub fn entry(&self) -> Result<ProbeEntry, String> {
        let number = |s: &str, what: &str| -> Result<f64, String> {
            s.trim()
                .parse::<f64>()
                .map_err(|_| format!("{what}: '{}' is not a number", s.trim()))
        };
        let whole = |s: &str, what: &str| -> Result<u64, String> {
            s.trim()
                .parse::<u64>()
                .map_err(|_| format!("{what}: '{}' is not a whole number", s.trim()))
        };
        let mut e = self.entry.clone();
        e.every = match self.every {
            0 => match whole(&self.every_text, "every")? {
                1 => None,
                n => Some(ProbeEvery {
                    steps: Some(n),
                    ..Default::default()
                }),
            },
            1 => Some(ProbeEvery {
                seconds: Some(number(&self.every_text, "every")?),
                ..Default::default()
            }),
            _ => Some(ProbeEvery {
                times: Some(
                    self.every_text
                        .split([',', ' '])
                        .filter(|s| !s.is_empty())
                        .map(|s| number(s, "times"))
                        .collect::<Result<_, _>>()?,
                ),
                ..Default::default()
            }),
        };
        e.keep = match self.keep {
            // The default is left out, so the safety net for grids holds.
            0 => match whole(&self.keep_text, "keep")? as usize {
                THIN_SAMPLES => None,
                n => Some(ProbeKeep {
                    samples: Some(n),
                    ..Default::default()
                }),
            },
            1 => Some(ProbeKeep {
                mb: Some(number(&self.keep_text, "keep")?),
                ..Default::default()
            }),
            2 => Some(ProbeKeep {
                last: Some(whole(&self.keep_text, "keep")? as usize),
                ..Default::default()
            }),
            _ => Some(ProbeKeep {
                all: true,
                ..Default::default()
            }),
        };
        let to = self.to.trim();
        e.to = (!to.is_empty()).then(|| {
            if to.ends_with(".csv") {
                to.to_string()
            } else {
                format!("{to}.csv")
            }
        });
        e.stream = self.stream && e.to.is_some();
        e.check()?;
        Ok(e)
    }

    /// Draw the window; `Some` when it is applied or closed.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<Done> {
        let mut done = None;
        let mut open = true;
        egui::Window::new(format!("Probe {}", self.entry.name))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.weak(if self.entry.record.is_empty() {
                    "Records nothing yet: wire signals into it on the diagram.".to_string()
                } else {
                    format!("Records {}", self.entry.record.join(", "))
                });
                egui::Grid::new("probe settings")
                    .num_columns(3)
                    .show(ui, |ui| {
                        ui.label("Every");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.every_text).desired_width(120.0),
                        )
                        .on_hover_text("Steps or seconds; for times, a list such as 0, 10, 25");
                        egui::ComboBox::from_id_salt("probe every")
                            .selected_text(EVERY[self.every])
                            .show_ui(ui, |ui| {
                                for (k, l) in EVERY.iter().enumerate() {
                                    ui.selectable_value(&mut self.every, k, *l);
                                }
                            });
                        ui.end_row();
                        ui.label("Keep");
                        ui.add_enabled(
                            self.keep != 3,
                            egui::TextEdit::singleline(&mut self.keep_text).desired_width(120.0),
                        )
                        .on_hover_text(
                            "What it keeps in memory, for views and filters. Thinned: every \
                             other sample dropped past it (grids are thinned sooner unless you \
                             give a number).",
                        );
                        egui::ComboBox::from_id_salt("probe keep")
                            .selected_text(KEEP[self.keep])
                            .show_ui(ui, |ui| {
                                for (k, l) in KEEP.iter().enumerate() {
                                    ui.selectable_value(&mut self.keep, k, *l);
                                }
                            });
                        ui.end_row();
                        ui.label("File");
                        ui.add(egui::TextEdit::singleline(&mut self.to).desired_width(120.0))
                            .on_hover_text(
                                "A CSV of its signals written beside the model by Runs › Save \
                                 probe data; grids and point sets go to .npy files beside it. \
                                 Empty: memory only.",
                            );
                        ui.weak(".csv");
                        ui.end_row();
                        ui.label("");
                        ui.add_enabled(
                            !self.to.trim().is_empty(),
                            egui::Checkbox::new(&mut self.stream, "write as it records"),
                        )
                        .on_hover_text(
                            "Append every sample to the file as the run goes, so it holds the \
                             whole run whatever Keep keeps in memory. Written beside the model \
                             once it is saved; a rewind cuts the file back too.",
                        );
                        ui.end_row();
                    });
                if let Some(p) = &self.problem {
                    ui.colored_label(ui.visuals().error_fg_color, p);
                }
                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        match self.entry() {
                            Ok(e) => done = Some(Done::Apply(Box::new(e))),
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
    fn typed_settings_become_the_probe() {
        let entry = ProbeEntry {
            name: "p".into(),
            record: vec!["a.x".into()],
            every: None,
            keep: None,
            to: None,
            stream: false,
            canvas: None,
        };
        let mut ed = ProbeEditor::new(&entry);
        assert_eq!(ed.entry().unwrap(), entry);
        ed.every = 2;
        ed.every_text = "0, 1.5 3".into();
        ed.keep = 2;
        ed.keep_text = "10".into();
        ed.to = "out".into();
        let e = ed.entry().unwrap();
        assert_eq!(e.every.unwrap().times.unwrap(), [0.0, 1.5, 3.0]);
        assert_eq!(e.keep.unwrap().last, Some(10));
        assert_eq!(e.to.as_deref(), Some("out.csv"));
        let again = ProbeEditor::new(&ed.entry().unwrap()).entry().unwrap();
        assert_eq!(again, ed.entry().unwrap());
        ed.stream = true;
        assert!(ed.entry().unwrap().stream);
        // Thinned past samples: the default 100 000 is left out.
        ed.keep = 0;
        ed.keep_text = "100000".into();
        assert_eq!(ed.entry().unwrap().keep, None);
        ed.keep_text = "5000".into();
        assert_eq!(ed.entry().unwrap().keep.unwrap().samples, Some(5000));
        ed.keep = 2;
        ed.keep_text = "0".into();
        assert!(ed.entry().is_err());
        ed.keep_text = "ten".into();
        assert!(ed.entry().unwrap_err().contains("not a whole number"));
    }
}
