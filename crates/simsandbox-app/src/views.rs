//! The views of the open model: one state per view type the user has
//! visited, so switching keeps each view's bindings, style and camera, and
//! the active one is drawn and saved (ADR 0015).

use simsandbox_file::{Binding, CameraControl, ViewEntry};
use simsandbox_views::{Bindings, ViewRegistry, ViewState, ViewType};
use std::collections::BTreeMap;

pub struct Views {
    pub registry: ViewRegistry,
    states: BTreeMap<String, ViewState>,
    active: Option<String>,
}

impl Views {
    pub fn new(registry: ViewRegistry) -> Self {
        Self {
            registry,
            states: BTreeMap::new(),
            active: None,
        }
    }

    /// Start over for a model with these recorded `signals`: the first saved
    /// view if it fits, else the time series. Returns warnings for saved
    /// views that cannot be used.
    pub fn open(&mut self, signals: &[String], saved: &[ViewEntry]) -> Vec<String> {
        self.states.clear();
        self.active = None;
        let mut warnings = Vec::new();
        for (i, entry) in saved.iter().enumerate() {
            match self.restore(entry, signals) {
                Ok(state) => {
                    let name = state.type_name.clone();
                    if self.active.is_none() {
                        self.active = Some(name.clone());
                    }
                    self.states.entry(name).or_insert(state);
                }
                Err(e) => warnings.push(format!("controls.views[{i}]: {e}; ignored")),
            }
        }
        if self.active.is_none() {
            let first_fit = ["plot.time"]
                .into_iter()
                .chain(self.registry.types().iter().map(|t| t.name))
                .find(|&n| self.available(n, signals));
            if let Some(n) = first_fit {
                self.select(n, signals);
            }
        }
        warnings
    }

    fn restore(&self, e: &ViewEntry, signals: &[String]) -> Result<ViewState, String> {
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
        ViewState::restore(t, bind, e.style.clone(), camera, signals)
    }

    /// Whether a view type can show these signals.
    pub fn available(&self, type_name: &str, signals: &[String]) -> bool {
        self.states.contains_key(type_name)
            || self
                .registry
                .get(type_name)
                .is_some_and(|t| (t.default_bindings)(signals).is_some())
    }

    /// Show a view type, creating it with default bindings the first time.
    /// Returns false if it cannot show these signals.
    pub fn select(&mut self, type_name: &str, signals: &[String]) -> bool {
        if !self.states.contains_key(type_name) {
            let Some(state) = self
                .registry
                .get(type_name)
                .and_then(|t| ViewState::with_defaults(t, signals))
            else {
                return false;
            };
            self.states.insert(type_name.into(), state);
        }
        self.active = Some(type_name.into());
        true
    }

    pub fn active_name(&self) -> Option<&str> {
        self.active.as_deref()
    }

    /// The active view and its type.
    pub fn active_mut(&mut self) -> Option<(&ViewType, &mut ViewState)> {
        let name = self.active.as_ref()?;
        let t = self.registry.get(name)?;
        let state = self.states.get_mut(name)?;
        Some((t, state))
    }

    /// The active view as saved in `controls.views`.
    pub fn entries(&self) -> Vec<ViewEntry> {
        let Some(state) = self.active.as_ref().and_then(|n| self.states.get(n)) else {
            return Vec::new();
        };
        vec![ViewEntry {
            type_name: state.type_name.clone(),
            bind: state
                .bind
                .iter()
                .filter(|(_, v)| !v.is_empty())
                .map(|(k, v)| (k.clone(), Binding::from(v.clone())))
                .collect(),
            style: state.style.clone(),
            camera: state
                .camera()
                .map(|[yaw, pitch, zoom]| CameraControl { yaw, pitch, zoom }),
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signals() -> Vec<String> {
        vec!["l.xyz[0]".into(), "l.xyz[1]".into(), "l.xyz[2]".into()]
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
        }
    }

    #[test]
    fn opens_the_saved_view_and_saves_it_back_unchanged() {
        let mut v = Views::new(ViewRegistry::builtin());
        let saved = entry(
            "plot.3d",
            &[
                ("x", "l.xyz[2]"),
                ("y", "l.xyz[0]"),
                ("z", "l.xyz[1]"),
                ("colour", "t"),
            ],
        );
        assert!(v.open(&signals(), std::slice::from_ref(&saved)).is_empty());
        assert_eq!(v.active_name(), Some("plot.3d"));
        assert_eq!(v.entries(), [saved]);
    }

    #[test]
    fn falls_back_to_the_time_series_with_a_warning() {
        let mut v = Views::new(ViewRegistry::builtin());
        let unknown = entry("plot.4d", &[]);
        let unplotted = entry("plot.phase", &[("x", "l.xyz[0]"), ("y", "other.x")]);
        let w = v.open(&signals(), &[unknown, unplotted]);
        assert_eq!(w.len(), 2, "{w:?}");
        assert_eq!(v.active_name(), Some("plot.time"));
    }

    #[test]
    fn switching_keeps_each_views_settings() {
        let mut v = Views::new(ViewRegistry::builtin());
        v.open(&signals(), &[]);
        assert!(v.select("plot.3d", &signals()));
        v.active_mut()
            .unwrap()
            .1
            .style
            .insert("trail_length".into(), 9.0);
        assert!(v.select("plot.time", &signals()));
        assert!(v.select("plot.3d", &signals()));
        assert_eq!(v.active_mut().unwrap().1.style["trail_length"], 9.0);
        // Too few signals for 3D.
        let mut few = Views::new(ViewRegistry::builtin());
        few.open(&signals()[..2], &[]);
        assert!(!few.available("plot.3d", &signals()[..2]));
        assert!(!few.select("plot.3d", &signals()[..2]));
    }
}
