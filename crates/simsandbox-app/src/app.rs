//! The window: open and save a model, run, pause, rewind, change parameters,
//! and watch it plot live.

use crate::diagram::{DiagramTab, Outcome};
use crate::dock::{Dock, Pane};
use crate::live::{Event, Live, Model};
use crate::log::{Kind as LogKind, Log};
use eframe::egui;
use simsandbox_core::registry::Registry;
use simsandbox_session::diagram::{Diagram, Edit, Scope};
use simsandbox_session::examples::{registry, EXAMPLES};
use simsandbox_session::session::{ParamRow, Session};
use simsandbox_session::views::Views;
use simsandbox_views::{Available, History, ViewState, ViewType};
use std::collections::{BTreeMap, HashMap};
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

/// A tab to bring to the front once the dock is drawn again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    Tile(u64),
}

/// Something that replaces the open model, asked about first if it has
/// unsaved changes.
#[derive(Clone, Debug, PartialEq)]
pub enum Discard {
    OpenPath(String),
    OpenDialog,
    Example(usize),
    /// Apply the equation editor's new model.
    NewModel,
    Close,
}

pub struct SimApp {
    /// The "+ parameter" fields: name, value, unit.
    new_param: [String; 3],
    /// The app as shared with popped-out windows (set by `shared`).
    this: std::sync::Weak<std::sync::Mutex<SimApp>>,
    /// What popped-out windows asked for, carried out by the main window.
    popped_actions: Vec<DockAction>,
    /// Values keys are driving (ADR 0058), by input block, while a key is
    /// held or until its release is sent.
    key_values: HashMap<String, f64>,
    /// "Save run…" while open: the run's name and note.
    run_dialog: Option<(String, String)>,
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
    /// The inspector, diagram and views as dockable tabs (ADR 0053).
    dock: Dock,
    /// A tab to bring to the front.
    focus: Option<Focus>,
    /// The inspector's search text.
    inspect_query: String,
    /// What the inspector shows as selected: for each block, the subsystem
    /// it is inside (`None` on the model level) and its name; picked on the
    /// diagram or by clicking a curve.
    picked: Vec<(Option<String>, String)>,
    /// The diagram's selection when last looked at, to tell a new pick.
    last_canvas: Option<simsandbox_session::canvas::Selection>,
    /// List values formulas set too.
    show_computed: bool,
    diagram: DiagramTab,
    started: Instant,
    /// The open model as saved or opened, to tell unsaved changes.
    baseline: Option<String>,
    /// Whether the model differs from `baseline`, as last checked on the
    /// worker (for the toolbar's marker).
    unsaved: std::sync::Arc<std::sync::atomic::AtomicBool>,
    last_unsaved_check: Instant,
    /// Waiting for an answer: save, discard or cancel.
    asking: Option<Discard>,
    /// Set while doing what the person agreed to discard for.
    discard_ok: bool,
    screenshot: Option<ScreenshotRequest>,
    screenshot_requested: bool,
}

impl SimApp {
    /// The app shared between the main window and popped-out ones.
    pub fn shared(self) -> std::sync::Arc<std::sync::Mutex<SimApp>> {
        std::sync::Arc::new_cyclic(|this| {
            let mut app = self;
            app.this = this.clone();
            std::sync::Mutex::new(app)
        })
    }

