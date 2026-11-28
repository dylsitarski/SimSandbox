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
        // One selection (ADR 0108): the block picked anywhere, at the
        // model's own level, is shown where views draw it.
        let selected_block = match (
            &self.diagram.canvas.selected,
            self.diagram.inside.is_empty(),
        ) {
            (Some(crate::canvas::Selection::Block(b)), true) => Some(b.clone()),
            _ => None,
        };
        // The Stage's own bar (ADR 0108): views are made here.
        if grouped {
            drop(history);
            ui.horizontal(|ui| {
                ui.menu_button("+ view", |ui| self.add_view_items(ui))
                    .response
                    .on_hover_text("Add a view to the Stage: a plot or picture of what the model records, or a view file's");
                ui.weak("Press on a picture to select what it shows; grab what can be grabbed; drag an output from the Overview onto a view to show it");
            });
            if self.add_views_asked {
                self.add_views_asked = false;
                self.add_views_dialog();
            }
            return self.view_tiles(ui, ids, false, actions);
        }
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
            // The selected block, if this view draws it: its definition
            // is one step away (ADR 0108).
            let defined = selected_block
                .clone()
                .filter(|b| super::definition::draws(tile, b));
            // A signal dragged here binds to it (ADR 0108 point 4).
            let dropping = egui::DragAndDrop::payload::<super::DraggedSignal>(ui.ctx())
                .filter(|_| ui.rect_contains_pointer(cell));
            if let Some(p) = &dropping {
                if ui.input(|i| i.pointer.any_released()) {
                    actions.push(DockAction::Bind(tile.id, p.0.clone()));
                }
            }
            let builder = egui::UiBuilder::new()
                .id_salt(("tile", tile.id))
                .max_rect(cell);
            ui.scope_builder(builder, |ui| {
                let stroke = if dropping.is_some() {
                    egui::Stroke::new(2.0, ui.visuals().selection.stroke.color)
                } else if selected {
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
                                    if let Some(b) = &defined {
                                        if ui
                                            .small_button("definition")
                                            .on_hover_text(format!(
                                                "Show where {b}, selected, is defined: its equation sheet, its bodies, its design, its model or its place on the diagram"
                                            ))
                                            .clicked()
                                        {
                                            actions.push(DockAction::ShowDefinition(b.clone()));
                                        }
                                        // From what is drawn to a controller
                                        // (ADR 0108, connecting by intent).
                                        super::wiring::handle(ui, b);
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
                        tile.state.selected.clone_from(&selected_block);
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

    /// A plot of these signals against time on the Stage (ADR 0108: a view
    /// is made from where a thing is shown), brought to the front.
    pub(super) fn stage_plot(&mut self, signals: Vec<String>) {
        let Some(live) = &self.session else {
            return;
        };
        let (m, history) = (live.model(), live.history());
        let available = m.available(&history);
        let bind: simsandbox_views::Bindings = [
            ("x".to_string(), vec!["t".to_string()]),
            ("y".to_string(), signals),
        ]
        .into();
        if self.views.add_bound("plot.xy", bind, &available) {
            if let Some(id) = self.views.selected {
                self.focus = Some(Focus::Tile(id));
            }
        } else {
            self.error = Some("nothing here can be plotted against time".into());
        }
    }

    /// A signal dropped on a view: bound where it fits, or why not.
    pub(super) fn bind_dropped(&mut self, tile: u64, signal: &str) {
        let Some(live) = &self.session else {
            return;
        };
        let (m, history) = (live.model(), live.history());
        let available = m.available(&history);
        match self.views.bind_dropped(tile, signal, &available) {
            Ok(slot) => self.log.info(format!("{signal} shown as {slot}")),
            Err(e) => self.error = Some(e),
        }
    }

    /// A picture (a template a part offers: a used model's `left/cartpole`,
    /// ADR 0113, or a block's `cartpole/bodies`, ADR 0114) on the Stage:
    /// into a new scene of its unit, or into scene tile `into` as a new
    /// layer on top. The view is brought to the front.
    pub(super) fn stage_draw(&mut self, template: &str, into: Option<u64>) {
        if let Some(tile) = into {
            self.edit_scene(
                tile,
                &crate::scene_edit::SceneEdit::AddPicture(template.into()),
            );
            match self.scene_drafts.get(&tile) {
                Some(d) => self.error = Some(format!("{template}: {}", d.error)),
                None => self.focus = Some(Focus::Tile(tile)),
            }
            return;
        }
        let Some(live) = &self.session else {
            return;
        };
        let (m, history) = (live.model(), live.history());
        let available = m.available(&history);
        let mut scene = simsandbox_file::SceneSettings {
            unit: Some(self.views.template_unit(template)),
            ..Default::default()
        };
        scene
            .layers
            .push(crate::scene_edit::picture_layer(&scene, template));
        let entry = simsandbox_file::ViewEntry {
            scene,
            type_name: simsandbox_views::scene::SCENE.to_string(),
            ..Default::default()
        };
        match self.views.add_entry(&entry, &available) {
            Ok(id) => self.focus = Some(Focus::Tile(id)),
            Err(e) => self.error = Some(format!("{template}: {e}")),
        }
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
        // A block's port, or what a used model offers by name (`left/x`,
        // ADR 0110), which selects that use.
        let node = match signal.split_once('.') {
            Some((node, _)) => node,
            None if signal.contains('/') => signal,
            None => {
                self.log
                    .info(format!("{signal} is computed by a filter: see Data…"));
                return;
            }
        };
        let top = node.split('/').next().unwrap_or(node);
        if !m.file.blocks.iter().any(|b| b.name == top) {
            return;
        }
        // A body picked: its row of the mechanism's table opens (the
        // interface report, section 6: "a body selects its mechanism row").
        if let (Some(entry), Some((_, key))) = (
            m.file
                .blocks
                .iter()
                .find(|b| b.name == top && !b.bodies.is_empty()),
            signal.split_once('.'),
        ) {
            if let Some(k) = super::definition::body_row(&entry.bodies, key) {
                self.mech_open = Some((top.to_string(), k));
            }
        }
        self.picked = vec![(None, top.to_string())];
        self.inspected_view = None;
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

/// Where a picture can be drawn on the Stage (ADR 0114 point 4): a new
/// view, or a scene of its unit. Read before the parts are shown.
pub(super) struct PictureTargets {
    /// Each picture's unit, and the scenes in each unit as (tile id, place).
    units: std::collections::BTreeMap<String, String>,
    scenes: std::collections::BTreeMap<String, Vec<(u64, usize)>>,
}

impl PictureTargets {
    pub(super) fn of(views: &simsandbox_session::views::Views) -> Self {
        let units: std::collections::BTreeMap<String, String> = views
            .templates
            .keys()
            .map(|t| (t.clone(), views.template_unit(t)))
            .collect();
        let scenes = units
            .values()
            .map(|u| (u.clone(), views.scenes_in(u)))
            .collect();
        Self { units, scenes }
    }

    /// "Draw NAME": a button for a new view if no scene can take it, else
    /// a menu, "New view" or "Into view N". The picture and where it goes
    /// (`None` a new view), when chosen.
    pub(super) fn menu(
        &self,
        ui: &mut egui::Ui,
        template: &str,
        label: &str,
    ) -> Option<(String, Option<u64>)> {
        let hover = format!("Its picture {template}, on the Stage");
        let scenes = self
            .units
            .get(template)
            .and_then(|u| self.scenes.get(u))
            .filter(|s| !s.is_empty());
        let Some(scenes) = scenes else {
            return ui
                .small_button(label)
                .on_hover_text(format!("{hover}, in a view of its own"))
                .clicked()
                .then(|| (template.to_string(), None));
        };
        let mut chosen = None;
        ui.menu_button(label, |ui| {
            if ui
                .button("New view")
                .on_hover_text("A scene holding it alone")
                .clicked()
            {
                chosen = Some((template.to_string(), None));
                ui.close();
            }
            for &(id, k) in scenes {
                if ui
                    .button(format!("Into view {}", k + 1))
                    .on_hover_text("On top of what that scene draws, in its own frame")
                    .clicked()
                {
                    chosen = Some((template.to_string(), Some(id)));
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text(hover);
        chosen
    }
}
