//! The Diagram tab (ADR 0042): the node canvas, and carrying out what its
//! edits ask for (ADR 0050), including edits made elsewhere through it.

use super::*;

impl SimApp {
    /// Make an edit through the diagram, as if on the canvas at `scope`:
    /// it can be undone, and a model that does not load yet keeps it as a
    /// draft.
    pub(super) fn edit_at(&mut self, scope: Scope, edit: Edit) {
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

    /// Select a node on the canvas, as a click does: a block (`car`) or a
    /// data-layer node (`view:1`, whose scene the inspector then edits).
    pub fn select(&mut self, node: &str) {
        self.diagram.canvas.selected = Some(crate::canvas::Selection::Block(node.to_string()));
    }

    /// The node canvas (ADR 0042), and what its edits ask for.
    pub(super) fn diagram_tab(&mut self, ui: &mut egui::Ui) {
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
        self.diagram.view_problems = self.view_problems();
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
    pub(super) fn diagram_outcomes(&mut self, m: &crate::live::Model, outcomes: Vec<Outcome>) {
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
    pub(super) fn apply_data(
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
    pub(super) fn open_block(&mut self, m: &crate::live::Model, name: &str) {
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

    /// Apply edits to the model open on the canvas (inside a study): the
    /// edit question first where the model is used elsewhere too.
    pub(super) fn edit_inside(&mut self, edits: Vec<Edit>) {
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
}