    pub fn new(
        path: Option<String>,
        view: Option<&str>,
        window: Option<&str>,
        screenshot: Option<ScreenshotRequest>,
    ) -> Self {
        let mut app = Self {
            new_param: Default::default(),
            this: std::sync::Weak::new(),
            popped_actions: Vec::new(),
            key_values: HashMap::new(),
            run_dialog: None,
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
            views: Views::new(simsandbox_session::examples::view_registry()),
            dock: Dock::standard(&[]),
            focus: None,
            inspect_query: String::new(),
            picked: Vec::new(),
            last_canvas: None,
            show_computed: false,
            diagram: DiagramTab::default(),
            started: Instant::now(),
            baseline: None,
            unsaved: Default::default(),
            last_unsaved_check: Instant::now(),
            asking: None,
            discard_ok: false,
            screenshot,
            screenshot_requested: false,
        };
        // Hobbyists first (ADR 0009): a model runs as soon as it opens.
        match path {
            Some(p) => app.open_path(&p),
            None => {
                app.open_text(EXAMPLES[0].name, EXAMPLES[0].text);
                app.mark_saved();
            }
        }
        // A --view flag overrides the view saved in the file.
        if view == Some("diagram") {
            app.dock.focus(|p| *p == Pane::Diagram);
        } else if let (Some(v), Some(live)) = (view, &app.session) {
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
        if self.ask_first(Discard::OpenPath(path.into())) {
            return;
        }
        if path.ends_with(".run.json") {
            self.open_run(Path::new(path));
            return;
        }
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let title = Path::new(path)
                    .file_name()
                    .map_or(path.into(), |n| n.to_string_lossy().into_owned());
                self.open_text(&title, &text);
                if self.error.is_none() {
                    self.path = Some(path.into());
                    self.mark_saved();
                }
            }
            Err(e) => self.error = Some(format!("{path}: {e}")),
        }
    }

    /// Open a built-in example.
    fn open_example(&mut self, k: usize) {
        if self.ask_first(Discard::Example(k)) {
            return;
        }
        let e = &EXAMPLES[k];
        self.open_text(e.name, e.text);
        if self.error.is_none() {
            self.path = None;
            self.mark_saved();
        }
    }

    /// The open model as it is now becomes the saved state.
    fn mark_saved(&mut self) {
        self.baseline = self.saved_text().ok();
        self.unsaved
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }

    /// Whether the open model has changes a save would write: parameters,
    /// edits, sliders, views, speed or step.
    fn has_unsaved(&self) -> bool {
        match (&self.session, &self.baseline) {
            (Some(_), Some(b)) => self.saved_text().is_ok_and(|t| &t != b),
            _ => false,
        }
    }

    /// Before replacing the open model: if it has unsaved changes, ask
    /// (Save, Don't save, Cancel) and return true; the action runs once
    /// answered.
    fn ask_first(&mut self, action: Discard) -> bool {
        if self.discard_ok || !self.has_unsaved() {
            return false;
        }
        self.asking = Some(action);
        true
    }

    /// Do what was asked about, the changes saved or let go.
    fn proceed(&mut self, ctx: &egui::Context, action: Discard) {
        self.discard_ok = true;
        match action {
            Discard::OpenPath(p) => self.open_path(&p),
            Discard::OpenDialog => self.open_dialog(),
            Discard::Example(k) => self.open_example(k),
            Discard::NewModel => self.apply_equations(),
            Discard::Close => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                // Stay permissive: the window is closing.
                return;
            }
        }
        self.discard_ok = false;
    }

    /// The question about unsaved changes, while one is pending.
    fn unsaved_dialog(&mut self, ctx: &egui::Context) {
        let Some(action) = self.asking.clone() else {
            return;
        };
        let title = self
            .session
            .as_ref()
            .map(|l| l.model().title.clone())
            .unwrap_or_default();
        let mut answer = None;
        egui::Modal::new(egui::Id::new("unsaved")).show(ctx, |ui| {
            ui.heading("Unsaved changes");
            ui.label(format!("Save the changes to {title} first?"));
            ui.weak("Parameters, edits, sliders, views and speed that a save would write.");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    answer = Some(0);
                }
                if ui.button("Don't save").clicked() {
                    answer = Some(1);
                }
                if ui.button("Cancel").clicked() {
                    answer = Some(2);
                }
            });
        });
        match answer {
            Some(0) => {
                self.asking = None;
                self.save();
                // A cancelled Save as leaves the changes: stop there.
                if !self.has_unsaved() {
                    self.proceed(ctx, action);
                }
            }
            Some(1) => {
                self.asking = None;
                self.proceed(ctx, action);
            }
            Some(_) => self.asking = None,
            None => {}
        }
    }

    /// Ask before the window closes on unsaved changes.
    fn guard_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested())
            && !self.discard_ok
            && self.asking.is_none()
            && self.has_unsaved()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.asking = Some(Discard::Close);
        }
    }

    /// Check now and then, on the worker, whether there are unsaved
    /// changes, for the toolbar's marker.
    fn check_unsaved(&mut self) {
        if self.last_unsaved_check.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.last_unsaved_check = Instant::now();
        let (Some(live), Some(base)) = (&self.session, self.baseline.clone()) else {
            return;
        };
        let (speed, views, layout) = (self.speed, self.views.entries(), self.layout());
        let flag = self.unsaved.clone();
        live.send(Some("unsaved-check"), move |s, _| {
            let differs = s
                .save_json_with(speed, Some(views), Some(layout))
                .is_ok_and(|t| t != base);
            flag.store(differs, std::sync::atomic::Ordering::Relaxed);
        });
    }

    /// Open a model chosen in the system file dialog.
    fn open_dialog(&mut self) {
        if self.ask_first(Discard::OpenDialog) {
            return;
        }
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
        if self.error.is_none() {
            self.diagram.canvas.refit();
        }
    }

    /// Open a model; it runs at once if `running` (sweeps are evaluated
    /// either way). The previous model's worker stops.
    fn open_text_with(&mut self, title: &str, text: &str, running: bool) {
        self.open_inner(title, text, running, true);
    }

    fn open_inner(&mut self, title: &str, text: &str, running: bool, announce: bool) {
        match Session::load_deferred(title, text, &self.registry) {
            Ok(mut s) => {
                self.speed = s.file.controls.speed.unwrap_or(1.0);
                // Tabs shown in their own windows stay there when the model
                // is rebuilt (canvas edits).
                let popped = if announce {
                    Vec::new()
                } else {
                    self.popped_panes()
                };
                let w = {
                    let h = s.signals();
                    self.views.open(&s.available(&h), &s.file.controls.views)
                };
                s.warnings.extend(w);
                let tiles = self.tile_ids();
                let opened = self.views.opened.clone();
                let tile = |k: usize| opened.get(k).copied().flatten();
                self.diagram.canvas.open = s.file.controls.open.iter().cloned().collect();
                self.dock = match &s.file.controls.layout {
                    Some(l) => Dock::from_layout(l, &tile, &tiles),
                    None => Dock::standard(&tiles),
                };
                for p in popped {
                    let pane = match p {
                        Popped::Controls => Some(Pane::Controls),
                        Popped::Inspector => Some(Pane::Inspector),
                        Popped::Diagram => Some(Pane::Diagram),
                        Popped::View(k) => tile(k).map(Pane::View),
                    };
                    if let Some(pane) = pane {
                        self.dock.pop_out(&pane);
                    }
                }
                for w in &s.warnings {
                    self.log.warn(format!("{title}: {w}"));
                }
                if announce {
                    for c in &s.conversions {
                        self.log.info(format!(
                            "{title} was written in an older form: {c}; saving writes the current form"
                        ));
                    }
                }
                let running = running && s.sweep().is_none();
                self.session = Some(Live::start(s, self.speed, running));
                self.error = None;
                self.param_edits.clear();
                self.pending.clear();
                self.diagram.reset();
                if announce {
                    // Opened, or rebuilt outside the canvas: canvas undo
                    // starts afresh (ADR 0050).
                    self.diagram.forget_history();
                    self.log.info(format!("Opened {title}"));
                }
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
                for (k, e) in EXAMPLES.iter().enumerate() {
                    if ui.button(e.name).clicked() {
                        self.open_example(k);
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
            if has_model {
                ui.menu_button("Runs", |ui| self.runs_menu(ui))
                    .response
                    .on_hover_text("Save this run's input beside the model, and replay saved runs");
                ui.separator();
                self.view_menus(ui);
                self.solving_menu(ui);
            }
            ui.separator();
            match &self.path {
                Some(p) => ui.weak(p.display().to_string()),
                None if has_model => ui.weak("built-in example: Save asks where to save it"),
                None => ui.weak(""),
            };
            if has_model && self.unsaved.load(std::sync::atomic::Ordering::Relaxed) {
                ui.colored_label(ui.visuals().warn_fg_color, "● unsaved")
                    .on_hover_text("Changes a save would write: parameters, edits, sliders, views, speed");
            }
        });
        self.shortcuts(ui);
        self.key_inputs(ui);

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
                if ui
                    .button("Start from here")
                    .on_hover_text(
                        "Make the current state where the run starts, and restart from it at \
                         t = 0: e.g. to study a transient from a steady state. Saved with the \
                         model: initial-condition parameters where they can say it, else a \
                         start state per block.",
                    )
                    .clicked()
                {
                    live.send(None, |s, c| {
                        match s.start_from_now() {
                            Ok(notes) => {
                                for n in notes {
                                    c.info(n);
                                }
                            }
                            Err(e) => c.error(e),
                        }
                        c.reset_pacer();
                    });
                    self.param_edits.clear();
                    self.rewind_t = 0.0;
                }
                if m.file.blocks.iter().any(|b| b.start.is_some())
                    && ui
                        .button("Forget start state")
                        .on_hover_text("Start every block from its parameters again, and restart")
                        .clicked()
                {
                    live.send(None, |s, c| {
                        match s.forget_start_states() {
                            Ok(()) => c.info("Starting from the parameters again"),
                            Err(e) => c.error(e),
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
                if st.replaying {
                    ui.separator();
                    ui.colored_label(ui.visuals().hyperlink_color, "Replaying input")
                        .on_hover_text(
                            "Recorded input lies ahead and is applied as the run goes on. \
                             Any live input from here on takes over (a new branch).",
                        );
                }
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
                            .stick_to_bottom(true)
                            .show(ui, |ui| {
                                for e in log.entries() {
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
                    .on_hover_text("Every notice, warning and error, newest last");
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

    /// The view tiles' ids, in order.
    fn tile_ids(&self) -> Vec<u64> {
        self.views.tiles.iter().map(|t| t.id).collect()
    }

    /// The tabs in their own windows, by what they show.
    fn popped_panes(&self) -> Vec<Popped> {
        let index = |id: u64| self.views.tiles.iter().position(|t| t.id == id);
        self.dock
            .popped
            .iter()
            .filter_map(|p| match p {
                Pane::Controls => Some(Popped::Controls),
                Pane::Inspector => Some(Popped::Inspector),
                Pane::Diagram => Some(Popped::Diagram),
                p => p.tiles().first().and_then(|&t| index(t)).map(Popped::View),
            })
            .collect()
    }

    /// The arrangement to save (ADR 0053), or `None` for the standard one.
    fn layout(&self) -> Option<simsandbox_file::layout::Layout> {
        let index = |id: u64| self.views.tiles.iter().position(|t| t.id == id);
        let l = self.dock.to_layout(&index)?;
        let standard = Dock::standard(&self.tile_ids()).to_layout(&index);
        (Some(&l) != standard.as_ref()).then_some(l)
    }

    /// The docked tabs (ADR 0053), and the ones popped out into windows.
    fn docked(&mut self, ui: &mut egui::Ui) {
        if self.session.is_none() {
            ui.centered_and_justified(|ui| ui.label("Open an example or a model file."));
            return;
        }
        let tiles = self.tile_ids();
        self.dock.sync(&tiles);
        if let Some(Focus::Tile(t)) = self.focus.take() {
            self.dock.focus(|p| p.tiles().contains(&t));
        }
        let mut dock = std::mem::replace(&mut self.dock, Dock::standard(&[]));
        let mut tabs = Tabs {
            app: self,
            actions: Vec::new(),
        };
        egui_dock::DockArea::new(&mut dock.state)
            .id(egui::Id::new("dock"))
            .style(egui_dock::Style::from_egui(ui.style()))
            .show_leaf_collapse_buttons(false)
            .show_leaf_close_all_buttons(false)
            .show_inside(ui, &mut tabs);
        let mut actions = tabs.actions;
        actions.append(&mut self.popped_actions);
        // Popped-out tabs are deferred viewports: each window is drawn on
        // its own, from the shared app, so one window never waits for
        // another (ADR 0054).
        for pane in dock.popped.clone() {
            let title = self.pane_title(&pane);
            let id = popped_viewport(&pane);
            let builder = egui::ViewportBuilder::default()
                .with_title(format!("{title} — SimSandbox"))
                .with_inner_size([760.0, 520.0]);
            let this = self.this.clone();
            ui.ctx().show_viewport_deferred(id, builder, move |ui, _| {
                let Some(app) = this.upgrade() else {
                    return;
                };
                // Never held while another window draws; if it were,
                // draw next frame instead of waiting.
                let Ok(mut app) = app.try_lock() else {
                    ui.ctx().request_repaint();
                    return;
                };
                app.popped_ui(ui, &pane, &title);
            });
            // Draw it whenever the main window draws (new data).
            ui.ctx().request_repaint_of(id);
        }
        self.dock = dock;
        for a in actions {
            match a {
                DockAction::PopOut(p) => self.dock.pop_out(&p),
                DockAction::DockBack(p) => self.dock.dock_back(&p),
                DockAction::Close(t) => self.views.close(t),
                DockAction::OwnTab(t) => self.dock.own_tab(t),
                DockAction::IntoGroup(t) => self.dock.tile_back(t),
                DockAction::TileAll => {
                    let tiles = self.tile_ids();
                    self.dock.tile_all(&tiles);
                }
                DockAction::Select(t) => self.views.selected = Some(t),
                DockAction::Inspect(signal) => self.inspect_signal(&signal),
                DockAction::Pointer(block, p) => {
                    for (name, v) in ["x", "y", "down"].into_iter().zip(p) {
                        self.send_input(format!("{block}.{name}"), v);
                    }
                }
            }
        }
    }

    /// A popped-out tab's window: its title bar with "Dock", and the tab.
    /// What it asks for is carried out by the main window.
    fn popped_ui(&mut self, ui: &mut egui::Ui, pane: &Pane, title: &str) {
        if self.session.is_none() {
            return;
        }
        let mut actions = Vec::new();
        egui::Panel::top(egui::Id::new(("popped-bar", pane_key(pane)))).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong(title);
                if ui
                    .small_button("Dock")
                    .on_hover_text("Put it back among the tabs")
                    .clicked()
                {
                    actions.push(DockAction::DockBack(pane.clone()));
                }
            });
        });
        egui::CentralPanel::default().show(ui, |ui| {
            self.pane_ui(ui, pane, &mut actions);
        });
        if ui.input(|i| i.viewport().close_requested()) {
            actions.push(DockAction::DockBack(pane.clone()));
        }
        if !actions.is_empty() {
            self.popped_actions.extend(actions);
        }
        // Edits made here show in the main window too.
        ui.ctx().request_repaint_of(egui::ViewportId::ROOT);
    }

    /// A tab's title.
    fn pane_title(&self, pane: &Pane) -> String {
        match pane {
            Pane::Controls => "Controls".into(),
            Pane::Inspector => "Inspector".into(),
            Pane::Diagram => match self.diagram.draft_problem() {
                Some(_) => "Diagram ⚠".into(),
                None => "Diagram".into(),
            },
            Pane::View(t) => self
                .views
                .tiles
                .iter()
                .find(|x| x.id == *t)
                .and_then(|x| self.views.registry.get(&x.state.type_name))
                .map_or("View".into(), |vt| vt.label.to_string()),
            Pane::Views { tiles, .. } if tiles.len() == 1 => "Views".into(),
            Pane::Views { tiles, .. } => format!("Views ({})", tiles.len()),
        }
    }

    /// What a tab shows.
    fn pane_ui(&mut self, ui: &mut egui::Ui, pane: &Pane, actions: &mut Vec<DockAction>) {
        match pane {
            Pane::Controls => {
                egui::ScrollArea::both().id_salt("controls").show(ui, |ui| {
                    self.controls_panel(ui);
                });
            }
            Pane::Inspector => {
                egui::ScrollArea::both()
                    .id_salt("inspector")
                    .show(ui, |ui| {
                        self.param_panel(ui);
                    });
            }
            Pane::Diagram => self.diagram_tab(ui),
            Pane::View(t) => self.view_tiles(ui, &[*t], false, actions),
            Pane::Views { tiles, .. } => self.view_tiles(ui, tiles, true, actions),
        }
    }

    /// Make an edit through the diagram, as if on the canvas at `scope`:
    /// it can be undone, and a model that does not load yet keeps it as a
    /// draft.
    fn edit_at(&mut self, scope: Scope, edit: Edit) {
        let Some(live) = &self.session else {
            return;
        };
        let m = live.model();
        let (speed, views, layout) = (self.speed, self.views.entries(), self.layout());
        let current = || {
            let (views, layout) = (views.clone(), layout.clone());
            live.call(move |s, _| s.save_json_with(speed, Some(views), Some(layout)))?
        };
        let here = std::mem::replace(&mut self.diagram.scope, scope);
        let outcomes = self.diagram.handle(vec![edit], &self.registry, &current);
        self.diagram.scope = here;
        self.diagram_outcomes(&m, outcomes);
    }

    /// The node canvas (ADR 0042), and what its edits ask for.
    fn diagram_tab(&mut self, ui: &mut egui::Ui) {
        let Some(live) = &self.session else {
            return;
        };
        let m = live.model();
        let (speed, views, layout) = (self.speed, self.views.entries(), self.layout());
        let current = || {
            let (views, layout) = (views.clone(), layout.clone());
            live.call(move |s, _| s.save_json_with(speed, Some(views), Some(layout)))?
        };
        let outcomes = self.diagram.show(
            ui,
            &m,
            &self.registry,
            (&views, &self.views.registry),
            &current,
        );
        self.diagram_outcomes(&m, outcomes);
    }

    /// Carry out what the canvas asked for.
    fn diagram_outcomes(&mut self, m: &crate::live::Model, outcomes: Vec<Outcome>) {
        for o in outcomes {
            match o {
                Outcome::Moved(scope, moves) => {
                    let Some(live) = &self.session else {
                        return;
                    };
                    for (block, at) in moves {
                        let key = format!("at:{scope:?}:{block}");
                        let scope = scope.clone();
                        live.send(Some(&key), move |s, c| {
                            if let Err(e) = s.set_position_in(&scope, &block, at) {
                                c.error(e);
                            }
                        });
                    }
                }
                Outcome::Apply { text, notes } => {
                    let running = self.session.as_ref().is_some_and(|l| l.status().running);
                    let path = self.path.clone();
                    let canvas = self.diagram.canvas.clone();
                    self.open_inner(&m.title, &text, running, false);
                    self.path = path;
                    self.diagram.canvas = canvas;
                    for n in notes.iter().filter(|n| !n.is_empty()) {
                        self.log.info(format!("{n}; restarted"));
                    }
                }
                Outcome::Open(name) => self.open_block(m, &name),
                Outcome::Error(e) => self.error = Some(e),
                Outcome::Placed { filters, views } => {
                    let Some(live) = &self.session else {
                        return;
                    };
                    for (name, at) in filters {
                        live.send(Some(&format!("place:{name}")), move |s, c| {
                            if let Err(e) = s.place_filter(&name, Some(at)) {
                                c.error(e);
                            }
                        });
                    }
                    for (k, at) in views {
                        if let Some(t) = self.views.tiles.get_mut(k) {
                            t.canvas = Some(at.map(|v| (v * 10.0).round() / 10.0));
                        }
                    }
                }
                Outcome::Layout(file) => {
                    let Some(live) = &self.session else {
                        return;
                    };
                    for f in &file.controls.filters {
                        let (name, at) = (f.name.clone(), f.canvas);
                        live.send(None, move |s, _| {
                            let _ = s.place_filter(&name, at);
                        });
                    }
                    for (t, v) in self.views.tiles.iter_mut().zip(&file.controls.views) {
                        t.canvas = v.canvas;
                    }
                    live.send(None, move |s, _| {
                        simsandbox_session::undo::copy_layout(&file, &mut s.file);
                    });
                }
                Outcome::Data {
                    filters,
                    views,
                    notes,
                } => self.apply_data(m, filters, views, notes),
                Outcome::EditData(_) => {
                    self.signals_editor = Some(SignalsEditor::new(m));
                }
                Outcome::SetValue { key, value } => {
                    if let Some(row) = m.params.iter().find(|r| r.key == key) {
                        self.apply_changes(vec![(row.clone(), value)]);
                    }
                }
                Outcome::Opened(open) => {
                    if let Some(live) = &self.session {
                        live.send(None, move |s, _| s.file.controls.open = open);
                    }
                }
                Outcome::ShowView(k) => {
                    if let Some(t) = self.views.tiles.get(k) {
                        self.views.selected = Some(t.id);
                        self.focus = Some(Focus::Tile(t.id));
                    }
                }
            }
        }
    }

    /// Apply filters and views edited on the canvas to the running model
    /// (ADR 0050): filters are evaluated over the history again only if
    /// they changed; tiles whose view is unchanged stay as they are.
    fn apply_data(
        &mut self,
        m: &crate::live::Model,
        filters: Vec<simsandbox_file::FilterEntry>,
        views: Vec<simsandbox_file::ViewEntry>,
        notes: Vec<String>,
    ) {
        let Some(live) = &self.session else {
            return;
        };
        if filters != m.filters {
            match live.call(move |s, _| s.set_filters(filters)) {
                Ok(Ok(())) => {}
                Ok(Err(problems)) => {
                    self.error = Some(problems.join("; "));
                    return;
                }
                Err(e) => {
                    self.error = Some(e);
                    return;
                }
            }
        }
        let history = live.history();
        let now = live.model();
        let available = now.available(&history);
        for w in self.views.set_entries(&views, &available) {
            self.log.warn(w);
        }
        for n in notes.iter().filter(|n| !n.is_empty()) {
            self.log.info(n.clone());
        }
    }

    /// Open a block's definition: the equation editor for typed equations.
    fn open_block(&mut self, m: &crate::live::Model, name: &str) {
        // `subsystem::block` for a block inside a subsystem (ADR 0043).
        let blocks = match name.split_once("::") {
            Some((def, _)) => m.file.subsystems.get(def).map(|d| &d.blocks),
            None => Some(&m.file.blocks),
        };
        let inner = name.rsplit("::").next().unwrap_or(name);
        let Some(b) = blocks.and_then(|bs| bs.iter().find(|b| b.name == inner)) else {
            return;
        };
        if b.type_name == simsandbox_eq::registry::EQUATIONS {
            self.editor = Some(Editor {
                block: Some(name.to_string()),
                text: b.equations.join("\n"),
                error: None,
            });
        } else {
            self.log.info(format!(
                "{name} ({}) has built-in equations; its parameters and settings are in the side panel",
                b.type_name
            ));
        }
    }

    /// "More views…" and "Data…": add a view, edit the filters.
    fn view_menus(&mut self, ui: &mut egui::Ui) {
        let Some(live) = &self.session else {
            return;
        };
        let m = live.model();
        let history = live.history();
        let available = m.available(&history);
        let views = &mut self.views;
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
    }

    /// Views as tiles in an automatic grid, each with its slot pickers, a
    /// close button and a button to move it to a tab of its own (`grouped`)
    /// or back among the others.
    fn view_tiles(
        &mut self,
        ui: &mut egui::Ui,
        ids: &[u64],
        grouped: bool,
        actions: &mut Vec<DockAction>,
    ) {
        let Some(live) = &self.session else {
            return;
        };
        let m = live.model();
        // Read while drawing; the worker keeps stepping and appends later.
        let history = live.history();
        let available = m.available(&history);
        let views = &mut self.views;
        let (text, error) = (&mut self.param_edits, &mut self.error);
        let area = ui.available_rect_before_wrap();
        let shown: Vec<usize> = ids
            .iter()
            .filter_map(|id| views.tiles.iter().position(|t| t.id == *id))
            .collect();
        if shown.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(
                    "No views here: add one from \"More views…\", or choose \"tile\" on a \
                     view in a tab of its own.",
                )
            });
            return;
        }
        // Views feeding an `input.pointer` (ADR 0058), by tile.
        let senders: HashMap<u64, String> = m
            .file
            .controls
            .inputs
            .iter()
            .filter_map(|(block, b)| {
                let tile = (*views.opened.get(b.view?)?)?;
                Some((tile, block.clone()))
            })
            .collect();
        let (cols, rows) = simsandbox_session::views::grid(shown.len());
        let mut swap = false;
        for (k, &i) in shown.iter().enumerate() {
            let tile = &mut views.tiles[i];
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
                                actions.push(DockAction::Select(tile.id));
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .small_button("×")
                                        .on_hover_text("Close this view")
                                        .clicked()
                                    {
                                        actions.push(DockAction::Close(tile.id));
                                    }
                                    let (label, hint, action) = if grouped {
                                        (
                                            "own tab",
                                            "Show it in a tab of its own, to move, stack or pop out",
                                            DockAction::OwnTab(tile.id),
                                        )
                                    } else {
                                        (
                                            "tile",
                                            "Tile it with the other views",
                                            DockAction::IntoGroup(tile.id),
                                        )
                                    };
                                    if ui.small_button(label).on_hover_text(hint).clicked() {
                                        actions.push(action);
                                    }
                                    let id = tile.id;
                                    ui.menu_button("style", |ui| {
                                        style_editor(ui, t, &mut tile.state, &history, id, text, error);
                                    })
                                    .response
                                    .on_hover_text("How this view draws: trails, scales and what it picks");
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
                        let sender = senders.get(&tile.id);
                        tile.state.sends_pointer = sender.is_some();
                        tile.state.show(ui, t, &history);
                        if let Some(signal) = tile.state.picked() {
                            actions.push(DockAction::Inspect(signal));
                        }
                        if let (Some(block), Some(p)) = (sender, tile.state.pointer()) {
                            actions.push(DockAction::Pointer(block.clone(), p));
                        }
                    });
            });
        }
        ui.allocate_rect(area, egui::Sense::hover());
    }

    /// Live input from keys or a view (ADR 0058): recorded when it changes
    /// a value. A newer value for the same key replaces one not yet sent.
    fn send_input(&self, key: String, value: f64) {
        let Some(live) = self.session.as_ref() else {
            return;
        };
        live.send(Some(&format!("input:{key}")), move |s, c| {
            if let Err(e) = s.set_input(&key, value) {
                c.error(format!("input {key}: {e}"));
            }
        });
    }

    /// Apply typed or dragged parameter values to the running model.
    fn apply_changes(&self, changes: Vec<(ParamRow, f64)>) {
        let Some(live) = self.session.as_ref() else {
            return;
        };
        for (row, v) in changes {
            let key = row.key.clone();
            live.send(Some(&format!("param:{key}")), move |s, c| {
                match s.set_param_key(&row.key, v) {
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
    }

    /// A curve was clicked: show the block it comes from in the inspector,
    /// and select it on the diagram when that shows the model level
    /// (`car1/car.v` comes from the use `car1`; a sweep's `grid.spring.x`
    /// from `spring`). Signals of filters have no block.
    fn inspect_signal(&mut self, signal: &str) {
        let Some(live) = &self.session else {
            return;
        };
        let m = live.model();
        let base = signal.strip_prefix("grid.").unwrap_or(signal);
        let Some((node, _)) = base.split_once('.') else {
            self.log
                .info(format!("{signal} is computed by a filter: see Data…"));
            return;
        };
        let top = node.split('/').next().unwrap_or(node);
        if !m.file.blocks.iter().any(|b| b.name == top) {
            return;
        }
        self.picked = vec![(None, top.to_string())];
        if self.diagram.scope == simsandbox_session::diagram::Scope::Model {
            let selection = simsandbox_session::canvas::Selection::Block(top.to_string());
            self.diagram.canvas.selected = Some(selection.clone());
            self.last_canvas = Some(selection);
        }
    }

    /// "Solving…": how blocks are solved together (ADR 0023).
    fn solving_menu(&mut self, ui: &mut egui::Ui) {
        let Some(live) = self.session.as_ref() else {
            return;
        };
        let m = live.model();
        let mut fusion = None;
        ui.menu_button("Solving…", |ui| {
            ui.set_max_width(340.0);
            fusion = solving_section(ui, &m);
        })
        .response
        .on_hover_text("Which blocks are solved together, and with what method");
        if let Some(on) = fusion {
            self.set_fusion(on);
        }
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
                self.mark_saved();
            }
            Err(e) => self.error = Some(format!("{}: {e}", path.display())),
        }
    }

    /// The Runs menu (ADR 0059): save this run, replay a saved run of
    /// this model, or make a run the model's default schedule.
    fn runs_menu(&mut self, ui: &mut egui::Ui) {
        if ui
            .button("Save run…")
            .on_hover_text("Write this run's schedule and recorded input beside the model, to replay it later or headless")
            .clicked()
        {
            self.run_dialog = Some((String::new(), String::new()));
            ui.close();
        }
        if ui
            .button("Use this run as the model's schedule")
            .on_hover_text("Its schedule and recorded input become run.inputs: every run of the model applies them")
            .clicked()
        {
            match self.current_run("this run", "") {
                Ok(run) => self.use_as_schedule(&run, "this run"),
                Err(e) => self.error = Some(e),
            }
            ui.close();
        }
        ui.separator();
        let Some(model) = self.path.clone() else {
            ui.weak("Save the model to keep runs beside it.");
            return;
        };
        let runs = runs_beside(&model);
        if runs.is_empty() {
            ui.weak("No saved runs of this model.");
        }
        for (path, run) in runs {
            let name = if run.name.is_empty() {
                path.file_name()
                    .map_or(String::new(), |n| n.to_string_lossy().into_owned())
            } else {
                run.name.clone()
            };
            let mut hover = format!(
                "{} events scheduled, {} recorded",
                run.inputs.len(),
                run.track.len()
            );
            if !run.created.is_empty() {
                hover = format!("{}\n{hover}", run.created);
            }
            if !run.note.is_empty() {
                hover = format!("{}\n{hover}", run.note);
            }
            ui.horizontal(|ui| {
                if ui
                    .button(&name)
                    .on_hover_text(format!("Restart and replay it\n{hover}"))
                    .clicked()
                {
                    self.open_run(&path);
                    ui.close();
                }
                if ui
                    .small_button("as schedule")
                    .on_hover_text("Use its events as the model's schedule (run.inputs)")
                    .clicked()
                {
                    self.use_as_schedule(&run, &name);
                    ui.close();
                }
            });
        }
    }

    /// This run as a run file (ADR 0059), its model named relative to the
    /// model's folder.
    fn current_run(
        &self,
        name: &str,
        note: &str,
    ) -> Result<simsandbox_file::runs::RunFile, String> {
        let live = self.session.as_ref().ok_or("no model is open")?;
        let model = self
            .path
            .as_deref()
            .and_then(Path::file_name)
            .map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let (name, note, created) = (name.to_string(), note.to_string(), utc_now());
        live.call(move |s, _| s.run_file(&model, &name, &note, &created))
    }

    /// The name and note asked by "Save run…".
    fn run_dialog(&mut self, ctx: &egui::Context) {
        let Some((name, note)) = self.run_dialog.as_mut() else {
            return;
        };
        let mut answer = None;
        egui::Modal::new(egui::Id::new("save-run")).show(ctx, |ui| {
            ui.heading("Save run");
            ui.weak("The schedule and the input recorded so far, beside the model.");
            ui.add_space(6.0);
            egui::Grid::new("save-run-fields")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(name);
                    ui.end_row();
                    ui.label("Note");
                    ui.text_edit_multiline(note);
                    ui.end_row();
                });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!name.trim().is_empty(), egui::Button::new("Save"))
                    .clicked()
                {
                    answer = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    answer = Some(false);
                }
            });
        });
        match answer {
            Some(true) => {
                let (name, note) = self.run_dialog.take().unwrap_or_default();
                self.save_run(name.trim(), note.trim());
            }
            Some(false) => self.run_dialog = None,
            None => {}
        }
    }

    /// Write this run beside the model (ADR 0059), saving a model that has
    /// never been saved first.
    fn save_run(&mut self, name: &str, note: &str) {
        if self.path.is_none() {
            self.save_as_dialog();
        }
        let Some(model) = self.path.clone() else {
            return;
        };
        let run = match self.current_run(name, note) {
            Ok(run) => run,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        let model_name = model
            .file_name()
            .map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let dir = model.parent().unwrap_or(Path::new(""));
        let file = simsandbox_file::runs::run_file_name(&model_name, name);
        let mut path = dir.join(&file);
        let mut k = 2;
        while path.exists() {
            let stem = file.trim_end_matches(".run.json");
            path = dir.join(format!("{stem}-{k}.run.json"));
            k += 1;
        }
        if !run_fits_disk(&run, &model, &self.registry) {
            self.log.warn(format!(
                "{}: the model changed since this run started (unsaved or since saved); \
                 the run opens with a warning. Save the model, restart and record again for one that fits",
                path.display()
            ));
        }
        match std::fs::write(&path, run.to_json()) {
            Ok(()) => self.log.info(format!(
                "Saved run '{name}' ({} scheduled, {} recorded) to {}",
                run.inputs.len(),
                run.track.len(),
                path.display()
            )),
            Err(e) => self.error = Some(format!("{}: {e}", path.display())),
        }
    }

    /// Open a run file (ADR 0059): its model from disk with the run's
    /// seed, settings and schedule, then restart and replay its input.
    fn open_run(&mut self, path: &Path) {
        let fail = |e: String| format!("{}: {e}", path.display());
        let run = match std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|t| simsandbox_file::runs::RunFile::from_json(&t).map_err(|e| e.to_string()))
        {
            Ok(run) => run,
            Err(e) => {
                self.error = Some(fail(e));
                return;
            }
        };
        let model = path.parent().unwrap_or(Path::new("")).join(&run.model);
        let file = match std::fs::read_to_string(&model)
            .map_err(|e| format!("its model {}: {e}", model.display()))
            .and_then(|t| {
                simsandbox_file::ModelFile::from_json(&t)
                    .map(|(f, _)| f)
                    .map_err(|e| format!("its model {}: {e}", model.display()))
            }) {
            Ok(f) => f,
            Err(e) => {
                self.error = Some(fail(e));
                return;
            }
        };
        let replayed = run.apply_to(&file);
        let differs = replayed.run != file.run;
        let title = model
            .file_name()
            .map_or(String::new(), |n| n.to_string_lossy().into_owned());
        self.open_text(&title, &replayed.to_json());
        if self.error.is_some() {
            return;
        }
        self.path = Some(model.clone());
        self.mark_saved();
        if !run_fits_disk(&run, &model, &self.registry) {
            self.log.warn(format!(
                "{}: the model changed since this run was recorded; replaying it anyway",
                path.display()
            ));
        }
        let track: Vec<simsandbox_session::session::InputEvent> = run
            .track
            .iter()
            .map(|(t, key, value)| simsandbox_session::session::InputEvent {
                t: *t,
                key: key.clone(),
                value: *value,
            })
            .collect();
        let replay = self
            .session
            .as_ref()
            .map(|live| live.call(move |s, _| s.replay(track)));
        match replay {
            Some(Ok(Ok(()))) => {
                let name = if run.name.is_empty() {
                    "run"
                } else {
                    &run.name
                };
                self.log.info(format!(
                    "Replaying {name} ({} recorded events)",
                    run.track.len()
                ));
                if differs {
                    self.log.info(
                        "The run's seed, step or schedule differ from the model's and are in use; \
                         saving the model would keep them",
                    );
                }
            }
            Some(Ok(Err(e)) | Err(e)) => self.error = Some(fail(e)),
            None => {}
        }
    }

    /// Make a run's events the model's default schedule (`run.inputs`).
    fn use_as_schedule(&mut self, run: &simsandbox_file::runs::RunFile, name: &str) {
        let mut events = run.inputs.clone();
        events.extend(
            run.track
                .iter()
                .map(|(t, key, value)| simsandbox_file::ScheduledChange {
                    t: *t,
                    set: key.clone(),
                    to: *value,
                }),
        );
        // Stable: at equal times the schedule stays before the recording,
        // as they were applied.
        events.sort_by(|a, b| a.t.total_cmp(&b.t));
        let n = events.len();
        self.rebuild(
            move |f| f.run.inputs = events,
            &format!("The model's schedule is now {name}'s {n} events"),
        );
    }

    /// The open model as a file, with the current parameters, step, speed
    /// and views.
    fn saved_text(&self) -> Result<String, String> {
        let live = self.session.as_ref().ok_or("no model is open")?;
        let (speed, views, layout) = (self.speed, self.views.entries(), self.layout());
        live.call(move |s, _| s.save_json_with(speed, Some(views), Some(layout)))?
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

    /// Keys bound to `input.value` blocks in `controls.inputs` (ADR 0058).
    /// With a rate, the first key moves the value down and the second up
    /// at that rate per second, and it stays where it is let go; without,
    /// the value is its range's low or high end while a key is held and 0
    /// when none is. Keys typed into a field are not input.
    fn key_inputs(&mut self, ui: &mut egui::Ui) {
        let Some(live) = self.session.as_ref() else {
            return;
        };
        let m = live.model();
        if m.file.controls.inputs.values().all(|b| b.keys.is_empty()) {
            return;
        }
        let typing = ui.ctx().egui_wants_keyboard_input();
        let dt = ui.input(|i| i.stable_dt).min(0.1) as f64;
        let mut sends = Vec::new();
        for (block, b) in &m.file.controls.inputs {
            let [down, up] = match b.keys.as_slice() {
                [d, u] => [d, u].map(|k| egui::Key::from_name(k)),
                _ => continue,
            };
            let held =
                |k: Option<egui::Key>| !typing && k.is_some_and(|k| ui.input(|i| i.key_down(k)));
            let dir = held(up) as i32 - held(down) as i32;
            let key = format!("{block}.y");
            let Some(row) = m.params.iter().find(|r| r.key == key) else {
                continue;
            };
            let [lo, hi] = row.suggested;
            let value = match (b.rate, dir) {
                (_, 0) if held(up) || held(down) => continue,
                (Some(_), 0) => {
                    self.key_values.remove(block);
                    continue;
                }
                (None, 0) => match self.key_values.remove(block) {
                    Some(_) => 0.0_f64.clamp(lo, hi),
                    None => continue,
                },
                (Some(rate), d) => {
                    let from = self.key_values.get(block).copied().unwrap_or(row.value);
                    (from + d as f64 * rate * dt).clamp(lo, hi)
                }
                (None, d) => {
                    if d > 0 {
                        hi
                    } else {
                        lo
                    }
                }
            };
            if dir != 0 {
                self.key_values.insert(block.clone(), value);
                ui.ctx().request_repaint();
            }
            sends.push((key, value));
        }
        for (key, value) in sends {
            self.send_input(key, value);
        }
    }

    /// New values, and pin and slider choices (saved in `controls`, ADR
    /// 0012).
    fn apply_edits(
        &mut self,
        changes: Vec<(ParamRow, f64)>,
        controls: Vec<(String, simsandbox_file::ParamControl)>,
    ) {
        if let Some(live) = self.session.as_ref() {
            for (key, control) in controls {
                live.send(None, move |s, _| {
                    s.file.controls.params.insert(key, control);
                });
            }
        }
        self.apply_changes(changes);
    }

    /// The Controls pane (ADR 0053): the pinned parameters, each with an
    /// optional slider (ADR 0011).
    fn controls_panel(&mut self, ui: &mut egui::Ui) {
        let Some(live) = self.session.as_ref() else {
            return;
        };
        let m = live.model();
        let mut changes: Vec<(ParamRow, f64)> = Vec::new();
        let mut controls: Vec<(String, simsandbox_file::ParamControl)> = Vec::new();
        let mut file_edits: Vec<(Scope, Edit)> = Vec::new();
        let pinned: Vec<&ParamRow> = m
            .params
            .iter()
            .filter(|r| {
                m.file
                    .controls
                    .params
                    .get(&r.key)
                    .is_some_and(|c| c.pinned())
            })
            .collect();
        if pinned.is_empty() {
            ui.weak(
                "Nothing pinned. Pin a value with 📌 in the inspector to keep it here, \
                 with a slider if you like.",
            );
        }
        ui.spacing_mut().slider_width = 90.0;
        egui::Grid::new("controls")
            .num_columns(4)
            .spacing([6.0, 4.0])
            .show(ui, |ui| {
                for row in pinned {
                    let mut r = row.clone();
                    r.name = r.key.clone();
                    let edits = Edits {
                        changes: &mut changes,
                        controls: &mut controls,
                        text: &mut self.param_edits,
                        pending: &mut self.pending,
                        error: &mut self.error,
                        file: &mut file_edits,
                    };
                    param_row(ui, &m, &r, Place::Controls, None, edits);
                }
            });
        self.apply_edits(changes, controls);
        for (scope, e) in file_edits {
            self.edit_at(scope, e);
        }
    }

    /// The inspector (ADR 0053): search, what is selected, and every
    /// parameter where it is set.
    fn param_panel(&mut self, ui: &mut egui::Ui) {
        let Some(live) = self.session.as_ref() else {
            return;
        };
        let m = live.model();
        let mut update: Option<(String, simsandbox_agents::Update)> = None;
        let mut changes: Vec<(ParamRow, f64)> = Vec::new();
        let mut controls: Vec<(String, simsandbox_file::ParamControl)> = Vec::new();
        let mut open_editor: Option<(String, String)> = None;
        // Edits that go through the diagram (a conversion's units, a new
        // block's values), at the level they apply to.
        let mut canvas_edits: Vec<(Scope, Edit)> = Vec::new();
        macro_rules! edits {
            () => {
                Edits {
                    changes: &mut changes,
                    controls: &mut controls,
                    text: &mut self.param_edits,
                    pending: &mut self.pending,
                    error: &mut self.error,
                    file: &mut canvas_edits,
                }
            };
        }
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 100.0).max(80.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.inspect_query)
                    .hint_text("Search parameters")
                    .desired_width(width),
            );
            ui.checkbox(&mut self.show_computed, "computed")
                .on_hover_text("Also list the values formulas set (shown, not edited)");
        });
        let query = self.inspect_query.trim().to_string();
        if !query.is_empty() {
            let found = crate::inspector::search(&m.params, &query);
            ui.weak(match found.len() {
                0 => "Nothing matches".to_string(),
                1 => "1 parameter".to_string(),
                n => format!("{n} parameters"),
            });
            egui::Grid::new("found")
                .num_columns(4)
                .spacing([6.0, 4.0])
                .show(ui, |ui| {
                    for row in found {
                        let mut r = row.clone();
                        r.name = r.key.clone();
                        param_row(ui, &m, &r, Place::Tree, None, edits!());
                    }
                });
        } else {
            let o = crate::inspector::organise(&m.params, &m.file, self.show_computed);
            // What was picked last, on the diagram or in a plot (ADR 0053).
            let canvas = self.diagram.canvas.selected.clone();
            if canvas != self.last_canvas {
                self.last_canvas = canvas.clone();
                let scope = match &self.diagram.scope {
                    Scope::Model => None,
                    Scope::Subsystem(s) => Some(s.clone()),
                };
                self.picked = canvas
                    .map(|s| s.blocks())
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|n| !n.contains(':'))
                    .map(|n| (scope.clone(), n))
                    .collect();
            }
            let picked = self.picked.clone();
            if !picked.is_empty() {
                let title = match picked.as_slice() {
                    [(Some(s), name)] => format!("Selected: {name} in {s}"),
                    [(None, name)] => format!("Selected: {name}"),
                    many => format!("Selected: {} blocks", many.len()),
                };
                let draft = self.diagram.draft_file().cloned();
                egui::CollapsingHeader::new(egui::RichText::new(title).strong())
                    .id_salt("selected")
                    .default_open(true)
                    .show(ui, |ui| {
                        for (scope, name) in &picked {
                            let at = scope.clone().map_or(Scope::Model, Scope::Subsystem);
                            if picked.len() > 1 {
                                ui.strong(name);
                            }
                            let running = block_entry(&m.file, scope.as_deref(), name);
                            let drafted = draft
                                .as_ref()
                                .and_then(|f| block_entry(f, scope.as_deref(), name));
                            let mut shown = false;
                            if let Some(b) = running {
                                if let Some(t) = crate::inspector::selected(
                                    &m.params,
                                    &m.file,
                                    scope.as_deref(),
                                    name,
                                ) {
                                    uses_table(ui, &t, &m.file.controls.params, edits!());
                                    shown = true;
                                }
                                for e in unit_fields(ui, b, &mut self.param_edits) {
                                    canvas_edits.push((at.clone(), e));
                                    shown = true;
                                }
                                shown |= b.type_name == simsandbox_eq::convert::CONVERT;
                            } else if let (Some(b), Some(f)) = (drafted, &draft) {
                                ui.weak(
                                    "Not in the running model yet: it joins when the model loads",
                                )
                                .on_hover_text(self.diagram.draft_problem().unwrap_or_default());
                                let d = Diagram::of(f, &at, &self.registry);
                                if let Some(n) = d.nodes.iter().find(|n| n.name == *name) {
                                    for e in
                                        draft_rows(ui, n, &mut self.param_edits, &mut self.error)
                                    {
                                        canvas_edits.push((at.clone(), e));
                                    }
                                }
                                for e in unit_fields(ui, b, &mut self.param_edits) {
                                    canvas_edits.push((at.clone(), e));
                                }
                                shown = true;
                            }
                            if !shown {
                                ui.weak("Nothing to set");
                            }
                        }
                    });
                ui.add_space(6.0);
            }
            let small = m.params.len() <= 16;
            egui::CollapsingHeader::new(egui::RichText::new("Model").strong())
                .id_salt("model-params")
                .default_open(true)
                .show(ui, |ui| {
                    egui::Grid::new("model")
                        .num_columns(4)
                        .spacing([6.0, 4.0])
                        .show(ui, |ui| {
                            for row in &o.model {
                                let used = o.used_by.get(&row.key);
                                param_row(ui, &m, row, Place::Tree, used, edits!());
                            }
                        });
                    if let Some(e) = declare_form(ui, &mut self.new_param, &mut self.error) {
                        canvas_edits.push((Scope::Model, e));
                    }
                })
                .header_response
                .on_hover_text(
                    "Parameters of the model itself: block values read them with \
                     '= expression'",
                );
            for t in &o.tables {
                let title = format!("{} × {}", t.subsystem, t.uses.len());
                egui::CollapsingHeader::new(egui::RichText::new(title).strong())
                    .id_salt(("uses", &t.subsystem))
                    .default_open(small || t.rows.len() <= 6)
                    .show(ui, |ui| {
                        uses_table(ui, t, &m.file.controls.params, edits!());
                    })
                    .header_response
                    .on_hover_text(format!(
                        "The subsystem {} is used {} times: a column per use",
                        t.subsystem,
                        t.uses.len()
                    ));
            }
            for g in &o.groups {
                let entry = m.file.blocks.iter().find(|b| b.name == g.name);
                let typed = entry
                    .filter(|b| b.type_name == simsandbox_eq::registry::EQUATIONS)
                    .map(|b| b.equations.join("\n"));
                let agents = entry.filter(|b| b.type_name.starts_with("agents."));
                let convert = entry.filter(|b| b.type_name == simsandbox_eq::convert::CONVERT);
                let kind = if g.instance {
                    format!("subsystem {}", g.type_name)
                } else {
                    g.type_name.clone()
                };
                let header = egui::RichText::new(format!("{}  ({kind})", g.name)).strong();
                egui::CollapsingHeader::new(header)
                    .id_salt(("group", &g.name))
                    .default_open(small || g.rows.len() <= 4)
                    .show(ui, |ui| {
                        if agents.is_some() || typed.is_some() {
                            ui.horizontal(|ui| {
                                if let Some(b) = agents {
                                    // The update order is part of the model
                                    // (ADR 0034).
                                    let now = simsandbox_agents::Update::from_setting(
                                        b.update.as_deref(),
                                    )
                                    .unwrap_or_default();
                                    egui::ComboBox::from_id_salt(("update", &g.name))
                                        .selected_text(now.name())
                                        .show_ui(ui, |ui| {
                                            for u in simsandbox_agents::Update::ALL {
                                                if ui
                                                    .selectable_label(u == now, u.name())
                                                    .on_hover_text(u.description())
                                                    .clicked()
                                                    && u != now
                                                {
                                                    update = Some((g.name.clone(), u));
                                                }
                                            }
                                        })
                                        .response
                                        .on_hover_text(
                                            "The order agents act in each step; it changes \
                                         results. Changing it restarts the run.",
                                        );
                                }
                                if let Some(text) = &typed {
                                    if ui
                                        .small_button("Edit equations…")
                                        .on_hover_text("Change this block's equations")
                                        .clicked()
                                    {
                                        open_editor = Some((g.name.clone(), text.clone()));
                                    }
                                }
                            });
                        }
                        if let Some(b) = convert {
                            for e in unit_fields(ui, b, &mut self.param_edits) {
                                canvas_edits.push((Scope::Model, e));
                            }
                        }
                        egui::Grid::new(("params", &g.name))
                            .num_columns(4)
                            .spacing([6.0, 4.0])
                            .show(ui, |ui| {
                                // Starting values come last, each marked ↻.
                                for (name, row) in &g.rows {
                                    let mut r = row.clone();
                                    r.name = name.clone();
                                    let used = o.used_by.get(&row.key);
                                    param_row(ui, &m, &r, Place::Tree, used, edits!());
                                }
                            });
                    });
            }
            // Blocks added on the diagram that wait, as a draft, for the
            // model to load.
            if let Some(f) = self.diagram.draft_file().cloned() {
                let d = Diagram::of(&f, &Scope::Model, &self.registry);
                for n in d
                    .nodes
                    .iter()
                    .filter(|n| n.is_block() && !m.file.blocks.iter().any(|b| b.name == n.name))
                {
                    let header = egui::RichText::new(format!(
                        "{}  ({}, not running yet)",
                        n.name, n.type_name
                    ))
                    .strong();
                    egui::CollapsingHeader::new(header)
                        .id_salt(("draft", &n.name))
                        .default_open(true)
                        .show(ui, |ui| {
                            for e in draft_rows(ui, n, &mut self.param_edits, &mut self.error) {
                                canvas_edits.push((Scope::Model, e));
                            }
                            if let Some(b) = f.blocks.iter().find(|b| b.name == n.name) {
                                for e in unit_fields(ui, b, &mut self.param_edits) {
                                    canvas_edits.push((Scope::Model, e));
                                }
                            }
                        })
                        .header_response
                        .on_hover_text(format!(
                            "Added on the diagram; it joins the run once the model loads: {}",
                            self.diagram.draft_problem().unwrap_or_default()
                        ));
                }
            }
            if o.computed > 0 && !self.show_computed {
                ui.add_space(4.0);
                ui.weak(format!(
                    "{} values set by formulas are not listed (tick \"computed\")",
                    o.computed
                ));
            }
        }
        if let Some((block, text)) = open_editor {
            self.editor = Some(Editor {
                block: Some(block),
                text,
                error: None,
            });
        }

        self.apply_edits(changes, controls);
        for (scope, e) in canvas_edits {
            self.edit_at(scope, e);
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
            let mut entries = ed.entries();
            // Filters keep their place on the canvas (ADR 0050).
            for e in &mut entries {
                e.canvas = live
                    .model()
                    .filters
                    .iter()
                    .find(|f| f.name == e.name)
                    .and_then(|f| f.canvas);
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
    fn apply_equations(&mut self) {
        let Some(editor) = self.editor.as_ref() else {
            return;
        };
        let lines: Vec<String> = editor.text.lines().map(str::to_string).collect();
        let block = editor.block.clone();
        // A new model replaces the open one.
        if block.is_none() && self.ask_first(Discard::NewModel) {
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
                    self.mark_saved();
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
        // `subsystem::block` edits a block inside a subsystem (ADR 0043).
        let (def, inner) = match block.split_once("::") {
            Some((d, b)) => (Some(d.to_string()), b),
            None => (None, block),
        };
        let set = simsandbox_eq::EquationSet::parse(inner, &lines).map_err(|e| e.to_string())?;
        let keep: Vec<String> = simsandbox_eq::OdeSystem::spec(&set)
            .params
            .iter()
            .map(|p| p.name.clone())
            .collect();
        let text = self.saved_text()?;
        let (mut file, _) =
            simsandbox_file::ModelFile::from_json(&text).map_err(|e| e.to_string())?;
        let blocks = match &def {
            Some(d) => {
                &mut file
                    .subsystems
                    .get_mut(d)
                    .ok_or_else(|| format!("no subsystem '{d}'"))?
                    .blocks
            }
            None => &mut file.blocks,
        };
        let entry = blocks
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
        self.guard_close(&ctx);
        self.unsaved_dialog(&ctx);
        self.run_dialog(&ctx);
        self.check_unsaved();

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
        egui::CentralPanel::default().show(ui, |ui| self.docked(ui));

        self.handle_screenshot(&ctx);
    }
}

/// What a tab asked for, carried out once the dock is drawn.
#[derive(Clone, Debug)]
enum DockAction {
    PopOut(Pane),
    DockBack(Pane),
    /// Close a view.
    Close(u64),
    OwnTab(u64),
    IntoGroup(u64),
    TileAll,
    Select(u64),
    /// Show where a clicked curve comes from.
    Inspect(String),
    /// A view's pointer for an `input.pointer` block: `[x, y, down]`.
    Pointer(String, [f64; 3]),
}

/// A tab in its own window, by what it shows (tiles are renumbered when
/// the model is rebuilt).
enum Popped {
    Controls,
    Inspector,
    Diagram,
    /// The tab showing this view (by place among the views).
    View(usize),
}

/// The window of a popped-out tab.
fn popped_viewport(p: &Pane) -> egui::ViewportId {
    egui::ViewportId::from_hash_of(("popped", pane_key(p)))
}

/// A tab's stable key, for ids.
fn pane_key(p: &Pane) -> String {
    match p {
        Pane::Controls => "controls".into(),
        Pane::Inspector => "inspector".into(),
        Pane::Diagram => "diagram".into(),
        Pane::View(t) => format!("view:{t}"),
        Pane::Views { id, .. } => format!("views:{id}"),
    }
}

/// The dock's view of the app: each tab drawn by the app.
struct Tabs<'a> {
    app: &'a mut SimApp,
    actions: Vec<DockAction>,
}

impl egui_dock::TabViewer for Tabs<'_> {
    type Tab = Pane;

    fn id(&mut self, tab: &mut Pane) -> egui::Id {
        egui::Id::new(("tab", pane_key(tab)))
    }

    fn title(&mut self, tab: &mut Pane) -> egui::WidgetText {
        self.app.pane_title(tab).into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Pane) {
        let pane = tab.clone();
        self.app.pane_ui(ui, &pane, &mut self.actions);
    }

    fn context_menu(&mut self, ui: &mut egui::Ui, tab: &mut Pane, _path: egui_dock::NodePath) {
        if ui
            .button("Pop out")
            .on_hover_text("Show it in a window of its own")
            .clicked()
        {
            self.actions.push(DockAction::PopOut(tab.clone()));
            ui.close();
        }
        match tab {
            Pane::View(t) => {
                if ui.button("Tile with the other views").clicked() {
                    self.actions.push(DockAction::IntoGroup(*t));
                    ui.close();
                }
            }
            Pane::Views { .. }
                if ui
                    .button("Tile all views here")
                    .on_hover_text("Bring every view back into one tab")
                    .clicked() =>
            {
                self.actions.push(DockAction::TileAll);
                ui.close();
            }
            _ => {}
        }
    }

    fn is_closeable(&self, tab: &Pane) -> bool {
        matches!(tab, Pane::View(_))
    }

    fn on_close(&mut self, tab: &mut Pane) -> egui_dock::widgets::tab_viewer::OnCloseResponse {
        if let Pane::View(t) = tab {
            self.actions.push(DockAction::Close(*t));
        }
        egui_dock::widgets::tab_viewer::OnCloseResponse::Close
    }

    fn allowed_in_windows(&self, _tab: &mut Pane) -> bool {
        // Tabs leave the window with "Pop out", as windows of their own.
        false
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
    /// Formulas, and numbers in place of them: file edits (ADR 0056).
    file: &'a mut Vec<(Scope, Edit)>,
}

/// Parameters side by side for several uses of a subsystem (or one use):
/// a row per parameter, a column per use. Values formulas set are shown,
/// not edited; right-click a name to give every use the first one's value.
fn uses_table(
    ui: &mut egui::Ui,
    t: &crate::inspector::UsesTable,
    pins: &BTreeMap<String, simsandbox_file::ParamControl>,
    edits: Edits<'_>,
) {
    let Edits {
        changes,
        controls,
        text,
        error,
        file,
        ..
    } = edits;
    egui::Grid::new(("uses-table", &t.subsystem, t.uses.join(",")))
        .num_columns(t.uses.len() + 2)
        .spacing([8.0, 4.0])
        .striped(true)
        .show(ui, |ui| {
            if t.uses.len() > 1 {
                ui.label("");
                ui.label("");
                for u in &t.uses {
                    ui.strong(u);
                }
                ui.end_row();
            }
            for (name, cells) in &t.rows {
                let first = cells.iter().flatten().next();
                let starting = first.is_some_and(|r| r.initial_condition);
                let label = if starting {
                    format!("{name} ↻")
                } else {
                    name.clone()
                };
                let hover = first.map_or(String::new(), |r| {
                    let mut h = r.description.clone();
                    if starting {
                        h.push_str("\nSets the initial state: applies on Restart");
                    }
                    h
                });
                let first_value = cells
                    .iter()
                    .flatten()
                    .find(|r| r.formula.is_none())
                    .map(|r| r.value);
                let response = ui
                    .label(label)
                    .on_hover_text(format!("{hover}\nRight-click to pin or set every use"));
                response.context_menu(|ui| {
                    if let Some(v) = first_value.filter(|_| t.uses.len() > 1) {
                        if ui
                            .button(format!("Set every use to {}", format_value(v)))
                            .clicked()
                        {
                            for r in cells.iter().flatten().filter(|r| r.formula.is_none()) {
                                changes.push((r.clone(), v));
                            }
                            ui.close();
                        }
                    }
                    for r in cells.iter().flatten() {
                        let mut c = pins.get(&r.key).cloned().unwrap_or_default();
                        let pinned = c.pinned();
                        let label = match pinned {
                            true => format!("Unpin {} from the Controls", r.key),
                            false => format!("Pin {} to the Controls", r.key),
                        };
                        if ui.button(label).clicked() {
                            c.set_pinned(!pinned);
                            controls.push((r.key.clone(), c));
                            ui.close();
                        }
                    }
                });
                ui.weak(first.map_or("", |r| r.unit.as_str()));
                for cell in cells {
                    match cell {
                        Some(r) => {
                            let key = format!("cell:{}", r.key);
                            let f = r.formula.as_deref();
                            if let Some(t) = value_entry(ui, &key, r.value, f, text, error) {
                                apply_typed(r, t, changes, file, error);
                            }
                        }
                        None => {
                            ui.weak("—");
                        }
                    }
                }
                ui.end_row();
            }
        });
}

/// A view's style properties (ADR 0015), only those that mean something
/// for what it shows (ADR 0025), and its own picks such as profile cuts
/// (ADR 0029).
fn style_editor(
    ui: &mut egui::Ui,
    t: &ViewType,
    state: &mut ViewState,
    history: &History,
    tile: u64,
    text: &mut HashMap<String, String>,
    error: &mut Option<String>,
) {
    let context = simsandbox_views::Context {
        history,
        bind: &state.bind,
    };
    let shown: Vec<_> = t.style.iter().filter(|p| (p.applies)(&context)).collect();
    state.edit_selection(ui, t, history);
    if shown.is_empty() {
        ui.weak("Nothing to set for this view");
        return;
    }
    egui::Grid::new(("style", t.name, tile))
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
                if let Some(v) = number_entry(ui, &key, value, text, error) {
                    state.style.insert(p.name.into(), v);
                }
                ui.label(p.unit);
                ui.end_row();
            }
        });
}

