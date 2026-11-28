//! The open model's views, shown together as tiles (ADR 0015, ADR 0016):
//! the views the model file lists, plus any the user adds from a view
//! type's presets under "More views…" (ADR 0018). Save writes the tiles as
//! they are.

use simsandbox_file::{Binding, CameraControl, SceneSettings, ViewEntry};
use simsandbox_views::{Available, Bindings, ViewRegistry, ViewState, ViewType};
use std::collections::BTreeMap;

/// One view on screen. The id stays fixed while other tiles come and go, so
/// each tile keeps its own plot zoom and widget state.
pub struct Tile {
    pub id: u64,
    pub state: ViewState,
    /// Where its node sits on the canvas's data layer (ADR 0050).
    pub canvas: Option<[f64; 2]>,
    /// The view as written when it draws templates (ADR 0113): saved back
    /// as it is; its state is the view expanded.
    pub written: Option<ViewEntry>,
}

pub struct Views {
    pub registry: ViewRegistry,
    pub tiles: Vec<Tile>,
    /// The tile whose style properties the side panel shows.
    pub selected: Option<u64>,
    /// For each view [`Views::open`] was given, the tile showing it
    /// (`None` if it could not be shown), so a saved arrangement can find
    /// its views (ADR 0053).
    pub opened: Vec<Option<u64>>,
    /// The model's templates (ADR 0113), which views may draw.
    pub templates: BTreeMap<String, serde_json::Value>,
    next_id: u64,
}

impl Views {
    pub fn new(registry: ViewRegistry) -> Self {
        Self {
            registry,
            tiles: Vec::new(),
            selected: None,
            opened: Vec::new(),
            templates: BTreeMap::new(),
            next_id: 0,
        }
    }

    /// Start over for a model offering `available`: the saved views, or the
    /// first preset that fits (a plot against time, if the model records
    /// scalars; a heat map for a sweep over two parameters) if none can be
    /// used.
    /// Returns warnings for saved views that cannot be used.
    pub fn open(&mut self, available: &Available<'_>, saved: &[ViewEntry]) -> Vec<String> {
        self.tiles.clear();
        self.selected = None;
        self.opened.clear();
        let mut warnings = Vec::new();
        for (i, entry) in saved.iter().enumerate() {
            match self.restore(entry, available) {
                Ok((state, problem)) => {
                    // A scene that does not check is kept, saying why it
                    // is not drawn, so it is saved back as it was.
                    if let Some(e) = problem {
                        warnings.push(format!("controls.views[{i}]: {e}; not drawn"));
                    }
                    let id = self.push(state);
                    self.tile_mut(id).canvas = entry.canvas;
                    self.tile_mut(id).written = written(entry);
                    self.opened.push(Some(id));
                }
                Err(e) => {
                    warnings.push(format!("controls.views[{i}]: {e}; ignored"));
                    self.opened.push(None);
                }
            }
        }
        let fallback = self.tiles.is_empty();
        // A model that writes no views draws what its blocks offer
        // (ADR 0114 point 4), layered, one scene per unit.
        if saved.is_empty() {
            for e in self.default_pictures() {
                if let Err(err) = self.add_entry(&e, available) {
                    warnings.push(format!("its blocks' pictures: {err}; not drawn"));
                }
            }
        }
        if fallback {
            // A study's results against what it varies (ADR 0070): over two
            // parameters (2D results, ADR 0071) best seen as a heat map.
            let studied = available
                .block_types
                .iter()
                .any(|t| t == crate::study::STUDY);
            let two_d = available.grids.iter().any(|g| !available.lines.contains(g));
            let heat = (studied && two_d).then_some(("plot.heatmap", 0));
            let study = studied.then_some(("plot.profile", 0));
            let first_fit = heat
                .into_iter()
                .chain(study)
                .chain(
                    self.registry
                        .types()
                        .iter()
                        .flat_map(|t| (0..t.presets.len()).map(move |p| (t.name, p))),
                )
                .find(|&(n, p)| self.fits(n, p, available));
            if let Some((n, p)) = first_fit {
                self.add(n, p, available);
            }
        }
        self.selected = self.tiles.first().map(|t| t.id);
        warnings
    }

