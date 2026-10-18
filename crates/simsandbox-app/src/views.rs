//! The open model's views, shown together as tiles (ADR 0015, ADR 0016):
//! the views the model file lists, plus any the user adds from a view
//! type's presets under "More views…" (ADR 0018). Save writes the tiles as
//! they are.

use simsandbox_file::{Binding, CameraControl, ViewEntry};
use simsandbox_views::{Available, Bindings, ViewRegistry, ViewState, ViewType};

/// One view on screen. The id stays fixed while other tiles come and go, so
/// each tile keeps its own plot zoom and widget state.
pub struct Tile {
    pub id: u64,
    pub state: ViewState,
}

pub struct Views {
    pub registry: ViewRegistry,
    pub tiles: Vec<Tile>,
    /// The tile whose style properties the side panel shows.
    pub selected: Option<u64>,
    next_id: u64,
}

impl Views {
    pub fn new(registry: ViewRegistry) -> Self {
        Self {
            registry,
            tiles: Vec::new(),
            selected: None,
            next_id: 0,
        }
    }

    /// Start over for a model offering `available`: the saved views, or the
    /// first preset that fits (a plot against time, if the model records
    /// scalars) if none can be used.
    /// Returns warnings for saved views that cannot be used.
    pub fn open(&mut self, available: &Available<'_>, saved: &[ViewEntry]) -> Vec<String> {
        self.tiles.clear();
        self.selected = None;
        let mut warnings = Vec::new();
        for (i, entry) in saved.iter().enumerate() {
            match self.restore(entry, available) {
                Ok(state) => {
                    self.push(state);
                }
                Err(e) => warnings.push(format!("controls.views[{i}]: {e}; ignored")),
            }
        }
        if self.tiles.is_empty() {
            let first_fit = self
                .registry
                .types()
                .iter()
                .flat_map(|t| (0..t.presets.len()).map(move |p| (t.name, p)))
                .find(|&(n, p)| self.fits(n, p, available));
            if let Some((n, p)) = first_fit {
                self.add(n, p, available);
            }
        }
        self.selected = self.tiles.first().map(|t| t.id);
        warnings
    }

    fn restore(&self, e: &ViewEntry, available: &Available<'_>) -> Result<ViewState, String> {
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
        if let Some(s) = &e.selection {
            state
                .set_selection(s)
                .map_err(|err| format!("selection: {err}"))?;
        }
        Ok(state)
    }

    fn push(&mut self, state: ViewState) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.tiles.push(Tile { id, state });
        id
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

    /// The tiles as saved in `controls.views`, in order.
    pub fn entries(&self) -> Vec<ViewEntry> {
        self.tiles
            .iter()
            .map(|tile| {
                let s = &tile.state;
                ViewEntry {
                    type_name: s.type_name.clone(),
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
                }
            })
            .collect()
    }
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

#[cfg(test)]
mod tests {
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
            selection: None,
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
            let s = Session::load(e.name, e.text, &registry()).unwrap();
            let mut v = Views::new(view_registry());
            let w = v.open(&s.available(), &s.file.controls.views);
            assert!(w.is_empty(), "{}: {w:?}", e.name);
            assert_eq!(v.entries(), s.file.controls.views, "{}", e.name);
        }
    }

    #[test]
    fn a_two_parameter_sweep_opens_a_heat_map_over_both() {
        use crate::examples::{registry, view_registry, EXAMPLES};
        use crate::session::Session;
        let e = EXAMPLES
            .iter()
            .find(|e| e.name.contains("Resonance"))
            .unwrap();
        let mut s = Session::load(e.name, e.text, &registry()).unwrap();
        s.set_sweep(&[
            ("spring.w".into(), [0.0, 3.0], 40),
            ("spring.c".into(), [0.1, 2.0], 30),
        ])
        .unwrap();
        let mut v = Views::new(view_registry());
        // The file's x-y plots have no scalars to show, so they are dropped.
        let w = v.open(&s.available(), &s.file.controls.views);
        assert_eq!(w.len(), s.file.controls.views.len());
        let b = &v.tiles[0].state.bind;
        assert_eq!(v.tiles[0].state.type_name, "plot.heatmap");
        assert_eq!(b["field"], ["spring.amplitude"]);
        assert_eq!(
            (b["x"][0].as_str(), b["y"][0].as_str()),
            ("spring.w", "spring.c")
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
