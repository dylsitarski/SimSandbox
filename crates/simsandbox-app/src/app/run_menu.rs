//! The Runs menu (ADR 0059): save this run with a name and note, replay a
//! saved run, make a run the model's schedule, and write the probes' files
//! (ADR 0062).

use super::*;

impl SimApp {
    /// Write the probes' files beside the model (ADR 0062).
    pub(super) fn save_probe_data(&mut self) {
        let (Some(live), Some(dir)) = (
            self.session.as_ref(),
            self.path.as_deref().and_then(Path::parent),
        ) else {
            return;
        };
        let dir = dir.to_path_buf();
        let shown = dir.display().to_string();
        match live.call(move |s, _| s.write_probe_files(&dir).map_err(|e| e.to_string())) {
            Ok(Ok((written, notes))) => {
                for n in notes {
                    self.log.warn(n);
                }
                let names: Vec<String> = written
                    .iter()
                    .filter_map(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .collect();
                self.log
                    .info(format!("Saved probe data in {shown}: {}", names.join(", ")));
            }
            Ok(Err(e)) | Err(e) => self.error = Some(format!("{shown}: {e}")),
        }
    }

    /// The Runs menu (ADR 0059): save this run, replay a saved run of
    /// this model, or make a run the model's default schedule.
    pub(super) fn runs_menu(&mut self, ui: &mut egui::Ui) {
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
        let with_files = self
            .session
            .as_ref()
            .is_some_and(|l| l.model().probes.iter().any(|p| p.to.is_some()));
        let hover = match (&self.path, with_files) {
            (_, false) => "No probe has a file: set one in a probe's settings (double-click it on the diagram)",
            (None, true) => "Save the model first: probe files are written beside it",
            _ => "Write what the probes with a file have recorded so far, beside the model (CSV, and .npy for grids)",
        };
        if ui
            .add_enabled(
                with_files && self.path.is_some(),
                egui::Button::new("Save probe data"),
            )
            .on_hover_text(hover)
            .on_disabled_hover_text(hover)
            .clicked()
        {
            self.save_probe_data();
            ui.close();
        }
        ui.separator();
        let Some(model) = self.path.clone() else {
            ui.weak("Save the model to keep runs beside it.");
            return;
        };
        let runs = runs::runs_beside(&model);
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
    pub(super) fn current_run(
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
        let (name, note, created) = (name.to_string(), note.to_string(), runs::utc_now());
        live.call(move |s, _| s.run_file(&model, &name, &note, &created))
    }

    /// The name and note asked by "Save run…".
    pub(super) fn run_dialog(&mut self, ctx: &egui::Context) {
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
    pub(super) fn save_run(&mut self, name: &str, note: &str) {
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
        let path = runs::new_run_path(&model, name);
        if !runs::fits_disk(&run, &model, &self.registry) {
            self.log.warn(format!(
                "{}: the model changed since this run started (unsaved or since saved); \
                 the run opens with a warning. Save the model, restart and record again for one that fits",
                path.display()
            ));
        }
        match runs::write_run(&path, &run) {
            Ok(()) => self.log.info(format!(
                "Saved run '{name}' ({} scheduled, {} recorded) to {}",
                run.inputs.len(),
                run.track.len(),
                path.display()
            )),
            Err(e) => self.error = Some(e),
        }
    }

    /// Open a run file (ADR 0059): its model from disk with the run's
    /// seed, settings and schedule, then restart and replay its input.
    pub(super) fn open_run(&mut self, path: &Path) {
        let fail = |e: String| format!("{}: {e}", path.display());
        // The app's wording of a model that cannot be read or is not one.
        let its_model =
            |model: &Path, e: String| fail(format!("its model {}: {e}", model.display()));
        let runs::Opened {
            run,
            model_path: model,
            text,
        } = match runs::read_run(path) {
            Ok(opened) => opened,
            Err(e) => {
                self.error = Some(match e {
                    runs::ReadError::Run { message, .. } => fail(message),
                    runs::ReadError::Model { model, message, .. } => its_model(&model, message),
                });
                return;
            }
        };
        let file = match simsandbox_file::ModelFile::from_json(&text) {
            Ok((f, _)) => f,
            Err(e) => {
                self.error = Some(its_model(&model, e.to_string()));
                return;
            }
        };
        let replayed = run.apply_to(&file);
        let differs = replayed.run != file.run;
        let title = model
            .file_name()
            .map_or(String::new(), |n| n.to_string_lossy().into_owned());
        let previous = self.path.replace(model.clone());
        self.open_text(&title, &replayed.to_json());
        if self.error.is_some() {
            self.path = previous;
            return;
        }
        self.mark_saved();
        if !runs::fits_disk(&run, &model, &self.registry) {
            self.log.warn(format!(
                "{}: the model changed since this run was recorded; replaying it anyway",
                path.display()
            ));
        }
        if let Some(note) = run.version_note() {
            self.log.warn(format!("{}: {note}", path.display()));
        }
        let track = runs::events(&run.track);
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
    pub(super) fn use_as_schedule(&mut self, run: &simsandbox_file::runs::RunFile, name: &str) {
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
}