    /// A tile's state for an entry, and the error of settings it holds
    /// without drawing them (a scene that does not check).
    fn restore(
        &self,
        e: &ViewEntry,
        available: &Available<'_>,
    ) -> Result<(ViewState, Option<String>), String> {
        // The view it draws, its templates put in place (ADR 0113).
        let expanded;
        let e = if simsandbox_file::templates::uses_templates(e) {
            expanded = simsandbox_file::templates::expand(e, &self.templates)?;
            &expanded
        } else {
            e
        };
        let t = self
            .registry
            .get(&e.type_name)
            .ok_or_else(|| format!("unknown view type '{}'", e.type_name))?;
        let bind: Bindings = e
            .bind
            .iter()
            .map(|(k, v)| (k.clone(), v.signals()))
            .collect();
        let camera = e.camera.map(|c| [c.yaw, c.pitch, c.zoom]);
        let mut state = ViewState::restore(t, bind, e.style.clone(), camera, available)?;
        // What it takes beyond slots: a scene's frames and layers
        // (ADR 0093), each problem at its path. A scene that does not
        // check holds them, to show why and save them back.
        let settings = e.scene.to_map();
        let mut problem = None;
        if let Err(err) = state.set_settings(&settings, available) {
            if !state.hold_settings(&settings, &err) {
                return Err(err);
            }
            problem = Some(err);
        }
        if let Some(s) = &e.selection {
            state
                .set_selection(s)
                .map_err(|err| format!("selection: {err}"))?;
        }
        Ok((state, problem))
    }

