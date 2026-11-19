//! Editing a scene's saved form (ADR 0093 point 8) from the Inspector:
//! each change one [`SceneEdit`] applied to [`SceneSettings`]. What the
//! result means (signals, units, frames) is checked by the view, not here;
//! these keep the form whole: a new layer has every required attribute
//! bound, a frame taken away leaves what was in it in its parent.

use simsandbox_file::{SceneFrame, SceneLayer, SceneSettings, SceneValue};
use simsandbox_views::scene::spec::{Kind, Unbound, PRIMITIVES};
use std::collections::BTreeMap;

/// One change to a scene, as the Inspector makes it.
#[derive(Debug, Clone, PartialEq)]
pub enum SceneEdit {
    Unit(String),
    /// The region shown, x and y ranges; `None` follows what is drawn.
    Extent(Option<[[f64; 2]; 2]>),
    ViewFrom(Option<String>),
    AddFrame(String),
    RemoveFrame(String),
    /// A frame's parent (`in`).
    FrameIn(String, Option<String>),
    /// A frame's `x`, `y` or `angle`; `None` is its default, 0.
    Pose(String, &'static str, Option<SceneValue>),
    /// A layer drawing this primitive, at the end (drawn last, on top).
    AddLayer(String),
    RemoveLayer(usize),
    /// Swap a layer with the one before it (drawn earlier).
    Raise(usize),
    /// A layer's primitive: bindings of the same name are kept.
    Draw(usize, String),
    /// The frame a layer is in.
    LayerIn(usize, Option<String>),
    /// An attribute's binding; `None` is its default.
    Bind(usize, String, Option<SceneValue>),
    /// The `input.pointer` block pressing on the layer sends the pointer
    /// to.
    Grab(usize, Option<String>),
}

/// What a required attribute is bound to in a new layer: a small shape at
/// the origin, so the layer shows at once.
fn starting_value(name: &str, kind: Kind) -> f64 {
    match (name, kind) {
        ("x2", _) => 1.0,
        ("width" | "height", Kind::Length) => 0.2,
        ("radius", _) => 0.1,
        _ => 0.0,
    }
}

/// Bind every required attribute of `draw` that `bind` leaves unbound.
fn fill_required(draw: &str, bind: &mut BTreeMap<String, SceneValue>) {
    if let Some(p) = PRIMITIVES.iter().find(|p| p.name == draw) {
        for a in p.attributes {
            if a.default == Unbound::Required && !bind.contains_key(a.name) {
                bind.insert(
                    a.name.to_string(),
                    SceneValue::Number(starting_value(a.name, a.kind)),
                );
            }
        }
    }
}

/// A frame's name: letters, digits and '_', as a block's.
fn frame_name(name: &str) -> Result<&str, String> {
    let n = name.trim();
    if n.is_empty() || !n.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return Err(format!("'{n}': a frame's name is letters, digits and '_'"));
    }
    Ok(n)
}

fn layer(s: &mut SceneSettings, k: usize) -> Result<&mut SceneLayer, String> {
    s.layers.get_mut(k).ok_or_else(|| format!("no layer {k}"))
}

fn frame<'s>(s: &'s mut SceneSettings, name: &str) -> Result<&'s mut SceneFrame, String> {
    s.frames
        .get_mut(name)
        .ok_or_else(|| format!("no frame '{name}'"))
}

