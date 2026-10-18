//! The window: open and save a model, run, pause, rewind, change parameters,
//! and watch it plot live.

use crate::examples::{registry, EXAMPLES};
use crate::log::{Kind as LogKind, Log};
use crate::pacing::Pacer;
use crate::session::{ParamRow, Session};
use crate::views::Views;
use eframe::egui;
use simsandbox_core::registry::Registry;
use simsandbox_core::SimError;
use simsandbox_views::{Available, ViewState, ViewType};
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
    /// Notices, warnings and errors, shown in the status tile.
    log: Log,
    /// The error last written to the log, so each is logged once.
    logged_error: Option<String>,
    /// The file the open model was loaded from; Save writes back to it.
    /// `None` for built-in examples, which Save turns into Save as.
    path: Option<PathBuf>,
    /// Text being typed into parameter fields, keyed by `"block.param"`.
    param_edits: HashMap<String, String>,
    /// The equation editor window, when open.
    editor: Option<Editor>,
    /// The derived-signals window, when open.
    signals_editor: Option<SignalsEditor>,
    last_frame: Option<Instant>,
    rewind_t: f64,
    views: Views,
    started: Instant,
    screenshot: Option<ScreenshotRequest>,
    screenshot_requested: bool,
}

impl SimApp {
    pub fn new(
        path: Option<String>,
        view: Option<&str>,
        window: Option<&str>,
        screenshot: Option<ScreenshotRequest>,
    ) -> Self {
        let mut app = Self {
            registry: registry(),
            session: None,
            pacer: Pacer::new(1.0),
            running: false,
            speed: 1.0,
            error: None,
            log: Log::default(),
            logged_error: None,
            path: None,
            param_edits: HashMap::new(),
            editor: None,
            signals_editor: None,
            last_frame: None,
            rewind_t: 0.0,
            views: Views::new(crate::examples::view_registry()),
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
        if let (Some(v), Some(session)) = (view, &app.session) {
            let (name, preset) = match v {
                "phase" => ("plot.xy", 1),
                "3d" => ("plot.3d", 0),
                _ => ("plot.xy", 0),
            };
            app.views.show_only(name, preset, &session.available());
        }
        // A --window flag opens a window at once (for headless checks).
        match (window, &app.session) {
            (Some("signals"), Some(session)) => {
                app.signals_editor = Some(SignalsEditor::new(session));
            }
            (Some("equations"), _) => {
                app.editor = Some(Editor {
                    block: None,
                    text: NEW_MODEL.into(),
                    error: None,
                });
            }
            _ => {}
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
                self.param_edits.clear();
                self.log.info(format!("Opened {title}"));
                if let Some(s) = self.session.as_mut() {
                    let w = self.views.open(&s.available(), &s.file.controls.views);
                    s.warnings.extend(w);
                    for w in &s.warnings {
                        self.log.warn(format!("{title}: {w}"));
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
        if session.sweep().is_some() {
            return;
        }
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
                self.log.info(format!(
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
            if ui
                .button("New model…")
                .on_hover_text("Type equations and run them")
                .clicked()
            {
                self.editor = Some(Editor {
                    block: None,
                    text: NEW_MODEL.into(),
                    error: None,
                });
            }
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
            // One line, so a long description cannot widen the window or push
            // the toolbar down; egui shows the whole text on hover when it is cut.
            if let Some(d) = &session.description {
                ui.add(egui::Label::new(d.as_str()).truncate());
            }
        });
        // Models with no states are swept, not run (ADR 0019, ADR 0027,
        // ADR 0028).
        if let Some(sw) = session.sweep().cloned() {
            self.running = false;
            match sweep_controls(ui, session, &sw) {
                Ok(None) => {}
                Ok(Some(notes)) => {
                    for n in notes {
                        self.log.warn(n);
                    }
                    let now = session.sweep().expect("still swept");
                    if structure(now) != structure(&sw) {
                        // Other signals, grids or drawing: the views start over.
                        self.views.open(&session.available(), &[]);
                        self.log.info(if now.is_grid() {
                            "Swept on a grid over two parameters: every signal is also a \
                             grid; views reset"
                        } else {
                            "Sweep changed; views reset"
                        });
                    }
                }
                Err(e) => self.error = Some(format!("sweep: {e}")),
            }
        }
        if session.sweep().is_none() {
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
        }
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
        if session.sweep().is_some() {
            return;
        }
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

    /// The log tile: the newest message on one line (bright while new or
    /// while an error stands, dim after), and a button opening the whole
    /// log. Its size never changes, so messages never move the window.
    fn log_tile(&mut self, ui: &mut egui::Ui, width: f32) {
        if self.error != self.logged_error {
            if let Some(e) = &self.error {
                self.log.push(LogKind::Error, e.clone());
            }
            self.logged_error = self.error.clone();
        }
        let height = ui.spacing().interact_size.y;
        let visuals = ui.visuals().clone();
        egui::Frame::group(ui.style())
            .inner_margin(egui::Margin::symmetric(4, 1))
            .show(ui, |ui| {
                ui.set_width(width);
                ui.set_height(height);
                ui.horizontal(|ui| {
                    let unread = self.log.unread();
                    let label = if unread > 0 {
                        format!("Log ({unread})")
                    } else {
                        "Log".to_string()
                    };
                    let log = &mut self.log;
                    ui.menu_button(label, |ui| {
                        log.mark_read();
                        ui.set_max_width(600.0);
                        if log.entries().is_empty() {
                            ui.weak("No messages");
                        }
                        egui::ScrollArea::vertical()
                            .max_height(320.0)
                            .show(ui, |ui| {
                                for e in log.entries().iter().rev() {
                                    let colour = log_colour(&visuals, e.kind, true);
                                    let ago = e.at.elapsed().as_secs();
                                    ui.horizontal_wrapped(|ui| {
                                        ui.weak(format!("{ago} s ago"));
                                        ui.colored_label(colour, &e.text);
                                    });
                                }
                            });
                        if !log.entries().is_empty() && ui.button("Clear").clicked() {
                            log.clear();
                            ui.close();
                        }
                    })
                    .response
                    .on_hover_text("Every notice, warning and error, newest first");
                    if let Some(e) = self.log.latest() {
                        let standing = e.kind == LogKind::Error && self.error.is_some();
                        let colour = log_colour(&visuals, e.kind, standing || e.is_fresh());
                        // egui shows the whole text on hover when it is cut.
                        ui.add(
                            egui::Label::new(egui::RichText::new(&e.text).color(colour)).truncate(),
                        );
                        if e.is_fresh() {
                            // Wake up to dim it even when nothing is running.
                            ui.ctx().request_repaint_after(crate::log::FRESH_FOR);
                        }
                    }
                });
            });
    }

    /// The model's views as tiles, each with its slot pickers, plus a "More
    /// views…" menu for any other view that fits (ADR 0016).
    fn plot(&mut self, ui: &mut egui::Ui) {
        let Some(session) = &self.session else {
            ui.centered_and_justified(|ui| ui.label("Open an example or a model file."));
            return;
        };
        let available = session.available();
        let views = &mut self.views;
        ui.horizontal(|ui| {
            ui.menu_button("More views…", |ui| {
                let presets: Vec<_> = views
                    .registry
                    .types()
                    .iter()
                    // Views for other kinds of blocks are not offered at all.
                    .filter(|t| t.applies(&available))
                    .flat_map(|t| {
                        t.presets
                            .iter()
                            .enumerate()
                            .map(move |(k, p)| (t.name, k, p.label, p.description))
                    })
                    .collect();
                for (name, k, label, description) in presets {
                    let fits = views.fits(name, k, &available);
                    let r = ui
                        .add_enabled(fits, egui::Button::new(label))
                        .on_hover_text(description)
                        .on_disabled_hover_text("This model records nothing this view can show");
                    if r.clicked() {
                        views.add(name, k, &available);
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text("Add another view of this model");
            if ui
                .button("Data…")
                .on_hover_text(
                    "Filters: signals computed from others (such as an energy), and cuts through 2D grids",
                )
                .clicked()
            {
                self.signals_editor = Some(SignalsEditor::new(session));
            }
            ui.weak("Click a view's name to edit its style in the side panel");
        });

        let area = ui.available_rect_before_wrap();
        let (cols, rows) = crate::views::grid(views.tiles.len());
        let mut close = None;
        let mut select = None;
        let mut swap = false;
        for (k, tile) in views.tiles.iter_mut().enumerate() {
            let Some(t) = views.registry.get(&tile.state.type_name) else {
                continue;
            };
            let (c, r) = (k % cols, k / cols);
            let w = area.width() / cols as f32;
            let h = area.height() / rows as f32;
            let cell = egui::Rect::from_min_size(
                area.min + egui::vec2(c as f32 * w, r as f32 * h),
                egui::vec2(w, h),
            )
            .shrink(3.0);
            let selected = views.selected == Some(tile.id);
            let builder = egui::UiBuilder::new()
                .id_salt(("tile", tile.id))
                .max_rect(cell);
            ui.scope_builder(builder, |ui| {
                let stroke = if selected {
                    ui.visuals().selection.stroke
                } else {
                    ui.visuals().widgets.noninteractive.bg_stroke
                };
                egui::Frame::group(ui.style())
                    .stroke(stroke)
                    .show(ui, |ui| {
                        ui.set_min_size(ui.available_size());
                        ui.horizontal(|ui| {
                            if ui
                                .selectable_label(selected, egui::RichText::new(t.label).strong())
                                .on_hover_text(t.description)
                                .clicked()
                            {
                                select = Some(tile.id);
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .small_button("×")
                                        .on_hover_text("Close this view")
                                        .clicked()
                                    {
                                        close = Some(tile.id);
                                    }
                                    if t.swap.is_some()
                                        && ui
                                            .small_button("swap axes")
                                            .on_hover_text(
                                                "Exchange what runs across and what runs up",
                                            )
                                            .clicked()
                                    {
                                        swap = true;
                                    }
                                },
                            );
                        });
                        if std::mem::take(&mut swap) {
                            tile.state.swap(t);
                        }
                        slot_editors(ui, t, &mut tile.state, &available, &session.signals);
                        tile.state.show(ui, t, &session.signals);
                    });
            });
        }
        ui.allocate_rect(area, egui::Sense::hover());
        if let Some(id) = select {
            views.selected = Some(id);
        }
        if let Some(id) = close {
            views.close(id);
        }
        if views.tiles.is_empty() {
            ui.centered_and_justified(|ui| ui.label("No views: add one from \"More views…\"."));
        }
    }

    /// Style properties of the selected view, edited like parameters
    /// (ADR 0015).
    fn style_panel(&mut self, ui: &mut egui::Ui) {
        let tile = self.views.selected.unwrap_or_default();
        let Some((t, state)) = self.views.selected_mut() else {
            return;
        };
        let Some(session) = self.session.as_ref() else {
            return;
        };
        // Only the properties that mean something here (ADR 0025).
        let context = simsandbox_views::Context {
            history: &session.signals,
            bind: &state.bind,
        };
        let shown: Vec<_> = t.style.iter().filter(|p| (p.applies)(&context)).collect();
        ui.add_space(10.0);
        ui.heading(format!("View: {}", t.label));
        // The view's own picks, such as profile cuts (ADR 0029).
        state.edit_selection(ui, t, &session.signals);
        egui::Grid::new(("style", t.name))
            .num_columns(3)
            .spacing([6.0, 4.0])
            .show(ui, |ui| {
                for p in shown {
                    ui.label(p.name).on_hover_text(format!(
                        "{}\nSuggested range: {} to {}",
                        p.description, p.suggested[0], p.suggested[1]
                    ));
                    let key = format!("style.{tile}.{}", p.name);
                    let value = state.style_value(p);
                    if let Some(v) =
                        number_entry(ui, &key, value, &mut self.param_edits, &mut self.error)
                    {
                        state.style.insert(p.name.into(), v);
                    }
                    ui.label(p.unit);
                    ui.end_row();
                }
            });
    }

    fn save_to(&mut self, path: &Path) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let result = session
            .save_json(self.speed, Some(self.views.entries()))
            .and_then(|text| std::fs::write(path, text).map_err(|e| e.to_string()));
        match result {
            Ok(()) => {
                self.error = None;
                self.log.info(format!("Saved to {}", path.display()));
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
        let mut fusion = None;
        if session.file.blocks.len() > 1 {
            fusion = solving_section(ui, session);
        }
        ui.heading("Parameters");
        let rows = session.params();
        let mut changes: Vec<(ParamRow, f64)> = Vec::new();
        {
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
                let typed = session
                    .file
                    .blocks
                    .iter()
                    .find(|b| b.name == block_name)
                    .filter(|b| b.type_name == simsandbox_eq::registry::EQUATIONS)
                    .map(|b| b.equations.join("\n"));
                ui.horizontal(|ui| {
                    ui.strong(&block_name);
                    if let Some(text) = typed {
                        if ui
                            .small_button("Edit equations…")
                            .on_hover_text("Change this block's equations")
                            .clicked()
                        {
                            self.editor = Some(Editor {
                                block: Some(block_name.clone()),
                                text,
                                error: None,
                            });
                        }
                    }
                });
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
        }

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
                        self.log
                            .info(format!("{} changed: press Restart to apply it", row.key));
                    }
                }
                Err(e) => self.error = Some(format!("{}: {e}", row.key)),
            }
        }
        if let Some(on) = fusion {
            self.set_fusion(on);
        }
    }

    /// The equation editor window (ADR 0017).
    fn equation_editor(&mut self, ctx: &egui::Context) {
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
    fn signals_window(&mut self, ctx: &egui::Context) {
        let (Some(ed), Some(session)) = (self.signals_editor.as_mut(), self.session.as_mut())
        else {
            return;
        };
        let mut open = true;
        let (mut apply, mut cancel) = (false, false);
        egui::Window::new("Data")
            .open(&mut open)
            .default_width(640.0)
            .min_width(480.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(520.0)
                    .show(ui, |ui| {
                        data_editor(ui, ed, session);
                    });
                for p in &ed.problems {
                    ui.colored_label(ui.visuals().error_fg_color, p);
                }
                ui.horizontal(|ui| {
                    apply = ui.button("Apply").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        if cancel || !open {
            self.signals_editor = None;
        } else if apply {
            let before: Vec<String> = session.filters().iter().map(|f| f.name.clone()).collect();
            match session.set_filters(ed.entries()) {
                Ok(()) => {
                    // A new cut filter opens a profile of its cuts.
                    let available = session.available();
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
                Err(p) => ed.problems = p,
            }
        }
    }

    /// Run the editor's equations: as a new model, or as the new equations
    /// of a block of the open model, keeping the values of parameters that
    /// still exist.
    fn apply_equations(&mut self) {
        let Some(editor) = self.editor.as_ref() else {
            return;
        };
        let lines: Vec<String> = editor.text.lines().map(str::to_string).collect();
        let block = editor.block.clone();
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
                    equations: lines,
                    params: Default::default(),
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
                }
                self.running = true;
                self.editor = None;
                self.log.info("Equations applied");
            }
        }
    }

    /// Turn automatic fusion on or off (ADR 0023). The model is rebuilt from
    /// its saved form, so the run starts again at t = 0.
    fn set_fusion(&mut self, on: bool) {
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let result = session
            .save_json(self.speed, Some(self.views.entries()))
            .and_then(|text| {
                simsandbox_file::ModelFile::from_json(&text).map_err(|e| e.to_string())
            });
        let mut file = match result {
            Ok((file, _)) => file,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        file.fusion = if on {
            simsandbox_file::Fusion::Auto
        } else {
            simsandbox_file::Fusion::Off
        };
        let title = session.title.clone();
        let path = self.path.clone();
        let running = self.running;
        self.open_text(&title, &file.to_json());
        self.path = path;
        self.running = running;
        self.log.info(if on {
            "Connected equation blocks are solved together; the run restarted"
        } else {
            "Each block is solved on its own (co-simulated); the run restarted"
        });
    }

    /// The open model as a file, with the equations of `block` replaced and
    /// parameters that no longer exist dropped.
    fn edited_model(
        &mut self,
        block: &str,
        lines: Vec<String>,
    ) -> Result<(String, String), String> {
        let Some(session) = self.session.as_mut() else {
            return Err("no model is open".into());
        };
        let set = simsandbox_eq::EquationSet::parse(block, &lines).map_err(|e| e.to_string())?;
        let keep: Vec<String> = simsandbox_eq::OdeSystem::spec(&set)
            .params
            .iter()
            .map(|p| p.name.clone())
            .collect();
        let text = session.save_json(self.speed, Some(self.views.entries()))?;
        let (mut file, _) =
            simsandbox_file::ModelFile::from_json(&text).map_err(|e| e.to_string())?;
        let entry = file
            .blocks
            .iter_mut()
            .find(|b| b.name == block)
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
        // Only formulas are swept (ADR 0027), and only parameters that remain.
        if let Some(sample) = &mut file.sample {
            let gone = |key: &str| {
                key.split_once('.')
                    .is_some_and(|(b, p)| b == block && !keep.iter().any(|k| k == p))
            };
            let removed: Vec<usize> = (0..sample.params.len())
                .filter(|&i| gone(&sample.params[i].param))
                .collect();
            for &i in removed.iter().rev() {
                sample.params.remove(i);
                for row in &mut sample.values {
                    row.remove(i);
                }
            }
            if sample.params.is_empty() || set.kind() != simsandbox_eq::Kind::Formulas {
                file.sample = None;
            }
        }
        Ok((session.title.clone(), file.to_json()))
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
        self.equation_editor(&ctx);
        self.signals_window(&ctx);

        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(4.0);
            self.toolbar(ui);
            ui.add_space(4.0);
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let width = (ui.available_width() * 0.4).clamp(220.0, 480.0);
                self.log_tile(ui, width);
                self.rewind_bar(ui);
            });
            ui.add_space(4.0);
        });
        if self.session.is_some() {
            egui::Panel::left("params")
                .resizable(true)
                .default_size(300.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        self.param_panel(ui);
                        self.style_panel(ui);
                    });
                });
        }
        egui::CentralPanel::default().show(ui, |ui| self.plot(ui));

        if self.running {
            ctx.request_repaint();
        }
        self.handle_screenshot(&ctx);
    }
}

/// How the model's blocks are solved (ADR 0023): the automatic-fusion switch
/// and the groups solved together. Returns the switch's new value if the
/// user changed it.
fn solving_section(ui: &mut egui::Ui, session: &Session) -> Option<bool> {
    ui.heading("Solving");
    let mut auto = session.file.fusion == simsandbox_file::Fusion::Auto;
    let changed = ui
        .checkbox(&mut auto, "Solve connected blocks together")
        .on_hover_text(
            "Connected equation blocks with the same method are solved as one system, \
             so their connections add no coupling error. Off: each block steps on its \
             own and they exchange values once per step (co-simulation). Changing this \
             restarts the run.",
        )
        .changed();
    for g in &session.groups {
        let source = if g.explicit {
            " (group set in the file)"
        } else {
            ""
        };
        ui.weak(format!(
            "{} together · {}{source}",
            g.blocks.join(", "),
            g.method
        ));
    }
    if session.groups.is_empty() {
        ui.weak("Every block steps on its own");
    }
    ui.add_space(6.0);
    changed.then_some(auto)
}

/// A log entry's colour; dimmed once it is no longer new.
fn log_colour(visuals: &egui::Visuals, kind: LogKind, bright: bool) -> egui::Color32 {
    let c = match kind {
        LogKind::Info => visuals.text_color(),
        LogKind::Warning => visuals.warn_fg_color,
        LogKind::Error => visuals.error_fg_color,
    };
    if bright {
        c
    } else {
        c.gamma_multiply(0.55)
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

    if let Some(v) = number_entry(ui, &row.key, row.value, edits, error) {
        changes.push((row.clone(), v));
    }

    ui.horizontal(|ui| {
        ui.label(&row.unit);
        if row.initial_condition {
            ui.weak("↻")
                .on_hover_text("Initial condition: applies on Restart");
        }
        if session.sweep().is_some_and(|s| s.keys().contains(&row.key)) {
            ui.weak("swept")
                .on_hover_text("Swept: its own value is not used while sweeping");
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

/// A typed number field. It shows a compact value and returns a new value
/// only when the user typed a different one, so the rounding of the display
/// never changes anything. `key` identifies the field's edit buffer.
fn number_entry(
    ui: &mut egui::Ui,
    key: &str,
    value: f64,
    edits: &mut HashMap<String, String>,
    error: &mut Option<String>,
) -> Option<f64> {
    let shown = format_value(value);
    let mut text = edits.get(key).cloned().unwrap_or_else(|| shown.clone());
    let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(90.0));
    if r.has_focus() {
        edits.insert(key.into(), text.clone());
    }
    if !r.lost_focus() {
        return None;
    }
    edits.remove(key);
    let typed = text.trim();
    if typed == shown {
        return None;
    }
    match typed.parse::<f64>() {
        Ok(v) => Some(v),
        Err(_) => {
            *error = Some(format!("{key}: '{typed}' is not a number"));
            None
        }
    }
}

/// Width of one slot picker; pickers are laid out in rows of as many as
/// fit the tile.
const SLOT_WIDTH: f32 = 150.0;

/// What decides the views a sweep can show: the swept parameters, whether
/// they form a two-parameter grid, and whether samples are drawn as points.
fn structure(sw: &crate::session::Sweep) -> (Vec<String>, bool, bool) {
    (sw.keys().to_vec(), sw.is_grid(), sw.unordered())
}

/// How a model with no states is swept (ADR 0028): the design, then one
/// row per parameter with its range and, on a grid, its number of values.
/// Returns the notes of an applied change, or why it was refused.
fn sweep_controls(
    ui: &mut egui::Ui,
    session: &mut Session,
    sw: &crate::session::Sweep,
) -> Result<Option<Vec<String>>, String> {
    use simsandbox_file::{Design, SweepControl, DEFAULT_SAMPLE_COUNT, DEFAULT_SEED, MAX_SAMPLES};
    let rows = session.params();
    let mut sampling = sw.sampling.clone();
    let designs = [
        (
            Design::Grid,
            "grid",
            "Every combination of evenly spaced values",
        ),
        (
            Design::Random,
            "random",
            "Independent uniform values from a seeded generator",
        ),
        (
            Design::LatinHypercube,
            "Latin hypercube",
            "One value in each of `count` equal slices of every parameter",
        ),
        (
            Design::Sobol,
            "Sobol",
            "A low-discrepancy sequence: spread evenly at any count",
        ),
        (Design::List, "list", "The points written in the file"),
    ];
    let name = |d: Design| designs.iter().find(|x| x.0 == d).map_or("?", |x| x.1);
    ui.horizontal(|ui| {
        ui.label("Sweep").on_hover_text(
            "This model has no states, so instead of running in time its outputs are \
             evaluated at samples of the parameters below",
        );
        egui::ComboBox::from_id_salt("sweep_design")
            .selected_text(name(sampling.design))
            .show_ui(ui, |ui| {
                for &(d, label, hover) in &designs {
                    // A list is written in the file, not built here.
                    if d == Design::List && sampling.design != Design::List {
                        continue;
                    }
                    ui.selectable_value(&mut sampling.design, d, label)
                        .on_hover_text(hover);
                }
            });
        if !matches!(sampling.design, Design::Grid | Design::List) {
            let mut count = sampling.count.unwrap_or(DEFAULT_SAMPLE_COUNT);
            ui.label("count");
            ui.add(egui::DragValue::new(&mut count).range(1..=MAX_SAMPLES));
            sampling.count = Some(count);
        }
        if matches!(sampling.design, Design::Random | Design::LatinHypercube) {
            let mut seed = sampling.seed.unwrap_or(DEFAULT_SEED);
            ui.label("seed");
            ui.add(egui::DragValue::new(&mut seed));
            sampling.seed = Some(seed);
        }
    });
    let grid = sampling.design == Design::Grid;
    let several = sampling.params.len() > 1;
    let mut remove = None;
    for (k, p) in sampling.params.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            egui::ComboBox::from_id_salt(("sweep_param", k))
                .selected_text(p.param.as_str())
                .show_ui(ui, |ui| {
                    for r in &rows {
                        if ui.selectable_label(p.param == r.key, &r.key).clicked()
                            && p.param != r.key
                        {
                            p.param = r.key.clone();
                            // The slider's or suggested range, as on load.
                            p.range = None;
                        }
                    }
                });
            let mut range = sw.plan.ranges.get(k).copied().unwrap_or([0.0, 1.0]);
            ui.label("from");
            let a = ui.add(egui::DragValue::new(&mut range[0]).speed(0.1));
            ui.label("to");
            let b = ui.add(egui::DragValue::new(&mut range[1]).speed(0.1));
            if a.changed() || b.changed() {
                p.range = Some(range);
            }
            if grid {
                let mut points = sw
                    .plan
                    .grid
                    .as_ref()
                    .and_then(|g| g.get(k).copied())
                    .unwrap_or(2);
                ui.label("points");
                if ui
                    .add(egui::DragValue::new(&mut points).range(2..=100_000))
                    .changed()
                {
                    p.points = Some(points);
                }
            }
            if several
                && ui
                    .small_button("×")
                    .on_hover_text("Stop sweeping this parameter")
                    .clicked()
            {
                remove = Some(k);
            }
        });
    }
    let unswept = rows
        .iter()
        .find(|r| !sampling.params.iter().any(|p| p.param == r.key));
    if let Some(r) = unswept {
        let more = ui
            .horizontal(|ui| {
                ui.add_space(16.0);
                ui.button("+ parameter").on_hover_text(if grid {
                    "Sweep another parameter; two on a grid make every signal a grid too"
                } else {
                    "Sweep another parameter"
                })
            })
            .inner
            .clicked();
        if more && sampling.design != Design::List {
            sampling.params.push(SweepControl {
                param: r.key.clone(),
                range: None,
                points: None,
            });
        }
    }
    if let Some(k) = remove {
        sampling.params.remove(k);
        for row in &mut sampling.values {
            row.remove(k);
        }
    }
    if sampling.params.len() != sw.sampling.params.len() && grid {
        // Start over from points that keep the grid within its limit.
        let n = sampling.params.len();
        let each = (MAX_SAMPLES as f64).powf(1.0 / n as f64).floor() as usize;
        for p in &mut sampling.params {
            p.points = (n > 2).then_some(each.min(100));
        }
    }
    if sampling == sw.sampling {
        return Ok(None);
    }
    session.set_sampling(sampling).map(Some)
}

/// Pickers for a view's slots: a menu of checkboxes for slots that take
/// several signals, a combo box otherwise (with "none" when optional).
/// `t` is shown as what it means: time, or the swept parameter (ADR 0021).
/// Slots that do not apply here are hidden (ADR 0025).
fn slot_editors(
    ui: &mut egui::Ui,
    t: &ViewType,
    state: &mut ViewState,
    available: &Available<'_>,
    history: &simsandbox_views::History,
) {
    let context = simsandbox_views::Context {
        history,
        bind: &state.bind,
    };
    let shown: Vec<_> = t.slots.iter().filter(|s| (s.applies)(&context)).collect();
    let gap = ui.spacing().item_spacing.x;
    let per_row = (((ui.available_width() + gap) / (SLOT_WIDTH + gap)).floor() as usize).max(1);
    for row in shown.chunks(per_row) {
        ui.horizontal(|ui| {
            for slot in row {
                slot_editor(ui, t.name, slot, state, available, history.index());
            }
        });
    }
}

fn slot_editor(
    ui: &mut egui::Ui,
    type_name: &str,
    slot: &simsandbox_views::view::Slot,
    state: &mut ViewState,
    available: &Available<'_>,
    index: &str,
) {
    // An axis lists only what can place the bound field's cells (ADR 0024).
    let choices = match (slot.kind, slot.of) {
        (simsandbox_views::SlotKind::Axis, Some(of)) => state
            .bind
            .get(of)
            .and_then(|v| v.first())
            .map_or_else(Vec::new, |field| available.axis_choices(field)),
        _ => available.choices(slot.kind),
    };
    let current = state.bind.get(slot.name).cloned().unwrap_or_default();
    // "t" is stored, but shown as what it is: time, or the swept parameter.
    let display = |s: &str| {
        if s == "t" {
            index.to_string()
        } else {
            s.to_string()
        }
    };
    if slot.many {
        let label = match current.as_slice() {
            [one] => format!("{}: {one}", slot.name),
            many => format!("{}: {} signals", slot.name, many.len()),
        };
        ui.menu_button(label, |ui| {
            // Lists take recorded signals; time is the index, not a curve.
            let signals: Vec<&str> = choices.iter().copied().filter(|&s| s != "t").collect();
            for &s in &signals {
                let mut on = current.iter().any(|c| c == s);
                // A required slot keeps at least one signal.
                let locked = on && slot.required && current.len() == 1;
                if ui
                    .add_enabled(!locked, egui::Checkbox::new(&mut on, s))
                    .changed()
                {
                    // Keep the recorded order, so colours stay stable.
                    let next: Vec<String> = signals
                        .iter()
                        .filter(|&&x| {
                            if x == s {
                                on
                            } else {
                                current.iter().any(|c| c == x)
                            }
                        })
                        .map(|x| x.to_string())
                        .collect();
                    state.bind.insert(slot.name.into(), next);
                }
            }
        })
        .response
        .on_hover_text(slot.description);
    } else {
        let shown = current.first().map_or("none", String::as_str);
        egui::ComboBox::from_id_salt(("slot", type_name, slot.name))
            .width(SLOT_WIDTH - 20.0)
            .truncate()
            .selected_text(format!("{}: {}", slot.name, display(shown)))
            .show_ui(ui, |ui| {
                if !slot.required && ui.selectable_label(current.is_empty(), "none").clicked() {
                    state.bind.remove(slot.name);
                }
                for &s in &choices {
                    if ui.selectable_label(shown == s, display(s)).clicked() {
                        state.bind.insert(slot.name.into(), vec![s.into()]);
                    }
                }
            })
            .response
            .on_hover_text(slot.description);
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

/// The equation editor: for a new model (`block` is `None`) or for a block
/// of the open model.
/// Rows of the derived-signals window: name and expression.
/// The Data window's working copy of the filters.
struct SignalsEditor {
    exprs: Vec<(String, String)>,
    cuts: Vec<CutEdit>,
    problems: Vec<String>,
}

/// One cut filter being edited.
struct CutEdit {
    name: String,
    input: String,
    settings: simsandbox_views::cut::CutSettings,
}

impl CutEdit {
    fn entry(&self) -> simsandbox_file::FilterEntry {
        let settings = match serde_json::to_value(&self.settings) {
            Ok(serde_json::Value::Object(m)) => m.into_iter().collect(),
            _ => Default::default(),
        };
        simsandbox_file::FilterEntry {
            name: self.name.trim().into(),
            type_name: "cut".into(),
            inputs: vec![self.input.clone()],
            settings,
        }
    }
}

impl SignalsEditor {
    /// The session's filters, ready to edit.
    fn new(session: &Session) -> Self {
        let mut ed = Self {
            exprs: Vec::new(),
            cuts: Vec::new(),
            problems: Vec::new(),
        };
        for f in session.filters() {
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
    fn entries(&self) -> Vec<simsandbox_file::FilterEntry> {
        self.exprs
            .iter()
            .map(|(name, text)| {
                simsandbox_file::FilterEntry::expression(name.trim(), text.trim(), Vec::new())
            })
            .chain(self.cuts.iter().map(CutEdit::entry))
            .collect()
    }
}

/// The Data window's contents: expressions, then cuts.
fn data_editor(ui: &mut egui::Ui, ed: &mut SignalsEditor, session: &Session) {
    use simsandbox_views::cut::{Interpolation, Mode};
    ui.heading("Signals");
    ui.label(
        "Each signal is computed from plotted signals (a.x, lorenz.xyz[0]), other signals \
         here, parameters (a.k) and t, using the same operators and functions as equations.",
    );
    let mut remove = None;
    // Fixed sizes: inside a grid the text boxes collapsed to a few
    // characters.
    let height = ui.spacing().interact_size.y;
    for (k, (name, text)) in ed.exprs.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.add_sized([120.0, height], egui::TextEdit::singleline(name));
            ui.label("=");
            let width = (ui.available_width() - 30.0).max(200.0);
            ui.add_sized(
                [width, height],
                egui::TextEdit::singleline(text).code_editor(),
            );
            if ui.small_button("×").on_hover_text("Remove").clicked() {
                remove = Some(k);
            }
        });
    }
    if let Some(k) = remove {
        ed.exprs.remove(k);
    }
    if ui.button("Add signal").clicked() {
        ed.exprs
            .push((format!("signal{}", ed.exprs.len() + 1), String::new()));
    }
    ui.weak(format!(
        "Plotted signals: {}",
        session.signals.names().join(", ")
    ));

    let inputs = session.cut_inputs();
    ui.add_space(8.0);
    ui.heading("Cuts");
    if inputs.is_empty() {
        ui.weak("Cuts take a 2D grid, such as the results of a two-parameter sweep; this model has none.");
        return;
    }
    ui.label(
        "Each cut filter cuts a 2D grid into lines, one per cut, which profiles and plots can \
         show. Cuts run along x at fixed y values, along y at fixed x values, or along free lines.",
    );
    let mut remove = None;
    for (k, c) in ed.cuts.iter_mut().enumerate() {
        let g = session.cut_geometry(&c.input);
        ui.separator();
        ui.horizontal(|ui| {
            ui.add_sized([100.0, height], egui::TextEdit::singleline(&mut c.name));
            ui.label("cuts");
            egui::ComboBox::from_id_salt(("cut_input", k))
                .selected_text(&c.input)
                .show_ui(ui, |ui| {
                    for i in &inputs {
                        ui.selectable_value(&mut c.input, i.clone(), i);
                    }
                });
            if ui
                .small_button("×")
                .on_hover_text("Remove this filter")
                .clicked()
            {
                remove = Some(k);
            }
        });
        let s = &mut c.settings;
        ui.horizontal(|ui| {
            let mode_name = |m: Mode| match m {
                Mode::AlongX => format!("along {} at fixed {}", g.label(0), g.label(1)),
                Mode::AlongY => format!("along {} at fixed {}", g.label(1), g.label(0)),
                Mode::Free => "along free lines".to_string(),
            };
            let before = s.mode;
            egui::ComboBox::from_id_salt(("cut_mode", k))
                .selected_text(mode_name(s.mode))
                .show_ui(ui, |ui| {
                    for m in [Mode::AlongX, Mode::AlongY, Mode::Free] {
                        ui.selectable_value(&mut s.mode, m, mode_name(m));
                    }
                });
            if s.mode != before {
                // Start the new mode with one cut through the middle.
                let mid = |axis: usize| {
                    let c = &g.centres[axis];
                    c[c.len() / 2]
                };
                let first = |axis: usize| g.centres[axis][0];
                let last = |axis: usize| g.centres[axis][g.centres[axis].len() - 1];
                match s.mode {
                    Mode::AlongX => s.at = vec![mid(1)],
                    Mode::AlongY => s.at = vec![mid(0)],
                    Mode::Free => s.lines = vec![[[first(0), first(1)], [last(0), last(1)]]],
                }
                s.range = None;
            }
            ui.label("between cells");
            ui.selectable_value(&mut s.interpolation, Interpolation::Bilinear, "bilinear")
                .on_hover_text("Interpolate between the four nearest cells");
            ui.selectable_value(&mut s.interpolation, Interpolation::Nearest, "nearest")
                .on_hover_text("Take the nearest cell");
        });
        match s.mode {
            Mode::AlongX | Mode::AlongY => {
                let (free, fixed) = if s.mode == Mode::AlongX {
                    (0, 1)
                } else {
                    (1, 0)
                };
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("at {} =", g.label(fixed)));
                    let mut gone = None;
                    for (i, v) in s.at.iter_mut().enumerate() {
                        ui.add(egui::DragValue::new(v).speed(0.01));
                        if ui.small_button("×").clicked() {
                            gone = Some(i);
                        }
                    }
                    if let Some(i) = gone {
                        s.at.remove(i);
                    }
                    if ui.small_button("+").on_hover_text("Add a cut").clicked() {
                        let c = &g.centres[fixed];
                        s.at.push(c[c.len() / 2]);
                    }
                });
                ui.horizontal(|ui| {
                    if ui
                        .checkbox(&mut s.whole, format!("whole {} range", g.label(free)))
                        .changed()
                        && !s.whole
                        && s.range.is_none()
                    {
                        let c = &g.centres[free];
                        let (a, b) = (c[0], c[c.len() - 1]);
                        s.range = Some([a.min(b), a.max(b)]);
                    }
                    if let (false, Some(r)) = (s.whole, s.range.as_mut()) {
                        ui.label("from");
                        ui.add(egui::DragValue::new(&mut r[0]).speed(0.01));
                        ui.label("to");
                        ui.add(egui::DragValue::new(&mut r[1]).speed(0.01));
                    }
                });
            }
            Mode::Free => {
                let mut gone = None;
                for (i, [a, b]) in s.lines.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("from ({}, {})", g.label(0), g.label(1)));
                        ui.add(egui::DragValue::new(&mut a[0]).speed(0.01));
                        ui.add(egui::DragValue::new(&mut a[1]).speed(0.01));
                        ui.label("to");
                        ui.add(egui::DragValue::new(&mut b[0]).speed(0.01));
                        ui.add(egui::DragValue::new(&mut b[1]).speed(0.01));
                        if ui.small_button("×").clicked() {
                            gone = Some(i);
                        }
                    });
                }
                if let Some(i) = gone {
                    s.lines.remove(i);
                }
                ui.horizontal(|ui| {
                    if ui.small_button("+").on_hover_text("Add a line").clicked() {
                        let last = s.lines.last().copied().unwrap_or([[0.0, 0.0], [1.0, 1.0]]);
                        s.lines.push(last);
                    }
                    ui.checkbox(&mut s.whole, "extend each line to the edges of the grid");
                });
            }
        }
    }
    if let Some(k) = remove {
        ed.cuts.remove(k);
    }
    ui.separator();
    if ui
        .button("Add cut filter")
        .on_hover_text("Cut a 2D grid into lines")
        .clicked()
    {
        let mut n = ed.cuts.len() + 1;
        while ed.cuts.iter().any(|c| c.name == format!("cuts{n}")) {
            n += 1;
        }
        let entry = session.default_cut(&format!("cuts{n}"), &inputs[0]);
        let object = serde_json::Value::Object(entry.settings.into_iter().collect());
        if let Ok(settings) = serde_json::from_value(object) {
            ed.cuts.push(CutEdit {
                name: format!("cuts{n}"),
                input: inputs[0].clone(),
                settings,
            });
        }
    }
}

