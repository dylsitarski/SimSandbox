//! The window: open and save a model, run, pause, rewind, change parameters,
//! and watch it plot live.

use crate::examples::{registry, EXAMPLES};
use crate::pacing::Pacer;
use crate::session::{ParamRow, Session};
use crate::view3d::{self, Camera};
use eframe::egui;
use egui_plot::{Legend, Line, MarkerShape, Plot, Points};
use simsandbox_core::registry::Registry;
use simsandbox_core::SimError;
use simsandbox_file::{CameraControl, ViewControl};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
    /// Success messages, e.g. "Saved to …".
    notice: Option<String>,
    /// The file the open model was loaded from; Save writes back to it.
    /// `None` for built-in examples, which Save turns into Save as.
    path: Option<PathBuf>,
    /// Text being typed into parameter fields, keyed by `"block.param"`.
    param_edits: HashMap<String, String>,
    last_frame: Option<Instant>,
    rewind_t: f64,
    view: View,
    started: Instant,
    screenshot: Option<ScreenshotRequest>,
    screenshot_requested: bool,
}

impl SimApp {
    pub fn new(
        path: Option<String>,
        view: Option<&str>,
        screenshot: Option<ScreenshotRequest>,
    ) -> Self {
        let mut app = Self {
            registry: registry(),
            session: None,
            pacer: Pacer::new(1.0),
            running: false,
            speed: 1.0,
            error: None,
            notice: None,
            path: None,
            param_edits: HashMap::new(),
            last_frame: None,
            rewind_t: 0.0,
            view: View::default(),
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
        // A --view flag overrides the view saved in the file.
        if let Some(v) = view {
            app.view.kind = match v {
                "phase" => ViewKind::Phase,
                "3d" => ViewKind::Trajectory,
                _ => ViewKind::TimeSeries,
            };
            let n = app.session.as_ref().map_or(0, |s| s.series.len());
            app.view.ensure_enough(n);
        }
        app
    }

    fn open_path(&mut self, path: &str) {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let title = Path::new(path)
                    .file_name()
                    .map_or(path.into(), |n| n.to_string_lossy().into_owned());
                self.open_text(&title, &text);
                if self.error.is_none() {
                    self.path = Some(path.into());
                }
            }
            Err(e) => self.error = Some(format!("{path}: {e}")),
        }
    }

    /// Open a model chosen in the system file dialog.
    fn open_dialog(&mut self) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Open model")
            .add_filter("SimSandbox model", &["json"]);
        if let Some(dir) = self.path.as_deref().and_then(Path::parent) {
            dialog = dialog.set_directory(dir);
        }
        if let Some(path) = dialog.pick_file() {
            self.open_path(&path.to_string_lossy());
            self.running = self.session.is_some() && self.error.is_none();
        }
    }

    /// Save to the file the model came from, or ask where for a built-in
    /// example.
    fn save(&mut self) {
        match self.path.clone() {
            Some(path) => self.save_to(&path),
            None => self.save_as_dialog(),
        }
    }

    /// Save to a file chosen in the system save dialog, which asks before
    /// overwriting.
    fn save_as_dialog(&mut self) {
        let Some(session) = &self.session else {
            return;
        };
        let name = self.path.as_deref().and_then(Path::file_name).map_or_else(
            || default_file_name(&session.title),
            |n| n.to_string_lossy().into_owned(),
        );
        let mut dialog = rfd::FileDialog::new()
            .set_title("Save model")
            .add_filter("SimSandbox model", &["json"])
            .set_file_name(name);
        if let Some(dir) = self.path.as_deref().and_then(Path::parent) {
            dialog = dialog.set_directory(dir);
        }
        if let Some(path) = dialog.save_file() {
            self.save_to(&with_model_extension(path));
        }
    }

    fn open_text(&mut self, title: &str, text: &str) {
        match Session::load(title, text, &self.registry) {
            Ok(s) => {
                self.speed = s.file.controls.speed.unwrap_or(1.0);
                self.session = Some(s);
                self.error = None;
                self.notice = None;
                self.param_edits.clear();
                if let Some(s) = self.session.as_mut() {
                    let names: Vec<&str> = s.series.iter().map(|x| x.name.as_str()).collect();
                    if let Some(w) = self.view.open(&names, s.file.controls.view.as_ref()) {
                        s.warnings.push(w);
                    }
                }
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
        let result = self.pacer.frame(
            elapsed,
            dt,
            || work_start.elapsed(),
            || {
                session.step().map_err(Stop::Failed)?;
                if session.take_end_reached() {
                    return Err(Stop::EndReached);
                }
                Ok(())
            },
        );
        match result {
            Ok(_) => {}
            Err(Stop::EndReached) => {
                self.running = false;
                self.notice = Some(format!(
                    "Paused at the file's end time t = {:.3} s; Run continues",
                    session.time()
                ));
            }
            Err(Stop::Failed(e)) => {
                self.running = false;
                self.error = Some(format!("stopped at t = {:.4}: {e}", session.time()));
            }
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
                        if self.error.is_none() {
                            self.path = None;
                        }
                        self.running = true;
                        ui.close();
                    }
                }
            });
            ui.separator();
            if ui
                .button("Open…")
                .on_hover_text("Open a model file (Ctrl+O); you can also drop one on the window")
                .clicked()
            {
                self.open_dialog();
            }
            let has_model = self.session.is_some();
            if ui
                .add_enabled(has_model, egui::Button::new("Save"))
                .on_hover_text("Save the model with its current parameters, step, sliders, speed and view (Ctrl+S)")
                .clicked()
            {
                self.save();
            }
            if ui
                .add_enabled(has_model, egui::Button::new("Save as…"))
                .on_hover_text("Save to a new file (Ctrl+Shift+S)")
                .clicked()
            {
                self.save_as_dialog();
            }
            ui.separator();
            match &self.path {
                Some(p) => ui.weak(p.display().to_string()),
                None if has_model => ui.weak("built-in example: Save asks where to save it"),
                None => ui.weak(""),
            };
        });
        self.shortcuts(ui);

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
            if ui
                .button("⏮ Restart")
                .on_hover_text("Start again from t = 0 with the current parameter values")
                .clicked()
            {
                if let Err(e) = session.restart() {
                    self.error = Some(format!("restart failed: {e}"));
                }
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
        if let Some(n) = &self.notice {
            ui.label(n);
        }
        if let Some(s) = &self.session {
            for w in &s.warnings {
                ui.colored_label(ui.visuals().warn_fg_color, format!("warning: {w}"));
            }
        }
    }

    fn plot(&mut self, ui: &mut egui::Ui) {
        let Some(session) = &self.session else {
            ui.centered_and_justified(|ui| ui.label("Open an example or a model file."));
            return;
        };
        let names: Vec<&str> = session.series.iter().map(|s| s.name.as_str()).collect();
        let view = &mut self.view;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut view.kind, ViewKind::TimeSeries, "Time series");
            ui.add_enabled_ui(names.len() >= 2, |ui| {
                ui.selectable_value(&mut view.kind, ViewKind::Phase, "Phase portrait")
                    .on_hover_text("One output against another");
            });
            ui.add_enabled_ui(names.len() >= 3, |ui| {
                ui.selectable_value(&mut view.kind, ViewKind::Trajectory, "3D trajectory")
                    .on_hover_text("Three outputs as a path in space");
            });
            ui.separator();
            match view.kind {
                ViewKind::TimeSeries => {}
                ViewKind::Phase => {
                    for (k, axis) in ["x", "y"].into_iter().enumerate() {
                        axis_combo(ui, axis, &names, &mut view.phase[k]);
                    }
                }
                ViewKind::Trajectory => {
                    for (k, axis) in ["x", "y", "z"].into_iter().enumerate() {
                        axis_combo(ui, axis, &names, &mut view.trajectory[k]);
                    }
                }
            }
        });

        match view.kind {
            ViewKind::TimeSeries => {
                Plot::new("time_series")
                    .legend(Legend::default())
                    .x_axis_label("t [s]")
                    .allow_scroll(false)
                    .show(ui, |plot| {
                        scroll_to_zoom(plot);
                        for s in &session.series {
                            plot.line(Line::new(s.name.clone(), s.points.as_slice()));
                        }
                    });
            }
            ViewKind::Phase => {
                let [a, b] = view.phase;
                let points = session.phase_points(a, b);
                let head = points.last().copied();
                Plot::new("phase")
                    .x_axis_label(names[a])
                    .y_axis_label(names[b])
                    .allow_scroll(false)
                    .show(ui, |plot| {
                        scroll_to_zoom(plot);
                        plot.line(Line::new(format!("{} vs {}", names[b], names[a]), points));
                        if let Some(h) = head {
                            plot.points(
                                Points::new("now", vec![h])
                                    .shape(MarkerShape::Circle)
                                    .radius(4.0),
                            );
                        }
                    });
            }
            ViewKind::Trajectory => {
                let axes = view.trajectory;
                let points = session.trajectory(axes);
                view3d::show(ui, &mut view.camera, &points, axes.map(|i| names[i]));
            }
        }
    }

    fn save_to(&mut self, path: &Path) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let names: Vec<String> = session.series.iter().map(|x| x.name.clone()).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let result = session
            .save_json(self.speed, Some(self.view.to_control(&names)))
            .and_then(|text| std::fs::write(path, text).map_err(|e| e.to_string()));
        match result {
            Ok(()) => {
                self.error = None;
                self.notice = Some(format!("Saved to {}", path.display()));
                self.path = Some(path.to_path_buf());
            }
            Err(e) => self.error = Some(format!("{}: {e}", path.display())),
        }
    }

    fn shortcuts(&mut self, ui: &mut egui::Ui) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        let save_as = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::S);
        let save = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
        let open = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
        // The more specific shortcut first, so Ctrl+Shift+S is not taken as Ctrl+S.
        if ui.input_mut(|i| i.consume_shortcut(&save_as)) && self.session.is_some() {
            self.save_as_dialog();
        } else if ui.input_mut(|i| i.consume_shortcut(&save)) && self.session.is_some() {
            self.save();
        } else if ui.input_mut(|i| i.consume_shortcut(&open)) {
            self.open_dialog();
        }
    }

    /// Typed entry for every parameter, plus opt-in sliders (ADR 0011) whose
    /// choices are saved in `controls` (ADR 0012).
    fn param_panel(&mut self, ui: &mut egui::Ui) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        ui.heading("Parameters");
        let rows = session.params();
        let mut changes: Vec<(ParamRow, f64)> = Vec::new();
        egui::ScrollArea::vertical().show(ui, |ui| {
            let mut blocks: Vec<(String, Vec<ParamRow>)> = Vec::new();
            for row in rows {
                let name = row.key.split_once('.').map_or("", |(b, _)| b).to_string();
                match blocks.last_mut() {
                    Some((b, list)) if *b == name => list.push(row),
                    _ => blocks.push((name, vec![row])),
                }
            }
            for (block_name, rows) in blocks {
                ui.add_space(6.0);
                ui.strong(&block_name);
                egui::Grid::new(("params", &block_name))
                    .num_columns(4)
                    .spacing([6.0, 4.0])
                    .show(ui, |ui| {
                        for row in rows {
                            param_row(
                                ui,
                                session,
                                &row,
                                &mut changes,
                                &mut self.param_edits,
                                &mut self.error,
                            );
                        }
                    });
            }
        });

        for (row, v) in changes {
            match session.set_param(row.block, &row.name, v) {
                Ok(()) => {
                    self.error = None;
                    // A typed value outside the slider range widens it (ADR 0011).
                    if let Some(c) = session.file.controls.params.get_mut(&row.key) {
                        if c.slider {
                            let [lo, hi] = c.range.unwrap_or(row.suggested);
                            if v < lo || v > hi {
                                c.range = Some([lo.min(v), hi.max(v)]);
                            }
                        }
                    }
                    if row.initial_condition && session.time() > 0.0 {
                        self.notice =
                            Some(format!("{} changed: press Restart to apply it", row.key));
                    }
                }
                Err(e) => self.error = Some(format!("{}: {e}", row.key)),
            }
        }
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
        if self.session.is_some() {
            egui::Panel::left("params")
                .resizable(true)
                .default_size(300.0)
                .show(ui, |ui| self.param_panel(ui));
        }
        egui::CentralPanel::default().show(ui, |ui| self.plot(ui));

        if self.running {
            ctx.request_repaint();
        }
        self.handle_screenshot(&ctx);
    }
}

