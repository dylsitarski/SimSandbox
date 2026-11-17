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
mod diagram_tab;
mod files;
mod forms;
mod run_menu;
mod toolbar;
mod views;
use forms::*;

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

#[cfg(test)]
mod tests;
