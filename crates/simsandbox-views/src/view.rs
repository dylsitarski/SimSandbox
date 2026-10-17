//! The view contract (ADR 0015).
//!
//! A [`ViewType`] is registered under a namespaced name and declares slots
//! (named inputs bound to signals) and style properties (declared like
//! parameters). A [`ViewState`] is one view on screen: a type, its slot
//! bindings, its style values and the drawing state (e.g. a 3D camera).

use crate::history::{History, SignalRef, TIME};
use std::collections::BTreeMap;

/// A named input of a view.
#[derive(Debug, Clone, Copy)]
pub struct Slot {
    pub name: &'static str,
    /// Takes a list of signals (e.g. the curves of a time series) rather
    /// than exactly one.
    pub many: bool,
    pub required: bool,
    pub description: &'static str,
}

/// A setting some models need to stay readable, declared like a parameter
/// (ADR 0015): a default, a suggested range and a description.
#[derive(Debug, Clone, Copy)]
pub struct StyleProperty {
    pub name: &'static str,
    pub default: f64,
    pub suggested: [f64; 2],
    pub unit: &'static str,
    pub description: &'static str,
}

/// Slot name to signal names (`"t"`, `"a.x"`, `"lorenz.xyz[0]"`).
pub type Bindings = BTreeMap<String, Vec<String>>;

/// What a view draws from in one frame.
pub struct ViewInput<'a> {
    pub history: &'a History,
    slots: BTreeMap<&'a str, Vec<SignalRef>>,
    style: &'a BTreeMap<String, f64>,
    view_type: &'a ViewType,
}

impl ViewInput<'_> {
    /// The signal bound to a single slot.
    pub fn one(&self, slot: &str) -> Option<SignalRef> {
        self.slots.get(slot).and_then(|v| v.first().copied())
    }

    /// The signals bound to a slot.
    pub fn many(&self, slot: &str) -> &[SignalRef] {
        self.slots.get(slot).map_or(&[], Vec::as_slice)
    }

    /// A style property's value, or its default.
    pub fn style(&self, name: &str) -> f64 {
        self.style.get(name).copied().unwrap_or_else(|| {
            self.view_type
                .style
                .iter()
                .find(|p| p.name == name)
                .map_or(0.0, |p| p.default)
        })
    }
}

/// Drawing behaviour of one view on screen.
pub trait View {
    fn show(&mut self, ui: &mut egui::Ui, input: &ViewInput<'_>);

    /// Orientation to save, for views that have one: yaw, pitch, zoom.
    fn camera(&self) -> Option<[f64; 3]> {
        None
    }

    fn set_camera(&mut self, _camera: [f64; 3]) {}
}

/// A kind of view, registered by name.
pub struct ViewType {
    /// Namespaced: `plot.xy`, `eq.pendulum`.
    pub name: &'static str,
    /// Shown in the view selector.
    pub label: &'static str,
    pub description: &'static str,
    pub slots: &'static [Slot],
    pub style: &'static [StyleProperty],
    /// Named starting bindings offered in "More views…" (ADR 0018); the
    /// first is the default.
    pub presets: &'static [Preset],
    pub create: fn() -> Box<dyn View>,
}

impl ViewType {
    pub fn slot(&self, name: &str) -> Option<&Slot> {
        self.slots.iter().find(|s| s.name == name)
    }

    /// Why these bindings and style values do not fit this type, if they
    /// do not. `signals` are the recorded signal names; `t` is always
    /// available.
    pub fn check(
        &self,
        bind: &Bindings,
        style: &BTreeMap<String, f64>,
        signals: &[String],
    ) -> Result<(), String> {
        for (slot, names) in bind {
            let s = self
                .slot(slot)
                .ok_or_else(|| format!("'{}' has no slot '{slot}'", self.name))?;
            if !s.many && names.len() > 1 {
                return Err(format!(
                    "slot '{slot}' takes one signal, got {}",
                    names.len()
                ));
            }
            if let Some(n) = names.iter().find(|n| *n != TIME && !signals.contains(n)) {
                return Err(format!("slot '{slot}': no plotted signal '{n}'"));
            }
        }
        if let Some(s) = self
            .slots
            .iter()
            .find(|s| s.required && bind.get(s.name).is_none_or(Vec::is_empty))
        {
            return Err(format!("slot '{}' is required", s.name));
        }
        for (name, v) in style {
            if !self.style.iter().any(|p| p.name == name) {
                return Err(format!("'{}' has no style property '{name}'", self.name));
            }
            if !v.is_finite() {
                return Err(format!("style '{name}' must be finite"));
            }
        }
        Ok(())
    }
}

/// A named way to start a view, e.g. "Phase portrait" for `plot.xy`.
pub struct Preset {
    pub label: &'static str,
    pub description: &'static str,
    /// Bindings for a model with these signals, or `None` if the preset
    /// cannot show them (e.g. a 3D view with fewer than three signals).
    pub bindings: fn(&[String]) -> Option<Bindings>,
}

/// View types by name.
#[derive(Default)]
pub struct ViewRegistry {
    types: Vec<ViewType>,
}

impl ViewRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// The built-in views (`plot.xy`, `plot.3d`).
    pub fn builtin() -> Self {
        let mut r = Self::new();
        crate::builtin::register(&mut r).expect("built-in views register once");
        r
    }