/// One parameter: name, typed entry, unit, slider toggle; then the
/// slider and its range when enabled.
fn param_row(
    ui: &mut egui::Ui,
    session: &mut Session,
    row: &ParamRow,
    changes: &mut Vec<(ParamRow, f64)>,
    edits: &mut HashMap<String, String>,
    error: &mut Option<String>,
) {
    let control = session
        .file
        .controls
        .params
        .get(&row.key)
        .cloned()
        .unwrap_or_default();
    let mut hover = format!(
        "{}\nValue: {}\nSuggested range: {} to {}",
        row.description, row.value, row.suggested[0], row.suggested[1]
    );
    if row.initial_condition {
        hover.push_str("\nSets the initial state: applies on Restart");
    }
    ui.label(&row.name).on_hover_text(hover);

    // Show a compact value; apply only what the user actually typed, so
    // the rounding of the display never changes a parameter.
    let shown = format_value(row.value);
    let mut text = edits
        .get(&row.key)
        .cloned()
        .unwrap_or_else(|| shown.clone());
    let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(90.0));
    if r.has_focus() {
        edits.insert(row.key.clone(), text.clone());
    }
    if r.lost_focus() {
        edits.remove(&row.key);
        if text.trim() != shown {
            match text.trim().parse::<f64>() {
                Ok(v) => changes.push((row.clone(), v)),
                Err(_) => {
                    *error = Some(format!("{}: '{}' is not a number", row.key, text.trim()));
                }
            }
        }
    }

    ui.horizontal(|ui| {
        ui.label(&row.unit);
        if row.initial_condition {
            ui.weak("↻")
                .on_hover_text("Initial condition: applies on Restart");
        }
    });
    let mut slider = control.slider;
    ui.checkbox(&mut slider, "slider");
    ui.end_row();
    if slider != control.slider {
        session
            .file
            .controls
            .params
            .entry(row.key.clone())
            .or_default()
            .slider = slider;
    }

    if slider {
        let entry = session
            .file
            .controls
            .params
            .entry(row.key.clone())
            .or_default();
        let [mut lo, mut hi] = entry.range.unwrap_or(row.suggested);
        ui.label("");
        let mut v = row.value;
        if ui
            .add(egui::Slider::new(&mut v, lo..=hi).show_value(false))
            .changed()
        {
            changes.push((row.clone(), v));
        }
        ui.weak("range");
        ui.horizontal(|ui| {
            let a = ui.add(egui::DragValue::new(&mut lo).speed(0.1));
            ui.weak("to");
            let b = ui.add(egui::DragValue::new(&mut hi).speed(0.1));
            if (a.changed() || b.changed()) && lo < hi {
                entry.range = Some([lo, hi]);
            }
        });
        ui.end_row();
    }
}

