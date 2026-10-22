//! The window: open and save a model, run, pause, rewind, change parameters,
//! and watch it plot live.

use crate::examples::{registry, EXAMPLES};
use crate::live::{Event, Live, Model};
use crate::log::{Kind as LogKind, Log};
use crate::session::{ParamRow, Session};
use crate::views::Views;
use eframe::egui;
use simsandbox_core::registry::Registry;
use simsandbox_views::{Available, History, ViewState, ViewType};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A step running longer than this is reported in the toolbar.
const SLOW_STEP: Duration = Duration::from_millis(500);

/// A screenshot request for documentation and visual checks
/// (`--screenshot FILE --after SECONDS`).
pub struct ScreenshotRequest {
    pub path: std::path::PathBuf,
    pub after: f64,
}

pub struct SimApp {
    registry: Registry,
    /// The open model, running on its worker thread (ADR 0035).
    session: Option<Live>,
    /// Requested speed, sent to the worker when changed.
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
    /// Values of sliders being dragged that apply on release (ADR 0037).
    pending: HashMap<String, f64>,
    /// The equation editor window, when open.
    editor: Option<Editor>,
    /// The derived-signals window, when open.
    signals_editor: Option<SignalsEditor>,
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
            speed: 1.0,
            error: None,
            log: Log::default(),
            logged_error: None,
            path: None,
            param_edits: HashMap::new(),
            pending: HashMap::new(),
            editor: None,
            signals_editor: None,
            rewind_t: 0.0,
            views: Views::new(crate::examples::view_registry()),
            started: Instant::now(),
            screenshot,
            screenshot_requested: false,
        };
        // Hobbyists first (ADR 0009): a model runs as soon as it opens.
        match path {
            Some(p) => app.open_path(&p),
            None => app.open_text(EXAMPLES[0].name, EXAMPLES[0].text),
        }
        // A --view flag overrides the view saved in the file.
        if let (Some(v), Some(live)) = (view, &app.session) {
            let (name, preset) = match v {
                "phase" => ("plot.xy", 1),
                "3d" => ("plot.3d", 0),
                _ => ("plot.xy", 0),
            };
            let (m, h) = (live.model(), live.history());
            app.views.show_only(name, preset, &m.available(&h));
        }
        // A --window flag opens a window at once (for headless checks).
        match (window, &app.session) {
            (Some("signals"), Some(live)) => {
                app.signals_editor = Some(SignalsEditor::new(&live.model()));
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
        let Some(live) = &self.session else {
            return;
        };
        let name = self.path.as_deref().and_then(Path::file_name).map_or_else(
            || default_file_name(&live.model().title),
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

    /// Open a model and start it running.
    fn open_text(&mut self, title: &str, text: &str) {
        self.open_text_with(title, text, true);
    }

    /// Open a model; it runs at once if `running` (sweeps are evaluated
    /// either way). The previous model's worker stops.
    fn open_text_with(&mut self, title: &str, text: &str, running: bool) {
        match Session::load_deferred(title, text, &self.registry) {
            Ok(mut s) => {
                self.speed = s.file.controls.speed.unwrap_or(1.0);
                let w = {
                    let h = s.signals();
                    self.views.open(&s.available(&h), &s.file.controls.views)
                };
                s.warnings.extend(w);
                for w in &s.warnings {
                    self.log.warn(format!("{title}: {w}"));
                }
                let running = running && s.sweep().is_none();
                self.session = Some(Live::start(s, self.speed, running));
                self.error = None;
                self.param_edits.clear();
                self.pending.clear();
                self.log.info(format!("Opened {title}"));
                self.rewind_t = 0.0;
            }
            Err(e) => self.error = Some(format!("{title}: {e}")),
        }
    }

    /// Take the worker's messages into the log and follow the run's time.
    fn poll(&mut self, ctx: &egui::Context) {
        let Some(live) = &self.session else {
            return;
        };
        live.set_repaint(ctx);
        for e in live.take_events() {
            match e {
                Event::Info(t) => self.log.info(t),
                Event::Error(t) => self.error = Some(t),
            }
        }
        let st = live.status();
        if st.running {
            self.rewind_t = st.time;
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

        let Some(live) = self.session.as_ref() else {
            return;
        };
        let m = live.model();
        let st = live.status();
        ui.horizontal(|ui| {
            ui.heading(&m.title);
            // One line, so a long description cannot widen the window or push
            // the toolbar down; egui shows the whole text on hover when it is cut.
            if let Some(d) = &m.description {
                ui.add(egui::Label::new(d.as_str()).truncate());
            }
        });
        // Models with no states are swept, not run (ADR 0019, ADR 0027,
        // ADR 0028).
        if let Some(sw) = m.sweep.clone() {
            match sweep_controls(ui, live, &m, &sw) {
                Ok(None) => {}
                Ok(Some(notes)) => {
                    for n in notes {
                        self.log.warn(n);
                    }
                    let now = live.model();
                    let now_sweep = now.sweep().expect("still swept");
                    if structure(now_sweep) != structure(&sw) {
                        // Other signals, grids or drawing: the views start over.
                        let h = live.history();
                        self.views.open(&now.available(&h), &[]);
                        self.log.info(if now_sweep.is_grid() {
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
        if m.sweep.is_none() {
            ui.horizontal(|ui| {
                let label = if st.running { "⏸ Pause" } else { "▶ Run" };
                if ui.button(label).clicked() {
                    live.set_running(!st.running);
                }
                if ui
                    .button("⏮ Restart")
                    .on_hover_text("Start again from t = 0 with the current parameter values")
                    .clicked()
                {
                    live.send(None, |s, c| {
                        if let Err(e) = s.restart() {
                            c.error(format!("restart failed: {e}"));
                        }
                        c.reset_pacer();
                    });
                    self.rewind_t = 0.0;
                }
                ui.separator();
                let r = ui
                    .add(
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
                if r.changed() {
                    live.set_speed(self.speed);
                }
                ui.separator();
                ui.label("step");
                let mut dt = m.dt;
                if ui
                    .add(
                        egui::DragValue::new(&mut dt)
                            .range(1e-6..=10.0)
                            .speed(0.001)
                            .suffix(" s"),
                    )
                    .on_hover_text("Communication step: simulated time per step")
                    .changed()
                {
                    live.send(Some("dt"), move |s, _| s.dt = dt);
                }
                ui.separator();
                ui.monospace(format!("t = {:.3} s", st.time));
            });
        }
        ui.horizontal(|ui| {
            if let Some(l) = m.labels {
                ui.label(format!("Speed class: {:?}", l.speed))
                    .on_hover_text("RealTime keeps a live loop; Responsive takes seconds; Batch runs as a job");
                ui.label(format!("Accuracy: {:?}", l.accuracy)).on_hover_text(
                    "Qualitative: right behaviour, untrusted numbers; Quantitative: numerically controlled; Verified: error estimated for this case",
                );
            }
            if st.limited {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!(
                        "running at {:.2}× of the requested {:.2}×",
                        st.achieved, self.speed
                    ),
                );
            }
            if let Some(d) = live.stepping_for().filter(|d| *d >= SLOW_STEP) {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!("a step has been running for {:.1} s", d.as_secs_f64()),
                )
                .on_hover_text("Commands, including Pause, take effect when it ends");
                ui.ctx().request_repaint_after(Duration::from_millis(100));
            }
            if let Some((done, total)) = st.sweep {
                ui.weak(format!("evaluating sample {done} of {total}"));
            }
        });
    }

    fn rewind_bar(&mut self, ui: &mut egui::Ui) {
        let Some(live) = self.session.as_ref() else {
            return;
        };
        if live.model().sweep.is_some() {
            return;
        }
        let (first, last) = live.status().history_range;
        ui.horizontal(|ui| {
            ui.label("Rewind");
            ui.spacing_mut().slider_width = (ui.available_width() - 120.0).max(100.0);
            let r = ui.add_enabled(
                last > first,
                egui::Slider::new(&mut self.rewind_t, first..=last.max(first + 1e-9)).suffix(" s"),
            );
            if r.drag_started() || r.changed() {
                live.set_running(false);
            }
            if r.drag_stopped() || (r.changed() && !r.dragged()) {
                let t = self.rewind_t;
                if let Ok(now) = live.call(move |s, c| {
                    s.rewind_to(t);
                    c.reset_pacer();
                    s.time()
                }) {
                    self.rewind_t = now;
                }
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
        let Some(live) = &self.session else {
            ui.centered_and_justified(|ui| ui.label("Open an example or a model file."));
            return;
        };
        let m = live.model();
        // Read while drawing; the worker keeps stepping and appends later.
        let history = live.history();
        let available = m.available(&history);
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
                self.signals_editor = Some(SignalsEditor::new(&m));
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
                        slot_editors(ui, t, &mut tile.state, &available, &history);
                        tile.state.show(ui, t, &history);
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
        let Some(live) = self.session.as_ref() else {
            return;
        };
        let history = live.history();
        // Only the properties that mean something here (ADR 0025).
        let context = simsandbox_views::Context {
            history: &history,
            bind: &state.bind,
        };
        let shown: Vec<_> = t.style.iter().filter(|p| (p.applies)(&context)).collect();
        ui.add_space(10.0);
        ui.heading(format!("View: {}", t.label));
        // The view's own picks, such as profile cuts (ADR 0029).
        state.edit_selection(ui, t, &history);
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
        let Some(live) = self.session.as_ref() else {
            return;
        };
        let result = self
            .saved_text()
            .and_then(|text| std::fs::write(path, text).map_err(|e| e.to_string()));
        let _ = live;
        match result {
            Ok(()) => {
                self.error = None;
                self.log.info(format!("Saved to {}", path.display()));
                self.path = Some(path.to_path_buf());
            }
            Err(e) => self.error = Some(format!("{}: {e}", path.display())),
        }
    }

    /// The open model as a file, with the current parameters, step, speed
    /// and views.
    fn saved_text(&self) -> Result<String, String> {
        let live = self.session.as_ref().ok_or("no model is open")?;
        let (speed, views) = (self.speed, self.views.entries());
        live.call(move |s, _| s.save_json(speed, Some(views)))?
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
        let Some(live) = self.session.as_ref() else {
            return;
        };
        let m = live.model();
        let mut fusion = None;
        let mut update: Option<(String, simsandbox_agents::Update)> = None;
        if m.file.blocks.len() > 1 {
            fusion = solving_section(ui, &m);
        }
        ui.heading("Parameters");
        let mut changes: Vec<(ParamRow, f64)> = Vec::new();
        let mut controls: Vec<(String, simsandbox_file::ParamControl)> = Vec::new();
        {
            let mut blocks: Vec<(String, Vec<ParamRow>)> = Vec::new();
            for row in m.params.iter().cloned() {
                let name = row.key.split_once('.').map_or("", |(b, _)| b).to_string();
                match blocks.last_mut() {
                    Some((b, list)) if *b == name => list.push(row),
                    _ => blocks.push((name, vec![row])),
                }
            }
            for (block_name, rows) in blocks {
                ui.add_space(6.0);
                let typed = m
                    .file
                    .blocks
                    .iter()
                    .find(|b| b.name == block_name)
                    .filter(|b| b.type_name == simsandbox_eq::registry::EQUATIONS)
                    .map(|b| b.equations.join("\n"));
                let entry = m.file.blocks.iter().find(|b| b.name == block_name);
                let agents = entry.filter(|b| b.type_name.starts_with("agents."));
                ui.horizontal(|ui| {
                    ui.strong(&block_name);
                    if let Some(b) = agents {
                        // The update order is part of the model (ADR 0034).
                        let now = simsandbox_agents::Update::from_setting(b.update.as_deref())
                            .unwrap_or_default();
                        egui::ComboBox::from_id_salt(("update", &block_name))
                            .selected_text(now.name())
                            .show_ui(ui, |ui| {
                                for u in simsandbox_agents::Update::ALL {
                                    if ui
                                        .selectable_label(u == now, u.name())
                                        .on_hover_text(u.description())
                                        .clicked()
                                        && u != now
                                    {
                                        update = Some((block_name.clone(), u));
                                    }
                                }
                            })
                            .response
                            .on_hover_text(
                                "The order agents act in each step; it changes results. \
                                 Changing it restarts the run.",
                            );
                    }
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
                                &m,
                                &row,
                                Edits {
                                    changes: &mut changes,
                                    controls: &mut controls,
                                    text: &mut self.param_edits,
                                    pending: &mut self.pending,
                                    error: &mut self.error,
                                },
                            );
                        }
                    });
            }
        }

        for (key, control) in controls {
            live.send(None, move |s, _| {
                s.file.controls.params.insert(key, control);
            });
        }
        for (row, v) in changes {
            let key = row.key.clone();
            live.send(Some(&format!("param:{key}")), move |s, c| {
                match s.set_param(row.block, &row.name, v) {
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
        if let Some(on) = fusion {
            self.set_fusion(on);
        }
        if let Some((block, u)) = update {
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
        let (Some(ed), Some(live)) = (self.signals_editor.as_mut(), self.session.as_ref()) else {
            return;
        };
        let mut open = true;
        let (mut apply, mut cancel) = (false, false);
        {
            let history = live.history();
            egui::Window::new("Data")
                .open(&mut open)
                .default_width(640.0)
                .min_width(480.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(520.0)
                        .show(ui, |ui| {
                            data_editor(ui, ed, &history);
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
            let entries = ed.entries();
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
                    update: None,
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
                self.editor = None;
                self.log.info("Equations applied");
            }
        }
    }

    /// Turn automatic fusion on or off (ADR 0023). The model is rebuilt from
    /// its saved form, so the run starts again at t = 0.
    fn set_fusion(&mut self, on: bool) {
        self.rebuild(
            |file| {
                file.fusion = if on {
                    simsandbox_file::Fusion::Auto
                } else {
                    simsandbox_file::Fusion::Off
                };
            },
            if on {
                "Connected equation blocks are solved together; the run restarted"
            } else {
                "Each block is solved on its own (co-simulated); the run restarted"
            },
        );
    }

    /// Change the open model's file with `edit` and open it again from t = 0,
    /// keeping its path and whether it runs.
    fn rebuild(&mut self, edit: impl FnOnce(&mut simsandbox_file::ModelFile), note: &str) {
        let Some(live) = self.session.as_ref() else {
            return;
        };
        let title = live.model().title.clone();
        let running = live.status().running;
        let result = self.saved_text().and_then(|text| {
            simsandbox_file::ModelFile::from_json(&text).map_err(|e| e.to_string())
        });
        let mut file = match result {
            Ok((file, _)) => file,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        edit(&mut file);
        let path = self.path.clone();
        self.open_text_with(&title, &file.to_json(), running);
        self.path = path;
        if self.error.is_none() {
            self.log.info(note);
        }
    }

    /// The open model as a file, with the equations of `block` replaced and
    /// parameters that no longer exist dropped.
    fn edited_model(
        &mut self,
        block: &str,
        lines: Vec<String>,
    ) -> Result<(String, String), String> {
        let title = match &self.session {
            Some(live) => live.model().title.clone(),
            None => return Err("no model is open".into()),
        };
        let set = simsandbox_eq::EquationSet::parse(block, &lines).map_err(|e| e.to_string())?;
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
        Ok((title, file.to_json()))
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if let Some(f) = dropped.first() {
            let path = f.path().to_string_lossy().into_owned();
            self.open_path(&path);
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
        self.poll(&ctx);
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

        self.handle_screenshot(&ctx);
    }
}

/// How the model's blocks are solved (ADR 0023): the automatic-fusion switch
/// and the groups solved together. Returns the switch's new value if the
/// user changed it.
fn solving_section(ui: &mut egui::Ui, session: &Model) -> Option<bool> {
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

/// What the parameter panel collects from its rows this frame.
struct Edits<'a> {
    /// New values to apply.
    changes: &'a mut Vec<(ParamRow, f64)>,
    /// Slider choices and ranges to save in `controls` (ADR 0012).
    controls: &'a mut Vec<(String, simsandbox_file::ParamControl)>,
    /// Text being typed, by field.
    text: &'a mut HashMap<String, String>,
    /// Values of sliders being dragged that apply on release (ADR 0037).
    pending: &'a mut HashMap<String, f64>,
    error: &'a mut Option<String>,
}

/// One parameter: name, typed entry, unit, slider toggle; then the
/// slider and its range when enabled.
fn param_row(ui: &mut egui::Ui, m: &Model, row: &ParamRow, edits: Edits<'_>) {
    let mut control = m
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

    // While a slider is dragged on a model that is not real-time, its field
    // shows the value that applies on release (ADR 0037).
    if let Some(p) = edits.pending.get(&row.key) {
        pending_field(ui, *p);
    } else if let Some(v) = number_entry(ui, &row.key, row.value, edits.text, edits.error) {
        edits.changes.push((row.clone(), v));
    }

    ui.horizontal(|ui| {
        ui.label(&row.unit);
        if row.initial_condition {
            ui.weak("↻")
                .on_hover_text("Initial condition: applies on Restart");
        }
        if m.sweep().is_some_and(|s| s.keys().contains(&row.key)) {
            ui.weak("swept")
                .on_hover_text("Swept: its own value is not used while sweeping");
        }
    });
    let before = control.clone();
    ui.checkbox(&mut control.slider, "slider");
    ui.end_row();

    if control.slider {
        let [mut lo, mut hi] = control.range.unwrap_or(row.suggested);
        ui.label("");
        let mut v = edits.pending.get(&row.key).copied().unwrap_or(row.value);
        let r = ui.add(egui::Slider::new(&mut v, lo..=hi).show_value(false));
        if apply_now(m.real_time, r.changed(), r.dragged(), r.drag_stopped()) {
            edits.pending.remove(&row.key);
            edits.changes.push((row.clone(), v));
        } else if r.changed() {
            edits.pending.insert(row.key.clone(), v);
        }
        ui.weak("range");
        ui.horizontal(|ui| {
            let a = ui.add(egui::DragValue::new(&mut lo).speed(0.1));
            ui.weak("to");
            let b = ui.add(egui::DragValue::new(&mut hi).speed(0.1));
            if (a.changed() || b.changed()) && lo < hi {
                control.range = Some([lo, hi]);
            }
        });
        ui.end_row();
    }
    if control != before {
        edits.controls.push((row.key.clone(), control));
    }
}

/// Whether a slider's value applies now (ADR 0037): on every change for a
/// real-time model, otherwise when the drag ends (or on a click or key).
fn apply_now(real_time: bool, changed: bool, dragged: bool, drag_stopped: bool) -> bool {
    if real_time {
        changed
    } else {
        drag_stopped || (changed && !dragged)
    }
}

/// A parameter field showing a slider's pending value: the same size as
/// the typed field, in the warning colour, not editable.
fn pending_field(ui: &mut egui::Ui, value: f64) {
    let mut text = format_value(value);
    ui.add(
        egui::TextEdit::singleline(&mut text)
            .desired_width(90.0)
            .interactive(false)
            .text_color(ui.visuals().warn_fg_color),
    )
    .on_hover_text("Pending: this model is not real-time, so the value applies when you let go");
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
    live: &Live,
    m: &Model,
    sw: &crate::session::Sweep,
) -> Result<Option<Vec<String>>, String> {
    use simsandbox_file::{Design, SweepControl, DEFAULT_SAMPLE_COUNT, DEFAULT_SEED, MAX_SAMPLES};
    let rows = &m.params;
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
                    for r in rows {
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
    live.call(move |s, _| s.set_sampling(sampling))?.map(Some)
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
    fn new(session: &Model) -> Self {
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
fn data_editor(ui: &mut egui::Ui, ed: &mut SignalsEditor, history: &History) {
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
    ui.weak(format!("Plotted signals: {}", history.names().join(", ")));

    let inputs = crate::session::cut_inputs(history);
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
        let g = crate::session::cut_geometry(history, &c.input);
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
        let entry = crate::session::default_cut(history, &format!("cuts{n}"), &inputs[0]);
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
        let live = app.session.as_ref().unwrap();
        assert_eq!(live.model().title, "Untitled");
        assert_eq!(
            live.history().names(),
            ["model.x", "model.v", "model.energy"]
        );
        assert!(app.path.is_none());
    }

    #[test]
    fn fusion_can_be_switched_off_and_on_keeping_parameters() {
        let mut app = SimApp::new(None, None, None, None);
        let e = &EXAMPLES[1];
        assert!(e.name.contains("oscillators"));
        app.open_text(e.name, e.text);
        let live = app.session.as_ref().unwrap();
        let m = live.model();
        assert_eq!(m.groups.len(), 1, "the example is fused by default");
        let id = live.call(|s, _| s.ids["a"]).unwrap();
        live.call(move |s, _| s.set_param(id, "kappa", 0.8))
            .unwrap()
            .unwrap();

        app.set_fusion(false);
        let m = app.session.as_ref().unwrap().model();
        assert!(m.groups.is_empty());
        assert_eq!(m.file.fusion, simsandbox_file::Fusion::Off);
        assert_eq!(
            m.params.iter().find(|r| r.key == "a.kappa").unwrap().value,
            0.8
        );
        assert!(app.error.is_none());

        app.set_fusion(true);
        assert_eq!(app.session.as_ref().unwrap().model().groups.len(), 1);
    }

    #[test]
    fn new_maps_and_formulas_get_the_right_method() {
        let mut app = SimApp::new(None, None, None, None);
        app.editor = editor(None, "x[n+1] = r*x*(1 - x)");
        app.apply_equations();
        assert!(app.editor.is_none());
        let method = |app: &SimApp| {
            app.session.as_ref().unwrap().model().file.blocks[0]
                .method
                .name
                .clone()
        };
        assert_eq!(method(&app), "iterate");

        app.editor = editor(None, "y = a*x^2");
        app.apply_equations();
        assert_eq!(method(&app), "evaluate");
        assert!(app.session.as_ref().unwrap().model().sweep.is_some());

        // Editing a formula block into a continuous one switches the method.
        app.editor = editor(Some("model"), "dy/dt = -a*y");
        app.apply_equations();
        assert_eq!(method(&app), "rk4");
        assert!(app.session.as_ref().unwrap().model().sweep.is_none());
    }

    #[test]
    fn mistakes_keep_the_editor_open_and_the_model_running() {
        let mut app = SimApp::new(None, None, None, None);
        app.editor = editor(None, "dx/dt = -k*");
        app.apply_equations();
        let e = app.editor.as_ref().unwrap().error.as_deref().unwrap();
        assert!(e.contains("line 1, column 12"), "{e}");
        assert_eq!(app.session.as_ref().unwrap().model().title, "Lorenz system");
        assert_eq!(app.error, None);
    }

    #[test]
    fn editing_a_block_keeps_parameter_values_that_still_exist() {
        let mut app = SimApp::new(None, None, None, None);
        let (name, text) = (EXAMPLES[2].name, EXAMPLES[2].text);
        app.open_text(name, text);
        let live = app.session.as_ref().unwrap();
        let id = live.call(|s, _| s.ids["ecosystem"]).unwrap();
        live.call(move |s, _| s.set_param(id, "alpha", 2.0))
            .unwrap()
            .unwrap();

        let edited = "dprey/dt = alpha*prey - beta*prey*predators\n\
                      dpredators/dt = delta*prey*predators - gamma*predators";
        app.editor = editor(Some("ecosystem"), edited);
        app.apply_equations();
        assert!(
            app.editor.is_none(),
            "{:?}",
            app.editor.as_ref().map(|e| &e.error)
        );
        let live = app.session.as_ref().unwrap();
        let m = live.model();
        assert_eq!(m.title, name);
        assert_eq!(
            m.params.iter().find(|r| r.name == "alpha").unwrap().value,
            2.0
        );
        assert_eq!(
            live.history().names(),
            ["ecosystem.prey", "ecosystem.predators"]
        );
        // Both saved views still fit.
        assert_eq!(app.views.tiles.len(), 2);
    }

    #[test]
    fn sliders_apply_while_dragged_only_for_real_time_models() {
        // (changed, dragged, drag stopped)
        let dragging = (true, true, false);
        let released = (false, false, true);
        let clicked = (true, false, false);
        for (real_time, case, expected) in [
            (true, dragging, true),
            (true, released, false),
            (true, clicked, true),
            (false, dragging, false),
            (false, released, true),
            (false, clicked, true),
        ] {
            let (c, d, s) = case;
            assert_eq!(
                apply_now(real_time, c, d, s),
                expected,
                "{real_time} {case:?}"
            );
        }
        // The examples: the Lorenz system is real-time; a sweep is once
        // evaluated quickly.
        let app = SimApp::new(None, None, None, None);
        let live = app.session.as_ref().unwrap();
        assert!(live.model().real_time);
    }

    #[test]
    fn agent_blocks_change_their_update_order() {
        let mut app = SimApp::new(None, None, None, None);
        let e = EXAMPLES
            .iter()
            .find(|e| e.name.contains("grazing"))
            .unwrap();
        app.open_text(e.name, e.text);
        app.rebuild(
            |f| f.blocks[1].update = Some("synchronous".into()),
            "changed",
        );
        assert!(app.error.is_none(), "{:?}", app.error);
        let m = app.session.as_ref().unwrap().model();
        assert_eq!(m.file.blocks[1].update.as_deref(), Some("synchronous"));
        // An unknown order is refused, naming where it is written.
        app.rebuild(|f| f.blocks[1].update = Some("bogus".into()), "changed");
        let err = app.error.clone().unwrap();
        assert!(err.contains("blocks[1].update"), "{err}");
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