/// Where a parameter row is drawn.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    /// The inspector's tree: a pin button.
    Tree,
    /// The Controls: the slider toggle and the pin button.
    Controls,
}

/// One parameter: name, typed entry, unit (↻ for a starting value), the
/// slider toggle among the Controls and a pin button; then the slider and
/// its range when enabled. `used_by` lists the formulas that read it.
fn param_row(
    ui: &mut egui::Ui,
    m: &Model,
    row: &ParamRow,
    place: Place,
    used_by: Option<&Vec<String>>,
    edits: Edits<'_>,
) {
    let mut control = m
        .file
        .controls
        .params
        .get(&row.key)
        .cloned()
        .unwrap_or_default();
    let before = control.clone();
    let mut hover = format!(
        "{}\nValue: {}\nSuggested range: {} to {}",
        row.description, row.value, row.suggested[0], row.suggested[1]
    );
    if row.initial_condition {
        hover.push_str("\nSets the initial state: applies on Restart");
    }
    if let Some(f) = &row.formula {
        hover.push_str(&format!("\nComputed: {f}"));
    }
    hover.push_str("\nType a number, or '= expression' of the parameters for a formula");
    if let Some(v) = used_by {
        hover.push_str(&format!("\nRead by formulas: {}", v.join(", ")));
    }
    ui.label(&row.name).on_hover_text(hover).context_menu(|ui| {
        let label = if control.pinned() {
            "Unpin from the Controls"
        } else {
            "Pin to the Controls"
        };
        if ui.button(label).clicked() {
            let pinned = control.pinned();
            control.set_pinned(!pinned);
            ui.close();
        }
        // The model's own parameters can go (ADR 0056); formulas reading
        // one keep it.
        if row.owner.is_empty()
            && row.block.is_none()
            && ui
                .button("Remove this parameter")
                .on_hover_text("Refused while a formula reads it")
                .clicked()
        {
            edits.file.push((
                Scope::Model,
                Edit::RemoveParam {
                    name: row.key.clone(),
                },
            ));
            ui.close();
        }
    });

    // A value a formula sets shows the formula, which can be edited or
    // replaced by a number (ADR 0056). While a slider is dragged on a model
    // that is not real-time, its field shows the value that applies on
    // release (ADR 0037).
    let formula = row.formula.as_deref();
    let typed = match edits.pending.get(&row.key) {
        Some(p) => {
            pending_field(ui, *p);
            None
        }
        None => value_entry(ui, &row.key, row.value, formula, edits.text, edits.error),
    };
    if let Some(t) = typed {
        apply_typed(row, t, edits.changes, edits.file, edits.error);
    }

    ui.horizontal(|ui| {
        if formula.is_some() {
            ui.weak(format_value(row.value))
                .on_hover_text("Its value now, from the formula");
        }
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
    ui.horizontal(|ui| {
        if place == Place::Controls && formula.is_none() {
            ui.checkbox(&mut control.slider, "")
                .on_hover_text("A slider under the value, between the two ends of its range");
        }
        pin_button(ui, &mut control);
    });
    ui.end_row();

    // The slider between its range's ends, which are typed in place.
    if place == Place::Controls && control.slider && formula.is_none() {
        let [mut lo, mut hi] = control.range.unwrap_or(row.suggested);
        let a = ui
            .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(egui::DragValue::new(&mut lo).speed(0.1))
                    .on_hover_text("The slider's lower end")
            })
            .inner;
        let mut v = edits.pending.get(&row.key).copied().unwrap_or(row.value);
        let r = ui.add(egui::Slider::new(&mut v, lo..=hi).show_value(false));
        if apply_now(m.real_time, r.changed(), r.dragged(), r.drag_stopped()) {
            edits.pending.remove(&row.key);
            edits.changes.push((row.clone(), v));
        } else if r.changed() {
            edits.pending.insert(row.key.clone(), v);
        }
        let b = ui
            .add(egui::DragValue::new(&mut hi).speed(0.1))
            .on_hover_text("The slider's upper end");
        if (a.changed() || b.changed()) && lo < hi {
            control.range = Some([lo, hi]);
        }
        ui.end_row();
    }
    if control != before {
        edits.controls.push((row.key.clone(), control));
    }
}

