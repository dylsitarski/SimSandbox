//! The view contract (ADR 0015).
//!
//! A [`ViewType`] is registered under a namespaced name and declares slots
//! (named inputs bound to signals) and style properties (declared like
//! parameters). A [`ViewState`] is one view on screen: a type, its slot
//! bindings, its style values and the drawing state (e.g. a 3D camera).

use crate::history::{split_source, Coordinate, History, Recorded, SignalRef, TIME};
use std::collections::BTreeMap;

/// What a slot accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotKind {
    /// A scalar signal or `t`.
    Scalar,
    /// A grid field.
    Grid,
    /// A point set.
    Points,
    /// A grid with one row (a line of cells).
    Line,
    /// An axis of a field picture: the index (`t`) or a coordinate of the
    /// field bound to the slot named in [`Slot::of`] (ADR 0024).
    Axis,
    /// A coordinate of the field bound to the slot named in [`Slot::of`],
    /// never the index.
    Coordinate,
}

/// What a slot or style property's applicability is judged on (ADR 0025).
pub struct Context<'a> {
    pub history: &'a History,
    pub bind: &'a Bindings,
}

/// Applies everywhere.
pub fn always(_: &Context<'_>) -> bool {
    true
}

/// Applies when the index is time, not a swept parameter.
pub fn over_time(c: &Context<'_>) -> bool {
    c.history.over_time()
}

/// A named input of a view.
#[derive(Debug, Clone, Copy)]
pub struct Slot {
    pub name: &'static str,
    pub kind: SlotKind,
    /// For an [`SlotKind::Axis`] slot: the slot holding the field it is an
    /// axis of.
    pub of: Option<&'static str>,
    /// Whether the slot means anything in this context; hidden if not.
    pub applies: fn(&Context<'_>) -> bool,
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
    /// Whether the property means anything in this context; where it does
    /// not, it is hidden and its default is used (ADR 0025).
    pub applies: fn(&Context<'_>) -> bool,
}

/// Slot name to signal names (`"t"`, `"a.x"`, `"lorenz.xyz[0]"`).
pub type Bindings = BTreeMap<String, Vec<String>>;

/// A bound signal in the recording it comes from (ADR 0063): its binding
/// as written, the recording and the signal there.
#[derive(Clone, Copy)]
pub struct Bound<'a> {
    pub name: &'a str,
    pub history: &'a History,
    pub signal: SignalRef,
}

/// What a view draws from in one frame.
pub struct ViewInput<'a> {
    /// The recording of the view's first bound signal (in slot order), or
    /// the model's: what [`ViewInput::one`] and [`ViewInput::many`] read.
    pub history: &'a History,
    pub bind: &'a Bindings,
    /// The view feeds an `input.pointer` (ADR 0058): dragging sends the
    /// pointer instead of panning, through [`View::pointer`].
    pub sends_pointer: bool,
    /// Bindings in `history`.
    slots: BTreeMap<&'a str, Vec<SignalRef>>,
    /// Every binding, in its own recording.
    any: BTreeMap<&'a str, Vec<Bound<'a>>>,
    style: &'a BTreeMap<String, f64>,
    view_type: &'a ViewType,
}

impl<'a> ViewInput<'a> {
    /// The signal bound to a single slot, if it is in [`ViewInput::history`].
    pub fn one(&self, slot: &str) -> Option<SignalRef> {
        self.slots.get(slot).and_then(|v| v.first().copied())
    }

    /// The signals bound to a slot that are in [`ViewInput::history`]:
    /// views that draw from one recording read these.
    pub fn many(&self, slot: &str) -> &[SignalRef] {
        self.slots.get(slot).map_or(&[], Vec::as_slice)
    }

    /// The signal bound to a single slot, from whichever recording.
    pub fn one_any(&self, slot: &str) -> Option<Bound<'a>> {
        self.any.get(slot).and_then(|v| v.first().copied())
    }

    /// The signals bound to a slot, each from its own recording: views that
    /// pair signals at common times read these (ADR 0063).
    pub fn many_any(&self, slot: &str) -> &[Bound<'a>] {
        self.any.get(slot).map_or(&[], Vec::as_slice)
    }

    /// A style property's value, or its default where it is unset or does
    /// not apply.
    pub fn style(&self, name: &str) -> f64 {
        let Some(p) = self.view_type.style.iter().find(|p| p.name == name) else {
            return 0.0;
        };
        let context = Context {
            history: self.history,
            bind: self.bind,
        };
        match self.style.get(name) {
            Some(&v) if (p.applies)(&context) => v,
            _ => p.default,
        }
    }

    /// The name bound to a slot, as written (e.g. `t` or `rod.position`).
    pub fn bound(&self, slot: &str) -> Option<&str> {
        self.bind.get(slot)?.first().map(String::as_str)
    }
}

