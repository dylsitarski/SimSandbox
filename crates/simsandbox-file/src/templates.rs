//! Templates (ADR 0113): views with an interface. A template is a view
//! that declares `inputs` (signals it needs), `params` (constants with a
//! default) and, for a scene, frames it `offers`; inside, it names them
//! with a `$` (`"$x"`) and binds no signal of its own.
//!
//! A template is used as a whole view (`{"template": "phase", "bind":
//! {...}, "params": {...}}`) or as a scene's layer (`{"draw": "template",
//! "name": "rig", "template": "cartpole", "in": FRAME, "bind": {...}}`).
//! [`expand`] turns a view using templates into the view it draws: `$`
//! names replaced by what they are bound to, a template layer's frames and
//! layers put in its place, its frames named under the layer's name
//! (`rig/cart`) and placed in the layer's frame. Everything that draws,
//! checks or records views reads the expanded view; the file keeps the
//! written one. A template computes nothing (ADR 0093 point 3).

use crate::{Binding, SceneLayer, SceneSettings, SceneValue, ViewEntry};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

/// A scene's layer drawing a template.
pub const TEMPLATE_LAYER: &str = "template";

/// The view type of a scene (`plot.scene`, ADR 0093).
const SCENE: &str = "plot.scene";

/// Templates nested deeper than this are refused (a template that draws
/// itself, through others).
const MAX_DEPTH: usize = 16;

/// What a template's input takes.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    /// The unit it is meant in, as written (`m`); documentation and, for
    /// what it reaches, checked where it is drawn.
    #[serde(default)]
    pub unit: String,
    /// `scalar` (a signal, or a number), `rows` (a quantity over a
    /// table's rows) or `pointer` (an `input.pointer` block, for `grab`).
    #[serde(default = "scalar")]
    pub kind: String,
    #[serde(default)]
    pub description: String,
}

fn scalar() -> String {
    "scalar".into()
}

/// A template's parameter: a constant with a default.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Param {
    /// A number, or a name (a palette colour).
    pub value: Value,
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub description: String,
}

/// A template, as read from a file's `templates`.
#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    pub description: String,
    pub inputs: BTreeMap<String, Input>,
    pub params: BTreeMap<String, Param>,
    /// Frames a parent scene may draw in (`rig/tip`).
    pub offers: Vec<String>,
    /// The view itself, `$` names unbound.
    pub view: ViewEntry,
}

const KINDS: [&str; 3] = ["scalar", "rows", "pointer"];

impl Template {
    /// Read the template `name` as written.
    pub fn read(name: &str, written: &Value) -> Result<Self, String> {
        let at = |e: String| format!("templates.{name}: {e}");
        let Some(object) = written.as_object() else {
            return Err(at("a template is an object: a view with inputs".into()));
        };
        let mut rest = object.clone();
        let take = |rest: &mut serde_json::Map<String, Value>, key: &str| {
            rest.remove(key).unwrap_or(Value::Null)
        };
        let description = match take(&mut rest, "description") {
            Value::Null => String::new(),
            Value::String(s) => s,
            _ => return Err(at("description: text".into())),
        };
        let inputs: BTreeMap<String, Input> = match take(&mut rest, "inputs") {
            Value::Null => BTreeMap::new(),
            v => serde_json::from_value(v).map_err(|e| at(format!("inputs: {e}")))?,
        };
        let params: BTreeMap<String, Param> = match take(&mut rest, "params") {
            Value::Null => BTreeMap::new(),
            v => serde_json::from_value(v).map_err(|e| at(format!("params: {e}")))?,
        };
        let offers: Vec<String> = match take(&mut rest, "offers") {
            Value::Null => Vec::new(),
            v => serde_json::from_value(v).map_err(|e| at(format!("offers: {e}")))?,
        };
        for (n, i) in &inputs {
            if !crate::subsystems::valid_name(n) {
                return Err(at(format!("inputs.{n}: a name is letters, digits and '_'")));
            }
            if !KINDS.contains(&i.kind.as_str()) {
                return Err(at(format!(
                    "inputs.{n}.kind: '{}' is not one of {}",
                    i.kind,
                    KINDS.join(", ")
                )));
            }
        }
        for (n, p) in &params {
            if !crate::subsystems::valid_name(n) || inputs.contains_key(n) {
                return Err(at(format!(
                    "params.{n}: a name is letters, digits and '_', not an input's"
                )));
            }
            if !(p.value.is_number() || p.value.is_string()) {
                return Err(at(format!("params.{n}.value: a number or a name")));
            }
        }
        let view: ViewEntry =
            serde_json::from_value(Value::Object(rest)).map_err(|e| at(e.to_string()))?;
        if view.type_name.is_empty() {
            return Err(at("its view has no type".into()));
        }
        if view.template.is_some() {
            return Err(at(
                "a template is a view, not a use of another (draw one as a layer)".into(),
            ));
        }
        if !offers.is_empty() && view.type_name != SCENE {
            return Err(at("only a scene offers frames".into()));
        }
        Ok(Self {
            description,
            inputs,
            params,
            offers,
            view,
        })
    }
}