/// A file name for saving a model titled `title`, e.g. "Lorenz system" →
/// "lorenz-system.ssbx.json".
fn default_file_name(title: &str) -> String {
    if title.ends_with(".json") {
        return title.into();
    }
    let stem: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    format!(
        "{}{}",
        if stem.is_empty() { "model" } else { &stem },
        simsandbox_file::EXTENSION
    )
}

/// Add the model extension when a chosen name has no `.json` extension.
fn with_model_extension(path: PathBuf) -> PathBuf {
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("json"))
    {
        path
    } else {
        let mut s = path.into_os_string();
        s.push(simsandbox_file::EXTENSION);
        s.into()
    }
}

/// What the central area shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewKind {
    TimeSeries,
    Phase,
    Trajectory,
}

struct View {
    kind: ViewKind,
    /// Series indices for the phase portrait's x and y.
    phase: [usize; 2],
    /// Series indices for the 3D trajectory's axes.
    trajectory: [usize; 3],
    camera: Camera,
}

impl Default for View {
    fn default() -> Self {
        Self {
            kind: ViewKind::TimeSeries,
            phase: [0, 1],
            trajectory: [0, 1, 2],
            camera: Camera::default(),
        }
    }
}

impl View {
    /// Open a model whose plotted series are `names`: the view saved in its
    /// `controls` (ADR 0014), else the time series with sensible axes ready
    /// (x–z for Lorenz). Returns a warning if the saved view cannot be used.
    fn open(&mut self, names: &[&str], saved: Option<&ViewControl>) -> Option<String> {
        let n = names.len();
        self.kind = ViewKind::TimeSeries;
        self.phase = if n >= 3 { [0, 2] } else { [0, 1] };
        self.trajectory = [0, 1, 2];
        self.camera = Camera::default();
        let saved = saved?;
        let Some(idx) = saved
            .axes
            .iter()
            .map(|a| names.iter().position(|n| n == a))
            .collect::<Option<Vec<usize>>>()
        else {
            return Some(
                "controls.view: an axis is not among the plotted outputs; opening the time series"
                    .into(),
            );
        };
        match saved.kind.as_str() {
            "phase" => {
                self.kind = ViewKind::Phase;
                self.phase = [idx[0], idx[1]];
            }
            "3d" => {
                self.kind = ViewKind::Trajectory;
                self.trajectory = [idx[0], idx[1], idx[2]];
                if let Some(c) = saved.camera {
                    self.camera = Camera {
                        yaw: c.yaw as f32,
                        pitch: c.pitch as f32,
                        zoom: c.zoom as f32,
                    };
                }
            }
            _ => {}
        }
        None
    }