/// Drawing behaviour of one view on screen.
pub trait View: Send {
    fn show(&mut self, ui: &mut egui::Ui, input: &ViewInput<'_>);

    /// Orientation to save, for views that have one: yaw, pitch, zoom.
    fn camera(&self) -> Option<[f64; 3]> {
        None
    }

    fn set_camera(&mut self, _camera: [f64; 3]) {}

    /// What the user picked inside the view, saved with it (e.g. profile
    /// cuts, ADR 0029).
    fn selection(&self) -> Option<serde_json::Value> {
        None
    }

    /// Restore a saved selection; an error says why it cannot be used.
    fn set_selection(&mut self, _selection: &serde_json::Value) -> Result<(), String> {
        Err("this view keeps no selection".into())
    }

    /// Controls for the selection, shown in the side panel.
    fn edit_selection(&mut self, _ui: &mut egui::Ui, _input: &ViewInput<'_>) {}

    /// Draw transposed from now on, or back (for [`Swap::Transpose`]).
    fn transpose(&mut self) {}

    /// The signal the user clicked since last asked (a curve), so the app
    /// can show where it comes from (ADR 0053).
    fn picked(&mut self) -> Option<String> {
        None
    }

    /// While the view sends input ([`ViewInput::sends_pointer`]): the
    /// pointer since last asked, `[x, y, down]` in the view's coordinates
    /// (ADR 0058). Views that cannot send input return nothing.
    fn pointer(&mut self) -> Option<[f64; 3]> {
        None
    }
}

/// What a model offers views: its recorded signals by kind and its block
/// types (for views that apply to particular blocks, ADR 0016).
#[derive(Debug, Clone, Copy, Default)]
pub struct Available<'a> {
    pub scalars: &'a [String],
    pub grids: &'a [String],
    pub points: &'a [String],
    pub block_types: &'a [String],
    /// Grids with one row (lines of cells), which can be drawn over time.
    pub lines: &'a [String],
    /// Which fields place which others' cells (ADR 0024).
    pub coordinates: &'a [Coordinate],
    /// In a sweep, the swept parameters' keys, also among `scalars`
    /// (ADR 0028).
    pub swept: &'a [String],
}

impl Available<'_> {
    /// Whether `name` can fill a slot of this kind; from a probe
    /// (`probe:NAME/signal`), whether the model has the signal.
    pub fn accepts(&self, kind: SlotKind, name: &str) -> bool {
        let name = split_source(name).1;
        match kind {
            SlotKind::Scalar => name == TIME || self.scalars.iter().any(|s| s == name),
            SlotKind::Grid => self.grids.iter().any(|s| s == name),
            SlotKind::Points => self.points.iter().any(|s| s == name),
            SlotKind::Line => self.lines.iter().any(|s| s == name),
            // Whether it places the bound field is checked with the field,
            // so the reason can be given.
            SlotKind::Axis => name == TIME || self.grids.iter().any(|s| s == name),
            SlotKind::Coordinate => self.grids.iter().any(|s| s == name),
        }
    }

    /// What an axis slot of `field` can take: `t` for a line of cells
    /// (drawn over time), then the field's coordinates.
    pub fn axis_choices(&self, field: &str) -> Vec<&str> {
        let time = self.lines.iter().any(|l| l == field).then_some(TIME);
        time.into_iter()
            .chain(
                self.coordinates
                    .iter()
                    .filter(|c| c.of == field)
                    .map(|c| c.name.as_str()),
            )
            .collect()
    }