/// Whether a view draws templates: is one, or a scene with a template
/// layer.
pub fn uses_templates(v: &ViewEntry) -> bool {
    v.template.is_some() || v.scene.layers.iter().any(|l| l.draw == TEMPLATE_LAYER)
}

/// The views as they draw, each expanded; a view that does not expand is
/// left out (where it is shown, its problem is reported).
pub fn expand_all(views: &[ViewEntry], templates: &BTreeMap<String, Value>) -> Vec<ViewEntry> {
    views
        .iter()
        .filter_map(|v| expand(v, templates).ok())
        .collect()
}

/// The view `v` draws, its templates (from `templates`, a file's) put in
/// place; `v` itself if it uses none.
pub fn expand(v: &ViewEntry, templates: &BTreeMap<String, Value>) -> Result<ViewEntry, String> {
    expand_at(v, templates, 0)
}

fn expand_at(
    v: &ViewEntry,
    templates: &BTreeMap<String, Value>,
    depth: usize,
) -> Result<ViewEntry, String> {
    if depth > MAX_DEPTH {
        return Err("templates nest too deep (one draws itself?)".into());
    }
    if !uses_templates(v) {
        return Ok(v.clone());
    }
    let mut out = match &v.template {
        None => v.clone(),
        Some(name) => {
            let t = find(templates, name)?;
            // Its inputs, bound by the view's `bind`.
            let mut env = BTreeMap::new();
            for (k, b) in &v.bind {
                if !t.inputs.contains_key(k) {
                    return Err(format!(
                        "bind.{k}: template '{name}' has no input '{k}' (inputs: {})",
                        names(&t.inputs)
                    ));
                }
                match b {
                    Binding::One(s) => env.insert(k.clone(), SceneValue::Name(s.clone())),
                    Binding::Many(_) => return Err(format!("bind.{k}: an input takes one signal")),
                };
            }
            bind_params(&t, &v.params, &mut env, "params")?;
            unbound(&t, &env, "bind")?;
            let mut view = substitute(&t.view, &env)?;
            view.canvas = v.canvas;
            view.camera = v.camera.or(view.camera);
            view.selection = v.selection.clone().or(view.selection);
            view.style.extend(v.style.clone());
            view
        }
    };
    if out.type_name == SCENE {
        out.scene = expand_layers(&out.scene, templates, depth)?;
    }
    Ok(out)
}

