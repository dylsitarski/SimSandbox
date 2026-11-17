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
use simsandbox_session::runs;
use simsandbox_session::session::{ParamRow, Session};
use simsandbox_session::views::Views;
use simsandbox_views::{Available, History, ViewState, ViewType};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
mod editors;
use editors::*;
mod data;
use data::*;
mod files;
mod run_menu;

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
    /// What the views read, as last told to the session (ADR 0063).
    view_reads: Vec<String>,
    /// The folder probes write their files into as they record, as last
    /// told to the session: the saved model's (ADR 0063).
    probe_dir: Option<PathBuf>,
    /// The dock as it settled by itself (egui_dock fits splits to their
    /// contents and to the window), and the arrangement that stands for:
    /// such changes are not edits, so they neither mark the model unsaved
    /// nor get saved.
    dock_settled: Option<(
        simsandbox_file::layout::Layout,
        Option<simsandbox_file::layout::Layout>,
    )>,
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
    /// The memory budget in MB (ADR 0062 step 5), an app setting kept
    /// between launches.
    memory_mb: f64,
    error: Option<String>,
    /// Success messages, e.g. "Saved to …".
    /// Notices, warnings and errors, shown in the status tile.
    log: Log,
    /// The error last written to the log, so each is logged once.
    logged_error: Option<String>,
    /// The file the open model was loaded from; Save writes back to it.
    /// `None` for built-in examples, which Save turns into Save as.
    path: Option<PathBuf>,
    /// A built-in example that reads files (ADR 0069 step 3): the folder
    /// it was unpacked into, which it reads them from while it has no
    /// path, and its place in `EXAMPLES`; Save as copies its files beside
    /// the model.
    example_folder: Option<(PathBuf, usize)>,
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
    last_canvas: Option<crate::canvas::Selection>,
    /// List values formulas set too.
    show_computed: bool,
    /// What the study picked last can vary: its name, its model's text and
    /// the parameters (ADR 0070), kept while they stay the same.
    study_keys: HashMap<String, (String, std::sync::Arc<Studied>)>,
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
    /// Frames drawn before the window is maximized: once it is showing,
    /// as a person would (created maximized, Windows keeps the drawing
    /// area at the first size). `None` when done, or for screenshots.
    maximize_in: Option<u8>,
}

impl SimApp {
    /// The app shared between the main window and popped-out ones.
    /// The memory budget in MB (ADR 0062 step 5), kept between launches.
    pub fn memory_budget(&self) -> f64 {
        self.memory_mb
    }