    /// The dimension an axis binding claims for `field`: `None` for time,
    /// `Some(axis)` for a coordinate; an error if it cannot be an axis of
    /// it.
    pub fn axis_of(&self, field: &str, name: &str) -> Result<Option<usize>, String> {
        let (field, name) = (split_source(field).1, split_source(name).1);
        if name == TIME {
            return if self.lines.iter().any(|l| l == field) {
                Ok(None)
            } else {
                Err(format!(
                    "'{field}' is not a line of cells, so it cannot be drawn over t"
                ))
            };
        }
        self.coordinates
            .iter()
            .find(|c| c.name == name && c.of == field)
            .map(|c| Some(c.axis))
            .ok_or_else(|| format!("'{name}' is not a coordinate of '{field}'"))
    }

    /// The names a slot of this kind can take (`t` first for scalars).
    pub fn choices(&self, kind: SlotKind) -> Vec<&str> {
        match kind {
            SlotKind::Scalar => std::iter::once(TIME)
                .chain(self.scalars.iter().map(String::as_str))
                .collect(),
            SlotKind::Grid => self.grids.iter().map(String::as_str).collect(),
            SlotKind::Points => self.points.iter().map(String::as_str).collect(),
            SlotKind::Line => self.lines.iter().map(String::as_str).collect(),
            SlotKind::Axis => std::iter::once(TIME)
                .chain(self.coordinates.iter().map(|c| c.name.as_str()))
                .collect(),
            SlotKind::Coordinate => self.coordinates.iter().map(|c| c.name.as_str()).collect(),
        }
    }
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
    /// Block types this view applies to; empty for generic views, which fit
    /// any model whose signals fill their slots (ADR 0016).
    pub applies_to: &'static [&'static str],
    /// What the view's "swap axes" button does, if it has one.
    pub swap: Option<Swap>,
    /// Named starting bindings offered in "More views…" (ADR 0018); the
    /// first is the default.
    pub presets: &'static [Preset],
    pub create: fn() -> Box<dyn View>,
}

impl ViewType {
    pub fn slot(&self, name: &str) -> Option<&Slot> {
        self.slots.iter().find(|s| s.name == name)
    }

    /// Whether this view applies to a model with these block types.
    pub fn applies(&self, available: &Available<'_>) -> bool {
        self.applies_to.is_empty()
            || available
                .block_types
                .iter()
                .any(|b| self.applies_to.contains(&b.as_str()))
    }

