//! The Inspector's scene editor (ADR 0093): with a scene's node or tile
//! selected, its unit, extent, frames and layers as lists, each binding a
//! number, a signal or a table's quantity. Edits apply to the tile live,
//! as its other settings do, and are saved with the view. An edit that
//! does not check is kept as a draft beside the scene still drawn, with
//! `check`'s error next to the field it concerns.

use super::*;
use crate::scene_edit::{apply, SceneEdit};
use simsandbox_file::{SceneSettings, SceneValue};
use simsandbox_views::scene::spec::{
    error_path, Attribute, Kind, Unbound, FRAME, PALETTE, PRIMITIVES,
};
use simsandbox_views::scene::SCENE;

/// A scene's form changed in the Inspector that does not check yet: what
/// it was made from (the tile's settings then), what it is now, and why it
/// is not drawn.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct SceneDraft {
    base: SceneSettings,
    now: SceneSettings,
    pub(super) error: String,
}

/// What a binding can name: scalar signals, quantities over rows and
/// parameters no signal shadows (ADR 0102).
struct Choices {
    scalars: Vec<String>,
    rows: Vec<String>,
    params: Vec<String>,
}

/// What a frame's pose binds, in `FRAME`'s order.
const POSE: [&str; 3] = ["x", "y", "angle"];

impl SimApp {
    /// The scene tile the Inspector edits: one selected on the canvas
    /// (`view:K`) or by its title.
    pub(super) fn inspected_scene(&self) -> Option<u64> {
        let id = self.inspected_view?;
        let t = self.views.tiles.iter().find(|t| t.id == id)?;
        (t.state.type_name == SCENE).then_some(id)
    }

    /// Why a scene tile draws nothing or does not draw its form: its
    /// draft's error, else that of the saved form it holds.
    pub(super) fn scene_problem(&self, tile: u64) -> Option<String> {
        let t = self.views.tiles.iter().find(|t| t.id == tile)?;
        let saved = SceneSettings::from_map(&t.state.settings()).unwrap_or_default();
        match self.scene_drafts.get(&tile) {
            Some(d) if d.base == saved => Some(d.error.clone()),
            _ => t.state.problem().map(str::to_string),
        }
    }

    /// The tiles with a problem, by place, for their canvas nodes.
    pub(super) fn view_problems(&self) -> Vec<(usize, String)> {
        self.views
            .tiles
            .iter()
            .enumerate()
            .filter_map(|(k, t)| Some((k, self.scene_problem(t.id)?)))
            .collect()
    }

    /// A scene tile's form as the Inspector shows it: its draft, while the
    /// tile is still what the draft was made from, else the tile's own.
    pub(super) fn scene_form(&mut self, tile: u64) -> Option<SceneSettings> {
        let t = self.views.tiles.iter().find(|t| t.id == tile)?;
        let saved = shown_form(t);
        match self.scene_drafts.get(&tile) {
            Some(d) if d.base == saved => Some(d.now.clone()),
            Some(_) => {
                self.scene_drafts.remove(&tile);
                Some(saved)
            }
            None => Some(saved),
        }
    }

    /// Change scene tile `tile`'s form: applied to the tile at once if it
    /// checks, else kept as its draft with the error.
    pub(super) fn edit_scene(&mut self, tile: u64, edit: &SceneEdit) {
        let Some(mut form) = self.scene_form(tile) else {
            return;
        };
        if let Err(e) = apply(&mut form, edit) {
            self.error = Some(e);
            return;
        }
        let Some(live) = &self.session else {
            return;
        };
        let (m, history) = (live.model(), live.history());
        let available = m.available(&history);
        let templates = self.views.templates.clone();
        let Some(t) = self.views.tiles.iter_mut().find(|t| t.id == tile) else {
            return;
        };
        let base = shown_form(t);
        // A scene drawing templates is edited as written, and drawn
        // expanded (ADR 0113).
        // A scene gaining a template layer (a picture drawn into it, ADR
        // 0114) is written from then on.
        let as_written = t.written.clone().or_else(|| {
            form.layers
                .iter()
                .any(|l| l.draw == simsandbox_file::templates::TEMPLATE_LAYER)
                .then(|| simsandbox_session::views::shown_entry(t))
        });
        let (written, drawn) = match &as_written {
            Some(w) => {
                let w = simsandbox_file::ViewEntry {
                    scene: form.clone(),
                    ..w.clone()
                };
                let drawn = simsandbox_file::templates::expand(&w, &templates).map(|e| e.scene);
                (Some(w), drawn)
            }
            None => (None, Ok(form.clone())),
        };
        let checked = drawn.and_then(|d| t.state.set_settings(&d.to_map(), &available));
        match checked {
            Ok(()) => {
                if written.is_some() {
                    t.written = written;
                }
                self.scene_drafts.remove(&tile);
            }
            Err(error) => {
                self.scene_drafts.insert(
                    tile,
                    SceneDraft {
                        base,
                        now: form,
                        error,
                    },
                );
            }
        }
    }

