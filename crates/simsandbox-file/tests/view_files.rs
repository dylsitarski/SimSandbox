//! View files and replacing templates (ADR 0113 step 4).

use serde_json::{json, Value};
use simsandbox_file::templates::{apply_views, fitting, read_view_file, replace_layer};
use simsandbox_file::{runs, ModelFile, SceneLayer};
use std::collections::BTreeMap;

const CARTPOLE: &str = include_str!("../../../examples/cartpole.ssbx.json");
const SKETCH: &str = include_str!("../../../examples/cartpole-sketch.ssbv.json");

#[test]
fn a_view_file_holds_templates_and_views_only() {
    let v = read_view_file(SKETCH).unwrap();
    assert_eq!(v.templates.keys().collect::<Vec<_>>(), ["stick"]);
    assert_eq!(v.views.len(), 2);
    for (text, says) in [
        (
            json!({"format": 1, "blocks": []}),
            "blocks: a view file holds templates and views only",
        ),
        (
            json!({"controls": {"probes": []}}),
            "controls.probes: a view file's controls hold views only",
        ),
        (
            json!({"templates": {"t": {"inputs": {}}}}),
            "templates.t: its view has no type",
        ),
        (json!({"format": 99}), "format 99"),
    ] {
        let e = read_view_file(&text.to_string()).unwrap_err();
        assert!(e.contains(says), "{text}: {e}");
    }
}

#[test]
fn a_view_file_is_applied_in_place_of_or_after_the_models_views() {
    let v = read_view_file(SKETCH).unwrap();
    let replaced: Value = serde_json::from_str(&apply_views(CARTPOLE, &v, true).unwrap()).unwrap();
    assert_eq!(replaced["controls"]["views"].as_array().unwrap().len(), 2);
    assert!(replaced["controls"].get("layout").is_none());
    let t = replaced["templates"].as_object().unwrap();
    assert!(t.contains_key("stick") && t.contains_key("cartpole"));
    let added: Value = serde_json::from_str(&apply_views(CARTPOLE, &v, false).unwrap()).unwrap();
    assert_eq!(added["controls"]["views"].as_array().unwrap().len(), 4);
    // A template of the same name that differs is refused.
    let mut clash = v.clone();
    clash
        .templates
        .insert("cartpole".into(), json!({"type": "plot.xy"}));
    assert!(apply_views(CARTPOLE, &clash, true)
        .unwrap_err()
        .contains("'cartpole'"));
    // Views and templates change no result: a run keeps its identity.
    let (before, _) = ModelFile::from_json(CARTPOLE).unwrap();
    let (after, _) = ModelFile::from_json(&apply_views(CARTPOLE, &v, true).unwrap()).unwrap();
    assert_eq!(runs::model_hash(&before), runs::model_hash(&after));
}

#[test]
fn a_template_layer_is_replaced_by_one_its_bindings_feed() {
    let (file, _) = ModelFile::from_json(CARTPOLE).unwrap();
    let mut templates: BTreeMap<String, Value> = file.templates.clone();
    templates.extend(read_view_file(SKETCH).unwrap().templates);
    let rig: SceneLayer = file.controls.views[0].scene.layers[1].clone();
    let bound: Vec<String> = rig.bind.keys().cloned().collect();
    // `stick` takes x and theta, which the rig binds; `body_axes` needs
    // rows it does not bind.
    assert_eq!(fitting(&templates, "cartpole", &bound, true), ["stick"]);
    let new = replace_layer(&rig, "stick", &templates).unwrap();
    assert_eq!(new.other["template"], json!("stick"));
    assert_eq!(new.bind.keys().collect::<Vec<_>>(), ["theta", "x"]);
    assert!(!new.other.contains_key("params"));
    assert!(replace_layer(&rig, "nope", &templates).is_err());
}