    /// Why these bindings and style values do not fit this type, if they
    /// do not.
    pub fn check(
        &self,
        bind: &Bindings,
        style: &BTreeMap<String, f64>,
        available: &Available<'_>,
    ) -> Result<(), String> {
        if !self.applies(available) {
            return Err(format!("'{}' does not apply to this model", self.name));
        }
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
            if let Some(n) = names.iter().find(|n| !available.accepts(s.kind, n)) {
                return Err(format!("slot '{slot}': no suitable signal '{n}'"));
            }
        }
        if let Some(s) = self
            .slots
            .iter()
            .find(|s| s.required && bind.get(s.name).is_none_or(Vec::is_empty))
        {
            return Err(format!("slot '{}' is required", s.name));
        }
        // Axis slots: each must be an axis of its field, and no two may
        // claim the same dimension (ADR 0024).
        let mut claimed: Vec<(&str, Option<usize>)> = Vec::new();
        for s in self
            .slots
            .iter()
            .filter(|s| matches!(s.kind, SlotKind::Axis | SlotKind::Coordinate))
        {
            let (Some(of), Some(name)) = (s.of, bind.get(s.name).and_then(|v| v.first())) else {
                continue;
            };
            let Some(field) = bind.get(of).and_then(|v| v.first()) else {
                continue;
            };
            let dim = available
                .axis_of(field, name)
                .map_err(|e| format!("slot '{}': {e}", s.name))?;
            if s.kind == SlotKind::Coordinate && dim.is_none() {
                return Err(format!("slot '{}': takes a coordinate, not t", s.name));
            }
            if claimed.iter().any(|&(f, d)| f == field && d == dim) {
                return Err(format!(
                    "slot '{}': '{name}' runs along the same direction as another axis",
                    s.name
                ));
            }
            claimed.push((field, dim));
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

/// How a view swaps its axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Swap {
    /// Exchange the bindings of two slots (e.g. a heat map's `x` and `y`).
    Slots([&'static str; 2]),
    /// Draw the same bindings transposed (e.g. a plot with several curves
    /// against one signal); the view keeps the state.
    Transpose,
}

/// A named way to start a view, e.g. "Phase portrait" for `plot.xy`.
pub struct Preset {
    pub label: &'static str,
    pub description: &'static str,
    /// Bindings for a model offering these signals, or `None` if the
    /// preset cannot show them (e.g. a 3D view with fewer than three).
    pub bindings: fn(&Available<'_>) -> Option<Bindings>,
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

/// What a view of type `t` draws from: its bindings resolved in their
/// recordings; bindings to signals that no longer exist (or a probe that is
/// gone) are skipped.
pub(crate) fn view_input<'a>(
    bind: &'a Bindings,
    style: &'a BTreeMap<String, f64>,
    t: &'a ViewType,
    recorded: Recorded<'a>,
) -> ViewInput<'a> {
    let any: BTreeMap<&str, Vec<Bound>> = bind
        .iter()
        .map(|(slot, names)| {
            let bound = names
                .iter()
                .filter_map(|n| {
                    let (history, signal) = recorded.resolve(n)?;
                    Some(Bound {
                        name: n.as_str(),
                        history,
                        signal,
                    })
                })
                .collect();
            (slot.as_str(), bound)
        })
        .collect();
    // The recording of the first signal bound, in slot order.
    let history = t
        .slots
        .iter()
        .filter_map(|s| any.get(s.name))
        .flatten()
        .find(|b| b.signal != SignalRef::Time)
        .map_or(recorded.all, |b| b.history);
    let slots = any
        .iter()
        .map(|(slot, bound)| {
            let refs = bound
                .iter()
                .filter(|b| b.signal == SignalRef::Time || std::ptr::eq(b.history, history))
                .map(|b| b.signal)
                .collect();
            (*slot, refs)
        })
        .collect();
    ViewInput {
        history,
        bind,
        sends_pointer: false,
        slots,
        any,
        style,
        view_type: t,
    }
}

/// A plot's pointer as input (ADR 0058): `[x, y, down]` in the plot's
/// coordinates while the pointer is pressed on it, and once more when it is
/// let go (`down` 0), else nothing.
#[derive(Debug, Default)]
pub struct PlotPointer {
    last: Option<[f64; 2]>,
    sent: Option<[f64; 3]>,
}

impl PlotPointer {
    /// Read the pointer from a plot's response, to be taken by
    /// [`PlotPointer::take`].
    pub fn read<R>(&mut self, response: &egui_plot::PlotResponse<R>) {
        let r = &response.response;
        if r.is_pointer_button_down_on() {
            if let Some(pos) = r.interact_pointer_pos() {
                let v = response.transform.value_from_position(pos);
                self.last = Some([v.x, v.y]);
                self.sent = Some([v.x, v.y, 1.0]);
            }
        } else if let Some([x, y]) = self.last.take() {
            self.sent = Some([x, y, 0.0]);
        }
    }

    /// What was read since last asked.
    pub fn take(&mut self) -> Option<[f64; 3]> {
        self.sent.take()
    }
}

/// One view on screen.
pub struct ViewState {
    pub type_name: String,
    pub bind: Bindings,
    /// Style values that differ from or override the defaults.
    pub style: BTreeMap<String, f64>,
    /// The view feeds an `input.pointer` (ADR 0058), set by the app.
    pub sends_pointer: bool,
    view: Box<dyn View>,
}

impl ViewState {
    /// A view of type `t` started from preset `preset` for a model.
    pub fn from_preset(t: &ViewType, preset: usize, available: &Available<'_>) -> Option<Self> {
        if !t.applies(available) {
            return None;
        }
        let bind = (t.presets.get(preset)?.bindings)(available)?;
        Some(Self {
            type_name: t.name.into(),
            bind,
            style: BTreeMap::new(),
            sends_pointer: false,
            view: (t.create)(),
        })
    }

    /// A view restored from a saved description, checked against `t`.
    pub fn restore(
        t: &ViewType,
        bind: Bindings,
        style: BTreeMap<String, f64>,
        camera: Option<[f64; 3]>,
        available: &Available<'_>,
    ) -> Result<Self, String> {
        t.check(&bind, &style, available)?;
        let mut view = (t.create)();
        if let Some(c) = camera {
            view.set_camera(c);
        }
        Ok(Self {
            type_name: t.name.into(),
            bind,
            style,
            sends_pointer: false,
            view,
        })
    }

    pub fn selection(&self) -> Option<serde_json::Value> {
        self.view.selection()
    }

    pub fn set_selection(&mut self, selection: &serde_json::Value) -> Result<(), String> {
        self.view.set_selection(selection)
    }

    /// The view's selection controls, for the side panel.
    pub fn edit_selection<'a>(
        &mut self,
        ui: &mut egui::Ui,
        t: &ViewType,
        recorded: impl Into<Recorded<'a>>,
    ) {
        let input = view_input(&self.bind, &self.style, t, recorded.into());
        self.view.edit_selection(ui, &input);
    }

    /// Draw from what is recorded: the model's recording, and probes' for
    /// bindings that name one. Bindings to signals that no longer exist
    /// are skipped.
    pub fn show<'a>(&mut self, ui: &mut egui::Ui, t: &ViewType, recorded: impl Into<Recorded<'a>>) {
        let mut input = view_input(&self.bind, &self.style, t, recorded.into());
        input.sends_pointer = self.sends_pointer;
        self.view.show(ui, &input);
    }

    /// The pointer the view sent since last asked (ADR 0058).
    pub fn pointer(&mut self) -> Option<[f64; 3]> {
        self.view.pointer()
    }

    pub fn camera(&self) -> Option<[f64; 3]> {
        self.view.camera()
    }

    /// The signal clicked in the view since last asked.
    pub fn picked(&mut self) -> Option<String> {
        self.view.picked()
    }

    /// The "swap axes" button: exchange two slots' bindings, or transpose.
    pub fn swap(&mut self, t: &ViewType) {
        match t.swap {
            Some(Swap::Slots([a, b])) => {
                let (va, vb) = (self.bind.remove(a), self.bind.remove(b));
                if let Some(v) = va {
                    self.bind.insert(b.into(), v);
                }
                if let Some(v) = vb {
                    self.bind.insert(a.into(), v);
                }
            }
            Some(Swap::Transpose) => self.view.transpose(),
            None => {}
        }
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

    fn avail(scalars: &[String]) -> Available<'_> {
        Available {
            scalars,
            ..Default::default()
        }
    }

    #[test]
    fn builtin_types_register_once_with_unique_names() {
        let mut r = ViewRegistry::builtin();
        let names: Vec<_> = r.types().iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            [
                "plot.xy",
                "plot.3d",
                "plot.heatmap",
                "plot.profile",
                "plot.scatter",
                "plot.surface",
                "plot.contour"
            ]
        );
        assert!(crate::builtin::register(&mut r).is_err());
    }