    /// The current view as saved in `controls` (ADR 0014).
    fn to_control(&self, names: &[&str]) -> ViewControl {
        let axes = |idx: &[usize]| idx.iter().map(|&i| names[i].to_string()).collect();
        match self.kind {
            ViewKind::TimeSeries => ViewControl {
                kind: "time".into(),
                axes: Vec::new(),
                camera: None,
            },
            ViewKind::Phase => ViewControl {
                kind: "phase".into(),
                axes: axes(&self.phase),
                camera: None,
            },
            ViewKind::Trajectory => ViewControl {
                kind: "3d".into(),
                axes: axes(&self.trajectory),
                camera: Some(CameraControl {
                    yaw: self.camera.yaw.into(),
                    pitch: self.camera.pitch.into(),
                    zoom: self.camera.zoom.into(),
                }),
            },
        }
    }

    /// Fall back to the time series if there are too few series for the
    /// chosen view.
    fn ensure_enough(&mut self, n: usize) {
        let enough = match self.kind {
            ViewKind::TimeSeries => true,
            ViewKind::Phase => n >= 2,
            ViewKind::Trajectory => n >= 3,
        };
        if !enough {
            self.kind = ViewKind::TimeSeries;
        }
    }
}

/// The scroll wheel zooms around the pointer instead of panning; drag pans
/// and double-click resets, as egui_plot does by default.
fn scroll_to_zoom(plot: &mut egui_plot::PlotUi<'_>) {
    if !plot.response().hovered() {
        return;
    }
    let dy = plot.ctx().input(|i| i.smooth_scroll_delta.y);
    if dy != 0.0 {
        plot.zoom_bounds_around_hovered(egui::Vec2::splat(zoom_factor(dy)));
    }
}