/// A scene's template layers put in place.
fn expand_layers(
    scene: &SceneSettings,
    templates: &BTreeMap<String, Value>,
    depth: usize,
) -> Result<SceneSettings, String> {
    if depth > MAX_DEPTH {
        return Err("templates nest too deep (one draws itself?)".into());
    }
    let mut out = SceneSettings {
        layers: Vec::new(),
        ..scene.clone()
    };
    // Each template layer's name and the frames it offers.
    let mut offered: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (k, layer) in scene.layers.iter().enumerate() {
        if layer.draw != TEMPLATE_LAYER {
            out.layers.push(layer.clone());
            continue;
        }
        let at = |e: String| format!("layers[{k}]{e}");
        let mut other = layer.other.clone();
        let name = match other.remove("name") {
            Some(Value::String(n)) if valid_layer_name(&n) => n,
            Some(_) => return Err(at(".name: letters, digits and '_'".into())),
            None => return Err(at(": a template layer has a name, for its frames".into())),
        };
        if offered.contains_key(&name) || scene.frames.contains_key(&name) {
            return Err(at(format!(".name: '{name}' is taken")));
        }
        let template = match other.remove("template") {
            Some(Value::String(t)) => t,
            _ => return Err(at(".template: the template it draws, by name".into())),
        };
        let params: BTreeMap<String, Value> = match other.remove("params") {
            None => BTreeMap::new(),
            Some(Value::Object(m)) => m.into_iter().collect(),
            Some(_) => return Err(at(".params: an object".into())),
        };
        if let Some(key) = other.keys().next() {
            return Err(at(format!(".{key}: a template layer does not take it")));
        }
        let t = find(templates, &template).map_err(|e| at(format!(".template: {e}")))?;
        if t.view.type_name != SCENE {
            return Err(at(format!(
                ".template: '{template}' is a {}, not a scene",
                t.view.type_name
            )));
        }
        let unit = |s: &SceneSettings| s.unit.clone().unwrap_or_else(|| "m".into());
        if unit(&t.view.scene) != unit(scene) {
            return Err(at(format!(
                ".template: '{template}' draws in {}, this scene in {}",
                unit(&t.view.scene),
                unit(scene)
            )));
        }
        let mut env = BTreeMap::new();
        for (input, value) in &layer.bind {
            if !t.inputs.contains_key(input) {
                return Err(at(format!(
                    ".bind.{input}: '{template}' has no input '{input}' (inputs: {})",
                    names(&t.inputs)
                )));
            }
            env.insert(input.clone(), value.clone());
        }
        bind_params(&t, &params, &mut env, &format!("layers[{k}].params"))?;
        unbound(&t, &env, &format!("layers[{k}].bind"))?;
        if let Some(g) = &layer.grab {
            return Err(at(format!(
                ".grab: '{g}': a template's own layers grab; bind its pointer input"
            )));
        }
        // The template's own scene, its templates put in place first.
        let inner = expand_layers(&substitute(&t.view, &env)?.scene, templates, depth + 1)
            .map_err(|e| at(format!(" ({template}): {e}")))?;
        let placed = |f: &Option<String>| match f {
            Some(f) => Some(format!("{name}/{f}")),
            None => layer.frame.clone(),
        };
        for (f, frame) in inner.frames {
            let mut frame = frame;
            frame.parent = placed(&frame.parent);
            out.frames.insert(format!("{name}/{f}"), frame);
        }
        for l in inner.layers {
            out.layers.push(SceneLayer {
                frame: placed(&l.frame),
                ..l
            });
        }
        offered.insert(name, t.offers.clone());
    }
    // A template's frames are its own, but those it offers (ADR 0113
    // point 6).
    let reach = |what: String, f: &str| -> Result<(), String> {
        let Some((layer, frame)) = f.split_once('/') else {
            return Ok(());
        };
        match offered.get(layer) {
            Some(offers) if !offers.iter().any(|o| o == frame) => Err(format!(
                "{what}: frame '{frame}' is inside template layer '{layer}' (it offers: {})",
                if offers.is_empty() {
                    "none".to_string()
                } else {
                    offers.join(", ")
                }
            )),
            _ => Ok(()),
        }
    };
    for (f, frame) in &scene.frames {
        if let Some(p) = &frame.parent {
            reach(format!("frames.{f}.in"), p)?;
        }
    }
    for (k, l) in scene.layers.iter().enumerate() {
        if let Some(f) = &l.frame {
            reach(format!("layers[{k}].in"), f)?;
        }
    }
    if let Some(f) = &scene.view_from {
        reach("view_from".into(), f)?;
    }
    Ok(out)
}

fn valid_layer_name(n: &str) -> bool {
    crate::subsystems::valid_name(n)
}

fn names<T>(m: &BTreeMap<String, T>) -> String {
    if m.is_empty() {
        "none".into()
    } else {
        m.keys().cloned().collect::<Vec<_>>().join(", ")
    }
}

fn find(templates: &BTreeMap<String, Value>, name: &str) -> Result<Template, String> {
    let written = templates
        .get(name)
        .ok_or_else(|| format!("no template '{name}' (templates: {})", names(templates)))?;
    Template::read(name, written)
}

/// The parameters' values: those given, else the defaults.
fn bind_params(
    t: &Template,
    given: &BTreeMap<String, Value>,
    env: &mut BTreeMap<String, SceneValue>,
    at: &str,
) -> Result<(), String> {
    for k in given.keys() {
        if !t.params.contains_key(k) {
            return Err(format!(
                "{at}.{k}: no parameter '{k}' (params: {})",
                names(&t.params)
            ));
        }
    }
    for (k, p) in &t.params {
        let v = given.get(k).unwrap_or(&p.value);
        let value = match v {
            Value::Number(n) => SceneValue::Number(n.as_f64().unwrap_or(0.0)),
            Value::String(s) => SceneValue::Name(s.clone()),
            _ => return Err(format!("{at}.{k}: a number or a name")),
        };
        env.insert(k.clone(), value);
    }
    Ok(())
}

/// Every input bound.
fn unbound(t: &Template, env: &BTreeMap<String, SceneValue>, at: &str) -> Result<(), String> {
    match t.inputs.keys().find(|k| !env.contains_key(*k)) {
        Some(k) => Err(format!(
            "{at}.{k}: the template's input '{k}' is not bound{}",
            match t.inputs[k].unit.as_str() {
                "" => String::new(),
                u => format!(" (it takes {u})"),
            }
        )),
        None => Ok(()),
    }
}