    #[test]
    fn presets_fit_the_signals_available() {
        let r = ViewRegistry::builtin();
        let three = signals();
        let xy = r.get("plot.xy").unwrap();
        let time = ViewState::from_preset(xy, 0, &avail(&three)).unwrap();
        assert_eq!(time.bind["x"], ["t"]);
        assert_eq!(time.bind["y"].len(), 3);
        let phase = ViewState::from_preset(xy, 1, &avail(&three)).unwrap();
        assert_eq!(phase.bind["x"], ["l.xyz[0]"]);
        assert_eq!(phase.bind["y"], ["l.xyz[2]"]);
        assert_eq!(phase.bind["colour"], ["t"]);
        // A phase portrait needs two signals, 3D three, a time plot one.
        assert!(ViewState::from_preset(xy, 1, &avail(&three[..1])).is_none());
        assert!(ViewState::from_preset(xy, 0, &avail(&three[..1])).is_some());
        assert!(
            ViewState::from_preset(r.get("plot.3d").unwrap(), 0, &avail(&three[..2])).is_none()
        );
        assert!(ViewState::from_preset(xy, 0, &avail(&[])).is_none());
        assert!(ViewState::from_preset(xy, 9, &avail(&three)).is_none());
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
            &avail(&signals()),
        )
        .unwrap();
        assert_eq!(v.camera(), Some([1.0, 0.5, 2.0]));