/// Zoom per scrolled point; scrolling up (positive) zooms in.
fn zoom_factor(scroll: f32) -> f32 {
    (scroll * 0.002).exp()
}

fn axis_combo(ui: &mut egui::Ui, axis: &str, names: &[&str], selected: &mut usize) {
    egui::ComboBox::from_id_salt(("axis", axis))
        .selected_text(format!("{axis}: {}", names[*selected]))
        .show_ui(ui, |ui| {
            for (i, name) in names.iter().enumerate() {
                ui.selectable_value(selected, i, *name);
            }
        });
}

/// Why stepping stopped within a frame.
enum Stop {
    /// The file's `run.t_end` was reached (ADR 0013).
    EndReached,
    Failed(SimError),
}

/// A compact display of a value: up to 6 significant digits.
fn format_value(v: f64) -> String {
    if v == 0.0 {
        "0".into()
    } else if (1e-4..1e6).contains(&v.abs()) {
        let digits = (5 - v.abs().log10().floor() as i32).clamp(0, 9) as usize;
        let s = format!("{v:.digits$}");
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s
        }
    } else {
        format!("{v:e}")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn views_open_as_saved_and_save_as_shown() {
        let names = ["l.xyz[0]", "l.xyz[1]", "l.xyz[2]"];
        let mut v = View::default();
        assert!(v.open(&names, None).is_none());
        assert_eq!(v.kind, ViewKind::TimeSeries);
        assert_eq!(v.phase, [0, 2]);

        let saved = ViewControl {
            kind: "3d".into(),
            axes: vec!["l.xyz[2]".into(), "l.xyz[0]".into(), "l.xyz[1]".into()],
            camera: Some(CameraControl {
                yaw: 1.0,
                pitch: 0.5,
                zoom: 2.0,
            }),
        };
        assert!(v.open(&names, Some(&saved)).is_none());
        assert_eq!(v.kind, ViewKind::Trajectory);
        assert_eq!(v.trajectory, [2, 0, 1]);
        assert_eq!(v.to_control(&names), saved);

        let unplotted = ViewControl {
            kind: "phase".into(),
            axes: vec!["l.xyz[0]".into(), "other.x".into()],
            camera: None,
        };
        assert!(v.open(&names, Some(&unplotted)).is_some());
        assert_eq!(v.kind, ViewKind::TimeSeries);
    }

    #[test]
    fn save_names_get_the_model_extension() {
        assert_eq!(
            default_file_name("Lorenz system"),
            "lorenz-system.ssbx.json"
        );
        assert_eq!(
            default_file_name("Coupled oscillators"),
            "coupled-oscillators.ssbx.json"
        );
        assert_eq!(default_file_name("my.ssbx.json"), "my.ssbx.json");
        assert_eq!(default_file_name("!!!"), "model.ssbx.json");
        assert_eq!(
            with_model_extension("a/b".into()),
            PathBuf::from("a/b.ssbx.json")
        );
        assert_eq!(
            with_model_extension("a/b.json".into()),
            PathBuf::from("a/b.json")
        );
        assert_eq!(
            with_model_extension("a/b.JSON".into()),
            PathBuf::from("a/b.JSON")
        );
    }

    #[test]
    fn saving_an_example_then_reopening_it_keeps_the_path() {
        let dir = std::env::temp_dir().join(format!("simsandbox-app-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("lorenz.ssbx.json");

        let mut app = SimApp::new(None, None, None);
        assert!(app.path.is_none(), "built-in examples have no path");
        app.save_to(&path);
        assert_eq!(app.error, None);
        assert_eq!(app.path.as_deref(), Some(path.as_path()));

        let mut reopened = SimApp::new(Some(path.to_string_lossy().into_owned()), None, None);
        assert_eq!(reopened.error, None);
        assert_eq!(reopened.path.as_deref(), Some(path.as_path()));
        assert_eq!(reopened.view.kind, ViewKind::Trajectory);

        // A failed open keeps the previous file as the save target.
        reopened.open_path(&dir.join("missing.ssbx.json").to_string_lossy());
        assert!(reopened.error.is_some());
        assert_eq!(reopened.path.as_deref(), Some(path.as_path()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn scrolling_up_zooms_in() {
        assert!(zoom_factor(50.0) > 1.0);
        assert!(zoom_factor(-50.0) < 1.0);
        assert_eq!(zoom_factor(0.0), 1.0);
    }

    #[test]
    fn values_display_compactly() {
        assert_eq!(format_value(28.0), "28");
        assert_eq!(format_value(8.0 / 3.0), "2.66667");
        assert_eq!(format_value(-0.5), "-0.5");
        assert_eq!(format_value(0.0), "0");
        assert_eq!(format_value(123456.0), "123456");
        assert_eq!(format_value(1e-7), "1e-7");
        assert_eq!(format_value(2.5e9), "2.5e9");
    }
}
