//! Opening and saving: a file, an example, a dropped file or the landing
//! page's choices; the question about unsaved changes before the model is
//! replaced; Save and Save as; and rebuilding the open model from its
//! saved form.

use super::*;

impl SimApp {
    pub(super) fn open_path(&mut self, path: &str) {
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
                // Blocks read their files from the model's folder (ADR 0063).
                let previous = self.path.replace(path.into());
                let example = self.example_folder.take();
                self.diagram.forget_files();
                self.open_text(&title, &text);
                if self.error.is_none() {
                    self.mark_saved();
                } else {
                    self.path = previous;
                    self.example_folder = example;
                }
            }
            Err(e) => self.error = Some(format!("{path}: {e}")),
        }
    }

    /// Open a built-in example.
    pub(super) fn open_example(&mut self, k: usize) {
        if self.ask_first(Discard::Example(k)) {
            return;
        }
        let e = &EXAMPLES[k];
        self.diagram.forget_files();
        // An example that reads files opens from a folder holding them, a
        // fresh copy each time it is opened.
        let previous = self.example_folder.take();
        if !e.files.is_empty() {
            let dir = std::env::temp_dir()
                .join("simsandbox-examples")
                .join(e.file_name().trim_end_matches(".ssbx.json"));
            if let Err(err) = e.unpack(&dir) {
                self.example_folder = previous;
                self.error = Some(format!("{}: {err}", dir.display()));
                return;
            }
            self.example_folder = Some((dir, k));
        }
        let path = self.path.take();
        self.open_text(e.name, e.text);
        if self.error.is_none() {
            self.mark_saved();
        } else {
            self.path = path;
            self.example_folder = previous;
        }
    }

    /// The open model as it is now becomes the saved state.
    pub(super) fn mark_saved(&mut self) {
        self.baseline = self.saved_text().ok();
        self.unsaved
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }

    /// Whether the open model has changes a save would write: parameters,
    /// edits, sliders, views, speed or step.
    pub(super) fn has_unsaved(&self) -> bool {
        // Used model files edited from here (ADR 0066 step 4c).
        if !self.diagram.edited.is_empty() {
            return true;
        }
        match (&self.session, &self.baseline) {
            (Some(_), Some(b)) => self.saved_text().is_ok_and(|t| &t != b),
            _ => false,
        }
    }

    /// Before replacing the open model: if it has unsaved changes, ask
    /// (Save, Don't save, Cancel) and return true; the action runs once
    /// answered.
    pub(super) fn ask_first(&mut self, action: Discard) -> bool {
        if matches!(self.modal, Some(Modal::Discarding | Modal::Closing(_))) || !self.has_unsaved()
        {
            return false;
        }
        self.modal = Some(Modal::Unsaved(action));
        true
    }

    /// Do what was asked about, the changes saved or let go.
    pub(super) fn proceed(&mut self, ctx: &egui::Context, action: Discard) {
        self.modal = Some(Modal::Discarding);
        match action {
            Discard::OpenPath(p) => self.open_path(&p),
            Discard::OpenDialog => self.open_dialog(),
            Discard::Example(k) => self.open_example(k),
            Discard::NewModel => self.apply_equations(),
            Discard::Close => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                // Let the close through, for a few frames.
                self.modal = Some(Modal::Closing(super::CLOSING_FRAMES));
                return;
            }
        }
        if self.modal == Some(Modal::Discarding) {
            self.modal = None;
        }
    }

    /// The question about unsaved changes, while one is pending.
    pub(super) fn unsaved_dialog(&mut self, ctx: &egui::Context) {
        let Some(Modal::Unsaved(action)) = self.modal.clone() else {
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
                self.modal = None;
                self.save();
                // A cancelled Save as leaves the changes: stop there.
                if !self.has_unsaved() {
                    self.proceed(ctx, action);
                }
            }
            Some(1) => {
                self.modal = None;
                self.proceed(ctx, action);
            }
            Some(_) => self.modal = None,
            None => {}
        }
    }

    /// Ask before the window closes on unsaved changes. A close agreed to
    /// is let through; if it has not come within a few frames, the
    /// question is back for the next.
    pub(super) fn guard_close(&mut self, ctx: &egui::Context) {
        let requested = ctx.input(|i| i.viewport().close_requested());
        if let Some(Modal::Closing(frames)) = self.modal {
            if !requested {
                self.modal = frames.checked_sub(1).map(Modal::Closing);
            }
            return;
        }
        if requested
            && !matches!(self.modal, Some(Modal::Discarding | Modal::Unsaved(_)))
            && self.has_unsaved()
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.modal = Some(Modal::Unsaved(Discard::Close));
        }
    }

    /// Check now and then, on the worker, whether there are unsaved
    /// changes, for the toolbar's marker.
    pub(super) fn check_unsaved(&mut self) {
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
    pub(super) fn open_dialog(&mut self) {
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
    pub(super) fn save(&mut self) {
        match self.path.clone() {
            Some(path) => self.save_to(&path),
            None => self.save_as_dialog(),
        }
    }

    /// Save to a file chosen in the system save dialog, which asks before
    /// overwriting.
    pub(super) fn save_as_dialog(&mut self) {
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
    pub(super) fn open_text(&mut self, title: &str, text: &str) {
        self.open_text_with(title, text, true);
        if self.error.is_none() {
            self.diagram.canvas.refit();
        }
    }

    /// Open a model; it runs at once if `running`. The previous model's
    /// worker stops.
    pub(super) fn open_text_with(&mut self, title: &str, text: &str, running: bool) {
        self.open_inner(title, text, running, true);
    }

    pub(super) fn open_inner(&mut self, title: &str, text: &str, running: bool, announce: bool) {
        let folder = self
            .path
            .as_deref()
            .and_then(Path::parent)
            .or(self.example_folder.as_ref().map(|e| e.0.as_path()));
        self.diagram.folder = folder.map(Path::to_path_buf);
        // Blocks the window builds (the diagram's nodes, the Inspector,
        // equation edits) read their files from the model's folder, as the
        // session's do (ADR 0063, ADR 0069 step 3).
        simsandbox_core::registry::set_folder(folder.map(Path::to_path_buf));
        // Its studies run on the worker, which shows their progress
        // (ADR 0071): the window never waits for them.
        let loaded = simsandbox_session::study::without_running_studies(|| {
            Session::load_in(title, text, &self.registry, folder)
        });
        match loaded {
            Ok(mut s) => {
                self.speed = s.file.controls.speed.unwrap_or(1.0);
                s.set_memory_budget(self.memory_mb);
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
                // The history records what the views read (ADR 0063).
                self.view_reads = simsandbox_session::session::view_reads(&self.views.entries());
                s.set_view_reads(self.view_reads.clone());
                let tiles = self.tile_ids();
                let opened = self.views.opened.clone();
                let tile = |k: usize| opened.get(k).copied().flatten();
                self.diagram.canvas.open = s.file.controls.open.iter().cloned().collect();
                self.diagram.expanded.clone_from(&s.file.controls.expand);
                self.dock = match &s.file.controls.layout {
                    Some(l) => Dock::from_layout(l, &tile, &tiles),
                    None => Dock::standard(&tiles),
                };
                self.dock_settled = None;
                self.probe_dir = None;
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

    /// The landing page, while no model is open: start a model, open a
    /// file or pick an example.
    pub(super) fn landing(&mut self, ui: &mut egui::Ui) {
        let mut open = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // One column, centred.
                let width = ui.available_width().min(760.0);
                {
                    ui.vertical_centered(|ui| {
                        ui.set_max_width(width);
                        ui.add_space(48.0);
                        ui.heading(egui::RichText::new("SimSandbox").size(34.0).strong());
                        ui.add_space(4.0);
                        ui.label("Build a model, run it live, and change it while it runs.");
                        ui.add_space(20.0);
                        let button = |text: &str| {
                            egui::Button::new(egui::RichText::new(text).size(16.0))
                                .min_size(egui::vec2(150.0, 36.0))
                        };
                        let gap = ui.spacing().item_spacing.x;
                        ui.horizontal(|ui| {
                            ui.add_space(((width - 300.0 - gap) / 2.0).max(0.0));
                            if ui
                                .add(button("New model…"))
                                .on_hover_text("Type equations and run them")
                                .clicked()
                            {
                                self.editor = Some(Editor {
                                    block: None,
                                    text: NEW_MODEL.into(),
                                    error: None,
                                });
                            }
                            if ui
                                .add(button("Open…"))
                                .on_hover_text(
                                    "A model file (.ssbx.json) or a saved run (.run.json)",
                                )
                                .clicked()
                            {
                                self.open_dialog();
                            }
                        });
                        ui.add_space(6.0);
                        ui.weak("or drop a model file on this window");
                        ui.add_space(32.0);
                        ui.label(egui::RichText::new("Examples").strong().size(16.0));
                        ui.add_space(8.0);
                        // Two columns, each name a link with its description
                        // on hover.
                        let column = (width - gap) / 2.0;
                        let half = EXAMPLES.len().div_ceil(2);
                        ui.horizontal_top(|ui| {
                            for part in [0..half, half..EXAMPLES.len()] {
                                ui.allocate_ui_with_layout(
                                    egui::vec2(column, 0.0),
                                    egui::Layout::top_down(egui::Align::Min),
                                    |ui| {
                                        ui.set_width(column);
                                        for k in part {
                                            if ui
                                                .link(EXAMPLES[k].name)
                                                .on_hover_text(example_description(k))
                                                .clicked()
                                            {
                                                open = Some(k);
                                            }
                                            ui.add_space(2.0);
                                        }
                                    },
                                );
                            }
                        });
                        ui.add_space(24.0);
                    });
                }
            });
        if let Some(k) = open {
            self.open_example(k);
        }
    }

    pub(super) fn save_to(&mut self, path: &Path) {
        if self.session.is_none() {
            return;
        }
        let result = self
            .saved_text()
            .and_then(|text| std::fs::write(path, text).map_err(|e| e.to_string()));
        // Used model files edited from here go with it (ADR 0066 step 4c).
        let result =
            result.and_then(|()| simsandbox_session::edit_use::save(&mut self.diagram.edited));
        // An example's files go beside it, unless files of those names
        // are there already.
        let result = result.and_then(|written| {
            let mut written = written;
            if let (Some((_, k)), Some(dir)) = (&self.example_folder, path.parent()) {
                for (name, text) in EXAMPLES[*k].files {
                    let to = dir.join(name);
                    if !to.exists() {
                        std::fs::write(&to, text).map_err(|e| format!("{}: {e}", to.display()))?;
                        written.push(to);
                    }
                }
            }
            Ok(written)
        });
        match result {
            Ok(written) => {
                self.error = None;
                for f in written {
                    self.log.info(format!("Saved {}", f.display()));
                }
                self.log.info(format!("Saved to {}", path.display()));
                self.path = Some(path.to_path_buf());
                self.example_folder = None;
                self.diagram.folder = path.parent().map(Path::to_path_buf);
                simsandbox_core::registry::set_folder(self.diagram.folder.clone());
                self.mark_saved();
            }
            Err(e) => self.error = Some(format!("{}: {e}", path.display())),
        }
    }

    /// The open model as a file, with the current parameters, step, speed
    /// and views.
    pub(super) fn saved_text(&self) -> Result<String, String> {
        let live = self.session.as_ref().ok_or("no model is open")?;
        let (speed, views, layout) = (self.speed, self.views.entries(), self.layout());
        live.call(move |s, _| s.save_json_with(speed, Some(views), Some(layout)))?
    }

    /// Change the open model's file with `edit` and open it again from t = 0,
    /// keeping its path and whether it runs.
    pub(super) fn rebuild(
        &mut self,
        edit: impl FnOnce(&mut simsandbox_file::ModelFile),
        note: &str,
    ) {
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

    /// Open a file dropped on the window, unless a question or form
    /// covers it: egui's modals hold the pointer, not dropped files.
    pub(super) fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if self.modal.is_some() {
            return;
        }
        if let Some(f) = dropped.first() {
            let path = f.path().to_string_lossy().into_owned();
            self.open_path(&path);
        }
    }
}

/// A file name for saving a model titled `title`, e.g. "Lorenz system" →
/// "lorenz-system.ssbx.json".
pub(super) fn default_file_name(title: &str) -> String {
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
pub(super) fn with_model_extension(path: PathBuf) -> PathBuf {
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

/// A built-in example's description, from its file, read once.
pub(super) fn example_description(k: usize) -> &'static str {
    static DESCRIPTIONS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let all = DESCRIPTIONS.get_or_init(|| {
        EXAMPLES
            .iter()
            .map(|e| {
                serde_json::from_str::<serde_json::Value>(e.text)
                    .ok()
                    .and_then(|v| v["description"].as_str().map(String::from))
                    .unwrap_or_default()
            })
            .collect()
    });
    all.get(k).map_or("", String::as_str)
}
