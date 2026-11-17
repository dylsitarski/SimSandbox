//! The toolbar and the status bar: files, examples, the Runs, view, Data,
//! Memory and Solving menus, run controls and what the run is doing,
//! keyboard shortcuts and key input (ADR 0058), the rewind slider and the
//! log tile.

use super::*;

impl SimApp {
    pub(super) fn toolbar(&mut self, ui: &mut egui::Ui) {
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

    pub(super) fn rewind_bar(&mut self, ui: &mut egui::Ui) {
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
    pub(super) fn log_tile(&mut self, ui: &mut egui::Ui, width: f32) {
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

    /// "More views…" and "Data…": add a view, edit the filters.
    pub(super) fn view_menus(&mut self, ui: &mut egui::Ui) {
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
    pub(super) fn memory_menu(&mut self, ui: &mut egui::Ui) {
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

    /// "Solving…": how blocks are solved together (ADR 0023).
    pub(super) fn solving_menu(&mut self, ui: &mut egui::Ui) {
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

    pub(super) fn shortcuts(&mut self, ui: &mut egui::Ui) {
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
    pub(super) fn key_inputs(&mut self, ui: &mut egui::Ui) {
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

    /// Turn automatic fusion on or off (ADR 0023). The model is rebuilt from
    /// its saved form, so the run starts again at t = 0.
    pub(super) fn set_fusion(&mut self, on: bool) {
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
}

/// A step running longer than this is reported in the toolbar.
pub(super) const SLOW_STEP: Duration = Duration::from_millis(500);

/// How the model's blocks are solved (ADR 0023): the automatic-fusion switch
/// and the groups solved together. Returns the switch's new value if the
/// user changed it.
pub(super) fn solving_section(ui: &mut egui::Ui, session: &Model) -> Option<bool> {
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
pub(super) fn log_colour(visuals: &egui::Visuals, kind: LogKind, bright: bool) -> egui::Color32 {
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

/// Bytes as megabytes for people (`12.5 MB`).
pub(super) fn megabytes(bytes: usize) -> String {
    let mb = bytes as f64 / 1e6;
    if mb < 10.0 {
        format!("{mb:.1} MB")
    } else {
        format!("{mb:.0} MB")
    }
}

/// What typed equations' native code is doing (ADR 0089), when anything:
/// being built, in use, or not used for a difference or a failed build.
pub(super) fn native_status(ui: &mut egui::Ui) {
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