    /// Set the memory budget in MB (ADR 0062 step 5), for the model
    /// running and those opened later.
    pub fn set_memory_budget(&mut self, mb: f64) {
        if !(mb.is_finite() && mb > 0.0) || mb == self.memory_mb {
            return;
        }
        self.memory_mb = mb;
        if let Some(live) = &self.session {
            live.send(Some("memory"), move |s, _| s.set_memory_budget(mb));
        }
    }

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
            view_reads: Vec::new(),
            probe_dir: None,
            dock_settled: None,
            key_values: HashMap::new(),
            run_dialog: None,
            registry: registry(),
            session: None,
            speed: 1.0,
            memory_mb: simsandbox_session::session::DEFAULT_MEMORY_MB,
            error: None,
            log: Log::default(),
            logged_error: None,
            path: None,
            example_folder: None,
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
            study_keys: HashMap::new(),
            diagram: DiagramTab::default(),
            started: Instant::now(),
            baseline: None,
            unsaved: Default::default(),
            last_unsaved_check: Instant::now(),
            asking: None,
            discard_ok: false,
            maximize_in: screenshot.is_none().then_some(2),
            screenshot,
            screenshot_requested: false,
        };
        // A file given opens and runs at once (ADR 0009); without one, the
        // landing page offers a new model, a file or an example.
        if let Some(p) = path {
            app.open_path(&p);
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
        // Probes writing as they record write beside the saved model.
        let dir = self
            .path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        if dir != self.probe_dir {
            self.diagram.folder = dir.clone();
            self.probe_dir = dir.clone();
            live.send(Some("probe-dir"), move |s, _| s.set_probe_dir(dir));
        }
        // Views bound, rebound or closed: the history follows (ADR 0063).
        let reads = simsandbox_session::session::view_reads(&self.views.entries());
        if reads != self.view_reads {
            self.view_reads = reads.clone();
            live.send(Some("view-reads"), move |s, _| s.set_view_reads(reads));
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
        if m.timeless {
            // Nothing to run, pause, step or rewind (ADR 0072).
            ui.horizontal(|ui| {
                ui.label("Evaluated, not run in time").on_hover_text(
                    "Nothing in this model changes with time (no states, data, clocks or \
                     schedule): it is evaluated when opened and again whenever a value it \
                     reads changes",
                );
            });
        } else if m.iterates {
            // Rounds, not seconds: nothing to run, pause, pace or rewind
            // (ADR 0083).
            ui.horizontal(|ui| {
                let state = if st.iterating {
                    ui.ctx().request_repaint_after(Duration::from_millis(50));
                    "iterating"
                } else if st.held_still {
                    "holds still"
                } else {
                    "reached its last round"
                };
                ui.label(format!("Iterates: round {} — {state}", st.round))
                    .on_hover_text(
                        "Only maps that never read time: its steps are rounds, not seconds. \
                         It runs as fast as it can when opened and again from round 0 whenever \
                         a value it reads changes, until a round changes nothing or its last \
                         round (the file's t_end)",
                    );
            });
        } else {
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
                if st.gaps && m.recorder.is_some() {
                    ui.separator();
                    if ui
                        .button("Fill the past")
                        .on_hover_text(
                            "Something views show was wired after the run started, so it \
                             has no past. Run again from the start, replaying the input as \
                             recorded, to fill it; the run then carries on from where it is.",
                        )
                        .clicked()
                    {
                        live.send(None, |s, c| match s.recompute() {
                            Ok(()) => c.info("Filled the past of what views show"),
                            Err(e) => c.error(e),
                        });
                    }
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
            // Past the memory budget (ADR 0062 step 5): say so, and what
            // holds most.
            if let Some(mem) = st.memory.as_ref().filter(|m| m.over()) {
                let most = mem
                    .largest()
                    .map(|(part, bytes)| format!("; most: {part}, {}", megabytes(*bytes)))
                    .unwrap_or_default();
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!(
                        "recordings hold {} of a {} memory budget{most}",
                        megabytes(mem.held),
                        megabytes(mem.budget)
                    ),
                )
                .on_hover_text(
                    "Raise the budget under Memory…, or give the largest probe a keep of its own (last samples, or a size)",
                );
            }
            let study = live.study_runs();
            if let Some(d) = live
                .stepping_for()
                .filter(|d| *d >= SLOW_STEP && study.is_none())
            {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!("a step has been running for {:.1} s", d.as_secs_f64()),
                )
                .on_hover_text("Commands, including Pause, take effect when it ends");
                ui.ctx().request_repaint_after(Duration::from_millis(100));
            }
            // A study in the step: its runs so far (ADR 0071 step 2).
            if study.is_none() && m.unevaluated {
                ui.weak(if m.timeless {
                    "about to evaluate"
                } else {
                    "studies about to run"
                })
                .on_hover_text("Its studies run first, on the simulation's thread");
            }
            if let Some((done, total)) = study {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!("study: {done} of {total} runs"),
                )
                .on_hover_text(
                    "Changing what it reads, Pause or Run gives it up and starts it again from \
                     the new values",
                );
                ui.ctx().request_repaint_after(Duration::from_millis(100));
            }
        });
    }

    fn rewind_bar(&mut self, ui: &mut egui::Ui) {
        let Some(live) = self.session.as_ref() else {
            return;
        };
        // Without a recorder there is nothing to rewind to (ADR 0064).
        let m = live.model();
        if m.recorder.is_none() || m.timeless || m.iterates {
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
        // As it settled by itself: what it was before.
        if let Some((now, was)) = &self.dock_settled {
            if *now == l {
                return was.clone();
            }
        }
        let standard = Dock::standard(&self.tile_ids()).to_layout(&index);
        (Some(&l) != standard.as_ref()).then_some(l)
    }

    /// The docked tabs (ADR 0053), and the ones popped out into windows.
    fn docked(&mut self, ui: &mut egui::Ui) {
        if self.session.is_none() {
            self.landing(ui);
            return;
        }
        let tiles = self.tile_ids();
        self.dock.sync(&tiles);
        if let Some(Focus::Tile(t)) = self.focus.take() {
            self.dock.focus(|p| p.tiles().contains(&t));
        }
        let saved_before = self.layout();
        let arranged = |app: &Self, dock: &Dock| {
            dock.to_layout(&|id: u64| app.views.tiles.iter().position(|t| t.id == id))
        };
        let mut dock = std::mem::replace(&mut self.dock, Dock::standard(&[]));
        let before = arranged(self, &dock);
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
        // The dock changed with no pointer at work: it fitted itself to
        // its contents or the window. That is not an edit.
        let after = arranged(self, &dock);
        let pointer = ui
            .input(|i| i.pointer.any_down() || i.pointer.any_pressed() || i.pointer.any_released());
        if before != after && !pointer {
            if let Some(after) = after {
                self.dock_settled = Some((after, saved_before));
            }
        }
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
        self.diagram.activations = live.status().activations;
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
                Outcome::Placed {
                    filters,
                    views,
                    probes,
                    recorder,
                } => {
                    let Some(live) = &self.session else {
                        return;
                    };
                    if let Some(at) = recorder {
                        live.send(Some("place-recorder"), move |s, _| {
                            let r = s
                                .recorder()
                                .cloned()
                                .map(|r| simsandbox_file::RecorderEntry {
                                    canvas: Some(at.map(|v| (v * 10.0).round() / 10.0)),
                                    ..r
                                });
                            s.set_recorder(r);
                        });
                    }
                    for (name, at) in probes {
                        live.send(Some(&format!("place-probe:{name}")), move |s, c| {
                            if let Err(e) = s.place_probe(&name, Some(at)) {
                                c.error(e);
                            }
                        });
                    }
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
                    for p in &file.controls.probes {
                        let (name, at) = (p.name.clone(), p.canvas);
                        live.send(None, move |s, _| {
                            let _ = s.place_probe(&name, at);
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
                    probes,
                    recorder,
                    notes,
                } => {
                    // The recorder is set apart: it never needs a check.
                    if let Some(live) = &self.session {
                        if recorder != m.recorder {
                            live.send(None, move |s, _| s.set_recorder(recorder));
                        }
                    }
                    self.apply_data(m, (filters, views, probes), notes)
                }
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
                Outcome::Expanded(paths) => {
                    if let Some(live) = &self.session {
                        live.send(None, move |s, _| s.file.controls.expand = paths);
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
        (filters, views, probes): (
            Vec<simsandbox_file::FilterEntry>,
            Vec<simsandbox_file::ViewEntry>,
            Vec<simsandbox_file::ProbeEntry>,
        ),
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
        // Probes follow the filters, which they may record (ADR 0062).
        if probes != live.model().probes {
            match live.call(move |s, _| s.set_probe_entries(&probes)) {
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
        // `use/path::block` for a block inside a used model (ADR 0068).
        let inside = match name.split_once("::") {
            Some((path, _)) => {
                let path: Vec<String> = path.split('/').map(str::to_string).collect();
                crate::diagram::used_file(&m.file, self.diagram.folder.as_deref(), &path).ok()
            }
            None => Some(m.file.clone()),
        };
        let inner = name.rsplit("::").next().unwrap_or(name);
        let Some(b) = inside
            .as_ref()
            .and_then(|f| f.blocks.iter().find(|b| b.name == inner))
        else {
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
        drop(history);
        self.memory_menu(ui);
        native_status(ui);
    }

    /// "Memory…" (ADR 0062 step 5): what the recordings hold, part by
    /// part, against the memory budget, which is set here and kept
    /// between launches. Snapshots (unless the recorder sets its own),
    /// the history, and each probe and filter that keeps what it records
    /// by default thin past a quarter of it.
    fn memory_menu(&mut self, ui: &mut egui::Ui) {
        let memory = self.session.as_ref().and_then(|l| l.status().memory);
        let mut mb = self.memory_mb;
        ui.menu_button("Memory…", |ui| {
            ui.horizontal(|ui| {
                ui.label("Memory budget");
                ui.add(
                    egui::DragValue::new(&mut mb)
                        .range(16.0..=1_048_576.0)
                        .speed(8.0)
                        .suffix(" MB"),
                )
                .on_hover_text(
                    "Shared by rewind's snapshots and what probes, the history and filters keep in memory; each takes a quarter by default and thins past it",
                );
            });
            if let Some(m) = &memory {
                ui.separator();
                ui.label(format!(
                    "Held now: {} of {}",
                    megabytes(m.held),
                    megabytes(m.budget)
                ));
                for (part, bytes) in m.parts.iter().filter(|p| p.1 > 0) {
                    ui.weak(format!("{part}: {}", megabytes(*bytes)));
                }
            }
        })
        .response
        .on_hover_text("The memory recordings may use, and what they hold");
        self.set_memory_budget(mb);
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
        // Bindings from a probe draw what it recorded (ADR 0063).
        let probe_guards: Vec<_> = m
            .probe_histories
            .iter()
            .map(|(n, h)| (n.as_str(), h.read()))
            .collect();
        let probes: Vec<(&str, &simsandbox_views::History)> =
            probe_guards.iter().map(|(n, g)| (*n, &**g)).collect();
        // Outputs of filters with clocks of their own, from their recordings.
        let derived_guards: Vec<_> = m.derived_histories.iter().map(|(_, h)| h.read()).collect();
        let derived: Vec<&simsandbox_views::History> =
            derived_guards.iter().map(|g| &**g).collect();
        let recorded = simsandbox_views::Recorded {
            all: &history,
            probes: &probes,
            derived: &derived,
        };
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
                                        style_editor(ui, t, &mut tile.state, recorded, id, text, error);
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
                        tile.state.show(ui, t, recorded);
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
    /// (`car1/car.v` comes from the use `car1`). Signals of filters have no
    /// block.
    fn inspect_signal(&mut self, signal: &str) {
        let Some(live) = &self.session else {
            return;
        };
        let m = live.model();
        let Some((node, _)) = signal.split_once('.') else {
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
            let selection = crate::canvas::Selection::Block(top.to_string());
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

    fn shortcuts(&mut self, ui: &mut egui::Ui) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        let save_as = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::S);
        let save = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
        let open = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
        // F11: full screen and back.
        if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F11)) {
            let full = ui.ctx().input(|i| i.viewport().fullscreen.unwrap_or(false));
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Fullscreen(!full));
        }
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

    /// What the study `name` of `file` can vary (its model's parameters).
    fn study_keys(&mut self, file: &simsandbox_file::ModelFile, name: &str) -> Vec<StudyKey> {
        self.studied(file, name)
            .map(|s| s.keys.clone())
            .unwrap_or_default()
    }

    /// The model the study `name` of `file` studies: what it can vary and
    /// its blocks' values, worked out again only when it changed.
    fn studied(
        &mut self,
        file: &simsandbox_file::ModelFile,
        name: &str,
    ) -> Option<std::sync::Arc<Studied>> {
        let b = file
            .blocks
            .iter()
            .find(|b| b.name == name && b.type_name == simsandbox_file::STUDY)?;
        let label = match (&b.model, &b.file) {
            (Some(m), _) => m.clone(),
            (None, Some(f)) => f.clone(),
            _ => return None,
        };
        let folder = self.path.as_deref().and_then(Path::parent);
        let model =
            simsandbox_session::edit_use::model_at(file, folder, &[name.to_string()]).ok()?;
        let text = model.to_json();
        if let Some((t, s)) = self.study_keys.get(name) {
            if *t == text {
                return Some(s.clone());
            }
        }
        let at = model.folder.as_deref().or(folder);
        let keys = simsandbox_session::diagram::study_parameters(&model, &self.registry, at);
        let d = Diagram::new(&model, &self.registry);
        let blocks = d
            .nodes
            .into_iter()
            .filter(|n| !n.is_observer() && !n.values.is_empty())
            .map(|n| (n.name, n.values))
            .collect();
        let s = std::sync::Arc::new(Studied {
            model: label,
            keys,
            blocks,
        });
        self.study_keys.insert(name.to_string(), (text, s.clone()));
        Some(s)
    }

    /// Apply edits to the model open on the canvas (inside a study): the
    /// edit question first where the model is used elsewhere too.
    fn edit_inside(&mut self, edits: Vec<Edit>) {
        let Some(live) = &self.session else {
            return;
        };
        let m = live.model();
        let (speed, views, layout) = (self.speed, self.views.entries(), self.layout());
        let current = || {
            let (views, layout) = (views.clone(), layout.clone());
            live.call(move |s, _| s.save_json_with(speed, Some(views), Some(layout)))?
        };
        let outcomes = self.diagram.begin_edit(edits, &self.registry, &current);
        self.diagram_outcomes(&m, outcomes);
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
        // Where a file reader's data is named from (ADR 0063).
        let folder = self
            .path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        let folder = folder.as_deref();
        // Edits that go through the diagram (a conversion's units, a new
        // block's values), at the level they apply to.
        let mut canvas_edits: Vec<(Scope, Edit)> = Vec::new();
        // Edits of the model open on the canvas inside a study.
        let mut inside_edits: Vec<Edit> = Vec::new();
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
                // Inside a used model, picks are named inside it: its path
                // (`car1`, `vehicle/plant`) says where.
                let scope: Option<String> =
                    (!self.diagram.inside.is_empty()).then(|| self.diagram.inside.join("/"));
                // A block in a model expanded in place is named by its path
                // (`shaken/mass`): inside that use, as when it is opened.
                self.picked = canvas
                    .map(|s| s.blocks())
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|n| !n.contains(':'))
                    .map(|n| match n.rsplit_once('/') {
                        Some((path, name)) => {
                            let full = match &scope {
                                Some(s) => format!("{s}/{path}"),
                                None => path.to_string(),
                            };
                            (Some(full), name.to_string())
                        }
                        None => (scope.clone(), n),
                    })
                    .collect();
            }
            let picked = self.picked.clone();
            // A study picked: what its model can vary, for its form.
            let keys = match picked.as_slice() {
                [(None, name)] => self.study_keys(&m.file, name),
                _ => Vec::new(),
            };
            // Every study's model, for the tree (ADR 0070).
            let studies: HashMap<String, std::sync::Arc<Studied>> = m
                .file
                .blocks
                .iter()
                .filter(|b| b.type_name == simsandbox_file::STUDY)
                .map(|b| b.name.clone())
                .collect::<Vec<_>>()
                .into_iter()
                .filter_map(|n| self.studied(&m.file, &n).map(|s| (n, s)))
                .collect();
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
                            let at = Scope::Model;
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
                                for e in file_fields(ui, b, &mut self.param_edits, folder) {
                                    canvas_edits.push((at.clone(), e));
                                }
                                for e in study_fields(ui, b, &mut self.param_edits, &keys) {
                                    canvas_edits.push((at.clone(), e));
                                }
                                shown |= b.type_name == simsandbox_eq::convert::CONVERT
                                    || b.type_name == simsandbox_eq::file_input::FILE
                                    || b.type_name == simsandbox_file::STUDY;
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
                                for e in file_fields(ui, b, &mut self.param_edits, folder) {
                                    canvas_edits.push((at.clone(), e));
                                }
                                for e in study_fields(ui, b, &mut self.param_edits, &keys) {
                                    canvas_edits.push((at.clone(), e));
                                }
                                shown = true;
                            }
                            // A block inside a used model: its values by path.
                            if running.is_none() && drafted.is_none() {
                                if let Some(t) = crate::inspector::selected(
                                    &m.params,
                                    &m.file,
                                    scope.as_deref(),
                                    name,
                                ) {
                                    uses_table(ui, &t, &m.file.controls.params, edits!());
                                    shown = true;
                                } else if let Some(values) = scope.as_deref().and_then(|s| {
                                    let path: Vec<String> =
                                        s.split('/').map(str::to_string).collect();
                                    self.diagram.inside_values(&path, name)
                                }) {
                                    // Inside a study: the model's values, as
                                    // written; a change edits the model.
                                    ui.weak(
                                        "Values of the studied model: a change edits the model",
                                    );
                                    for e in studied_rows(ui, name, &values, &mut self.param_edits)
                                    {
                                        inside_edits.push(e);
                                    }
                                    shown = !values.is_empty();
                                }
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
                    // What models using this one see of it (ADR 0073).
                    for e in offered_outputs(ui, &m) {
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
                let studied = studies.get(&g.name);
                let kind = match entry.filter(|b| b.is_use()).map(|b| b.file.as_deref()) {
                    _ if studied.is_some() => {
                        format!("study of {}", studied.map_or("", |s| s.model.as_str()))
                    }
                    Some(Some(file)) => format!("uses {file}"),
                    Some(None) => format!(
                        "uses the model {}",
                        entry.and_then(|b| b.model.as_deref()).unwrap_or_default()
                    ),
                    None if g.instance => format!("subsystem {}", g.type_name),
                    None => g.type_name.clone(),
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
                        // When a used model acts (ADR 0088).
                        if let Some(b) = entry.filter(|b| b.is_use() && studied.is_none()) {
                            for e in use_activation(ui, b) {
                                canvas_edits.push((Scope::Model, e));
                            }
                        }
                        // Starting values come last, each marked ↻; a used
                        // model's parameters nest by path (ADR 0066).
                        // A study's model, nested: its blocks' values as
                        // written (ADR 0070).
                        if let Some(st) = studied {
                            studied_tree(ui, &g.name, st);
                        }
                        let nested = crate::inspector::nest(&g.rows);
                        nested_rows(ui, &nested, &g.name, &mut |ui, name, row| {
                            let mut r = row.clone();
                            r.name = name.to_string();
                            let used = o.used_by.get(&row.key);
                            param_row(ui, &m, &r, Place::Tree, used, edits!());
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
        if !inside_edits.is_empty() {
            self.edit_inside(inside_edits);
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
        // Fill the screen once the window shows.
        match self.maximize_in {
            Some(0) => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
                self.maximize_in = None;
            }
            Some(n) => {
                self.maximize_in = Some(n - 1);
                ctx.request_repaint();
            }
            None => {}
        }
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
                    .find(|r| r.formula.is_none() && r.driven.is_none())
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
                            for r in cells
                                .iter()
                                .flatten()
                                .filter(|r| r.formula.is_none() && r.driven.is_none())
                            {
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
                        Some(r) if r.driven.is_some() => {
                            let from = r.driven.as_deref().unwrap_or_default();
                            ui.weak(format!("from {from}"))
                                .on_hover_text(format!("Driven by {from} (ADR 0075)"));
                        }
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
    recorded: simsandbox_views::Recorded<'_>,
    tile: u64,
    text: &mut HashMap<String, String>,
    error: &mut Option<String>,
) {
    let context = simsandbox_views::Context {
        history: recorded.all,
        bind: &state.bind,
    };
    let shown: Vec<_> = t.style.iter().filter(|p| (p.applies)(&context)).collect();
    state.edit_selection(ui, t, recorded);
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
    // Driven by a wire (ADR 0075): its source, not a value to type.
    let driven = row.driven.as_deref();
    let typed = match (driven, edits.pending.get(&row.key)) {
        (Some(from), _) => {
            ui.weak(format!("from {from}")).on_hover_text(format!(
                "Driven by {from}: it takes that signal's value (ADR 0075). Drag the wire \
                 off its dot on the diagram to set it again"
            ));
            None
        }
        (None, Some(p)) => {
            pending_field(ui, *p);
            None
        }
        (None, None) => value_entry(ui, &row.key, row.value, formula, edits.text, edits.error),
    };
    if let Some(t) = typed {
        apply_typed(row, t, edits.changes, edits.file, edits.error);
    }

    ui.horizontal(|ui| {
        if formula.is_some() || driven.is_some() {
            ui.weak(format_value(row.value))
                .on_hover_text("Its value now");
        }
        ui.label(&row.unit);
        if row.initial_condition {
            ui.weak("↻")
                .on_hover_text("Initial condition: applies on Restart");
        }
    });
    ui.horizontal(|ui| {
        if place == Place::Controls && formula.is_none() && driven.is_none() {
            ui.checkbox(&mut control.slider, "")
                .on_hover_text("A slider under the value, between the two ends of its range");
        }
        pin_button(ui, &mut control);
    });
    ui.end_row();

    // The slider between its range's ends, which are typed in place.
    if place == Place::Controls && control.slider && formula.is_none() && driven.is_none() {
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
    // Inside a used model it is not one of the file's own blocks.
    scope
        .is_none()
        .then(|| file.blocks.iter().find(|b| b.name == name))
        .flatten()
}

/// A text field for one setting of block `b`; a changed value is an edit
/// (empty clears the setting).
fn setting_field(
    ui: &mut egui::Ui,
    b: &simsandbox_file::BlockEntry,
    key: &str,
    width: f32,
    hint: &str,
    edits: &mut HashMap<String, String>,
) -> Option<Edit> {
    let now = b.settings.get(key).cloned().unwrap_or_default();
    let id = format!("setting:{}:{key}", b.name);
    let mut text = edits.get(&id).cloned().unwrap_or_else(|| now.clone());
    let r = ui
        .add(egui::TextEdit::singleline(&mut text).desired_width(width))
        .on_hover_text(hint);
    if r.has_focus() {
        edits.insert(id.clone(), text.clone());
    }
    if r.lost_focus() {
        edits.remove(&id);
        if text.trim() != now {
            return Some(Edit::SetSetting {
                block: b.name.clone(),
                key: key.into(),
                text: text.trim().into(),
            });
        }
    }
    None
}

/// The outputs the model offers to models using it (ADR 0073): each by
/// name with the signal it shows, removable, and a picker to offer another
/// block output.
fn offered_outputs(ui: &mut egui::Ui, m: &crate::live::Model) -> Vec<Edit> {
    let mut out = Vec::new();
    let offered = &m.file.outputs;
    ui.add_space(4.0);
    ui.label("Offered to models using this one").on_hover_text(
        "Ports of this model where another model uses it, by name: they stay valid when \
             its insides change. A model nothing uses needs none.",
    );
    for o in offered {
        ui.horizontal(|ui| {
            ui.monospace(&o.name);
            ui.weak(format!("shows {}", o.from));
            if ui
                .small_button("×")
                .on_hover_text("Stop offering it")
                .clicked()
            {
                out.push(Edit::RemoveOutput {
                    name: o.name.clone(),
                });
            }
        });
    }
    // Whole outputs of the model's own blocks, not yet offered.
    let blocks: Vec<&str> = m
        .file
        .blocks
        .iter()
        .filter(|b| !b.is_use())
        .map(|b| b.name.as_str())
        .collect();
    let candidates: Vec<&String> = m
        .scalars
        .iter()
        .chain(m.grids())
        .filter(|s| !s.contains('[') && !s.contains('/'))
        .filter(|s| s.split_once('.').is_some_and(|(b, _)| blocks.contains(&b)))
        .filter(|s| !offered.iter().any(|o| &o.from == *s))
        .collect();
    if candidates.is_empty() {
        return out;
    }
    egui::ComboBox::from_id_salt("offer-output")
        .selected_text("+ offer an output")
        .show_ui(ui, |ui| {
            for c in candidates {
                if ui.selectable_label(false, c.as_str()).clicked() {
                    let port = c.split_once('.').map_or(c.as_str(), |(_, p)| p);
                    let taken = |x: &str| offered.iter().any(|o| o.name == x);
                    let name = if taken(port) {
                        simsandbox_session::diagram::free_name(&c.replace('.', "_"), taken)
                    } else {
                        port.to_string()
                    };
                    out.push(Edit::DeclareOutput {
                        name,
                        from: c.clone(),
                    });
                }
            }
        });
    out
}

/// The values of block `block` of a studied model, as written: a field
/// each, a changed value an edit of the model; values formulas set are
/// shown, not edited.
fn studied_rows(
    ui: &mut egui::Ui,
    block: &str,
    values: &[simsandbox_session::diagram::NodeValue],
    edits: &mut HashMap<String, String>,
) -> Vec<Edit> {
    let mut out = Vec::new();
    egui::Grid::new(("studied-rows", block))
        .num_columns(3)
        .spacing([6.0, 4.0])
        .show(ui, |ui| {
            for v in values {
                ui.label(&v.name);
                match (&v.driven, &v.formula) {
                    (Some(from), _) => {
                        ui.weak(format!("from {from}"));
                    }
                    (None, Some(f)) => {
                        ui.weak(format!("= {f}"));
                    }
                    (None, None) => {
                        let id = format!("studied:{block}.{}", v.name);
                        let now = simsandbox_views::values::number(v.value);
                        let mut text = edits.get(&id).cloned().unwrap_or_else(|| now.clone());
                        let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(70.0));
                        if r.has_focus() {
                            edits.insert(id.clone(), text.clone());
                        }
                        if r.lost_focus() {
                            edits.remove(&id);
                            match text.trim().parse::<f64>() {
                                Ok(x) if x.is_finite() && text.trim() != now => {
                                    out.push(Edit::SetParam {
                                        block: block.to_string(),
                                        param: v.name.clone(),
                                        value: x,
                                    })
                                }
                                _ => {}
                            }
                        }
                    }
                }
                ui.label(if v.initial {
                    format!("{} ↻", v.unit)
                } else {
                    v.unit.clone()
                });
                ui.end_row();
            }
        });
    out
}

/// A study's model in the model tree: each block's values as written, to
/// read; changed by opening the study's node.
fn studied_tree(ui: &mut egui::Ui, study: &str, s: &Studied) {
    egui::CollapsingHeader::new(format!("model {}", s.model))
        .id_salt(("studied", study))
        .default_open(false)
        .show(ui, |ui| {
            ui.weak("Its values as written; open the study's node and pick a block to change them");
            for (block, values) in &s.blocks {
                ui.strong(block);
                egui::Grid::new(("studied-tree", study, block))
                    .num_columns(2)
                    .spacing([8.0, 2.0])
                    .show(ui, |ui| {
                        for v in values {
                            ui.label(&v.name);
                            let text = match (&v.driven, &v.formula) {
                                (Some(from), _) => format!("from {from}"),
                                (None, Some(f)) => format!("= {f}"),
                                (None, None) => format!(
                                    "{} {}",
                                    simsandbox_views::values::number(v.value),
                                    v.unit
                                ),
                            };
                            ui.monospace(text);
                            ui.end_row();
                        }
                    });
            }
        })
        .header_response
        .on_hover_text("The model this study runs at each of its points");
}

/// The model a study studies, as the Inspector shows it: its name, what
/// it can vary, and each block's values as its file writes them.
struct Studied {
    model: String,
    keys: Vec<StudyKey>,
    blocks: Vec<(String, Vec<simsandbox_session::diagram::NodeValue>)>,
}

/// A parameter a study can vary: its key, a range for it and whether it
/// is an initial condition.
type StudyKey = (String, [f64; 2], bool);

/// A study's form (ADR 0070): what it studies and how many runs, its
/// design, a row per parameter it varies (picked from `keys`, the studied
/// model's parameters with ranges), count and seed, replicates and seeds,
/// its named results, and the whole sample as JSON. Changes are kept as a
/// draft while typed or dragged and applied when a field is left, a
/// choice made or a button pressed. Nothing for other blocks.
/// When a used model acts (ADR 0088): always, or on its `activate` input
/// in a mode; and whether a `restart` input starts it again.
fn use_activation(ui: &mut egui::Ui, b: &simsandbox_file::BlockEntry) -> Vec<Edit> {
    let mut out = Vec::new();
    ui.horizontal(|ui| {
        let now = b.activate.clone();
        let label = |m: &Option<String>| match m.as_deref() {
            None => "always".to_string(),
            Some("while") => "while 'activate' is high".to_string(),
            Some(m) => format!("starting again on 'activate' ({m})"),
        };
        ui.label("Runs");
        egui::ComboBox::from_id_salt(("use-activate", &b.name))
            .selected_text(label(&now))
            .show_ui(ui, |ui| {
                for m in [
                    None,
                    Some("rise"),
                    Some("fall"),
                    Some("change"),
                    Some("while"),
                ] {
                    let m = m.map(String::from);
                    if ui.selectable_label(now == m, label(&m)).clicked() && now != m {
                        out.push(Edit::SetActivate {
                            block: b.name.clone(),
                            mode: m,
                        });
                    }
                }
            })
            .response
            .on_hover_text(
                "Activated, it gets an 'activate' input: it starts again from its starting \
                 values when the input rises, falls or changes, or runs only while it is high \
                 and holds otherwise",
            );
        let mut restart = b.restart;
        if ui
            .checkbox(&mut restart, "restart input")
            .on_hover_text(
                "A 'restart' input: it starts again from its starting values when the input rises",
            )
            .changed()
        {
            out.push(Edit::SetRestart {
                block: b.name.clone(),
                on: restart,
            });
        }
    });
    out
}

fn study_fields(
    ui: &mut egui::Ui,
    b: &simsandbox_file::BlockEntry,
    edits: &mut HashMap<String, String>,
    keys: &[StudyKey],
) -> Vec<Edit> {
    use simsandbox_file::{Design, Sampling, Seeds, SweepControl};
    let mut out = Vec::new();
    if b.type_name != simsandbox_file::STUDY {
        return out;
    }
    let saved = b.sample.clone().unwrap_or_default();
    let id = format!("study:{}", b.name);
    let mut s: Sampling = edits
        .get(&id)
        .and_then(|t| serde_json::from_str(t).ok())
        .unwrap_or_else(|| saved.clone());
    // When it runs (ADR 0088).
    ui.horizontal(|ui| {
        let now = b.activate.clone();
        let label = |m: &Option<String>| match m.as_deref() {
            None => "when its inputs change".to_string(),
            Some("while") => "while 'activate' is high".to_string(),
            Some(m) => format!("on 'activate' ({m})"),
        };
        ui.label("Runs");
        egui::ComboBox::from_id_salt(("activate", &b.name))
            .selected_text(label(&now))
            .show_ui(ui, |ui| {
                for m in [
                    None,
                    Some("rise"),
                    Some("fall"),
                    Some("change"),
                    Some("while"),
                ] {
                    let m = m.map(String::from);
                    if ui.selectable_label(now == m, label(&m)).clicked() && now != m {
                        out.push(Edit::SetActivate {
                            block: b.name.clone(),
                            mode: m,
                        });
                    }
                }
            })
            .response
            .on_hover_text(
                "Activated, a study gets an 'activate' input: it runs once when it rises, falls \
                 or changes, or keeps running while it is high, and holds its results otherwise",
            );
    });
    let mut changed = false;
    let mut commit = false;
    let runs = match s.design {
        Design::Rows => "a run per row".to_string(),
        _ => format!("{} runs", study_runs(&s)),
    };
    ui.weak(match (&b.model, &b.file) {
        (Some(m), _) => {
            format!("Studies the model {m}: {runs} (open the node to edit the model)")
        }
        (None, Some(f)) => {
            format!("Studies the file {f}: {runs} (open the node to edit the model)")
        }
        _ => "Names no model to study".into(),
    });
    let range_of = |key: &str| {
        keys.iter()
            .find(|(k, _, _)| k == key)
            .map_or([0.0, 1.0], |(_, r, _)| *r)
    };
    egui::Grid::new(("study-form", &b.name))
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label("design");
            let designs = [
                (Design::Grid, "grid: every combination"),
                (Design::List, "list of points"),
                (Design::Random, "random"),
                (Design::LatinHypercube, "Latin hypercube"),
                (Design::Sobol, "Sobol sequence"),
                (Design::File, "points from a file"),
                (Design::Inputs, "points from its inputs"),
                (Design::Rows, "a point per row of a table"),
            ];
            let label = designs.iter().find(|(d, _)| *d == s.design).map_or("", |(_, l)| l);
            egui::ComboBox::from_id_salt(("study-design", &b.name))
                .selected_text(label)
                .show_ui(ui, |ui| {
                    for (d, l) in designs {
                        if ui.selectable_value(&mut s.design, d, l).clicked() {
                            // Points from inputs need their number; a
                            // table's rows are as many as come.
                            if d == Design::Inputs && s.count.is_none() {
                                s.count = Some(simsandbox_file::DEFAULT_SAMPLE_COUNT);
                            }
                            if d == Design::Rows {
                                s.count = None;
                                s.replicates = None;
                            }
                            // ... and have no ranges: their values arrive.
                            if d.from_inputs() {
                                for p in &mut s.params {
                                    p.range = None;
                                    p.points = None;
                                }
                            }
                            changed = true;
                            commit = true;
                        }
                    }
                });
            ui.end_row();
            if matches!(
                s.design,
                Design::Random | Design::LatinHypercube | Design::Sobol | Design::Inputs
            ) {
                ui.label("points");
                let mut n = s.count.unwrap_or(simsandbox_file::DEFAULT_SAMPLE_COUNT);
                let r = ui.add(egui::DragValue::new(&mut n).range(1..=simsandbox_file::MAX_SAMPLES));
                if r.changed() {
                    s.count = Some(n);
                    changed = true;
                }
                commit |= r.drag_stopped() || r.lost_focus();
                ui.end_row();
            }
            if matches!(s.design, Design::Random | Design::LatinHypercube) {
                ui.label("seed");
                let mut seed = s.seed.unwrap_or(simsandbox_file::DEFAULT_SEED);
                let r = ui.add(egui::DragValue::new(&mut seed));
                if r.changed() {
                    s.seed = Some(seed);
                    changed = true;
                }
                commit |= r.drag_stopped() || r.lost_focus();
                ui.end_row();
            }
            if s.design == Design::File {
                ui.label("file");
                let mut f = s.file.clone().unwrap_or_default();
                let r = ui
                    .add(egui::TextEdit::singleline(&mut f).hint_text("points.csv"))
                    .on_hover_text("A CSV from the model's folder: a column per parameter, by path; an id column optional");
                if r.changed() {
                    s.file = (!f.trim().is_empty()).then_some(f);
                    changed = true;
                }
                commit |= r.lost_focus();
                ui.end_row();
            }
            // A table's rows run once each (ADR 0079 stage 1d).
            if s.design != Design::Rows {
                ui.label("replicates");
                let mut reps = s.replicates.unwrap_or(1);
                let r = ui
                    .add(egui::DragValue::new(&mut reps).range(1..=10_000))
                    .on_hover_text("Runs of each point, each replicate with a seed of its own");
                if r.changed() {
                    s.replicates = (reps > 1).then_some(reps);
                    changed = true;
                }
                commit |= r.drag_stopped() || r.lost_focus();
                ui.end_row();
            }
            ui.label("seeds");
            egui::ComboBox::from_id_salt(("study-seeds", &b.name))
                .selected_text(match s.seeds {
                    Seeds::Common => "common to every point",
                    Seeds::PerPoint => "one per point",
                })
                .show_ui(ui, |ui| {
                    for (v, l) in [
                        (Seeds::Common, "common to every point"),
                        (Seeds::PerPoint, "one per point"),
                    ] {
                        if ui.selectable_value(&mut s.seeds, v, l).clicked() {
                            changed = true;
                            commit = true;
                        }
                    }
                });
            ui.end_row();
        });
    // What it varies.
    if s.design == Design::File {
        ui.weak("The file's columns are the parameters it varies.");
    } else if s.design == Design::List {
        ui.weak(format!(
            "{} parameters, {} points: edit the list as JSON below",
            s.params.len(),
            s.values.len()
        ));
    } else {
        // Points from inputs have no range: another model sends the values
        // (ADR 0078).
        let ranged = !s.design.from_inputs();
        ui.strong("varies");
        if s.design == Design::Rows {
            ui.weak("Each parameter's values arrive as a table's rows through the study's input of the same name; each row is a point, and the results leave as rows with the same ids.");
        } else if !ranged {
            ui.weak("Each parameter's values arrive through the study's input of the same name, one per point.");
        }
        let mut remove = None;
        egui::Grid::new(("study-params", &b.name))
            .num_columns(5)
            .spacing([6.0, 4.0])
            .show(ui, |ui| {
                if !s.params.is_empty() && ranged {
                    for h in ["parameter", "from", "to"] {
                        ui.weak(h);
                    }
                    ui.weak(if s.design == Design::Grid { "points" } else { "" });
                    ui.end_row();
                }
                for (k, p) in s.params.iter_mut().enumerate() {
                    egui::ComboBox::from_id_salt(("study-param", &b.name, k))
                        .selected_text(&p.param)
                        .show_ui(ui, |ui| {
                            for (key, range, ic) in keys {
                                let l = if *ic { format!("{key} (start)") } else { key.clone() };
                                if ui.selectable_label(*key == p.param, l).clicked() && *key != p.param {
                                    p.param = key.clone();
                                    p.range = ranged.then_some(*range);
                                    changed = true;
                                    commit = true;
                                }
                            }
                        });
                    if !ranged {
                        if ui.small_button("×").on_hover_text("Stop varying it").clicked() {
                            remove = Some(k);
                        }
                        ui.end_row();
                        continue;
                    }
                    let [mut lo, mut hi] = p.range.unwrap_or_else(|| range_of(&p.param));
                    let speed = ((hi - lo).abs() / 200.0).max(1e-6);
                    let a = ui.add(egui::DragValue::new(&mut lo).speed(speed)).on_hover_text("From");
                    let z = ui.add(egui::DragValue::new(&mut hi).speed(speed)).on_hover_text("To");
                    if a.changed() || z.changed() {
                        p.range = Some([lo, hi]);
                        changed = true;
                    }
                    commit |= a.drag_stopped() || a.lost_focus() || z.drag_stopped() || z.lost_focus();
                    if s.design == Design::Grid {
                        let mut n = p.points.unwrap_or(0);
                        let r = ui
                            .add(egui::DragValue::new(&mut n).range(0..=100_000).custom_formatter(|v, _| {
                                if v == 0.0 { "auto".into() } else { format!("{v:.0}") }
                            }))
                            .on_hover_text("Values along this parameter (auto: 400 for one parameter, 100 each for several)");
                        if r.changed() {
                            p.points = (n >= 2).then_some(n);
                            changed = true;
                        }
                        commit |= r.drag_stopped() || r.lost_focus();
                    } else {
                        ui.label("");
                    }
                    if ui.small_button("×").on_hover_text("Stop varying it").clicked() {
                        remove = Some(k);
                    }
                    ui.end_row();
                }
            });
        if let Some(k) = remove {
            s.params.remove(k);
            changed = true;
            commit = true;
        }
        let unused = keys
            .iter()
            .find(|(k, _, _)| !s.params.iter().any(|p| &p.param == k));
        if let Some((key, range, _)) = unused {
            if ui
                .small_button("+ parameter")
                .on_hover_text("Vary another parameter of the studied model")
                .clicked()
            {
                s.params.push(SweepControl {
                    param: key.clone(),
                    range: ranged.then_some(*range),
                    points: None,
                });
                changed = true;
                commit = true;
            }
        }
    }
    // Its named results.
    ui.strong("results").on_hover_text(
        "Each run reduced to named values, e.g. max(car.v) or first_time(abs(cruise.e) < 0.5); \
         without any, each output's final value",
    );
    let mut renamed: Option<(String, String)> = None;
    let mut dropped = None;
    for (name, formula) in s.reduce.iter_mut() {
        ui.horizontal(|ui| {
            let mut n = name.clone();
            let r = ui.add_sized([70.0, 20.0], egui::TextEdit::singleline(&mut n));
            if r.changed() {
                renamed = Some((name.clone(), n));
            }
            commit |= r.lost_focus();
            ui.label("=");
            let width = (ui.available_width() - 30.0).max(120.0);
            let r = ui.add_sized([width, 20.0], egui::TextEdit::singleline(formula));
            if r.changed() {
                changed = true;
            }
            commit |= r.lost_focus();
            if ui
                .small_button("×")
                .on_hover_text("Drop this result")
                .clicked()
            {
                dropped = Some(name.clone());
            }
        });
    }
    if let Some((from, to)) = renamed {
        if let Some(f) = s.reduce.remove(&from) {
            s.reduce.insert(to, f);
            changed = true;
        }
    }
    if let Some(n) = dropped {
        s.reduce.remove(&n);
        changed = true;
        commit = true;
    }
    if ui
        .small_button("+ result")
        .on_hover_text("Reduce each run to one more named value")
        .clicked()
    {
        let mut k = s.reduce.len() + 1;
        while s.reduce.contains_key(&format!("r{k}")) {
            k += 1;
        }
        s.reduce.insert(format!("r{k}"), "final(t)".into());
        changed = true;
        commit = true;
    }
    if changed {
        edits.insert(id.clone(), serde_json::to_string(&s).unwrap_or_default());
    }
    if commit {
        edits.remove(&id);
        if s != saved {
            out.push(Edit::SetSample {
                block: b.name.clone(),
                sample: serde_json::to_string(&s).unwrap_or_default(),
            });
        }
    }
    out.extend(search_fields(ui, b, &s, edits, keys));
    // Everything, as written in the file.
    egui::CollapsingHeader::new("sample as JSON")
        .id_salt(("study-json", &b.name))
        .show(ui, |ui| {
            let now = serde_json::to_string_pretty(&saved).unwrap_or_default();
            let jid = format!("sample:{}", b.name);
            let mut text = edits.get(&jid).cloned().unwrap_or_else(|| now.clone());
            let r = ui.add(
                egui::TextEdit::multiline(&mut text)
                    .code_editor()
                    .desired_rows(6)
                    .desired_width(f32::INFINITY),
            );
            if r.has_focus() {
                edits.insert(jid.clone(), text.clone());
            }
            if r.lost_focus() {
                edits.remove(&jid);
                if text.trim() != now.trim() {
                    out.push(Edit::SetSample {
                        block: b.name.clone(),
                        sample: text,
                    });
                }
            }
        });
    out
}

/// "Search…" (ADR 0078 point 7): a strategy model written to drive this
/// study over its parameters (their ranges as it has them): pattern
/// search, Levenberg–Marquardt, refinement or continuation, with the one
/// formula each needs. The study's sample is replaced and a strategy
/// block wired to it, which can be opened and changed like any other.
fn search_fields(
    ui: &mut egui::Ui,
    b: &simsandbox_file::BlockEntry,
    s: &simsandbox_file::Sampling,
    edits: &mut HashMap<String, String>,
    keys: &[StudyKey],
) -> Option<Edit> {
    let mut out = None;
    egui::CollapsingHeader::new("Search…")
        .id_salt(("study-search", &b.name))
        .show(ui, |ui| {
            let field = |edits: &HashMap<String, String>, f: &str, default: &str| {
                edits
                    .get(&format!("search-{f}:{}", b.name))
                    .cloned()
                    .unwrap_or_else(|| default.to_string())
            };
            let kinds = [
                ("minimize", "lower an objective (pattern search)"),
                ("fit", "fit to data (Levenberg–Marquardt)"),
                ("refine", "find where a result changes (refinement)"),
                ("continue", "follow a branch (continuation)"),
            ];
            let mut kind = field(edits, "kind", "minimize");
            egui::ComboBox::from_id_salt(("search-kind", &b.name))
                .selected_text(kinds.iter().find(|k| k.0 == kind).map_or("", |k| k.1))
                .show_ui(ui, |ui| {
                    for (k, l) in kinds {
                        ui.selectable_value(&mut kind, k.to_string(), l);
                    }
                });
            edits.insert(format!("search-kind:{}", b.name), kind.clone());
            let first = s.reduce.values().next().cloned().unwrap_or_default();
            let (label, hint) = match kind.as_str() {
                "minimize" => ("objective", "lower is better, e.g. first_time(abs(cruise.e) < 0.5)"),
                "fit" => ("residuals", "a vector per run, e.g. at_samples(osc.x - data.x, data)"),
                "refine" => ("result", "split where it changes, e.g. final(bi.x) > 0"),
                _ => ("settled", "the state a run settles at, e.g. final(bi.x)"),
            };
            let mut formula = field(edits, "formula", &first);
            ui.horizontal(|ui| {
                ui.label(label);
                ui.add(egui::TextEdit::singleline(&mut formula).hint_text(hint));
            });
            edits.insert(format!("search-formula:{}", b.name), formula.clone());
            // Each swept parameter with its range: the study's, or the
            // model's suggested one.
            let params: Vec<(String, [f64; 2])> = s
                .params
                .iter()
                .map(|p| {
                    let range = p.range.or_else(|| {
                        keys.iter().find(|k| k.0 == p.param).map(|k| k.1)
                    });
                    (p.param.clone(), range.unwrap_or([0.0, 1.0]))
                })
                .collect();
            let mut state = field(edits, "state", "");
            let mut step = field(edits, "step", "0.02").parse::<f64>().unwrap_or(0.02);
            if kind == "continue" {
                ui.horizontal(|ui| {
                    ui.label("starting value");
                    egui::ComboBox::from_id_salt(("search-state", &b.name))
                        .selected_text(state.clone())
                        .show_ui(ui, |ui| {
                            for k in keys.iter().filter(|k| k.2) {
                                ui.selectable_value(&mut state, k.0.clone(), &k.0);
                            }
                        });
                    ui.label("step");
                    ui.add(egui::DragValue::new(&mut step).speed(0.001));
                });
                edits.insert(format!("search-state:{}", b.name), state.clone());
                edits.insert(format!("search-step:{}", b.name), step.to_string());
            }
            let mut budget = field(edits, "budget", "60").parse::<f64>().unwrap_or(60.0);
            ui.horizontal(|ui| {
                ui.label("rounds at most");
                ui.add(egui::DragValue::new(&mut budget).range(1.0..=100_000.0));
            });
            edits.insert(format!("search-budget:{}", b.name), budget.to_string());
            ui.weak(match kind.as_str() {
                "refine" => format!(
                    "Over {}, each across its range: refined where the result changes.",
                    params.iter().map(|p| p.0.as_str()).collect::<Vec<_>>().join(", ")
                ),
                "continue" => format!(
                    "Over {} from {} to {}.",
                    params.first().map_or("(no parameter)", |p| p.0.as_str()),
                    params.first().map_or(0.0, |p| p.1[0]),
                    params.first().map_or(0.0, |p| p.1[1])
                ),
                _ => format!(
                    "Over {}, starting at the middle of each range.",
                    params.iter().map(|p| p.0.as_str()).collect::<Vec<_>>().join(", ")
                ),
            });
            let mid = |r: [f64; 2]| 0.5 * (r[0] + r[1]);
            let spec = match kind.as_str() {
                "minimize" => serde_json::json!({"kind": "minimize", "objective": formula, "budget": budget,
                    "params": params.iter().map(|(k, r)| serde_json::json!(
                        {"param": k, "start": mid(*r), "span": r[1] - r[0], "range": r})).collect::<Vec<_>>()}),
                "fit" => serde_json::json!({"kind": "fit", "residuals": formula, "budget": budget,
                    "params": params.iter().map(|(k, r)| serde_json::json!(
                        {"param": k, "start": mid(*r)})).collect::<Vec<_>>()}),
                // A mesh of simplices over the parameters, segments over
                // one (ADR 0085 point 7).
                "refine" => serde_json::json!({"kind": "refine", "result": formula,
                    "budget": budget, "points": if params.len() > 1 { 5 } else { 9 },
                    "max": if params.len() > 1 { 2000 } else { 200 },
                    "params": params.iter().map(|(k, r)| serde_json::json!(
                        {"param": k, "range": r, "tol": (r[1] - r[0]) * 1e-3})).collect::<Vec<_>>()}),
                _ => serde_json::json!({"kind": "continue", "settled": formula, "budget": budget,
                    "state": state, "step": step,
                    "param": params.first().map(|p| p.0.clone()).unwrap_or_default(),
                    "range": params.first().map_or([0.0, 1.0], |p| p.1)}),
            };
            if ui
                .button("Write strategy")
                .on_hover_text("Replace this study's sample and add a strategy block that drives it, one round a step")
                .clicked()
            {
                out = Some(Edit::Search {
                    study: b.name.clone(),
                    strategy: spec.to_string(),
                });
            }
        });
    out
}

/// Bytes as megabytes for people (`12.5 MB`).
fn megabytes(bytes: usize) -> String {
    let mb = bytes as f64 / 1e6;
    if mb < 10.0 {
        format!("{mb:.1} MB")
    } else {
        format!("{mb:.0} MB")
    }
}

/// How many runs a study's sample makes: points times replicates.
fn study_runs(s: &simsandbox_file::Sampling) -> usize {
    use simsandbox_file::Design;
    let points = match s.design {
        Design::Grid => {
            let auto = if s.params.len() == 1 {
                simsandbox_file::DEFAULT_SWEEP_POINTS
            } else {
                simsandbox_file::DEFAULT_GRID_POINTS
            };
            s.params
                .iter()
                .map(|p| p.points.unwrap_or(auto))
                .product::<usize>()
                * usize::from(!s.params.is_empty())
        }
        Design::List => s.values.len(),
        Design::Random | Design::LatinHypercube | Design::Sobol | Design::Inputs => {
            s.count.unwrap_or(simsandbox_file::DEFAULT_SAMPLE_COUNT)
        }
        // As many as arrive.
        Design::File | Design::Rows => 0,
    };
    points * s.replicates.unwrap_or(1)
}

/// A file reader's settings (ADR 0063): the file, typed or chosen in the
/// system dialog (relative to the model's `folder` when inside it), how
/// values go between samples, the unit, and for a `.npy` its times and
/// shape. Nothing for other blocks.
fn file_fields(
    ui: &mut egui::Ui,
    b: &simsandbox_file::BlockEntry,
    edits: &mut HashMap<String, String>,
    folder: Option<&Path>,
) -> Vec<Edit> {
    let mut out = Vec::new();
    if b.type_name != simsandbox_eq::file_input::FILE {
        return out;
    }
    let set = |key: &str, text: String| Edit::SetSetting {
        block: b.name.clone(),
        key: key.into(),
        text,
    };
    let relative = |path: std::path::PathBuf| {
        folder
            .and_then(|f| path.strip_prefix(f).ok())
            .map_or(path.clone(), Path::to_path_buf)
            .to_string_lossy()
            .replace('\\', "/")
    };
    ui.horizontal(|ui| {
        ui.label("reads");
        let hint = if folder.is_some() {
            "A .csv (time, then a column per output) or .npy, in the model's folder or a full path"
        } else {
            "A .csv or .npy: save the model first to name it from the model's folder, or give a full path"
        };
        out.extend(setting_field(ui, b, "file", 160.0, hint, edits));
        if ui.small_button("Choose…").clicked() {
            let mut dialog = rfd::FileDialog::new()
                .set_title("Data to read")
                .add_filter("Data", &["csv", "npy"]);
            if let Some(dir) = folder {
                dialog = dialog.set_directory(dir);
            }
            if let Some(path) = dialog.pick_file() {
                out.push(set("file", relative(path)));
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label("between samples");
        let now = b.settings.get("interpolate").map_or("hold", String::as_str);
        for (choice, hint) in [
            (
                "hold",
                "The last sample, as a probe feeding a block gives it",
            ),
            ("linear", "A straight line between samples"),
        ] {
            if ui
                .selectable_label(now == choice, choice)
                .on_hover_text(hint)
                .clicked()
                && now != choice
            {
                out.push(set("interpolate", choice.into()));
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label("unit");
        out.extend(setting_field(
            ui,
            b,
            "unit",
            70.0,
            "For columns whose header gives none (\"car.v [m/s]\" does), e.g. m/s",
            edits,
        ));
    });
    if b.settings.get("file").is_some_and(|f| f.ends_with(".npy")) {
        ui.horizontal(|ui| {
            ui.label("times from");
            out.extend(setting_field(
                ui,
                b,
                "times",
                110.0,
                "A CSV whose first column gives the samples' times, e.g. the probe's CSV",
                edits,
            ));
            ui.label("or every");
            out.extend(setting_field(
                ui,
                b,
                "dt",
                50.0,
                "Seconds between samples, from 0",
                edits,
            ));
            ui.label("s");
        });
    }
    out
}

/// Parameter rows nested by path: this level's in a grid, then each
/// level inside, collapsed, by name (ADR 0066 step 4b).
fn nested_rows(
    ui: &mut egui::Ui,
    n: &crate::inspector::Nested,
    salt: &str,
    row: &mut dyn FnMut(&mut egui::Ui, &str, &simsandbox_session::session::ParamRow),
) {
    if !n.rows.is_empty() {
        egui::Grid::new(("params", salt))
            .num_columns(4)
            .spacing([6.0, 4.0])
            .show(ui, |ui| {
                for (name, r) in &n.rows {
                    row(ui, name, r);
                }
            });
    }
    for (name, inner) in &n.inner {
        let salt = format!("{salt}/{name}");
        egui::CollapsingHeader::new(name)
            .id_salt(("nested", &salt))
            .default_open(false)
            .show(ui, |ui| nested_rows(ui, inner, &salt, row));
    }
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

/// Pickers for a view's slots: a menu of checkboxes for slots that take
/// several signals, a combo box otherwise (with "none" when optional).
/// `t` is shown as what it means: the history's index, time.
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
    // "t" is stored, but shown as what it is: the history's index.
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

/// A compact display of a value: up to 6 significant digits.
fn format_value(v: f64) -> String {
    simsandbox_session::diagram_paint::format_number(v)
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

/// What typed equations' native code is doing (ADR 0089), when anything:
/// being built, in use, or not used for a difference or a failed build.
fn native_status(ui: &mut egui::Ui) {
    let (ready, building, failed) = simsandbox_compiled::native::summary();
    let hover = format!(
        "Typed equations' large grids as native code built with your Rust toolchain: {ready} in use, {building} being built, {failed} not used (a failed build or different results). The results are the same either way; SIMSANDBOX_NATIVE=off turns it off."
    );
    if building > 0 {
        ui.weak("Native code: building…").on_hover_text(hover);
    } else if ready > 0 {
        ui.weak("Native code").on_hover_text(hover);
    } else if failed > 0 {
        ui.weak("Native code: not used").on_hover_text(hover);
    }
}

#[cfg(test)]
mod tests;