    fn push(&mut self, state: ViewState) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.tiles.push(Tile {
            id,
            state,
            canvas: None,
            written: None,
        });
        id
    }

    fn tile_mut(&mut self, id: u64) -> &mut Tile {
        self.tiles
            .iter_mut()
            .find(|t| t.id == id)
            .expect("a tile just added")
    }

    /// Make the tiles these entries, as canvas edits leave them (ADR 0050):
    /// a tile whose entry is unchanged stays as it is (zoom and all); a new
    /// entry with nothing bound starts from its type's first preset.
    /// Returns warnings for entries that cannot be shown.
    pub fn set_entries(&mut self, entries: &[ViewEntry], available: &Available<'_>) -> Vec<String> {
        let old = self.entries();
        let mut kept: Vec<Option<Tile>> = std::mem::take(&mut self.tiles)
            .into_iter()
            .map(Some)
            .collect();
        let mut warnings = Vec::new();
        for (i, e) in entries.iter().enumerate() {
            let same = old.iter().enumerate().position(|(k, o)| {
                kept[k].is_some()
                    && ViewEntry {
                        canvas: e.canvas,
                        ..o.clone()
                    } == *e
            });
            // Kept as it is, unless it holds settings that did not check:
            // those are checked again.
            if let Some(k) = same.filter(|&k| {
                kept[k]
                    .as_ref()
                    .is_some_and(|t| t.state.problem().is_none())
            }) {
                let mut tile = kept[k].take().expect("checked");
                tile.canvas = e.canvas;
                self.tiles.push(tile);
                continue;
            }
            let state = match self.restore(e, available) {
                Ok((s, problem)) => {
                    if let Some(err) = problem {
                        warnings.push(format!("view {}: {err}; not drawn", i + 1));
                    }
                    Ok(s)
                }
                Err(err) if e.bind.is_empty() => self
                    .registry
                    .get(&e.type_name)
                    .and_then(|t| ViewState::from_preset(t, 0, available))
                    .ok_or(err),
                Err(err) => Err(err),
            };
            match state {
                Ok(state) => {
                    let id = self.push(state);
                    self.tile_mut(id).canvas = e.canvas;
                    self.tile_mut(id).written = written(e);
                }
                Err(err) => warnings.push(format!("view {}: {err}; not shown", i + 1)),
            }
        }
        if !self.tiles.iter().any(|t| Some(t.id) == self.selected) {
            self.selected = self.tiles.first().map(|t| t.id);
        }
        warnings
    }

    /// Whether a view type applies to the model and its preset can be
    /// filled from what the model offers.
    pub fn fits(&self, type_name: &str, preset: usize, available: &Available<'_>) -> bool {
        self.registry
            .get(type_name)
            .filter(|t| t.applies(available))
            .and_then(|t| t.presets.get(preset))
            .is_some_and(|p| (p.bindings)(available).is_some())
    }

    /// Add a tile of a view type started from one of its presets, and select
    /// it. Returns false if the preset cannot show these signals.
    pub fn add(&mut self, type_name: &str, preset: usize, available: &Available<'_>) -> bool {
        let Some(state) = self
            .registry
            .get(type_name)
            .and_then(|t| ViewState::from_preset(t, preset, available))
        else {
            return false;
        };
        let id = self.push(state);
        self.selected = Some(id);
        true
    }

    /// Add a tile with these bindings, and select it. Returns false if
    /// they do not fit the type.
    pub fn add_bound(
        &mut self,
        type_name: &str,
        bind: Bindings,
        available: &Available<'_>,
    ) -> bool {
        let Some(state) = self
            .registry
            .get(type_name)
            .and_then(|t| ViewState::restore(t, bind, Default::default(), None, available).ok())
        else {
            return false;
        };
        let id = self.push(state);
        self.selected = Some(id);
        true
    }

    /// The scenes a model that writes no views opens with: see
    /// [`default_pictures`].
    pub fn default_pictures(&self) -> Vec<ViewEntry> {
        default_pictures(&self.templates)
    }

    /// The view an entry draws, templates put in place, without adding a
    /// tile (a picture shown beside a definition, ADR 0108).
    pub fn state_for(&self, e: &ViewEntry, available: &Available<'_>) -> Result<ViewState, String> {
        let (state, problem) = self.restore(e, available)?;
        match problem {
            Some(p) => Err(p),
            None => Ok(state),
        }
    }

    /// A tile for an entry as written (a scene drawing a template, ADR
    /// 0113), selected; its id, or why it cannot be shown.
    pub fn add_entry(&mut self, e: &ViewEntry, available: &Available<'_>) -> Result<u64, String> {
        let (state, problem) = self.restore(e, available)?;
        if let Some(p) = problem {
            return Err(p);
        }
        let id = self.push(state);
        self.tile_mut(id).written = written(e);
        self.selected = Some(id);
        Ok(id)
    }

    /// Show only one view, from a preset (the `--view` flag).
    pub fn show_only(&mut self, type_name: &str, preset: usize, available: &Available<'_>) {
        let before = std::mem::take(&mut self.tiles);
        if !self.add(type_name, preset, available) {
            self.tiles = before;
        }
    }

    pub fn close(&mut self, id: u64) {
        self.tiles.retain(|t| t.id != id);
        if self.selected == Some(id) {
            self.selected = self.tiles.first().map(|t| t.id);
        }
    }

    /// The selected tile's view and type.
    pub fn selected_mut(&mut self) -> Option<(&ViewType, &mut ViewState)> {
        let id = self.selected?;
        let tile = self.tiles.iter_mut().find(|t| t.id == id)?;
        let t = self.registry.get(&tile.state.type_name)?;
        Some((t, &mut tile.state))
    }

    /// The tiles as saved in `controls.views`, in order: a view drawing
    /// templates as written (ADR 0113).
    pub fn entries(&self) -> Vec<ViewEntry> {
        self.tiles
            .iter()
            .zip(self.shown_entries())
            .map(|(tile, shown)| match &tile.written {
                Some(w) => ViewEntry {
                    canvas: tile.canvas,
                    ..w.clone()
                },
                None => shown,
            })
            .collect()
    }

    /// The tiles as they draw: templates put in place. What they read.
    pub fn shown_entries(&self) -> Vec<ViewEntry> {
        self.tiles.iter().map(shown_entry).collect()
    }

    /// A signal dropped on tile `tile` (ADR 0108 point 4: a view is bound
    /// by dragging a signal onto it): added to the first slot that takes
    /// several and can show it (a plot's `y`), else put in the first slot
    /// of one that can. The slot it went into, or why it cannot.
    pub fn bind_dropped(
        &mut self,
        tile: u64,
        signal: &str,
        available: &Available<'_>,
    ) -> Result<&'static str, String> {
        let tile = self
            .tiles
            .iter_mut()
            .find(|t| t.id == tile)
            .ok_or("no such view")?;
        let t = self
            .registry
            .get(&tile.state.type_name)
            .ok_or_else(|| format!("unknown view type '{}'", tile.state.type_name))?;
        if tile.written.is_some() || t.slots.is_empty() {
            return Err(format!(
                "{} binds in its form: select it and bind there",
                t.label
            ));
        }
        let takes = |s: &&simsandbox_views::view::Slot| available.choices(s.kind).contains(&signal);
        let slot = t
            .slots
            .iter()
            .filter(|s| s.many)
            .find(takes)
            .or_else(|| t.slots.iter().filter(|s| !s.many).find(takes))
            .ok_or_else(|| format!("{} cannot show {signal}", t.label))?;
        let bound = tile.state.bind.entry(slot.name.to_string()).or_default();
        if slot.many {
            if !bound.iter().any(|b| b == signal) {
                bound.push(signal.to_string());
            }
        } else {
            *bound = vec![signal.to_string()];
        }
        self.selected = Some(tile.id);
        Ok(slot.name)
    }

    /// The length unit a template draws in (`m` if it does not say).
    pub fn template_unit(&self, template: &str) -> String {
        self.templates
            .get(template)
            .and_then(|t| t.get("unit"))
            .and_then(|u| u.as_str())
            .unwrap_or("m")
            .to_string()
    }

    /// The pictures parts offer (ADR 0113, ADR 0114): templates named by a
    /// block's or a use's path, `cartpole/bodies`, `left/cartpole`.
    pub fn pictures(&self) -> Vec<&str> {
        self.templates
            .keys()
            .filter(|k| k.contains('/'))
            .map(String::as_str)
            .collect()
    }

    /// The scene tiles a picture in `unit` can be drawn into: their ids
    /// and places among the tiles (ADR 0114 point 4).
    pub fn scenes_in(&self, unit: &str) -> Vec<(u64, usize)> {
        self.tiles
            .iter()
            .enumerate()
            .filter(|(_, t)| t.state.type_name == simsandbox_views::scene::SCENE)
            .filter(|(_, t)| {
                let written = t.written.as_ref().map(|w| w.scene.unit.clone());
                let drawn = t
                    .state
                    .settings()
                    .get("unit")
                    .and_then(|u| u.as_str())
                    .map(str::to_string);
                written.unwrap_or(drawn).as_deref().unwrap_or("m") == unit
            })
            .map(|(k, t)| (t.id, k))
            .collect()
    }
}