        let missing_z = bind(&[("x", &["l.xyz[0]"]), ("y", &["l.xyz[1]"])]);
        assert!(t3.check(&missing_z, &style, &avail(&signals())).is_err());
        let unknown_slot = bind(&[("w", &["l.xyz[0]"])]);
        assert!(t3
            .check(&unknown_slot, &style, &avail(&signals()))
            .unwrap_err()
            .contains("no slot"));
        let two_in_one = bind(&[
            ("x", &["l.xyz[0]", "l.xyz[1]"]),
            ("y", &["l.xyz[1]"]),
            ("z", &["l.xyz[2]"]),
        ]);
        assert!(t3.check(&two_in_one, &style, &avail(&signals())).is_err());
        let unknown_signal = bind(&[("x", &["nope"]), ("y", &["l.xyz[1]"]), ("z", &["l.xyz[2]"])]);
        assert!(t3
            .check(&unknown_signal, &style, &avail(&signals()))
            .is_err());
        let bad_style: BTreeMap<String, f64> = [("glow".to_string(), 1.0)].into();
        assert!(t3.check(&good, &bad_style, &avail(&signals())).is_err());
    }

    #[test]
    fn axis_slots_take_only_what_can_place_the_field() {
        let r = ViewRegistry::builtin();
        let heat = r.get("plot.heatmap").unwrap();
        let grids: Vec<String> = ["rod.T", "rod.x", "rod.k"].map(String::from).to_vec();
        let lines = grids.clone();
        let coordinates = [Coordinate {
            name: "rod.x".into(),
            of: "rod.T".into(),
            axis: 0,
        }];
        let a = Available {
            grids: &grids,
            lines: &lines,
            coordinates: &coordinates,
            ..Default::default()
        };
        assert_eq!(a.axis_choices("rod.T"), ["t", "rod.x"]);
        let bind = |x: &str, y: &str| -> Bindings {
            [
                ("field".to_string(), vec!["rod.T".to_string()]),
                ("x".to_string(), vec![x.to_string()]),
                ("y".to_string(), vec![y.to_string()]),
            ]
            .into()
        };
        let none = BTreeMap::new();
        assert!(heat.check(&bind("t", "rod.x"), &none, &a).is_ok());
        assert!(heat.check(&bind("rod.x", "t"), &none, &a).is_ok());
        // Not a coordinate of the field, or two axes in one direction.
        let e = heat.check(&bind("t", "rod.k"), &none, &a).unwrap_err();
        assert!(e.contains("not a coordinate"), "{e}");
        assert!(heat.check(&bind("t", "t"), &none, &a).is_err());
        // A 2D grid cannot be drawn over t.
        let flat = Available { lines: &[], ..a };
        assert!(heat.check(&bind("t", "rod.x"), &none, &flat).is_err());
    }

    #[test]
    fn settings_that_do_not_apply_fall_back_to_their_defaults() {
        let r = ViewRegistry::builtin();
        let xy = r.get("plot.xy").unwrap();
        let bind: Bindings = [("x".to_string(), vec!["t".to_string()])].into();
        let style: BTreeMap<String, f64> = [("trail_length".to_string(), 5.0)].into();
        let input = |h: &History| -> f64 {
            ViewInput {
                history: h,
                bind: &bind,
                sends_pointer: false,
                slots: BTreeMap::new(),
                any: BTreeMap::new(),
                style: &style,
                view_type: xy,
            }
            .style("trail_length")
        };
        // Over time the saved trail applies; in a sweep it means nothing.
        assert_eq!(input(&History::new(vec![])), 5.0);
        assert_eq!(input(&History::with_index("spring.w", vec![])), -1.0);
    }

    #[test]
    fn plots_swap_by_transposing_and_heat_maps_by_exchanging_axes() {
        let r = ViewRegistry::builtin();
        let three = signals();
        let xy = r.get("plot.xy").unwrap();
        let mut time = ViewState::from_preset(xy, 0, &avail(&three)).unwrap();
        let before = time.bind.clone();
        time.swap(xy);
        // Same bindings, drawn transposed, and saved that way.
        assert_eq!(time.bind, before);
        assert_eq!(
            time.selection(),
            Some(serde_json::json!({"transposed": true}))
        );
        time.swap(xy);
        assert_eq!(time.selection(), None);
        assert!(time
            .set_selection(&serde_json::json!({"transposed": "yes"}))
            .is_err());

        let heat = r.get("plot.heatmap").unwrap();
        let mut state = ViewState::from_preset(
            heat,
            0,
            &Available {
                grids: &["g".to_string()],
                ..Default::default()
            },
        )
        .unwrap();
        state.bind.insert("x".into(), vec!["t".into()]);
        state.swap(heat);
        assert_eq!(state.bind.get("y"), Some(&vec!["t".to_string()]));
        assert!(!state.bind.contains_key("x"));
    }
}