/// The pin: highlighted while the value is among the Controls.
fn pin_button(ui: &mut egui::Ui, control: &mut simsandbox_file::ParamControl) {
    let pinned = control.pinned();
    let text = if pinned {
        egui::RichText::new("📌")
    } else {
        egui::RichText::new("📌").weak()
    };
    let hover = if pinned {
        "Pinned: shown in the Controls. Click to unpin."
    } else {
        "Pin to the Controls, where it can also have a slider"
    };
    if ui
        .add(
            egui::Button::new(text)
                .small()
                .frame(pinned)
                .selected(pinned),
        )
        .on_hover_text(hover)
        .clicked()
    {
        control.set_pinned(!pinned);
    }
}

/// The block named `name` in a file, on the model level or inside a
/// subsystem.
fn block_entry<'a>(
    file: &'a simsandbox_file::ModelFile,
    scope: Option<&str>,
    name: &str,
) -> Option<&'a simsandbox_file::BlockEntry> {
    let blocks = match scope {
        None => &file.blocks,
        Some(s) => &file.subsystems.get(s)?.blocks,
    };
    blocks.iter().find(|b| b.name == name)
}

/// A conversion block's `from` and `to` units (ADR 0045) as text fields;
/// a changed one is an edit, which restarts the run. Nothing for other
/// blocks.
fn unit_fields(
    ui: &mut egui::Ui,
    b: &simsandbox_file::BlockEntry,
    edits: &mut HashMap<String, String>,
) -> Vec<Edit> {
    let mut out = Vec::new();
    if b.type_name != simsandbox_eq::convert::CONVERT {
        return out;
    }
    ui.horizontal(|ui| {
        ui.label("converts")
            .on_hover_text("Connections need the same unit on both ends; this block converts");
        for (k, key) in ["from", "to"].into_iter().enumerate() {
            if k == 1 {
                ui.label("to");
            }
            let now = b.settings.get(key).cloned().unwrap_or_else(|| "1".into());
            let id = format!("convert:{}:{key}", b.name);
            let mut text = edits.get(&id).cloned().unwrap_or_else(|| now.clone());
            let r = ui
                .add(egui::TextEdit::singleline(&mut text).desired_width(70.0))
                .on_hover_text("A unit, e.g. km/h, m/s, mm, N*s/m; changing it restarts the run");
            if r.has_focus() {
                edits.insert(id.clone(), text.clone());
            }
            if r.lost_focus() {
                edits.remove(&id);
                let typed = text.trim().to_string();
                if !typed.is_empty() && typed != now {
                    out.push(Edit::SetSetting {
                        block: b.name.clone(),
                        key: key.to_string(),
                        text: typed,
                    });
                }
            }
        }
    });
    out
}