    /// The scene section of the Inspector, for scene tile `tile`.
    pub(super) fn scene_section(&mut self, ui: &mut egui::Ui, tile: u64) {
        let Some(form) = self.scene_form(tile) else {
            return;
        };
        let Some(live) = &self.session else {
            return;
        };
        // What can be bound, read once; the recording is let go before
        // the edits apply.
        let (choices, pointers) = {
            let (m, history) = (live.model(), live.history());
            let available = m.available(&history);
            let choices = Choices {
                scalars: available
                    .choices(simsandbox_views::SlotKind::Scalar)
                    .into_iter()
                    .chain(available.derived.iter().map(String::as_str))
                    .map(str::to_string)
                    .collect(),
                rows: available
                    .choices(simsandbox_views::SlotKind::Rows)
                    .into_iter()
                    .map(str::to_string)
                    .collect(),
                // A signal of the same name wins (ADR 0102).
                params: available
                    .params
                    .iter()
                    .filter(|p| {
                        !available.accepts(simsandbox_views::SlotKind::Scalar, p)
                            && !available.accepts(simsandbox_views::SlotKind::Rows, p)
                    })
                    .cloned()
                    .collect(),
            };
            let pointers: Vec<String> = available.pointers.unwrap_or_default().to_vec();
            (choices, pointers)
        };
        let k = self
            .views
            .tiles
            .iter()
            .position(|t| t.id == tile)
            .unwrap_or(0);
        let draft = self.scene_drafts.get(&tile).map(|d| d.error.clone());
        // A saved form that does not check: nothing is drawn.
        let held = self
            .views
            .tiles
            .iter()
            .find(|t| t.id == tile)
            .and_then(|t| t.state.problem())
            .map(str::to_string);
        let mut edits: Vec<SceneEdit> = Vec::new();
        let templates = self.views.templates.clone();
        let mut form_ui = FormUi {
            tile,
            templates: &templates,
            error: draft
                .as_deref()
                .or(held.as_deref())
                .map(|e| (error_path(e), e)),
            placed: false,
            text: &mut self.param_edits,
            typed: &mut self.error,
            edits: &mut edits,
        };
        let mut discard = false;
        egui::CollapsingHeader::new(
            egui::RichText::new(format!("Scene (view {})", k + 1)).strong(),
        )
        .id_salt(("scene", tile))
        .default_open(true)
        .show(ui, |ui| {
            if draft.is_none() && held.is_some() {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "Not drawn: the saved scene does not check; an edit that makes it check draws it",
                );
            }
            if draft.is_some() {
                ui.horizontal(|ui| {
                    let what = match &held {
                        Some(_) => "Not drawn: neither this form nor the saved one checks",
                        None => "Not drawn yet: the scene shows its last form",
                    };
                    ui.colored_label(ui.visuals().warn_fg_color, what);
                    discard = ui
                        .small_button("discard")
                        .on_hover_text("Go back to the form the scene draws")
                        .clicked();
                });
            }
            form_ui.settings(ui, &form);
            form_ui.frames(ui, &form, &choices);
            form_ui.layers(ui, &form, &choices, &pointers);
            if let (Some((_, e)), false) = (form_ui.error, form_ui.placed) {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
        });
        ui.add_space(6.0);
        if discard {
            self.scene_drafts.remove(&tile);
        }
        for e in edits {
            self.edit_scene(tile, &e);
        }
    }
}

/// Drawing one scene's form: where typed text and edits go, and the
/// draft's error, shown once beside its field.
struct FormUi<'a, 'e> {
    tile: u64,
    /// The templates the model can draw (ADR 0113), for "Replace with…".
    templates: &'a std::collections::BTreeMap<String, serde_json::Value>,
    error: Option<(Option<&'e str>, &'e str)>,
    /// Whether the error was shown beside a field.
    placed: bool,
    text: &'a mut HashMap<String, String>,
    typed: &'a mut Option<String>,
    edits: &'a mut Vec<SceneEdit>,
}

