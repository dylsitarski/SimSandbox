//! The window: open a model, run, pause, rewind, and watch it plot live.

use crate::examples::{registry, EXAMPLES};
use crate::pacing::Pacer;
use crate::session::Session;
use eframe::egui;
use egui_plot::{Legend, Line, Plot};
use simsandbox_core::registry::Registry;
use std::path::Path;
use std::time::Instant;

/// A screenshot request for documentation and visual checks
/// (`--screenshot FILE --after SECONDS`).
pub struct ScreenshotRequest {
    pub path: std::path::PathBuf,
    pub after: f64,
}

pub struct SimApp {
    registry: Registry,
    session: Option<Session>,
    pacer: Pacer,
    running: bool,
    /// Requested speed; the pacer runs at it while `running`.
    speed: f64,
    error: Option<String>,
    path_input: String,
    last_frame: Option<Instant>,
    rewind_t: f64,
    started: Instant,
    screenshot: Option<ScreenshotRequest>,
    screenshot_requested: bool,
}

impl SimApp {
    pub fn new(path: Option<String>, screenshot: Option<ScreenshotRequest>) -> Self {
        let mut app = Self {
            registry: registry(),
            session: None,
            pacer: Pacer::new(1.0),
            running: false,
            speed: 1.0,
            error: None,
            path_input: String::new(),
            last_frame: None,
            rewind_t: 0.0,
            started: Instant::now(),
            screenshot,
            screenshot_requested: false,
        };
        match path {
            Some(p) => app.open_path(&p),
            None => app.open_text(EXAMPLES[0].name, EXAMPLES[0].text),
        }
        // Hobbyists first (ADR 0009): a model runs as soon as it opens.
        app.running = app.session.is_some();
        app
    }

