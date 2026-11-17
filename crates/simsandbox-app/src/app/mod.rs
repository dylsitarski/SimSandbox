//! The window: open and save a model, run, pause, rewind, change parameters,
//! and watch it plot live.
//!
//! This module holds `SimApp`, its construction, the frame (`eframe::App`)
//! and the dock of tabs; the parts live beside it, by pane or concern:
//! `files` (open, save, the unsaved-changes question), `toolbar` (toolbar,
//! status bar, menus), `run_menu` (Runs menu, Save run), `inspector` (the
//! Inspector and Controls panes), `param_rows` (their rows and fields),
//! `forms` (forms of particular blocks), `views` (view tiles),
//! `diagram_tab` (the canvas and its outcomes), `editors` (equation editor
//! and Data window) and `data` (the Data window's contents).

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

mod data;
mod diagram_tab;
mod editors;
mod files;
mod forms;
mod inspector;
mod param_rows;
mod run_menu;
#[cfg(test)]
mod tests;
mod toolbar;
mod views;

// What the parts share; methods of `SimApp` need no import.
use data::*;
use editors::*;
use forms::*;
use inspector::*;
use param_rows::*;

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

/// Write an image as binary PPM (no image dependency needed).
fn write_ppm(path: &Path, image: &egui::ColorImage) -> std::io::Result<()> {
    let [w, h] = image.size;
    let mut bytes = format!("P6\n{w} {h}\n255\n").into_bytes();
    for p in &image.pixels {
        bytes.extend_from_slice(&[p.r(), p.g(), p.b()]);
    }
    std::fs::write(path, bytes)
}