impl FormUi<'_, '_> {
    /// A template layer (ADR 0113): what it draws and with what, and
    /// "Replace with…" among the templates its bindings can feed.
    fn template_layer(&mut self, ui: &mut egui::Ui, k: usize, l: &simsandbox_file::SceneLayer) {
        let text =
            |v: Option<&serde_json::Value>| v.and_then(|v| v.as_str()).unwrap_or("").to_string();
        let name = text(l.other.get("name"));
        let current = text(l.other.get("template"));
        ui.indent(("scene-template", self.tile, k), |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("{name}: template"));
                let bound: Vec<String> = l.bind.keys().cloned().collect();
                let fits =
                    simsandbox_file::templates::fitting(self.templates, &current, &bound, true);
                egui::ComboBox::from_id_salt(("scene-template-pick", self.tile, k))
                    .selected_text(&current)
                    .show_ui(ui, |ui| {
                        if fits.is_empty() {
                            ui.weak("no other template fits these bindings");
                        }
                        for f in &fits {
                            if ui.selectable_label(false, f).clicked() {
                                match simsandbox_file::templates::replace_layer(
                                    l,
                                    f,
                                    self.templates,
                                ) {
                                    Ok(new) => self.edits.push(SceneEdit::Layer(k, Box::new(new))),
                                    Err(e) => *self.typed = Some(e),
                                }
                            }
                        }
                    })
                    .response
                    .on_hover_text("Replace with another template its bindings can feed");
            });
            for (input, v) in &l.bind {
                let shown = match v {
                    simsandbox_file::SceneValue::Number(n) => n.to_string(),
                    simsandbox_file::SceneValue::Name(n) => n.clone(),
                    simsandbox_file::SceneValue::Other(o) => o.to_string(),
                };
                ui.weak(format!("{input} = {shown}"));
            }
        });
    }

    /// The error beside the field at `path`, in a grid row of its own.
    fn error_at(&mut self, ui: &mut egui::Ui, paths: &[&str]) {
        if let Some((Some(at), e)) = self.error {
            if paths.contains(&at) {
                ui.label("");
                let text = egui::RichText::new(e).color(ui.visuals().error_fg_color);
                ui.add(egui::Label::new(text).wrap());
                ui.end_row();
                self.placed = true;
            }
        }
    }

    fn number(&mut self, ui: &mut egui::Ui, path: &str, value: f64) -> Option<f64> {
        let key = format!("scene.{}.{path}", self.tile);
        number_entry(ui, &key, value, self.text, self.typed)
    }

    /// Unit, extent and the frame it is seen from.
    fn settings(&mut self, ui: &mut egui::Ui, s: &SceneSettings) {
        egui::Grid::new(("scene-settings", self.tile))
            .num_columns(2)
            .spacing([6.0, 4.0])
            .show(ui, |ui| {
                ui.label("unit")
                    .on_hover_text("The length unit of every position and size");
                let unit = s.unit.clone().unwrap_or_else(|| "m".into());
                let key = format!("scene.{}.unit", self.tile);
                let mut text = self.text.get(&key).cloned().unwrap_or(unit.clone());
                let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(60.0));
                if r.has_focus() {
                    self.text.insert(key.clone(), text.clone());
                }
                if r.lost_focus() {
                    self.text.remove(&key);
                    if text.trim() != unit {
                        self.edits.push(SceneEdit::Unit(text));
                    }
                }
                ui.end_row();
                self.error_at(ui, &["unit"]);
                let extent = extent_of(s);
                ui.label("extent")
                    .on_hover_text("The region shown; unticked, it follows what is drawn");
                let mut fixed = extent.is_some();
                if ui.checkbox(&mut fixed, "given").changed() {
                    let e = fixed.then_some([[-1.0, 1.0], [-1.0, 1.0]]);
                    self.edits.push(SceneEdit::Extent(e));
                }
                ui.end_row();
                // A row per axis: its low and high ends.
                if let Some([x, y]) = extent {
                    let mut next = [x, y];
                    for (axis, r) in ["x", "y"].into_iter().zip(next.iter_mut()) {
                        ui.label(format!("  {axis}"));
                        ui.horizontal(|ui| {
                            for (end, v) in r.iter_mut().enumerate() {
                                let path = format!("extent.{axis}.{end}");
                                if let Some(n) = self.number(ui, &path, *v) {
                                    *v = n;
                                }
                            }
                        });
                        ui.end_row();
                    }
                    if next != [x, y] {
                        self.edits.push(SceneEdit::Extent(Some(next)));
                    }
                }
                self.error_at(ui, &["extent", "extent.x", "extent.y"]);
                ui.label("view from")
                    .on_hover_text("The frame whose coordinates the picture is drawn in");
                if let Some(f) = frame_picker(
                    ui,
                    (self.tile, "view_from"),
                    s,
                    s.view_from.as_deref(),
                    None,
                ) {
                    self.edits.push(SceneEdit::ViewFrom(f));
                }
                ui.end_row();
                self.error_at(ui, &["view_from"]);
            });
    }

    /// Frames: each one's parent and pose; a name to add one.
    fn frames(&mut self, ui: &mut egui::Ui, s: &SceneSettings, choices: &Choices) {
        ui.add_space(4.0);
        ui.strong("Frames");
        for (name, f) in &s.frames {
            ui.horizontal(|ui| {
                // Its name, typed over to rename it: what names it follows.
                let key = format!("scene.{}.frame-name.{name}", self.tile);
                let mut text = self.text.get(&key).cloned().unwrap_or(name.clone());
                let r = ui
                    .add(egui::TextEdit::singleline(&mut text).desired_width(70.0))
                    .on_hover_text(
                        "Its name (letters, digits and '_'): type a new one and press Enter \
                         to rename it; frames and layers in it and 'view from' follow",
                    );
                if r.has_focus() {
                    self.text.insert(key.clone(), text.clone());
                }
                if r.lost_focus() {
                    self.text.remove(&key);
                    if text.trim() != name {
                        self.edits.push(SceneEdit::RenameFrame(
                            name.clone(),
                            text.trim().to_string(),
                        ));
                    }
                }
                ui.label("in");
                let id = (self.tile, "frame-in");
                if let Some(p) = frame_picker(
                    ui,
                    (id.0, name.as_str()),
                    s,
                    f.parent.as_deref(),
                    Some(name),
                ) {
                    self.edits.push(SceneEdit::FrameIn(name.clone(), p));
                }
                if ui
                    .small_button("×")
                    .on_hover_text("Remove this frame; what is in it moves to its parent")
                    .clicked()
                {
                    self.edits.push(SceneEdit::RemoveFrame(name.clone()));
                }
            });
            egui::Grid::new(("scene-frame", self.tile, name))
                .num_columns(2)
                .spacing([6.0, 4.0])
                .show(ui, |ui| {
                    self.error_at(
                        ui,
                        &[&format!("frames.{name}"), &format!("frames.{name}.in")],
                    );
                    for (a, value) in FRAME.iter().zip([&f.x, &f.y, &f.angle]) {
                        let path = format!("frames.{name}.{}", a.name);
                        ui.label(a.name).on_hover_text(a.description);
                        if let Some(v) = self.binding(ui, &path, a, value.as_ref(), choices) {
                            let attr = POSE.into_iter().find(|p| *p == a.name).unwrap_or("x");
                            self.edits.push(SceneEdit::Pose(name.clone(), attr, v));
                        }
                        ui.end_row();
                        self.error_at(ui, &[&path]);
                    }
                });
        }
        ui.horizontal(|ui| {
            let key = format!("scene.{}.new-frame", self.tile);
            let mut text = self.text.get(&key).cloned().unwrap_or_default();
            let r = ui.add(
                egui::TextEdit::singleline(&mut text)
                    .hint_text("frame name")
                    .desired_width(90.0),
            );
            self.text.insert(key.clone(), text.clone());
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.small_button("+ frame").clicked() || enter) && !text.trim().is_empty() {
                self.edits
                    .push(SceneEdit::AddFrame(text.trim().to_string()));
                self.text.remove(&key);
            }
        });
    }

    /// Layers, drawn first to last: each one's primitive, frame, grab and
    /// bindings; a primitive to add one.
    fn layers(
        &mut self,
        ui: &mut egui::Ui,
        s: &SceneSettings,
        choices: &Choices,
        pointers: &[String],
    ) {
        ui.add_space(4.0);
        ui.strong("Layers")
            .on_hover_text("Drawn first to last: the last is on top");
        for (k, l) in s.layers.iter().enumerate() {
            let here = format!("layers[{k}]");
            ui.horizontal(|ui| {
                // Numbered as `check` and the canvas name it.
                ui.label(format!("[{k}]"));
                egui::ComboBox::from_id_salt(("scene-draw", self.tile, k))
                    .width(80.0)
                    .selected_text(&l.draw)
                    .show_ui(ui, |ui| {
                        for p in PRIMITIVES {
                            if ui
                                .selectable_label(p.name == l.draw, p.name)
                                .on_hover_text(p.description)
                                .clicked()
                                && p.name != l.draw
                            {
                                self.edits.push(SceneEdit::Draw(k, p.name.into()));
                            }
                        }
                    });
                if ui
                    .add_enabled(k > 0, egui::Button::new("up").small())
                    .on_hover_text("Draw it earlier, under the one before")
                    .clicked()
                {
                    self.edits.push(SceneEdit::Raise(k));
                }
                if ui
                    .add_enabled(k + 1 < s.layers.len(), egui::Button::new("down").small())
                    .on_hover_text("Draw it later, over the one after")
                    .clicked()
                {
                    self.edits.push(SceneEdit::Raise(k + 1));
                }
                if ui
                    .small_button("×")
                    .on_hover_text("Remove this layer")
                    .clicked()
                {
                    self.edits.push(SceneEdit::RemoveLayer(k));
                }
            });
            if l.draw == simsandbox_file::templates::TEMPLATE_LAYER {
                self.template_layer(ui, k, l);
                continue;
            }
            let Some(p) = PRIMITIVES.iter().find(|p| p.name == l.draw) else {
                continue;
            };
            egui::Grid::new(("scene-layer", self.tile, k))
                .num_columns(2)
                .spacing([6.0, 4.0])
                .show(ui, |ui| {
                    let paths = [
                        here.clone(),
                        format!("{here}.draw"),
                        format!("{here}.in"),
                        format!("{here}.grab"),
                        format!("{here}.bind"),
                    ];
                    let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
                    self.error_at(ui, &paths);
                    ui.label("in")
                        .on_hover_text("The frame its positions and angles are in");
                    if let Some(f) =
                        frame_picker(ui, (self.tile, here.as_str()), s, l.frame.as_deref(), None)
                    {
                        self.edits.push(SceneEdit::LayerIn(k, f));
                    }
                    ui.end_row();
                    ui.label("grab").on_hover_text(
                        "Pressing on what this layer draws sends this input.pointer block the pointer",
                    );
                    egui::ComboBox::from_id_salt(("scene-grab", self.tile, k))
                        .width(130.0)
                        .selected_text(l.grab.as_deref().unwrap_or("none"))
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(l.grab.is_none(), "none").clicked()
                                && l.grab.is_some()
                            {
                                self.edits.push(SceneEdit::Grab(k, None));
                            }
                            for p in pointers {
                                if ui.selectable_label(l.grab.as_ref() == Some(p), p).clicked()
                                    && l.grab.as_ref() != Some(p)
                                {
                                    self.edits.push(SceneEdit::Grab(k, Some(p.clone())));
                                }
                            }
                        });
                    ui.end_row();
                    for a in p.attributes {
                        let path = format!("{here}.bind.{}", a.name);
                        ui.label(a.name).on_hover_text(a.description);
                        if let Some(v) = self.binding(ui, &path, a, l.bind.get(a.name), choices) {
                            self.edits.push(SceneEdit::Bind(k, a.name.into(), v));
                        }
                        ui.end_row();
                        self.error_at(ui, &[&path]);
                    }
                });
        }
        ui.horizontal(|ui| {
            ui.menu_button("+ layer", |ui| {
                for p in PRIMITIVES {
                    if ui.button(p.name).on_hover_text(p.description).clicked() {
                        self.edits.push(SceneEdit::AddLayer(p.name.into()));
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text("Add a layer, drawn on top of the others");
        });
    }

    /// A binding's picker (a number, a signal, a table's quantity, a
    /// palette colour or the default), and a number's field. `Some` when
    /// changed: the new binding, `None` for the default.
    fn binding(
        &mut self,
        ui: &mut egui::Ui,
        path: &str,
        a: &Attribute,
        current: Option<&SceneValue>,
        choices: &Choices,
    ) -> Option<Option<SceneValue>> {
        let mut out = None;
        let default = match a.default {
            Unbound::Required => "required".to_string(),
            Unbound::Number(x) => format!("default ({})", format_value(x)),
            Unbound::Colour(c) => format!("default ({c})"),
            Unbound::Nothing => "none".to_string(),
        };
        let shown = match current {
            Some(SceneValue::Number(_)) => "number".to_string(),
            Some(SceneValue::Name(n)) if choices.params.contains(n) => format!("{n} · parameter"),
            Some(SceneValue::Name(n)) => n.clone(),
            Some(SceneValue::Other(v)) => v.to_string(),
            None => default.clone(),
        };
        let start = match a.default {
            Unbound::Number(x) => x,
            _ => 0.0,
        };
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt(("scene-bind", self.tile, path))
                .width(130.0)
                .truncate()
                .selected_text(&shown)
                .show_ui(ui, |ui| {
                    let number = matches!(current, Some(SceneValue::Number(_)));
                    if ui.selectable_label(number, "number").clicked() && !number {
                        out = Some(Some(SceneValue::Number(start)));
                    }
                    if a.default != Unbound::Required
                        && ui.selectable_label(current.is_none(), &default).clicked()
                        && current.is_some()
                    {
                        out = Some(None);
                    }
                    let mut names = |ui: &mut egui::Ui, list: &mut dyn Iterator<Item = &str>| {
                        for n in list {
                            let on = matches!(current, Some(SceneValue::Name(c)) if c == n);
                            if ui.selectable_label(on, n).clicked() && !on {
                                out = Some(Some(SceneValue::Name(n.to_string())));
                            }
                        }
                    };
                    if a.kind == Kind::Colour {
                        ui.separator();
                        names(ui, &mut PALETTE.iter().map(|(n, _)| *n));
                    }
                    ui.separator();
                    names(ui, &mut choices.scalars.iter().map(String::as_str));
                    if !choices.rows.is_empty() {
                        ui.separator();
                        ui.weak("over a table's rows");
                        names(ui, &mut choices.rows.iter().map(String::as_str));
                    }
                    if !choices.params.is_empty() {
                        ui.separator();
                        ui.weak("parameters, at each instant drawn");
                        names(ui, &mut choices.params.iter().map(String::as_str));
                    }
                })
                .response
                .on_hover_text(format!("{} ({})", a.description, a.kind.unit()));
            if let Some(SceneValue::Number(x)) = current {
                if let Some(v) = self.number(ui, path, *x) {
                    out = Some(Some(SceneValue::Number(v)));
                }
            }
        });
        out
    }
}