    fn open_path(&mut self, path: &str) {
        self.path_input = path.into();
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let title = Path::new(path)
                    .file_name()
                    .map_or(path.into(), |n| n.to_string_lossy().into_owned());
                self.open_text(&title, &text);
            }
            Err(e) => self.error = Some(format!("{path}: {e}")),
        }
    }

    fn open_text(&mut self, title: &str, text: &str) {
        match Session::load(title, text, &self.registry) {
            Ok(s) => {
                self.speed = s.file.controls.speed.unwrap_or(1.0);
                self.session = Some(s);
                self.error = None;
                self.rewind_t = 0.0;
                self.pacer.reset();
            }
            Err(e) => self.error = Some(format!("{title}: {e}")),
        }
    }

    /// Advance the simulation for this frame (ADR 0010).
    fn advance(&mut self) {
        let now = Instant::now();
        let elapsed = self
            .last_frame
            .map_or(0.0, |t| now.duration_since(t).as_secs_f64());
        self.last_frame = Some(now);
        let Some(session) = self.session.as_mut() else {
            return;
        };
        self.pacer.speed = if self.running { self.speed } else { 0.0 };
        let dt = session.dt;
        let work_start = Instant::now();
        let result = self
            .pacer
            .frame(elapsed, dt, || work_start.elapsed(), || session.step());
        if let Err(e) = result {
            self.running = false;
            self.error = Some(format!("stopped at t = {:.4}: {e}", session.time()));
        }
        if self.running {
            self.rewind_t = session.time();
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.menu_button("Examples", |ui| {
                for e in EXAMPLES {
                    if ui.button(e.name).clicked() {
                        self.open_text(e.name, e.text);
                        self.running = true;
                        ui.close();
                    }
                }
            });
            ui.separator();
            ui.label("File:");
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.path_input)
                    .hint_text("path to a .ssbx.json file, or drop one on the window")
                    .desired_width(320.0),
            );
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("Open").clicked() || enter {
                let p = self.path_input.clone();
                self.open_path(&p);
            }
        });

        let Some(session) = self.session.as_mut() else {
            return;
        };
        ui.horizontal(|ui| {
            ui.heading(&session.title);
            if let Some(d) = &session.description {
                ui.label(d);
            }
        });
        ui.horizontal(|ui| {
            let label = if self.running { "⏸ Pause" } else { "▶ Run" };
            if ui.button(label).clicked() {
                self.running = !self.running;
                self.pacer.reset();
            }
            if ui.button("⏮ Reset").clicked() {
                session.reset();
                self.rewind_t = 0.0;
                self.pacer.reset();
            }
            ui.separator();
            ui.add(
                egui::Slider::new(&mut self.speed, 0.01..=100.0)
                    .logarithmic(true)
                    .clamping(egui::SliderClamping::Never)
                    .suffix("×")
                    .text("speed"),
            )
            .on_hover_text("Simulated seconds per wall-clock second");
            if self.speed <= 0.0 || !self.speed.is_finite() {
                self.speed = 0.01;
            }
            ui.separator();
            ui.label("step");
            ui.add(
                egui::DragValue::new(&mut session.dt)
                    .range(1e-6..=10.0)
                    .speed(0.001)
                    .suffix(" s"),
            )
            .on_hover_text("Communication step: simulated time per step");
            ui.separator();
            ui.monospace(format!("t = {:.3} s", session.time()));
        });
        ui.horizontal(|ui| {
            if let Some(l) = session.labels() {
                ui.label(format!("Speed class: {:?}", l.speed))
                    .on_hover_text("RealTime keeps a live loop; Responsive takes seconds; Batch runs as a job");
                ui.label(format!("Accuracy: {:?}", l.accuracy)).on_hover_text(
                    "Qualitative: right behaviour, untrusted numbers; Quantitative: numerically controlled; Verified: error estimated for this case",
                );
            }
            if self.running && self.pacer.is_limited() {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!(
                        "running at {:.2}× of the requested {:.2}×",
                        self.pacer.achieved_speed(),
                        self.speed
                    ),
                );
            }
        });
    }

    fn rewind_bar(&mut self, ui: &mut egui::Ui) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let (first, last) = session.history_range();
        ui.horizontal(|ui| {
            ui.label("Rewind");
            ui.spacing_mut().slider_width = (ui.available_width() - 120.0).max(100.0);
            let r = ui.add_enabled(
                last > first,
                egui::Slider::new(&mut self.rewind_t, first..=last.max(first + 1e-9)).suffix(" s"),
            );
            if r.drag_started() || r.changed() {
                self.running = false;
            }
            if r.drag_stopped() || (r.changed() && !r.dragged()) {
                session.rewind_to(self.rewind_t);
                self.rewind_t = session.time();
                self.pacer.reset();
            }
        })
        .response
        .on_hover_text("Pause and go back; running on from here starts a new branch");
    }

    fn messages(&self, ui: &mut egui::Ui) {
        if let Some(e) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        if let Some(s) = &self.session {
            for w in &s.warnings {
                ui.colored_label(ui.visuals().warn_fg_color, format!("warning: {w}"));
            }
        }
    }

    fn plot(&self, ui: &mut egui::Ui) {
        let Some(session) = &self.session else {
            ui.centered_and_justified(|ui| ui.label("Open an example or a model file."));
            return;
        };
        Plot::new("time_series")
            .legend(Legend::default())
            .x_axis_label("t [s]")
            .show(ui, |plot| {
                for s in &session.series {
                    plot.line(Line::new(s.name.clone(), s.points.as_slice()));
                }
            });
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if let Some(f) = dropped.first() {
            let path = f.path().to_string_lossy().into_owned();
            self.open_path(&path);
            self.running = self.session.is_some() && self.error.is_none();
        }
    }

    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        let Some(req) = &self.screenshot else {
            return;
        };
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            if let Err(e) = write_ppm(&req.path, &image) {
                eprintln!("screenshot: {}: {e}", req.path.display());
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if !self.screenshot_requested && self.started.elapsed().as_secs_f64() >= req.after {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            self.screenshot_requested = true;
        }
        ctx.request_repaint();
    }
}

impl eframe::App for SimApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.handle_dropped_files(&ctx);
        self.advance();

        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(4.0);
            self.toolbar(ui);
            ui.add_space(4.0);
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.add_space(4.0);
            self.rewind_bar(ui);
            self.messages(ui);
            ui.add_space(4.0);
        });
        egui::CentralPanel::default().show(ui, |ui| self.plot(ui));

        if self.running {
            ctx.request_repaint();
        }
        self.handle_screenshot(&ctx);
    }
}

/// Write an image as binary PPM (no image dependency needed).
fn write_ppm(path: &Path, image: &egui::ColorImage) -> std::io::Result<()> {
    let [w, h] = image.size;
    let mut bytes = format!("P6\n{w} {h}\n255\n").into_bytes();
    for p in &image.pixels {
        bytes.extend_from_slice(&[p.r(), p.g(), p.b()]);
    }
    std::fs::write(path, bytes)
}