/// A tile as it draws: its template put in place.
pub fn shown_entry(tile: &Tile) -> ViewEntry {
    let s = &tile.state;
    ViewEntry {
        type_name: s.type_name.clone(),
        template: None,
        params: BTreeMap::new(),
        bind: s
            .bind
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| (k.clone(), Binding::from(v.clone())))
            .collect(),
        style: s.style.clone(),
        camera: s.camera().map(|c| {
            let [yaw, pitch, zoom] = c.map(tidy);
            CameraControl { yaw, pitch, zoom }
        }),
        selection: s.selection(),
        canvas: tile.canvas,
        probe: None,
        scene: SceneSettings::from_map(&s.settings()).unwrap_or_default(),
    }
}

/// A view drawing templates, as written, to save back.
fn written(e: &ViewEntry) -> Option<ViewEntry> {
    simsandbox_file::templates::uses_templates(e).then(|| e.clone())
}

/// The camera turns in single precision; save its shortest decimal form
/// (0.35, not 0.3499999940395355).
fn tidy(v: f64) -> f64 {
    (v as f32).to_string().parse().unwrap_or(v)
}

/// Columns and rows for `n` tiles: as square as possible, filled row by row.
pub fn grid(n: usize) -> (usize, usize) {
    if n == 0 {
        return (0, 0);
    }
    let cols = (n as f64).sqrt().ceil() as usize;
    (cols, n.div_ceil(cols))
}