/// The values of a block not in the running model yet (the diagram's
/// draft), typed into the draft.
fn draft_rows(
    ui: &mut egui::Ui,
    n: &simsandbox_session::diagram::Node,
    text: &mut HashMap<String, String>,
    error: &mut Option<String>,
) -> Vec<Edit> {
    let mut out = Vec::new();
    egui::Grid::new(("draft-rows", &n.name))
        .num_columns(3)
        .spacing([6.0, 4.0])
        .show(ui, |ui| {
            for v in &n.values {
                let label = if v.initial {
                    format!("{} ↻", v.name)
                } else {
                    v.name.clone()
                };
                ui.label(label);
                let key = format!("draft:{}", v.key);
                let (block, param) = (n.name.clone(), v.name.clone());
                match value_entry(ui, &key, v.value, None, text, error) {
                    Some(Typed::Number(value)) => out.push(Edit::SetParam {
                        block,
                        param,
                        value,
                    }),
                    Some(Typed::Formula(formula)) => out.push(Edit::SetFormula {
                        block,
                        param,
                        formula,
                    }),
                    None => {}
                }
                ui.label(&v.unit);
                ui.end_row();
            }
        });
    out
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

/// "+ parameter": a name, a value and a unit, declared as a parameter of
/// the model (ADR 0056).
fn declare_form(
    ui: &mut egui::Ui,
    form: &mut [String; 3],
    error: &mut Option<String>,
) -> Option<Edit> {
    let mut add = false;
    ui.horizontal(|ui| {
        let [name, value, unit] = form;
        for (field, hint, width) in [
            (name, "name", 70.0),
            (value, "value", 60.0),
            (unit, "unit", 50.0),
        ] {
            let r = ui.add(
                egui::TextEdit::singleline(field)
                    .hint_text(hint)
                    .desired_width(width),
            );
            add |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        }
        add |= ui
            .small_button("+ parameter")
            .on_hover_text(
                "Declare a parameter of the model; type '= name' in a block's value to use it",
            )
            .clicked();
    });
    if !add || form[0].trim().is_empty() {
        return None;
    }
    let value = if form[1].trim().is_empty() {
        Ok(0.0)
    } else {
        form[1].trim().parse::<f64>()
    };
    match value {
        Ok(value) if value.is_finite() => {
            let e = Edit::DeclareParam {
                name: form[0].trim().to_string(),
                value,
                unit: form[2].trim().to_string(),
            };
            *form = Default::default();
            Some(e)
        }
        _ => {
            *error = Some(format!("'{}' is not a number", form[1].trim()));
            None
        }
    }
}

/// What was typed into a parameter's field (ADR 0056).
#[derive(Debug, Clone, PartialEq)]
enum Typed {
    Number(f64),
    /// `= expression`, without the `=`.
    Formula(String),
}

/// A parameter's field: a number, or `= expression` for a value a formula
/// sets. Like [`number_entry`], it returns only what was typed anew.
fn value_entry(
    ui: &mut egui::Ui,
    key: &str,
    value: f64,
    formula: Option<&str>,
    edits: &mut HashMap<String, String>,
    error: &mut Option<String>,
) -> Option<Typed> {
    let shown = match formula {
        Some(f) => format!("= {f}"),
        None => format_value(value),
    };
    let mut text = edits.get(key).cloned().unwrap_or_else(|| shown.clone());
    let r = ui
        .add(egui::TextEdit::singleline(&mut text).desired_width(90.0))
        .on_hover_text("A number, or '= expression' of the model's parameters (e.g. '= 2*kp')");
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
    if let Some(f) = typed.strip_prefix('=') {
        let f = f.trim();
        return (Some(f) != formula).then(|| Typed::Formula(f.to_string()));
    }
    match typed.parse::<f64>() {
        Ok(v) if v.is_finite() => Some(Typed::Number(v)),
        _ => {
            *error = Some(format!(
                "{key}: '{typed}' is not a number (or '= expression' for a formula)"
            ));
            None
        }
    }
}

/// Where a value's formula is written (ADR 0056): the block, or subsystem
/// use, on the model level, and the parameter. The model's own parameters
/// and values inside uses are numbers.
fn formula_target(row: &ParamRow) -> Result<(String, String), String> {
    if row.owner.is_empty() {
        return Err(format!(
            "{}: a parameter of the model is a number; formulas go in block values, which \
             can read it",
            row.key
        ));
    }
    if row.owner.contains('/') {
        return Err(format!(
            "{}: a value inside a subsystem use is a number; open the subsystem to give every \
             use a formula, or declare a subsystem parameter",
            row.key
        ));
    }
    let param = row
        .key
        .strip_prefix(&format!("{}.", row.owner))
        .unwrap_or(&row.name);
    Ok((row.owner.clone(), param.to_string()))
}

/// Carry out what was typed into a row: a number on a value without a
/// formula applies at once; a formula, or a number in place of one,
/// changes the file (and restarts).
fn apply_typed(
    row: &ParamRow,
    typed: Typed,
    changes: &mut Vec<(ParamRow, f64)>,
    file: &mut Vec<(Scope, Edit)>,
    error: &mut Option<String>,
) {
    if let (Typed::Number(v), None) = (&typed, &row.formula) {
        changes.push((row.clone(), *v));
        return;
    }
    match formula_target(row) {
        Ok((block, param)) => file.push((
            Scope::Model,
            match typed {
                Typed::Number(value) => Edit::SetParam {
                    block,
                    param,
                    value,
                },
                Typed::Formula(formula) => Edit::SetFormula {
                    block,
                    param,
                    formula,
                },
            },
        )),
        Err(e) => *error = Some(e),
    }
}

/// Width of one slot picker; pickers are laid out in rows of as many as
/// fit the tile.
const SLOT_WIDTH: f32 = 150.0;

/// What decides the views a sweep can show: the swept parameters, whether
/// they form a two-parameter grid, and whether samples are drawn as points.
fn structure(sw: &simsandbox_session::session::Sweep) -> (Vec<String>, bool, bool) {
    (sw.keys().to_vec(), sw.is_grid(), sw.unordered())
}

/// How a model with no states is swept (ADR 0028): the design, then one
/// row per parameter with its range and, on a grid, its number of values.
/// Returns the notes of an applied change, or why it was refused.
fn sweep_controls(
    ui: &mut egui::Ui,
    live: &Live,
    m: &Model,
    sw: &simsandbox_session::session::Sweep,
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
            canvas: None,
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

    let inputs = simsandbox_session::session::cut_inputs(history);
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
        let g = simsandbox_session::session::cut_geometry(history, &c.input);
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
        let entry =
            simsandbox_session::session::default_cut(history, &format!("cuts{n}"), &inputs[0]);
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
# x and v are states: where they start, then how they change.
state x = 1
state v = 0
dx/dt = v
dv/dt = -k*x - c*v
# a quantity computed from the states, plotted like them
energy = 0.5*v^2 + 0.5*k*x^2
";

const SYNTAX_HELP: &str = "\
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

/// A compact display of a value: up to 6 significant digits.
fn format_value(v: f64) -> String {
    simsandbox_session::canvas::format_number(v)
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

/// The run files beside `model` that name it (ADR 0059), by name.
fn runs_beside(model: &Path) -> Vec<(PathBuf, simsandbox_file::runs::RunFile)> {
    let Some(model_name) = model.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return Vec::new();
    };
    let dir = match model.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut runs: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().ends_with(".run.json"))
        .filter_map(|p| {
            let text = std::fs::read_to_string(&p).ok()?;
            let run = simsandbox_file::runs::RunFile::from_json(&text).ok()?;
            (run.model == model_name).then_some((p, run))
        })
        .collect();
    runs.sort_by(|a, b| a.1.name.cmp(&b.1.name).then(a.0.cmp(&b.0)));
    runs
}

/// Whether the model saved at `model` is the one `run` was made with.
fn run_fits_disk(run: &simsandbox_file::runs::RunFile, model: &Path, registry: &Registry) -> bool {
    std::fs::read_to_string(model)
        .ok()
        .and_then(|t| simsandbox_file::load(&t, registry).ok())
        .and_then(|l| run.fits(&l).ok())
        .unwrap_or(false)
}

/// Now as RFC 3339 in UTC, to the second.
fn utc_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
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

    /// Apply canvas edits to the open model as the Diagram tab does.
    fn canvas_edits(app: &mut SimApp, edits: Vec<simsandbox_session::diagram::Edit>) {
        let live = app.session.as_ref().unwrap();
        let m = live.model();
        let (speed, views, layout) = (app.speed, app.views.entries(), app.layout());
        let current = || {
            let (views, layout) = (views.clone(), layout.clone());
            live.call(move |s, _| s.save_json_with(speed, Some(views), Some(layout)))?
        };
        let outcomes = app.diagram.handle(edits, &app.registry, &current);
        app.diagram_outcomes(&m, outcomes);
    }

    /// Undo (`back`) or redo a canvas edit, as Ctrl+Z and Ctrl+Shift+Z do.
    fn canvas_step(app: &mut SimApp, back: bool) {
        let live = app.session.as_ref().unwrap();
        let m = live.model();
        let (speed, views, layout) = (app.speed, app.views.entries(), app.layout());
        let current = || {
            let (views, layout) = (views.clone(), layout.clone());
            live.call(move |s, _| s.save_json_with(speed, Some(views), Some(layout)))?
        };
        let outcomes = app.diagram.step(back, &app.registry, &current);
        app.diagram_outcomes(&m, outcomes);
    }

    #[test]
    fn the_arrangement_is_saved_and_survives_rebuilds() {
        use simsandbox_session::diagram::Edit;
        let mut app = SimApp::new(None, None, None, None);
        let e = EXAMPLES
            .iter()
            .find(|e| e.name.contains("oscillators"))
            .unwrap();
        app.open_text(e.name, e.text);
        // The standard arrangement is not written.
        assert!(!app.saved_text().unwrap().contains("\"layout\""));
        let tiles = app.tile_ids();
        assert_eq!(tiles.len(), 3);
        // The second view in a tab of its own; the diagram in a window.
        app.dock.own_tab(tiles[1]);
        app.dock.pop_out(&Pane::Diagram);
        let text = app.saved_text().unwrap();
        let (f, _) = simsandbox_file::ModelFile::from_json(&text).unwrap();
        let l = f.controls.layout.clone().expect("saved");
        let panes: Vec<String> = l.panes().iter().map(|p| format!("{p:?}")).collect();
        assert!(panes.iter().any(|p| p.contains("view:1")), "{panes:?}");
        assert!(panes.iter().any(|p| p.contains("diagram")), "{panes:?}");
        // Drawn, the popped-out diagram is a window of its own (here, with
        // no native windows, one inside the app).
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 700.0),
            )),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| app.docked(ui));
        out.textures_delta.clear();
        assert_eq!(app.dock.popped, [Pane::Diagram]);
        // A canvas edit rebuilds the model: the window and the tab stay.
        canvas_edits(
            &mut app,
            vec![Edit::Rename {
                from: "b".into(),
                to: "c".into(),
            }],
        );
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_eq!(app.dock.popped, [Pane::Diagram]);
        assert!(app.dock.panes().iter().any(|p| matches!(p, Pane::View(_))));
        // Opened again, the view has its own tab and the diagram is docked.
        app.open_text("again", &text);
        assert!(app.dock.popped.is_empty());
        assert!(app.dock.panes().iter().any(|p| matches!(p, Pane::View(_))));
        assert!(app.dock.panes().contains(&Pane::Diagram));
    }

    /// Draw one frame of the inspector.
    fn inspect(app: &mut SimApp) {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(500.0, 900.0),
            )),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| app.param_panel(ui));
        out.textures_delta.clear();
    }

    #[test]
    fn a_value_typed_on_a_node_applies_to_the_running_model() {
        let mut app = SimApp::new(None, None, None, None);
        let e = EXAMPLES.iter().find(|e| e.name.contains("Cruise")).unwrap();
        app.open_text(e.name, e.text);
        let live = app.session.as_ref().unwrap();
        live.call(|s, _| s.run_until(2.0)).unwrap().unwrap();
        let m = live.model();
        app.diagram_outcomes(
            &m,
            vec![Outcome::SetValue {
                key: "car.m".into(),
                value: 1500.0,
            }],
        );
        let live = app.session.as_ref().unwrap();
        live.sync();
        let m = live.model();
        let row = m.params.iter().find(|r| r.key == "car.m").unwrap();
        assert_eq!(row.value, 1500.0);
        // No restart.
        assert!(live.status().time > 1.9);
    }

    #[test]
    fn blocks_added_on_the_diagram_are_set_in_the_inspector_before_they_run() {
        use simsandbox_session::canvas::Selection;
        let mut app = SimApp::new(None, None, None, None);
        let e = EXAMPLES
            .iter()
            .find(|e| e.name.contains("Platoon"))
            .unwrap();
        app.open_text(e.name, e.text);
        // A conversion with nothing wired in does not load: a draft.
        for type_name in ["units.convert", "eq.pendulum"] {
            let add = Edit::Add {
                type_name: type_name.into(),
                at: [0.0, 0.0],
            };
            app.edit_at(Scope::Model, add);
        }
        assert!(app.diagram.draft_problem().is_some());
        // Its units and the new pendulum's values go into the draft.
        app.edit_at(
            Scope::Model,
            Edit::SetSetting {
                block: "convert1".into(),
                key: "from".into(),
                text: "km/h".into(),
            },
        );
        app.edit_at(
            Scope::Model,
            Edit::SetParam {
                block: "pendulum1".into(),
                param: "L".into(),
                value: 2.0,
            },
        );
        let draft = app.diagram.draft_file().unwrap();
        let block = |name: &str| draft.blocks.iter().find(|b| b.name == name).unwrap();
        assert_eq!(block("convert1").settings["from"], "km/h");
        assert_eq!(
            block("pendulum1").params["L"],
            simsandbox_file::ParamValue::Number(2.0)
        );
        // Both selected: the inspector shows each, from the draft.
        app.diagram.canvas.selected = Some(Selection::Many(vec![
            "convert1".into(),
            "pendulum1".into(),
            "leader".into(),
        ]));
        inspect(&mut app);
        assert_eq!(app.picked.len(), 3);
    }

    #[test]
    fn open_nodes_are_saved_and_inner_values_are_every_uses_default() {
        let mut app = SimApp::new(None, None, None, None);
        let e = EXAMPLES
            .iter()
            .find(|e| e.name.contains("Platoon"))
            .unwrap();
        app.open_text(e.name, e.text);
        let m = app.session.as_ref().unwrap().model();
        let open = vec!["car1".to_string(), "follower::car".to_string()];
        app.diagram_outcomes(&m, vec![Outcome::Opened(open.clone())]);
        let text = app.saved_text().unwrap();
        let (file, _) = simsandbox_file::ModelFile::from_json(&text).unwrap();
        assert_eq!(file.controls.open, open);
        // Opened again, the diagram opens the same nodes.
        let mut again = SimApp::new(None, None, None, None);
        again.open_text("again", &text);
        assert_eq!(again.diagram.canvas.open, open.into_iter().collect());
        // Inside the subsystem, a value is the default of every use.
        let e = Edit::SetParam {
            block: "car".into(),
            param: "c".into(),
            value: 80.0,
        };
        app.edit_at(Scope::Subsystem("follower".into()), e);
        let live = app.session.as_ref().unwrap();
        live.sync();
        let m = live.model();
        for car in ["car1", "car2", "car3"] {
            let key = format!("{car}/car.c");
            let row = m.params.iter().find(|r| r.key == key).unwrap();
            assert_eq!(row.value, 80.0, "{key}");
        }
    }

    #[test]
    fn values_take_formulas_and_the_model_declares_parameters() {
        let mut app = SimApp::new(None, None, None, None);
        let e = EXAMPLES
            .iter()
            .find(|e| e.name.contains("Platoon"))
            .unwrap();
        app.open_text(e.name, e.text);
        let row = |app: &SimApp, key: &str| {
            let m = app.session.as_ref().unwrap().model();
            m.params.iter().find(|r| r.key == key).unwrap().clone()
        };
        // Type into a row as the inspector does, then carry out file edits.
        let type_in = |app: &mut SimApp, key: &str, typed: Typed| {
            let r = row(app, key);
            let (mut changes, mut file) = (Vec::new(), Vec::new());
            app.error = None;
            apply_typed(&r, typed, &mut changes, &mut file, &mut app.error);
            app.apply_changes(changes);
            for (scope, e) in file {
                app.edit_at(scope, e);
            }
            app.session.as_ref().unwrap().sync();
        };
        let declare = Edit::DeclareParam {
            name: "tau".into(),
            value: 8.0,
            unit: "s".into(),
        };
        app.edit_at(Scope::Model, declare);
        assert_eq!(row(&app, "tau").value, 8.0);
        // A block's value reads it.
        type_in(&mut app, "leader.T", Typed::Formula("tau / 2".into()));
        let t = row(&app, "leader.T");
        assert_eq!((t.value, t.formula.as_deref()), (4.0, Some("tau / 2")));
        // A formula that does not work is refused; the model stays.
        type_in(&mut app, "leader.T", Typed::Formula("kp".into()));
        assert!(app.error.as_deref().unwrap().starts_with("Not applied"));
        type_in(&mut app, "leader.T", Typed::Formula("nope".into()));
        assert!(app.error.as_deref().unwrap().contains("nope"));
        assert!(app.diagram.draft_problem().is_none());
        assert_eq!(row(&app, "leader.T").formula.as_deref(), Some("tau / 2"));
        // A use's own value takes a formula too; the model's parameters and
        // values inside uses stay numbers.
        type_in(&mut app, "car3.mass", Typed::Formula("2*1000".into()));
        assert_eq!(row(&app, "car3.mass").value, 2000.0);
        type_in(&mut app, "kp", Typed::Formula("2".into()));
        assert!(app.error.as_deref().unwrap().contains("is a number"));
        type_in(&mut app, "car1/car.c", Typed::Formula("kp".into()));
        assert!(app.error.as_deref().unwrap().contains("open the subsystem"));
        // A parameter a formula reads cannot go.
        app.edit_at(Scope::Model, Edit::RemoveParam { name: "tau".into() });
        assert!(app.error.take().is_some());
        // A number replaces the formula; then the parameter can go.
        type_in(&mut app, "leader.T", Typed::Number(6.0));
        let t = row(&app, "leader.T");
        assert_eq!((t.value, t.formula), (6.0, None));
        app.edit_at(Scope::Model, Edit::RemoveParam { name: "tau".into() });
        let m = app.session.as_ref().unwrap().model();
        assert!(m.params.iter().all(|r| r.key != "tau"));
    }

    #[test]
    fn a_clicked_curve_selects_its_block_until_the_diagram_picks_another() {
        use simsandbox_session::canvas::Selection;
        let mut app = SimApp::new(None, None, None, None);
        let e = EXAMPLES
            .iter()
            .find(|e| e.name.contains("Platoon"))
            .unwrap();
        app.open_text(e.name, e.text);
        inspect(&mut app);
        // A curve inside a subsystem use selects the use.
        app.inspect_signal("car2/car.v");
        assert_eq!(app.picked, [(None, "car2".to_string())]);
        assert_eq!(
            app.diagram.canvas.selected,
            Some(Selection::Block("car2".into()))
        );
        inspect(&mut app);
        assert_eq!(app.picked, [(None, "car2".to_string())]);
        // Selecting on the diagram takes over.
        app.diagram.canvas.selected = Some(Selection::Block("leader".into()));
        inspect(&mut app);
        assert_eq!(app.picked, [(None, "leader".to_string())]);
        // A sweep's grid comes from the swept block; a filter's signal has
        // none.
        app.inspect_signal("grid.leader.v");
        assert_eq!(app.picked, [(None, "leader".to_string())]);
        app.inspect_signal("energy");
        assert_eq!(app.picked, [(None, "leader".to_string())]);
    }

    #[test]
    fn the_inspector_draws_selection_search_and_computed_values() {
        use simsandbox_session::canvas::Selection;
        use simsandbox_session::diagram::Scope;
        let mut app = SimApp::new(None, None, None, None);
        let e = EXAMPLES
            .iter()
            .find(|e| e.name.contains("Platoon"))
            .unwrap();
        app.open_text(e.name, e.text);
        inspect(&mut app);
        app.diagram.canvas.selected = Some(Selection::Block("car2".into()));
        inspect(&mut app);
        app.diagram.scope = Scope::Subsystem("follower".into());
        app.diagram.canvas.selected = Some(Selection::Block("control".into()));
        inspect(&mut app);
        app.show_computed = true;
        app.inspect_query = "mass".into();
        inspect(&mut app);
        assert!(app.error.is_none(), "{:?}", app.error);
    }

    #[test]
    fn data_edits_apply_live_and_canvas_edits_undo() {
        use simsandbox_session::diagram::Edit;
        let mut app = SimApp::new(None, None, None, None);
        let e = EXAMPLES.iter().find(|e| e.name.contains("Cruise")).unwrap();
        app.open_text(e.name, e.text);
        let live = app.session.as_ref().unwrap();
        live.call(|s, _| s.run_until(5.0)).unwrap().unwrap();
        let shown = |app: &SimApp| app.views.entries()[1].bind["y"].signals();

        // A data edit: the second view also shows the speed; the run goes
        // on.
        canvas_edits(
            &mut app,
            vec![Edit::Bind {
                from: "car.v".into(),
                to: "view:1.y".into(),
                many: true,
            }],
        );
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_eq!(shown(&app), ["cruise.F", "car.v"]);
        assert!(app.session.as_ref().unwrap().status().time > 4.9);
        // A filter added and moved on the canvas, kept with the model.
        canvas_edits(&mut app, vec![Edit::AddFilter { at: [500.0, 300.0] }]);
        canvas_edits(
            &mut app,
            vec![Edit::Move {
                block: "filter:signal1".into(),
                at: [520.0, 310.0],
            }],
        );
        let live = app.session.as_ref().unwrap();
        live.sync();
        let filters = live.model().filters.clone();
        assert_eq!(filters[0].name, "signal1");
        let saved = app.saved_text().unwrap();
        assert!(
            saved.contains("\"canvas\": [\n            520.0") || saved.contains("520.0"),
            "{saved}"
        );
        assert!(app.session.as_ref().unwrap().status().time > 4.9);

        // Undo steps back through them without a restart.
        canvas_step(&mut app, true); // the move
        canvas_step(&mut app, true); // the filter
        let live = app.session.as_ref().unwrap();
        live.sync();
        assert!(live.model().filters.is_empty());
        canvas_step(&mut app, true); // the binding
        assert_eq!(shown(&app), ["cruise.F"]);
        assert!(app.session.as_ref().unwrap().status().time > 4.9);
        canvas_step(&mut app, false); // redo the binding
        assert_eq!(shown(&app), ["cruise.F", "car.v"]);

        // A model edit that does not load waits as a draft; undoing it
        // goes back to the running model.
        canvas_edits(
            &mut app,
            vec![Edit::Delete {
                block: "cruise".into(),
            }],
        );
        // Removing cruise leaves car.F open: a draft, the run goes on.
        assert!(app.diagram.draft_problem().is_some());
        canvas_step(&mut app, true);
        assert!(
            app.diagram.draft_problem().is_none(),
            "{:?}",
            app.diagram.draft_problem()
        );
        let live = app.session.as_ref().unwrap();
        assert!(live.model().file.blocks.iter().any(|b| b.name == "cruise"));
        assert!(app.error.is_none(), "{:?}", app.error);
    }

    #[test]
    fn a_promoted_filter_becomes_a_block_feeding_the_model() {
        use simsandbox_session::diagram::Edit;
        let mut app = SimApp::new(None, None, None, None);
        let e = EXAMPLES.iter().find(|e| e.name.contains("Cruise")).unwrap();
        app.open_text(e.name, e.text);
        canvas_edits(&mut app, vec![Edit::AddFilter { at: [500.0, 300.0] }]);
        let live = app.session.as_ref().unwrap();
        let entries = vec![simsandbox_file::FilterEntry::expression(
            "push",
            "cruise.F",
            Vec::new(),
        )];
        live.call(move |s, _| s.set_filters(entries))
            .unwrap()
            .unwrap();
        canvas_edits(
            &mut app,
            vec![
                Edit::Disconnect { to: "car.F".into() },
                Edit::PromoteConnect {
                    filter: "push".into(),
                    to: "car.F".into(),
                },
            ],
        );
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!(
            app.diagram.draft_problem().is_none(),
            "{:?}",
            app.diagram.draft_problem()
        );
        let m = app.session.as_ref().unwrap().model();
        assert!(m.file.blocks.iter().any(|b| b.name == "push"));
        assert!(m
            .file
            .connections
            .iter()
            .any(|c| c.from == "push.push" && c.to == "car.F"));
        assert!(m.filters.is_empty());
    }

    #[test]
    fn a_dragged_run_is_saved_beside_the_model_and_replayed() {
        let dir = std::env::temp_dir().join(format!("simsandbox-runs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = SimApp::new(None, None, None, None);
        let e = EXAMPLES
            .iter()
            .find(|e| e.name.contains("Drag the bob"))
            .unwrap();
        app.open_text(e.name, e.text);
        let model = dir.join("drag.ssbx.json");
        app.save_to(&model);
        let live = app.session.as_ref().unwrap();
        live.set_running(false);
        live.call(|s, _| s.restart()).unwrap().unwrap();
        live.call(|s, _| s.run_until(0.5)).unwrap().unwrap();
        // The view's pointer, as the dock sends it; a repeat is not recorded.
        for [x, y, down] in [[1.0, 0.0, 1.0], [1.0, 0.0, 1.0]] {
            for (k, v) in [("x", x), ("y", y), ("down", down)] {
                app.send_input(format!("hand.{k}"), v);
            }
            app.session.as_ref().unwrap().sync();
        }
        let live = app.session.as_ref().unwrap();
        live.call(|s, _| s.run_until(1.0)).unwrap().unwrap();
        app.send_input("hand.down".into(), 0.0);
        let live = app.session.as_ref().unwrap();
        live.call(|s, _| s.run_until(1.5)).unwrap().unwrap();
        let theta = |app: &SimApp| {
            app.session
                .as_ref()
                .unwrap()
                .call(|s, _| {
                    let h = s.signals();
                    let k = h.names().iter().position(|n| n == "bob.theta").unwrap();
                    h.points(k)
                        .iter()
                        .map(|p| p.y.to_bits())
                        .collect::<Vec<_>>()
                })
                .unwrap()
        };
        let reference = theta(&app);
        assert_eq!(live.call(|s, _| s.recorded().len()).unwrap(), 3);

        app.save_run("pull", "dragged right");
        let runs = runs_beside(&model);
        assert_eq!(runs.len(), 1);
        let (path, run) = &runs[0];
        assert!(path.ends_with("drag.pull.run.json"));
        assert_eq!((run.name.as_str(), run.track.len()), ("pull", 3));
        assert!(run_fits_disk(run, &model, &app.registry));
        // Opening it replays the drag bit for bit.
        app.open_run(path);
        assert!(app.error.is_none(), "{:?}", app.error);
        let live = app.session.as_ref().unwrap();
        live.set_running(false);
        assert!(live.call(|s, _| s.replaying()).unwrap());
        live.call(|s, _| s.run_until(1.5)).unwrap().unwrap();
        assert_eq!(theta(&app), reference);
        // As the model's schedule, every run applies it.
        let run = run.clone();
        app.use_as_schedule(&run, "pull");
        let m = app.session.as_ref().unwrap().model();
        assert_eq!(m.file.run.inputs.len(), 3);
        assert!(utc_now().ends_with('Z') && utc_now().len() == 20);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn canvas_edits_apply_when_the_model_loads_and_wait_as_a_draft_otherwise() {
        use simsandbox_session::diagram::Edit;
        let mut app = SimApp::new(None, None, None, None);
        let e = EXAMPLES.iter().find(|e| e.name.contains("Cruise")).unwrap();
        app.open_text(e.name, e.text);
        let live = app.session.as_ref().unwrap();
        live.call(|s, _| s.run_until(5.0)).unwrap().unwrap();
        let id = live.call(|s, _| s.ids["cruise"]).unwrap();
        live.call(move |s, _| s.set_param(id, "kp", 900.0))
            .unwrap()
            .unwrap();

        // Moving only saves the layout: the run goes on.
        canvas_edits(
            &mut app,
            vec![Edit::Move {
                block: "car".into(),
                at: [10.0, 20.0],
            }],
        );
        let live = app.session.as_ref().unwrap();
        live.sync();
        assert!(live.status().time > 4.9);
        assert_eq!(live.model().file.blocks[0].canvas, Some([10.0, 20.0]));

        // Removing a wire leaves an input open: kept as a draft, the run
        // goes on.
        canvas_edits(&mut app, vec![Edit::Disconnect { to: "car.F".into() }]);
        let problem = app.diagram.draft_problem().unwrap().to_string();
        assert!(problem.contains("car.F"), "{problem}");
        assert!(app.session.as_ref().unwrap().status().time > 4.9);
        // Wiring it again applies, restarting at t = 0
        // with the parameter values and layout kept.
        canvas_edits(
            &mut app,
            vec![Edit::Connect {
                from: "cruise.F".into(),
                to: "car.F".into(),
            }],
        );
        assert!(app.diagram.draft_problem().is_none());
        assert!(app.error.is_none(), "{:?}", app.error);
        let live = app.session.as_ref().unwrap();
        let m = live.model();
        assert_eq!(live.status().time, 0.0);
        assert!(m
            .file
            .connections
            .iter()
            .any(|c| c.from == "cruise.F" && c.to == "car.F"));
        assert_eq!(
            m.params
                .iter()
                .find(|r| r.key == "cruise.kp")
                .unwrap()
                .value,
            900.0
        );
        assert_eq!(m.file.blocks[0].canvas, Some([10.0, 20.0]));
        assert!(
            m.file.blocks.iter().all(|b| b.canvas.is_some()),
            "the layout is fixed"
        );
    }

    #[test]
    fn start_from_here_restarts_at_the_current_state() {
        let mut app = SimApp::new(None, None, None, None);
        let e = EXAMPLES
            .iter()
            .find(|e| e.name.contains("grazing"))
            .unwrap();
        app.open_text(e.name, e.text);
        let live = app.session.as_ref().unwrap();
        live.set_running(false);
        live.call(|s, _| s.run_until(3.0)).unwrap().unwrap();
        let notes = live.call(|s, _| s.start_from_now()).unwrap().unwrap();
        assert!(notes.len() == 3, "{notes:?}");
        let m = live.model();
        assert!(m.file.blocks.iter().all(|b| b.start.is_some()));
        assert_eq!(live.status().time, 0.0);
    }

    #[test]
    fn subsystems_are_grouped_opened_and_edited_from_the_canvas() {
        use simsandbox_session::diagram::{Edit, Scope};
        let mut app = SimApp::new(None, None, None, None);
        let e = EXAMPLES.iter().find(|e| e.name.contains("Cruise")).unwrap();
        app.open_text(e.name, e.text);
        // Group both blocks: the model now uses a subsystem, and runs.
        canvas_edits(
            &mut app,
            vec![Edit::Group {
                blocks: vec!["car".into(), "cruise".into()],
            }],
        );
        assert!(app.error.is_none(), "{:?}", app.error);
        let m = app.session.as_ref().unwrap().model();
        assert!(m.file.subsystems.contains_key("group1"));
        assert!(m.params.iter().any(|r| r.key == "group1/cruise.kp"));
        // Inside it, an edit changes the subsystem.
        app.diagram.scope = Scope::Subsystem("group1".into());
        canvas_edits(
            &mut app,
            vec![Edit::Rename {
                from: "car".into(),
                to: "body".into(),
            }],
        );
        assert!(app.error.is_none(), "{:?}", app.error);
        let m = app.session.as_ref().unwrap().model();
        assert!(m.file.subsystems["group1"]
            .blocks
            .iter()
            .any(|b| b.name == "body"));
        assert!(m.params.iter().any(|r| r.key == "group1/body.m"));
        // And the equation editor works on a block inside it.
        app.open_block(&m, "group1::body");
        let ed = app.editor.as_mut().unwrap();
        ed.text = ed.text.replace("(F - c*v)/m", "(F - drag2*c*v)/m");
        app.apply_equations();
        assert!(app.error.is_none(), "{:?}", app.error);
        let m = app.session.as_ref().unwrap().model();
        assert!(m.params.iter().any(|r| r.key == "group1/body.drag2"));
    }

    #[test]
    fn conversions_change_their_units_from_the_panel() {
        let mut app = SimApp::new(None, None, None, None);
        let k = EXAMPLES
            .iter()
            .position(|e| e.name.starts_with("Units"))
            .unwrap();
        app.open_example(k);
        assert!(app.error.is_none(), "{:?}", app.error);
        let unit = |app: &SimApp, port: &str| {
            let m = app.session.as_ref().unwrap().model();
            m.file
                .blocks
                .iter()
                .find(|b| b.name == "b_cm")
                .unwrap()
                .settings[port]
                .clone()
        };
        let set = |app: &mut SimApp, text: &str| {
            let e = Edit::SetSetting {
                block: "b_cm".into(),
                key: "to".into(),
                text: text.into(),
            };
            app.edit_at(Scope::Model, e);
        };
        set(&mut app, "mm");
        assert!(app.error.is_none(), "{:?}", app.error);
        assert_eq!(unit(&app, "to"), "mm");
        // A different kind of quantity does not load: the edit waits as a
        // draft, and the running model stays.
        set(&mut app, "kg");
        let problem = app.diagram.draft_problem().unwrap();
        assert!(problem.contains("measure different things"), "{problem}");
        assert_eq!(unit(&app, "to"), "mm");
    }

    #[test]
    fn model_parameters_set_every_block_using_them() {
        let mut app = SimApp::new(None, None, None, None);
        let e = EXAMPLES
            .iter()
            .find(|e| e.name.contains("Platoon"))
            .unwrap();
        app.open_text(e.name, e.text);
        let live = app.session.as_ref().unwrap();
        live.call(|s, _| s.set_param_key("kp", 800.0))
            .unwrap()
            .unwrap();
        let m = live.model();
        for car in ["car1", "car2", "car3"] {
            let row = m
                .params
                .iter()
                .find(|r| r.key == format!("{car}/control.kp"))
                .unwrap();
            assert_eq!((row.value, row.formula.as_deref()), (800.0, Some("kp")));
        }
    }

    #[test]
    fn replacing_a_changed_model_asks_first() {
        let mut app = SimApp::new(None, None, None, None);
        assert!(!app.has_unsaved(), "a model just opened");
        // Opening something else needs no question yet.
        app.open_example(1);
        assert!(app.asking.is_none());
        let live = app.session.as_ref().unwrap();
        let id = live.call(|s, _| s.ids["a"]).unwrap();
        live.call(move |s, _| s.set_param(id, "kappa", 0.7))
            .unwrap()
            .unwrap();
        assert!(app.has_unsaved());
        // Now it asks, and nothing is replaced until answered.
        let title = app.session.as_ref().unwrap().model().title.clone();
        app.open_example(0);
        assert_eq!(app.asking, Some(Discard::Example(0)));
        assert_eq!(app.session.as_ref().unwrap().model().title, title);
        // "Don't save": it goes ahead.
        app.asking = None;
        app.proceed(&egui::Context::default(), Discard::Example(0));
        assert_ne!(app.session.as_ref().unwrap().model().title, title);
        assert!(!app.has_unsaved());
        assert!(!app.discard_ok);

        // Saving clears the question; canvas edits make changes too.
        let dir = std::env::temp_dir().join(format!("simsandbox-unsaved-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cruise.ssbx.json");
        let k = EXAMPLES
            .iter()
            .position(|e| e.name.contains("Cruise"))
            .unwrap();
        app.open_example(k);
        canvas_edits(
            &mut app,
            vec![
                simsandbox_session::diagram::Edit::Disconnect { to: "car.F".into() },
                simsandbox_session::diagram::Edit::Connect {
                    from: "cruise.F".into(),
                    to: "car.F".into(),
                },
            ],
        );
        assert!(app.has_unsaved(), "rewired");
        app.save_to(&path);
        assert!(!app.has_unsaved());
        // The speed is saved with the model, so changing it counts.
        app.speed = 2.0;
        assert!(app.has_unsaved());
        // A new model from the equation editor asks too.
        app.editor = editor(None, NEW_MODEL);
        app.apply_equations();
        assert_eq!(app.asking, Some(Discard::NewModel));
        assert!(app.editor.is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
