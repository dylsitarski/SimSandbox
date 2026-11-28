//! The Definitions projection (ADR 0108): each block where it is defined,
//! with its values now.

use simsandbox_session::definition::definition;
use simsandbox_session::examples::{registry, EXAMPLES};
use simsandbox_session::session::Session;

fn session(name: &str) -> Session {
    let e = EXAMPLES.iter().find(|e| e.name.starts_with(name)).unwrap();
    Session::load(e.name, e.text, &registry()).unwrap()
}

#[test]
fn typed_equations_are_a_sheet_with_each_lines_value() {
    let s = session("Cart-pole");
    let d = definition(&s, "control").unwrap();
    assert_eq!(d["kind"], "equations");
    let line = |name: &str| {
        d["lines"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["name"] == name)
            .unwrap_or_else(|| panic!("no line for {name}"))
            .clone()
    };
    // A parameter, with its value and unit and the note after it.
    let k1 = line("k1");
    assert_eq!(
        (k1["what"].as_str(), k1["unit"].as_str()),
        (Some("param"), Some("N/m"))
    );
    assert!(k1["note"].as_str().is_some_and(|n| !n.is_empty()));
    // An input: what its wire carries (the pole starts 0.1 rad from upright).
    let th = line("th");
    assert_eq!(th["what"], "input");
    assert!((th["value"].as_f64().unwrap() - 0.1).abs() < 1e-12);
    // A defined quantity: the block's output now.
    let f = line("F");
    assert_eq!(f["what"], "defined");
    assert_eq!(f["unit"], "N");
    assert!(f["value"].as_f64().is_some());
    // Every line is there, in order.
    let e = &s
        .file
        .blocks
        .iter()
        .find(|b| b.name == "control")
        .unwrap()
        .equations;
    assert_eq!(d["lines"].as_array().unwrap().len(), e.len());
}

#[test]
fn a_mechanism_a_study_a_use_and_other_blocks_are_shown_as_what_defines_them() {
    let s = session("Cart-pole");
    let d = definition(&s, "cartpole").unwrap();
    assert_eq!(d["kind"], "bodies");
    assert_eq!(d["bodies"].as_array().unwrap().len(), 2);
    assert_eq!(d["pictures"], serde_json::json!(["cartpole/bodies"]));
    let d = definition(&s, "hand").unwrap();
    assert_eq!(d["kind"], "parameters");
    assert!(d["params"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["name"] == "x"));
    assert!(definition(&s, "nothing").is_err());
    let d = definition(&session("Tuning"), "tuning").unwrap();
    assert_eq!(d["kind"], "study");
    assert!(d["sample"].is_object());
    let d = definition(&session("Platoon"), "car1").unwrap();
    assert_eq!(d["kind"], "model");
}