/// Apply `edit` to `s`. Refused, `s` is unchanged.
pub fn apply(s: &mut SceneSettings, edit: &SceneEdit) -> Result<(), String> {
    match edit {
        SceneEdit::Unit(u) => {
            let u = u.trim();
            s.unit = (!u.is_empty()).then(|| u.to_string());
        }
        SceneEdit::Extent(e) => {
            s.extent = e.map(|[x, y]| serde_json::json!({"x": x, "y": y}));
        }
        SceneEdit::ViewFrom(f) => s.view_from = f.clone(),
        SceneEdit::AddFrame(name) => {
            let name = frame_name(name)?;
            if s.frames.contains_key(name) {
                return Err(format!("there is already a frame '{name}'"));
            }
            s.frames.insert(name.to_string(), SceneFrame::default());
        }
        SceneEdit::RemoveFrame(name) => {
            let gone = s
                .frames
                .remove(name)
                .ok_or_else(|| format!("no frame '{name}'"))?;
            // What was in it is now in its parent.
            let here = Some(name.clone());
            for f in s.frames.values_mut().filter(|f| f.parent == here) {
                f.parent = gone.parent.clone();
            }
            for l in s.layers.iter_mut().filter(|l| l.frame == here) {
                l.frame = gone.parent.clone();
            }
            if s.view_from == here {
                s.view_from = None;
            }
        }
        SceneEdit::FrameIn(name, parent) => frame(s, name)?.parent = parent.clone(),
        SceneEdit::Pose(name, attr, value) => {
            let f = frame(s, name)?;
            let slot = match *attr {
                "x" => &mut f.x,
                "y" => &mut f.y,
                "angle" => &mut f.angle,
                other => return Err(format!("a frame has no '{other}'")),
            };
            *slot = value.clone();
        }
        SceneEdit::AddLayer(draw) => {
            if !PRIMITIVES.iter().any(|p| p.name == draw) {
                return Err(format!("no primitive '{draw}'"));
            }
            let mut bind = BTreeMap::new();
            fill_required(draw, &mut bind);
            s.layers.push(SceneLayer {
                draw: draw.clone(),
                frame: None,
                bind,
                grab: None,
                other: BTreeMap::new(),
            });
        }
        SceneEdit::RemoveLayer(k) => {
            layer(s, *k)?;
            s.layers.remove(*k);
        }
        SceneEdit::Raise(k) => {
            layer(s, *k)?;
            if *k > 0 {
                s.layers.swap(*k - 1, *k);
            }
        }
        SceneEdit::Draw(k, draw) => {
            let p = PRIMITIVES
                .iter()
                .find(|p| p.name == draw)
                .ok_or_else(|| format!("no primitive '{draw}'"))?;
            let l = layer(s, *k)?;
            l.draw = draw.clone();
            l.bind.retain(|a, _| p.attribute(a).is_some());
            fill_required(draw, &mut l.bind);
        }
        SceneEdit::LayerIn(k, f) => layer(s, *k)?.frame = f.clone(),
        SceneEdit::Bind(k, attr, value) => {
            let l = layer(s, *k)?;
            match value {
                Some(v) => {
                    l.bind.insert(attr.clone(), v.clone());
                }
                None => {
                    l.bind.remove(attr);
                }
            }
        }
        SceneEdit::Grab(k, g) => layer(s, *k)?.grab = g.clone(),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use simsandbox_views::scene::Scene;
    use simsandbox_views::Available;

    fn edited(edits: &[SceneEdit]) -> SceneSettings {
        let mut s = SceneSettings::default();
        for e in edits {
            apply(&mut s, e).unwrap();
        }
        s
    }

    #[test]
    fn new_layers_draw_at_once_and_keep_what_fits_a_new_primitive() {
        for p in PRIMITIVES {
            let s = edited(&[SceneEdit::AddLayer(p.name.into())]);
            // Every required attribute is bound: the scene checks.
            Scene::check(&s.to_map(), &Available::default()).unwrap();
        }
        let mut s = edited(&[
            SceneEdit::AddLayer("boxes".into()),
            SceneEdit::Bind(0, "colour".into(), Some(SceneValue::Name("red".into()))),
            SceneEdit::Draw(0, "discs".into()),
        ]);
        let l = &s.layers[0];
        assert_eq!(l.bind["colour"], SceneValue::Name("red".into()));
        assert!(l.bind.contains_key("radius") && !l.bind.contains_key("width"));
        // Raised and taken away.
        apply(&mut s, &SceneEdit::AddLayer("points".into())).unwrap();
        apply(&mut s, &SceneEdit::Raise(1)).unwrap();
        assert_eq!(s.layers[0].draw, "points");
        apply(&mut s, &SceneEdit::RemoveLayer(0)).unwrap();
        assert_eq!(s.layers.len(), 1);
        assert!(apply(&mut s, &SceneEdit::RemoveLayer(3)).is_err());
    }

    #[test]
    fn a_frame_taken_away_leaves_what_was_in_it_in_its_parent() {
        let mut s = edited(&[
            SceneEdit::AddFrame("cart".into()),
            SceneEdit::AddFrame("pole".into()),
            SceneEdit::FrameIn("pole".into(), Some("cart".into())),
            SceneEdit::AddFrame("tip".into()),
            SceneEdit::FrameIn("tip".into(), Some("pole".into())),
            SceneEdit::AddLayer("discs".into()),
            SceneEdit::LayerIn(0, Some("pole".into())),
            SceneEdit::ViewFrom(Some("pole".into())),
            SceneEdit::Pose("pole".into(), "angle", Some(SceneValue::Number(0.5))),
        ]);
        assert!(apply(&mut s, &SceneEdit::AddFrame("cart".into())).is_err());
        assert!(apply(&mut s, &SceneEdit::AddFrame("a.b".into())).is_err());
        apply(&mut s, &SceneEdit::RemoveFrame("pole".into())).unwrap();
        assert_eq!(s.frames["tip"].parent.as_deref(), Some("cart"));
        assert_eq!(s.layers[0].frame.as_deref(), Some("cart"));
        assert_eq!(s.view_from, None);
        Scene::check(&s.to_map(), &Available::default()).unwrap();
    }
}
