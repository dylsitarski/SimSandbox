//! The view tiles (ADR 0053): each view with its title bar, style menu and
//! slot pickers, laid out in a grid; and what views send back, a clicked
//! curve (shown in the inspector) or a pointer (ADR 0058), or a scene's
//! grabs (ADR 0093).

use super::*;

impl SimApp {
    /// Views as tiles in an automatic grid, each with its slot pickers, a
    /// close button and a button to move it to a tab of its own (`grouped`)
    /// or back among the others.
    pub(super) fn view_tiles(
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
                            actions.push(DockAction::Pointer(block.clone(), p.to_vec()));
                        }
                        // A scene's grabs, each to the block its layer
                        // names, in the scene's own frame (ADR 0093).
                        for (block, p) in tile.state.grabs() {
                            actions.push(DockAction::Pointer(block, p.to_vec()));
                        }
                    });
            });
        }
        ui.allocate_rect(area, egui::Sense::hover());
    }

    /// Live input from keys or a view (ADR 0058): recorded when it changes
    /// a value. A newer value for the same key replaces one not yet sent.
    pub(super) fn send_input(&self, key: String, value: f64) {
        let Some(live) = self.session.as_ref() else {
            return;
        };
        live.send(Some(&format!("input:{key}")), move |s, c| {
            if let Err(e) = s.set_input(&key, value) {
                c.error(format!("input {key}: {e}"));
            }
        });
    }

    /// A view's pointer for an `input.pointer` block, as live input:
    /// `[x, y, down]` from a view in `controls.inputs`, or `[x, y, down,
    /// picked]` from a scene's grab (ADR 0093 point 6). Only changes are
    /// recorded.
    pub(super) fn send_pointer(&self, block: &str, p: &[f64]) {
        for (name, &v) in ["x", "y", "down", "picked"].into_iter().zip(p) {
            self.send_input(format!("{block}.{name}"), v);
        }
    }

    /// A curve was clicked: show the block it comes from in the inspector,
    /// and select it on the diagram when that shows the model level
    /// (`car1/car.v` comes from the use `car1`). Signals of filters have no
    /// block.
    pub(super) fn inspect_signal(&mut self, signal: &str) {
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
}

/// A view's style properties (ADR 0015), only those that mean something
/// for what it shows (ADR 0025), and its own picks such as profile cuts
/// (ADR 0029).
pub(super) fn style_editor(
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

/// Width of one slot picker; pickers are laid out in rows of as many as
/// fit the tile.
pub(super) const SLOT_WIDTH: f32 = 150.0;

/// Pickers for a view's slots: a menu of checkboxes for slots that take
/// several signals, a combo box otherwise (with "none" when optional).
/// `t` is shown as what it means: the history's index, time.
/// Slots that do not apply here are hidden (ADR 0025).
pub(super) fn slot_editors(
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

pub(super) fn slot_editor(
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