    pub fn register(&mut self, t: ViewType) -> Result<(), String> {
        if self.get(t.name).is_some() {
            return Err(format!("view type '{}' is already registered", t.name));
        }
        self.types.push(t);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&ViewType> {
        self.types.iter().find(|t| t.name == name)
    }

    /// In registration order.
    pub fn types(&self) -> &[ViewType] {
        &self.types
    }
}

/// One view on screen.
pub struct ViewState {
    pub type_name: String,
    pub bind: Bindings,
    /// Style values that differ from or override the defaults.
    pub style: BTreeMap<String, f64>,
    view: Box<dyn View>,
}

impl ViewState {
    /// A view of type `t` started from preset `preset` for these signals.
    pub fn from_preset(t: &ViewType, preset: usize, signals: &[String]) -> Option<Self> {
        let bind = (t.presets.get(preset)?.bindings)(signals)?;
        Some(Self {
            type_name: t.name.into(),
            bind,
            style: BTreeMap::new(),
            view: (t.create)(),
        })
    }

    /// A view restored from a saved description, checked against `t`.
    pub fn restore(
        t: &ViewType,
        bind: Bindings,
        style: BTreeMap<String, f64>,
        camera: Option<[f64; 3]>,
        signals: &[String],
    ) -> Result<Self, String> {
        t.check(&bind, &style, signals)?;
        let mut view = (t.create)();
        if let Some(c) = camera {
            view.set_camera(c);
        }
        Ok(Self {
            type_name: t.name.into(),
            bind,
            style,
            view,
        })
    }

    /// Draw from `history`. Bindings to signals that no longer exist are
    /// skipped.
    pub fn show(&mut self, ui: &mut egui::Ui, t: &ViewType, history: &History) {
        let slots = self
            .bind
            .iter()
            .map(|(slot, names)| {
                let refs = names.iter().filter_map(|n| history.resolve(n)).collect();
                (slot.as_str(), refs)
            })
            .collect();
        let input = ViewInput {
            history,
            slots,
            style: &self.style,
            view_type: t,
        };
        self.view.show(ui, &input);
    }

    pub fn camera(&self) -> Option<[f64; 3]> {
        self.view.camera()
    }

    /// A style property's value, or its default.
    pub fn style_value(&self, p: &StyleProperty) -> f64 {
        self.style.get(p.name).copied().unwrap_or(p.default)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signals() -> Vec<String> {
        vec!["l.xyz[0]".into(), "l.xyz[1]".into(), "l.xyz[2]".into()]
    }

    #[test]
    fn builtin_types_register_once_with_unique_names() {
        let mut r = ViewRegistry::builtin();
        let names: Vec<_> = r.types().iter().map(|t| t.name).collect();
        assert_eq!(names, ["plot.xy", "plot.3d"]);
        assert!(crate::builtin::register(&mut r).is_err());
    }

    #[test]
    fn presets_fit_the_signals_available() {
        let r = ViewRegistry::builtin();
        let three = signals();
        let xy = r.get("plot.xy").unwrap();
        let time = ViewState::from_preset(xy, 0, &three).unwrap();
        assert_eq!(time.bind["x"], ["t"]);
        assert_eq!(time.bind["y"].len(), 3);
        let phase = ViewState::from_preset(xy, 1, &three).unwrap();
        assert_eq!(phase.bind["x"], ["l.xyz[0]"]);
        assert_eq!(phase.bind["y"], ["l.xyz[2]"]);
        assert_eq!(phase.bind["colour"], ["t"]);
        // A phase portrait needs two signals, 3D three, a time plot one.
        assert!(ViewState::from_preset(xy, 1, &three[..1]).is_none());
        assert!(ViewState::from_preset(xy, 0, &three[..1]).is_some());
        assert!(ViewState::from_preset(r.get("plot.3d").unwrap(), 0, &three[..2]).is_none());
        assert!(ViewState::from_preset(xy, 0, &[]).is_none());
        assert!(ViewState::from_preset(xy, 9, &three).is_none());
    }

    #[test]
    fn saved_views_are_checked_against_their_type() {
        let r = ViewRegistry::builtin();
        let t3 = r.get("plot.3d").unwrap();
        let bind = |pairs: &[(&str, &[&str])]| -> Bindings {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect()))
                .collect()
        };
        let good = bind(&[
            ("x", &["l.xyz[0]"]),
            ("y", &["l.xyz[1]"]),
            ("z", &["l.xyz[2]"]),
            ("colour", &["l.xyz[0]"]),
        ]);
        let style: BTreeMap<String, f64> = [("trail_length".to_string(), 5.0)].into();
        let v = ViewState::restore(
            t3,
            good.clone(),
            style.clone(),
            Some([1.0, 0.5, 2.0]),
            &signals(),
        )
        .unwrap();
        assert_eq!(v.camera(), Some([1.0, 0.5, 2.0]));

        let missing_z = bind(&[("x", &["l.xyz[0]"]), ("y", &["l.xyz[1]"])]);
        assert!(t3.check(&missing_z, &style, &signals()).is_err());
        let unknown_slot = bind(&[("w", &["l.xyz[0]"])]);
        assert!(t3
            .check(&unknown_slot, &style, &signals())
            .unwrap_err()
            .contains("no slot"));
        let two_in_one = bind(&[
            ("x", &["l.xyz[0]", "l.xyz[1]"]),
            ("y", &["l.xyz[1]"]),
            ("z", &["l.xyz[2]"]),
        ]);
        assert!(t3.check(&two_in_one, &style, &signals()).is_err());
        let unknown_signal = bind(&[("x", &["nope"]), ("y", &["l.xyz[1]"]), ("z", &["l.xyz[2]"])]);
        assert!(t3.check(&unknown_signal, &style, &signals()).is_err());
        let bad_style: BTreeMap<String, f64> = [("glow".to_string(), 1.0)].into();
        assert!(t3.check(&good, &bad_style, &signals()).is_err());
    }
}