/// The extent's ranges, if given as `{"x": [lo, hi], "y": [lo, hi]}`.
fn extent_of(s: &SceneSettings) -> Option<[[f64; 2]; 2]> {
    let e = s.extent.as_ref()?;
    let r = |a: &str| -> Option<[f64; 2]> {
        let v = e.get(a)?.as_array()?;
        Some([v.first()?.as_f64()?, v.get(1)?.as_f64()?])
    };
    Some([r("x")?, r("y")?])
}

/// A frame picker: the scene's own ("scene") or one of its frames, but
/// not `except`. `Some` when changed.
fn frame_picker(
    ui: &mut egui::Ui,
    id: (u64, &str),
    s: &SceneSettings,
    current: Option<&str>,
    except: Option<&str>,
) -> Option<Option<String>> {
    let mut out = None;
    egui::ComboBox::from_id_salt(("scene-frame-pick", id.0, id.1))
        .width(130.0)
        .selected_text(current.unwrap_or("scene"))
        .show_ui(ui, |ui| {
            if ui.selectable_label(current.is_none(), "scene").clicked() && current.is_some() {
                out = Some(None);
            }
            for n in s.frames.keys().filter(|n| Some(n.as_str()) != except) {
                let on = current == Some(n.as_str());
                if ui.selectable_label(on, n).clicked() && !on {
                    out = Some(Some(n.clone()));
                }
            }
        });
    out
}

/// A scene tile's form as written: the view the file keeps when it draws
/// templates (ADR 0113), else the settings it draws.
fn shown_form(t: &simsandbox_session::views::Tile) -> SceneSettings {
    match &t.written {
        Some(w) => w.scene.clone(),
        None => SceneSettings::from_map(&t.state.settings()).unwrap_or_default(),
    }
}