/// The view with each `$name` replaced by what `env` binds it to.
fn substitute(view: &ViewEntry, env: &BTreeMap<String, SceneValue>) -> Result<ViewEntry, String> {
    let value = |v: &SceneValue, at: &str| -> Result<SceneValue, String> {
        match v {
            SceneValue::Name(n) => match n.strip_prefix('$') {
                Some(k) => env
                    .get(k)
                    .cloned()
                    .ok_or_else(|| format!("{at}: no input or parameter '{k}'")),
                None => Ok(v.clone()),
            },
            _ => Ok(v.clone()),
        }
    };
    let name = |n: &str, at: &str| -> Result<String, String> {
        match n.strip_prefix('$') {
            Some(_) => match value(&SceneValue::Name(n.into()), at)? {
                SceneValue::Name(s) => Ok(s),
                _ => Err(format!("{at}: '{n}' must be bound to a name here")),
            },
            None => Ok(n.to_string()),
        }
    };
    let mut out = view.clone();
    for (k, b) in &mut out.bind {
        let at = format!("bind.{k}");
        *b = match b {
            Binding::One(s) => Binding::One(name(s, &at)?),
            Binding::Many(v) => Binding::Many(
                v.iter()
                    .map(|s| name(s, &at))
                    .collect::<Result<Vec<_>, _>>()?,
            ),
        };
    }
    for (f, frame) in &mut out.scene.frames {
        for (axis, slot) in [
            ("x", &mut frame.x),
            ("y", &mut frame.y),
            ("angle", &mut frame.angle),
        ] {
            if let Some(v) = slot {
                *v = value(v, &format!("frames.{f}.{axis}"))?;
            }
        }
    }
    for (k, l) in out.scene.layers.iter_mut().enumerate() {
        for (a, v) in &mut l.bind {
            *v = value(v, &format!("layers[{k}].bind.{a}"))?;
        }
        if let Some(g) = &mut l.grab {
            *g = name(g, &format!("layers[{k}].grab"))?;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn templates() -> BTreeMap<String, Value> {
        let t = json!({
            "rig": {
                "type": "plot.scene", "unit": "m",
                "inputs": {"x": {"unit": "m"}, "theta": {"unit": "rad"}, "grab": {"kind": "pointer"}},
                "params": {"colour": {"value": "orange"}, "width": {"value": 0.05, "unit": "m"}},
                "frames": {"cart": {"x": "$x", "y": 0}, "pole": {"in": "cart", "angle": "$theta"},
                           "tip": {"in": "pole", "y": 1.0}},
                "offers": ["tip"],
                "layers": [
                    {"draw": "boxes", "in": "cart", "bind": {"x": 0, "y": 0, "width": 0.4, "height": 0.2}},
                    {"draw": "boxes", "in": "pole", "grab": "$grab",
                     "bind": {"x": 0, "y": 0.5, "width": "$width", "height": 1.0, "colour": "$colour"}}
                ]
            },
            "phase": {"type": "plot.xy", "inputs": {"a": {}, "b": {}}, "bind": {"x": "$a", "y": ["$b"]}}
        });
        serde_json::from_value(t).unwrap()
    }

    fn scene(layers: Value) -> ViewEntry {
        serde_json::from_value(json!({"type": "plot.scene", "unit": "m", "layers": layers}))
            .unwrap()
    }

    #[test]
    fn a_template_layer_is_put_in_place_in_its_frame() {
        let v = scene(json!([
            {"draw": "segments", "bind": {"x1": -1, "y1": 0, "x2": 1, "y2": 0}},
            {"draw": "template", "name": "left", "template": "rig", "params": {"colour": "blue"},
             "bind": {"x": "m.x", "theta": "m.a", "grab": "hand"}},
            {"draw": "points", "in": "left/tip", "bind": {"x": 0, "y": 0}}
        ]));
        let e = expand(&v, &templates()).unwrap();
        assert!(!uses_templates(&e));
        let frames: Vec<&String> = e.scene.frames.keys().collect();
        assert_eq!(frames, ["left/cart", "left/pole", "left/tip"]);
        assert_eq!(
            e.scene.frames["left/cart"].x,
            Some(SceneValue::Name("m.x".into()))
        );
        assert_eq!(
            e.scene.frames["left/pole"].parent.as_deref(),
            Some("left/cart")
        );
        let draws: Vec<&str> = e.scene.layers.iter().map(|l| l.draw.as_str()).collect();
        assert_eq!(draws, ["segments", "boxes", "boxes", "points"]);
        let pole = &e.scene.layers[2];
        assert_eq!(pole.grab.as_deref(), Some("hand"));
        assert_eq!(pole.bind["colour"], SceneValue::Name("blue".into()));
        assert_eq!(pole.bind["width"], SceneValue::Number(0.05));
        // Unchanged when it uses none.
        let plain = scene(json!([{"draw": "points", "bind": {"x": 0, "y": 0}}]));
        assert_eq!(expand(&plain, &templates()).unwrap(), plain);
    }

    #[test]
    fn a_template_layer_sits_in_a_frame_of_its_scene() {
        let mut v = scene(json!([
            {"draw": "template", "name": "r", "template": "rig", "in": "base",
             "bind": {"x": 0, "theta": "m.a", "grab": "hand"}}
        ]));
        v.scene.frames.insert(
            "base".into(),
            serde_json::from_value(json!({"x": "m.b", "y": 0})).unwrap(),
        );
        let e = expand(&v, &templates()).unwrap();
        // Its frames with no parent are placed in the layer's frame.
        assert_eq!(e.scene.frames["r/cart"].parent.as_deref(), Some("base"));
        assert_eq!(e.scene.frames["r/cart"].x, Some(SceneValue::Number(0.0)));
    }

    #[test]
    fn a_whole_view_template_binds_its_inputs() {
        let v: ViewEntry =
            serde_json::from_value(json!({"template": "phase", "bind": {"a": "m.x", "b": "m.v"}}))
                .unwrap();
        let e = expand(&v, &templates()).unwrap();
        assert_eq!(e.type_name, "plot.xy");
        assert_eq!(e.bind["x"], Binding::One("m.x".into()));
        assert_eq!(e.bind["y"], Binding::Many(vec!["m.v".into()]));
    }

    #[test]
    fn what_does_not_fit_is_refused_at_its_path() {
        let t = templates();
        let cases = [
            (
                json!([{"draw": "template", "template": "rig", "bind": {}}]),
                "a template layer has a name",
            ),
            (
                json!([{"draw": "template", "name": "r", "template": "nope"}]),
                "no template 'nope' (templates: phase, rig)",
            ),
            (
                json!([{"draw": "template", "name": "r", "template": "phase"}]),
                "is a plot.xy, not a scene",
            ),
            (
                json!([{"draw": "template", "name": "r", "template": "rig", "bind": {"x": "m.x", "grab": "h"}}]),
                "layers[0].bind.theta: the template's input 'theta' is not bound (it takes rad)",
            ),
            (
                json!([{"draw": "template", "name": "r", "template": "rig",
                     "bind": {"x": "m.x", "theta": "m.a", "grab": "h", "z": 1}}]),
                "has no input 'z'",
            ),
            (
                json!([{"draw": "template", "name": "r", "template": "rig",
                     "bind": {"x": "m.x", "theta": "m.a", "grab": "h"}, "params": {"size": 2}}]),
                "layers[0].params.size: no parameter 'size' (params: colour, width)",
            ),
            (
                json!([{"draw": "template", "name": "r", "template": "rig",
                     "bind": {"x": "m.x", "theta": "m.a", "grab": "h"}},
                    {"draw": "points", "in": "r/pole", "bind": {"x": 0, "y": 0}}]),
                "layers[1].in: frame 'pole' is inside template layer 'r' (it offers: tip)",
            ),
        ];
        for (layers, says) in cases {
            let e = expand(&scene(layers.clone()), &t).unwrap_err();
            assert!(e.contains(says), "{layers}: {e}");
        }
        let mut cm = scene(json!([{"draw": "template", "name": "r", "template": "rig",
                                   "bind": {"x": "m.x", "theta": "m.a", "grab": "h"}}]));
        cm.scene.unit = Some("cm".into());
        assert!(expand(&cm, &t)
            .unwrap_err()
            .contains("draws in m, this scene in cm"));
        // A template that draws itself is refused, not followed forever.
        let mut looping = t.clone();
        looping.insert(
            "me".into(),
            json!({"type": "plot.scene", "layers": [{"draw": "template", "name": "again", "template": "me"}]}),
        );
        let e = expand(
            &scene(json!([{"draw": "template", "name": "a", "template": "me"}])),
            &looping,
        )
        .unwrap_err();
        assert!(e.contains("too deep"), "{e}");
        let bad = Template::read(
            "x",
            &json!({"type": "plot.xy", "inputs": {"a": {"kind": "vector"}}}),
        );
        assert!(bad.unwrap_err().contains("inputs.a.kind"));
    }
}