/// The scenes a model that writes no views opens with (ADR 0114 point
/// 4): every picture its own blocks offer (`block/name`), layered in
/// the scene's own frame, one scene per length unit, by block name.
pub fn default_pictures(templates: &BTreeMap<String, serde_json::Value>) -> Vec<ViewEntry> {
    let mut by_unit: BTreeMap<String, Vec<serde_json::Value>> = BTreeMap::new();
    for (name, t) in templates {
        let own_block = name.matches('/').count() == 1
            && t.get("offered_by").and_then(|v| v.as_str()) == Some("block");
        if !own_block {
            continue;
        }
        let unit = t
            .get("unit")
            .and_then(|u| u.as_str())
            .unwrap_or("m")
            .to_string();
        let layer: String = name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        by_unit.entry(unit).or_default().push(serde_json::json!({
            "draw": "template", "name": layer, "template": name
        }));
    }
    by_unit
        .into_iter()
        .filter_map(|(unit, layers)| {
            serde_json::from_value(serde_json::json!({
                "type": "plot.scene", "unit": unit, "layers": layers
            }))
            .ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_model_holding_a_study_opens_on_its_results_against_what_it_varies() {
        // ADR 0070 step 5: a profile of the study's results, its swept
        // parameter as the coordinate.
        use crate::examples::{registry, view_registry};
        use crate::session::Session;
        let model = crate::examples::spring();
        let file = serde_json::json!({"format": 1, "models": {"resonance": model},
            "blocks": [{"name": "s", "type": "study", "model": "resonance",
                "sample": {"design": "grid", "params": [{"param": "spring.w", "range": [0.5, 1.5], "points": 11}]}}]});
        let s = Session::load("study", &file.to_string(), &registry()).unwrap();
        let mut v = Views::new(view_registry());
        v.open(&s.available(&s.signals()), &[]);
        assert_eq!(v.tiles[0].state.type_name, "plot.profile");
        let b = &v.tiles[0].state.bind;
        assert_eq!(b["x"], ["s.spring_w"]);
        assert!(b["y"].contains(&"s.spring_amplitude".to_string()), "{b:?}");
    }

    use super::*;

    fn signals() -> Vec<String> {
        vec!["l.xyz[0]".into(), "l.xyz[1]".into(), "l.xyz[2]".into()]
    }

    fn avail(scalars: &[String]) -> Available<'_> {
        Available {
            scalars,
            ..Default::default()
        }
    }

    fn entry(type_name: &str, bind: &[(&str, &str)]) -> ViewEntry {
        ViewEntry {
            type_name: type_name.into(),
            bind: bind
                .iter()
                .map(|(k, v)| (k.to_string(), Binding::One(v.to_string())))
                .collect(),
            style: [("trail_length".to_string(), 5.0)].into(),
            camera: Some(CameraControl {
                yaw: 1.0,
                pitch: 0.5,
                zoom: 2.0,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn opens_the_saved_views_and_saves_them_back_unchanged() {
        let mut v = Views::new(ViewRegistry::builtin());
        let saved = [
            entry(
                "plot.3d",
                &[
                    ("x", "l.xyz[2]"),
                    ("y", "l.xyz[0]"),
                    ("z", "l.xyz[1]"),
                    ("colour", "t"),
                ],
            ),
            entry("plot.xy", &[("x", "l.xyz[0]"), ("y", "l.xyz[1]")]),
        ];
        let mut phase = saved[1].clone();
        phase.camera = None;
        assert!(v
            .open(&avail(&signals()), &[saved[0].clone(), phase.clone()])
            .is_empty());
        assert_eq!(v.tiles.len(), 2);
        assert_eq!(v.entries(), [saved[0].clone(), phase]);
        assert_eq!(v.selected, Some(v.tiles[0].id));
    }

    #[test]
    fn unusable_views_are_dropped_and_an_empty_list_gets_a_time_series() {
        let mut v = Views::new(ViewRegistry::builtin());
        let unknown = entry("plot.4d", &[]);
        let unplotted = entry("plot.xy", &[("x", "l.xyz[0]"), ("y", "other.x")]);
        let w = v.open(&avail(&signals()), &[unknown, unplotted]);
        assert_eq!(w.len(), 2, "{w:?}");
        assert_eq!(v.tiles.len(), 1);
        assert_eq!(v.tiles[0].state.type_name, "plot.xy");
        assert_eq!(v.tiles[0].state.bind["x"], ["t"]);
    }

    #[test]
    fn tiles_are_added_closed_and_keep_their_ids() {
        let mut v = Views::new(ViewRegistry::builtin());
        v.open(&avail(&signals()), &[]);
        assert!(v.add("plot.3d", 0, &avail(&signals())));
        assert!(v.add("plot.xy", 1, &avail(&signals())));
        let ids: Vec<u64> = v.tiles.iter().map(|t| t.id).collect();
        assert_eq!(v.selected, Some(ids[2]));
        v.close(ids[2]);
        assert_eq!(v.selected, Some(ids[0]));
        assert_eq!(v.tiles.iter().map(|t| t.id).collect::<Vec<_>>(), &ids[..2]);
        // New tiles never reuse an id.
        assert!(v.add("plot.xy", 1, &avail(&signals())));
        assert!(v.tiles.last().unwrap().id > ids[2]);
        // Too few signals for 3D.
        assert!(!v.fits("plot.3d", 0, &avail(&signals()[..2])));
        assert!(!v.add("plot.3d", 0, &avail(&signals()[..2])));
        v.show_only("plot.3d", 0, &avail(&signals()));
        assert_eq!(v.tiles.len(), 1);
    }

    #[test]
    fn every_example_opens_its_saved_views() {
        use crate::examples::{registry, view_registry, EXAMPLES};
        use crate::session::Session;
        for e in EXAMPLES {
            let s = Session::load_in(e.name, e.text, &registry(), e.folder().as_deref()).unwrap();
            let mut v = Views::new(view_registry());
            v.templates = s.templates();
            let w = v.open(&s.available(&s.signals()), &s.file.controls.views);
            assert!(w.is_empty(), "{}: {w:?}", e.name);
            assert_eq!(v.entries(), s.file.controls.views, "{}", e.name);
        }
    }

    #[test]
    fn a_study_over_two_parameters_opens_on_a_heat_map_of_its_results() {
        use crate::examples::{registry, view_registry, EXAMPLES};
        use crate::session::Session;
        let e = EXAMPLES
            .iter()
            .find(|e| e.name.contains("Resonance map"))
            .unwrap();
        let s = Session::load(e.name, e.text, &registry()).unwrap();
        let mut v = Views::new(view_registry());
        v.open(&s.available(&s.signals()), &[]);
        let b = &v.tiles[0].state.bind;
        assert_eq!(v.tiles[0].state.type_name, "plot.heatmap");
        assert_eq!(b["field"], ["map.spring_amplitude"]);
        assert_eq!((b["x"][0].as_str(), b["y"][0].as_str()), ("map.w", "map.c"));
    }

    /// A model whose typed equations compose the pose of a point drawn in
    /// a chain of frames (ADR 0093 Consequences): a frame on a moving
    /// base, turned by a constant, holding one turned by the model's angle
    /// and offset along it.
    const COMPOSED: &str = r#"{"format": 1, "run": {"dt": 0.01},
        "blocks": [{"name": "m", "type": "eq.equations", "method": {"name": "rk4"}, "equations": [
            "param v [m/s] = 0.3", "param w [1/s] = 0.7",
            "param y0 [m] = 0.5", "param d [m] = 0.2", "param a [m] = 0.4", "param b [m] = 0.25",
            "state cx [m] = 0", "dcx/dt = v", "state th = 0", "dth/dt = w",
            "ex = cx + d*cos(0.6) + a*cos(th - 0.6) - b*sin(th - 0.6)",
            "ey = y0 - d*sin(0.6) + a*sin(th - 0.6) + b*cos(th - 0.6)"]}],
        "controls": {"views": [
            {"type": "plot.scene", "frames": {
                "base": {"x": "m.cx", "y": 0.5},
                "turn": {"in": "base", "angle": -0.6},
                "arm": {"in": "turn", "x": 0.2, "angle": "m.th"}},
             "layers": [{"draw": "points", "in": "arm", "bind": {"x": 0.4, "y": 0.25}}]},
            {"type": "plot.xy", "bind": {"x": "m.ex", "y": ["m.ey"]}}]}}"#;

    /// Frames compose as typed equations compose the same pose: they
    /// agree to rounding at every sample drawn.
    #[test]
    fn frames_agree_with_the_same_pose_in_typed_equations() {
        use crate::examples::{registry, view_registry};
        use crate::session::Session;
        use simsandbox_views::scene::{eval::Placed, geometry::Instance, Scene};
        use simsandbox_views::Recorded;
        let mut s = Session::load("composed", COMPOSED, &registry()).unwrap();
        let mut v = Views::new(view_registry());
        let w = v.open(&s.available(&s.signals()), &s.file.controls.views);
        assert!(w.is_empty(), "{w:?}");
        // The scene's reads are recorded with the plot's.
        let reads = crate::session::view_reads(&s.file.controls.views);
        assert_eq!(reads, ["m.cx", "m.ex", "m.ey", "m.th"]);
        let settings = s.file.controls.views[0].scene.to_map();
        let mut worst: f64 = 0.0;
        for k in 1..=40 {
            s.run_until(0.25 * k as f64).unwrap();
            let h = s.signals();
            let scene = Scene::check(&settings, &s.available(&h)).unwrap();
            let placed: Vec<Placed> = scene.place(&Recorded::single(&h));
            let Instance::Point { at, .. } = placed[0].shape else {
                panic!("{placed:?}")
            };
            let n = h.len() - 1;
            let value = |name: &str| h.value(h.resolve(name).unwrap(), n);
            let want = [value("m.ex"), value("m.ey")];
            worst = worst.max((at[0] - want[0]).abs().max((at[1] - want[1]).abs()));
        }
        assert!(
            worst < 1e-12,
            "frames and typed equations differ by {worst}"
        );
    }

    /// A scene binding a parameter (the pendulum's rod, `bob.L`) draws it
    /// at its value at the drawn instant (ADR 0102): the history records
    /// it every step, so the picture follows a change, and a rewound or
    /// replayed run draws as it was then.
    #[test]
    fn a_parameter_a_scene_binds_is_drawn_at_its_value_then() {
        use crate::examples::{registry, EXAMPLES};
        use crate::session::Session;
        use simsandbox_views::scene::{geometry::Instance, Scene};
        use simsandbox_views::Recorded;
        let text = EXAMPLES
            .iter()
            .find(|e| e.name.starts_with("Pendulum (with its own view)"))
            .unwrap()
            .text;
        let mut s = Session::load("pendulum", text, &registry()).unwrap();
        s.set_recorder(Some(Default::default()));
        let settings = s.file.controls.views[1].scene.to_map();
        // The rod's far end, in the scene's frame, and its length.
        let rod = |s: &Session| {
            let h = s.signals();
            let scene = Scene::check(&settings, &s.available(&h)).unwrap();
            let placed = scene.place(&Recorded::single(&h));
            let Instance::Segment { from, to, .. } = placed[1].shape else {
                panic!("{placed:?}")
            };
            (to, (to[0] - from[0]).hypot(to[1] - from[1]))
        };
        // Recorded as a signal under its name, not offered as one.
        assert!(s.signals().resolve("bob.L").is_some());
        assert!(!s.bindable_scalars().contains(&"bob.L".to_string()));
        assert!(s.bindable_params().contains(&"bob.L".to_string()));
        s.run_until(0.5).unwrap();
        let (_, l) = rod(&s);
        assert!((l - 1.0).abs() < 1e-12, "{l}");
        let (at_half, _) = rod(&s);
        // The slider moves L: the next sample draws the new length.
        s.set_param_key("bob.L", 1.5).unwrap();
        s.run_until(1.0).unwrap();
        let (_, l) = rod(&s);
        assert!((l - 1.5).abs() < 1e-12, "{l}");
        let (at_one, _) = rod(&s);
        // Rewound to before the change, it is drawn as it was then.
        s.rewind_to(0.5);
        assert!(s.time() <= 0.5 + 1e-9);
        let (_, l) = rod(&s);
        assert!((l - 1.0).abs() < 1e-12, "{l}");
        s.run_until(0.5).unwrap();
        assert_eq!(rod(&s).0, at_half);
        // Running on replays the change: drawn as it first was.
        s.run_until(1.0).unwrap();
        assert_eq!(rod(&s).0, at_one);
        // Filters computed again over the history keep what it recorded.
        let column = |s: &Session| {
            let h = s.signals();
            let r = h.resolve("bob.L").unwrap();
            (0..h.len()).map(|k| h.value(r, k)).collect::<Vec<f64>>()
        };
        let lengths = column(&s);
        assert_eq!((lengths[0], lengths[lengths.len() - 1]), (1.0, 1.5));
        s.set_filters(s.filters().to_vec()).unwrap();
        assert_eq!(column(&s), lengths);
        // An output wins over a parameter of the same name: the
        // pointer's `hand.x`, a study's swept `curve.w` (a field) are
        // outputs, never recorded or offered as parameters.
        for (example, name) in [("Grab the bob", "hand.x"), ("Resonance", "curve.w")] {
            let e = EXAMPLES.iter().find(|e| e.name.contains(example)).unwrap();
            let other =
                Session::load_in(e.name, e.text, &registry(), e.folder().as_deref()).unwrap();
            assert!(
                !other.bindable_params().contains(&name.to_string()),
                "{name}"
            );
            let h = other.signals();
            assert!(
                h.names().iter().filter(|n| *n == name).count() <= 1,
                "{name}"
            );
        }
        // From the run file, headless: the same picture.
        let track = s.recorded().to_vec();
        let mut again = Session::load("pendulum", text, &registry()).unwrap();
        again.replay(track).unwrap();
        again.run_until(1.0).unwrap();
        assert_eq!(rod(&again).0, at_one);
    }

    /// A scene's problems are found where the app and `check` find them,
    /// at their paths; the view is kept, not drawn, saying why.
    #[test]
    fn a_scene_that_does_not_fit_is_reported_at_its_path() {
        use crate::examples::{registry, view_registry};
        use crate::session::Session;
        for (from, to, problem) in [
            (
                r#""angle": "m.th""#,
                r#""angle": "m.cx""#,
                "controls.views[0]: frames.arm.angle: 'm.cx' is in m; an angle takes radians",
            ),
            (
                r#""draw": "points""#,
                r#""draw": "arrows""#,
                "controls.views[0]: layers[0].draw: no primitive 'arrows'",
            ),
            (
                r#""in": "turn""#,
                r#""in": "elbow""#,
                "controls.views[0]: frames.arm.in: no frame 'elbow'",
            ),
            (
                r#""in": "base""#,
                r#""in": "arm""#,
                "controls.views[0]: frames.arm.in: the frames arm in turn in arm form a cycle",
            ),
            (
                r#""x": 0.4"#,
                r#""x": "m.ez""#,
                "controls.views[0]: layers[0].bind.x: no signal or parameter 'm.ez'",
            ),
            (
                r#""y": 0.25}"#,
                r#""y": 0.25, "z": 1}"#,
                "controls.views[0]: layers[0].bind.z: 'points' has no attribute 'z'",
            ),
        ] {
            let text = COMPOSED.replace(from, to);
            assert_ne!(text, COMPOSED, "{from}");
            let s = Session::load("composed", &text, &registry()).unwrap();
            let mut v = Views::new(view_registry());
            let w = v.open(&s.available(&s.signals()), &s.file.controls.views);
            assert_eq!(w.len(), 1, "{w:?}");
            assert!(w[0].starts_with(problem), "{}\n  not {problem}", w[0]);
            assert!(w[0].ends_with("; not drawn"), "{}", w[0]);
            // Kept as a tile that says why it is not drawn, and saved
            // back as it was.
            assert_eq!(v.tiles[0].state.type_name, "plot.scene");
            let held = v.tiles[0].state.problem().unwrap();
            assert!(
                held.starts_with(&problem["controls.views[0]: ".len()..]),
                "{held}"
            );
            assert_eq!(v.entries(), s.file.controls.views);
            // `check` says the same.
            let checked = crate::describe::check(&text, &registry());
            let warnings = checked["warnings"].as_array().unwrap();
            assert!(
                warnings
                    .iter()
                    .any(|x| x.as_str().unwrap().starts_with(problem)),
                "{warnings:?}"
            );
        }
        // Scene settings on another view are refused, not ignored.
        let other = COMPOSED.replace(
            r#""type": "plot.xy","#,
            r#""type": "plot.xy", "unit": "m","#,
        );
        let s = Session::load("composed", &other, &registry()).unwrap();
        let mut v = Views::new(view_registry());
        let w = v.open(&s.available(&s.signals()), &s.file.controls.views);
        assert_eq!(
            w,
            ["controls.views[1]: this view takes no setting 'unit'; ignored"]
        );
    }

    #[test]
    fn the_grid_stays_close_to_square() {
        assert_eq!(grid(0), (0, 0));
        assert_eq!(grid(1), (1, 1));
        assert_eq!(grid(2), (2, 1));
        assert_eq!(grid(3), (2, 2));
        assert_eq!(grid(4), (2, 2));
        assert_eq!(grid(5), (3, 2));
    }
}