struct Editor {
    block: Option<String>,
    text: String,
    error: Option<String>,
}

/// What "New model…" starts with: a damped oscillator that moves at once.
const NEW_MODEL: &str = "\
# A spring with friction. Change anything, then press Apply.
dx/dt = v
dv/dt = -k*x - c*v
# a quantity computed from the states, plotted like them
energy = 0.5*v^2 + 0.5*k*x^2
";

const SYNTAX_HELP: &str = "\
One equation per line:
  dx/dt = ...    (or x' = ...)   x is a state that changes over time
  E = ...                         a quantity computed from the states
Every other name is a parameter (default 1) you can change in the side panel.
Each state x also gets an initial value x0 (default 1).
t is time. Operators: + - * / ^ and parentheses; write 2*x, not 2x.
Functions: sin cos tan asin acos atan atan2 sinh cosh tanh exp ln log10
sqrt abs sign floor ceil min max pow. pi is 3.14159…
Lines starting with # are comments.";

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

        let mut app = SimApp::new(None, None, None, None);
        assert!(app.path.is_none(), "built-in examples have no path");
        app.save_to(&path);
        assert_eq!(app.error, None);
        assert_eq!(app.path.as_deref(), Some(path.as_path()));

        let mut reopened = SimApp::new(Some(path.to_string_lossy().into_owned()), None, None, None);
        assert_eq!(reopened.error, None);
        assert_eq!(reopened.path.as_deref(), Some(path.as_path()));
        assert_eq!(reopened.views.tiles[0].state.type_name, "plot.3d");

        // A failed open keeps the previous file as the save target.
        reopened.open_path(&dir.join("missing.ssbx.json").to_string_lossy());
        assert!(reopened.error.is_some());
        assert_eq!(reopened.path.as_deref(), Some(path.as_path()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn editor(block: Option<&str>, text: &str) -> Option<Editor> {
        Some(Editor {
            block: block.map(str::to_string),
            text: text.into(),
            error: None,
        })
    }

    #[test]
    fn new_models_run_from_typed_equations() {
        let mut app = SimApp::new(None, None, None, None);
        app.editor = editor(None, NEW_MODEL);
        app.apply_equations();
        assert!(app.editor.is_none(), "applied editors close");
        let s = app.session.as_ref().unwrap();
        assert_eq!(s.title, "Untitled");
        assert_eq!(s.signals.names(), ["model.x", "model.v", "model.energy"]);
        assert!(app.path.is_none());
    }

    #[test]
    fn fusion_can_be_switched_off_and_on_keeping_parameters() {
        let mut app = SimApp::new(None, None, None, None);
        let e = &EXAMPLES[1];
        assert!(e.name.contains("oscillators"));
        app.open_text(e.name, e.text);
        let s = app.session.as_mut().unwrap();
        assert_eq!(s.groups.len(), 1, "the example is fused by default");
        let id = s.ids["a"];
        s.set_param(id, "kappa", 0.8).unwrap();

        app.set_fusion(false);
        let s = app.session.as_ref().unwrap();
        assert!(s.groups.is_empty());
        assert_eq!(s.file.fusion, simsandbox_file::Fusion::Off);
        assert_eq!(
            s.params()
                .iter()
                .find(|r| r.key == "a.kappa")
                .unwrap()
                .value,
            0.8
        );
        assert!(app.error.is_none());

        app.set_fusion(true);
        assert_eq!(app.session.as_ref().unwrap().groups.len(), 1);
    }

    #[test]
    fn new_maps_and_formulas_get_the_right_method() {
        let mut app = SimApp::new(None, None, None, None);
        app.editor = editor(None, "x[n+1] = r*x*(1 - x)");
        app.apply_equations();
        assert!(app.editor.is_none());
        assert_eq!(
            app.session.as_ref().unwrap().file.blocks[0].method.name,
            "iterate"
        );

        app.editor = editor(None, "y = a*x^2");
        app.apply_equations();
        let s = app.session.as_ref().unwrap();
        assert_eq!(s.file.blocks[0].method.name, "evaluate");
        assert!(s.sweep().is_some());

        // Editing a formula block into a continuous one switches the method.
        app.editor = editor(Some("model"), "dy/dt = -a*y");
        app.apply_equations();
        let s = app.session.as_ref().unwrap();
        assert_eq!(s.file.blocks[0].method.name, "rk4");
        assert!(s.sweep().is_none());
    }

    #[test]
    fn mistakes_keep_the_editor_open_and_the_model_running() {
        let mut app = SimApp::new(None, None, None, None);
        app.editor = editor(None, "dx/dt = -k*");
        app.apply_equations();
        let e = app.editor.as_ref().unwrap().error.as_deref().unwrap();
        assert!(e.contains("line 1, column 12"), "{e}");
        assert_eq!(app.session.as_ref().unwrap().title, "Lorenz system");
        assert_eq!(app.error, None);
    }

    #[test]
    fn editing_a_block_keeps_parameter_values_that_still_exist() {
        let mut app = SimApp::new(None, None, None, None);
        let (name, text) = (EXAMPLES[2].name, EXAMPLES[2].text);
        app.open_text(name, text);
        let s = app.session.as_mut().unwrap();
        let id = s.ids["ecosystem"];
        s.set_param(id, "alpha", 2.0).unwrap();

        let edited = "dprey/dt = alpha*prey - beta*prey*predators\n\
                      dpredators/dt = delta*prey*predators - gamma*predators";
        app.editor = editor(Some("ecosystem"), edited);
        app.apply_equations();
        assert!(
            app.editor.is_none(),
            "{:?}",
            app.editor.as_ref().map(|e| &e.error)
        );
        let s = app.session.as_ref().unwrap();
        assert_eq!(s.title, name);
        assert_eq!(
            s.params().iter().find(|r| r.name == "alpha").unwrap().value,
            2.0
        );
        assert_eq!(s.signals.names(), ["ecosystem.prey", "ecosystem.predators"]);
        // Both saved views still fit.
        assert_eq!(app.views.tiles.len(), 2);
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
