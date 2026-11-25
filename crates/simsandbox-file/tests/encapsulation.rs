//! Encapsulated models (ADR 0110): what reaches inside a used model
//! converts to its interface as the file is read, with results unchanged;
//! uses of separate files are reported; `expose` and `offer_output` make
//! the interface by hand.

use serde_json::{json, Value};
use simsandbox_core::registry::Registry;
use simsandbox_file::compose::load_merged;
use simsandbox_file::encapsulate::{encapsulate, expose, offer_output};

fn registry() -> Registry {
    let mut r = Registry::new();
    simsandbox_eq::register(&mut r).unwrap();
    simsandbox_agents::register(&mut r).unwrap();
    simsandbox_mech::register(&mut r).unwrap();
    r
}

/// A model `spring` with a block `s` (`k` written, `c` the type's), used
/// twice.
fn two_uses(first: Value, second: Value) -> Value {
    json!({
        "format": 1,
        "run": {"dt": 0.1, "t_end": 0.3},
        "models": {"spring": {"blocks": [{
            "name": "s", "type": "eq.equations", "method": {"name": "euler"},
            "equations": ["param k [1/s^2] = 4", "param c [1/s] = 0.5", "state x [m] = 1",
                          "state v [m/s] = 0", "dx/dt = v", "dv/dt = -k*x - c*v"],
            "params": {"k": 3.0}
        }]}},
        "blocks": [
            {"name": "a", "model": "spring", "params": first},
            {"name": "b", "model": "spring", "params": second}
        ]
    })
}

#[test]
fn uses_setting_the_same_inner_value_share_one_declared_parameter() {
    let text = two_uses(json!({"s.k": 9.0, "s.c": 1.0}), json!({"s.k": 2.0})).to_string();
    let e = encapsulate(&text, &registry(), None).unwrap();
    assert_eq!(e.conversions.len(), 3, "{:?}", e.conversions);
    assert!(e.warnings.is_empty(), "{:?}", e.warnings);
    let v: Value = serde_json::from_str(&e.text).unwrap();
    let spring = &v["models"]["spring"];
    // The written value and the type's default become the defaults, with
    // the parameter's unit.
    assert_eq!(spring["params"]["k"]["value"], json!(3.0));
    assert_eq!(spring["params"]["c"]["value"], json!(0.5));
    for p in ["k", "c"] {
        let unit = spring["params"][p]["unit"].as_str().unwrap();
        assert!(unit.contains('s'), "{p}: {unit}");
    }
    assert_eq!(spring["blocks"][0]["params"]["k"], json!("k"));
    assert_eq!(spring["blocks"][0]["params"]["c"], json!("c"));
    assert_eq!(v["blocks"][0]["params"], json!({"k": 9.0, "c": 1.0}));
    assert_eq!(v["blocks"][1]["params"], json!({"k": 2.0}));
    // Converting again changes nothing.
    let again = encapsulate(&e.text, &registry(), None).unwrap();
    assert!(again.conversions.is_empty());
    assert_eq!(again.text, e.text);
}

#[test]
fn a_converted_model_runs_as_one_written_with_its_interface() {
    let old = two_uses(json!({"s.k": 9.0}), json!({})).to_string();
    let mut by_hand = two_uses(json!({"k": 9.0}), json!({}));
    by_hand["models"]["spring"]["params"] = json!({"k": {"value": 3.0, "unit": "1/s^2"}});
    by_hand["models"]["spring"]["blocks"][0]["params"]["k"] = json!("k");
    let run = |text: &str| {
        let loaded = load_merged(text, &registry(), None).unwrap();
        let ids = loaded.ids.clone();
        let mut sim = simsandbox_core::sim::Simulator::new(loaded.model, 0.0).unwrap();
        for _ in 0..3 {
            sim.step(0.1).unwrap();
        }
        ["a/s", "b/s"].map(|n| sim.output(ids[n], "x").unwrap()[0])
    };
    let (old_x, new_x) = (run(&old), run(&by_hand.to_string()));
    assert_eq!(old_x.map(f64::to_bits), new_x.map(f64::to_bits));
    // a's spring is stiffer than b's, which keeps the model's 3.
    assert!(old_x[0] < old_x[1]);
    let loaded = load_merged(&old, &registry(), None).unwrap();
    assert!(loaded.conversions.iter().any(|c| c.contains("ADR 0110")));
    assert_eq!(
        loaded.file.blocks[0].params.keys().collect::<Vec<_>>(),
        ["k"]
    );
}

#[test]
fn a_value_set_by_a_formula_inside_stays_and_is_reported() {
    let mut v = two_uses(json!({"s.k": 9.0}), json!({}));
    v["models"]["spring"]["params"] = json!({"stiff": {"value": 5.0}});
    v["models"]["spring"]["blocks"][0]["params"]["k"] = json!("2*stiff");
    let e = encapsulate(&v.to_string(), &registry(), None).unwrap();
    assert!(e.conversions.is_empty(), "{:?}", e.conversions);
    assert_eq!(e.warnings.len(), 1);
    assert!(
        e.warnings[0].contains("blocks[0].params.s.k"),
        "{}",
        e.warnings[0]
    );
    assert!(
        e.warnings[0].contains("formula '2*stiff'"),
        "{}",
        e.warnings[0]
    );
}

#[test]
fn reaching_inside_a_separate_file_is_reported_not_rewritten() {
    let dir = std::env::temp_dir().join(format!("ssbx-encapsulate-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let used = json!({"format": 1, "run": {"dt": 0.1}, "blocks": [{
        "name": "s", "type": "eq.equations", "method": {"name": "euler"},
        "equations": ["param k = 4", "state x = 1", "dx/dt = -k*x"]}]});
    std::fs::write(dir.join("spring.ssbx.json"), used.to_string()).unwrap();
    let text = json!({
        "format": 1, "run": {"t_end": 0.3},
        "blocks": [
            {"name": "a", "file": "spring.ssbx.json", "params": {"s.k": 2.0}},
            {"name": "g", "type": "eq.equations", "equations": ["input u", "y = 2*u"]}
        ],
        "connections": [{"from": "a/s.x", "to": "g.u"}]
    })
    .to_string();
    let e = encapsulate(&text, &registry(), Some(&dir)).unwrap();
    assert!(e.conversions.is_empty());
    assert_eq!(e.text, text, "a file is not rewritten for another's sake");
    assert_eq!(e.warnings.len(), 2, "{:?}", e.warnings);
    assert!(
        e.warnings[0].contains("expose it there"),
        "{}",
        e.warnings[0]
    );
    assert!(
        e.warnings[1].contains("offer it there"),
        "{}",
        e.warnings[1]
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn expose_declares_a_value_inside_and_gives_its_name() {
    let text = two_uses(json!({}), json!({})).to_string();
    let r = registry();
    let (text, name) = expose(&text, &r, None, Some("spring"), "s.c", None).unwrap();
    assert_eq!(name, "c");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["models"]["spring"]["params"]["c"]["value"], json!(0.5));
    // Exposed already: the same name.
    let (same, again) = expose(&text, &r, None, Some("spring"), "s.c", None).unwrap();
    assert_eq!((same.as_str(), again.as_str()), (text.as_str(), "c"));
    // A name given, taken, or not a name.
    let (_, named) = expose(&text, &r, None, Some("spring"), "s.k", Some("stiffness")).unwrap();
    assert_eq!(named, "stiffness");
    let taken = expose(&text, &r, None, Some("spring"), "s.k", Some("c")).unwrap_err();
    assert!(taken.contains("taken"), "{taken}");
    let missing = expose(&text, &r, None, Some("nothing"), "s.k", None).unwrap_err();
    assert!(missing.contains("no model named 'nothing'"), "{missing}");
    let wrong = expose(&text, &r, None, Some("spring"), "s/k", None).unwrap_err();
    assert!(wrong.contains("block.param"), "{wrong}");
}

#[test]
fn offer_output_offers_an_inner_output_once() {
    let text = two_uses(json!({}), json!({})).to_string();
    let (text, name) = offer_output(&text, Some("spring"), "s.x", None).unwrap();
    assert_eq!(name, "x");
    let (same, again) = offer_output(&text, Some("spring"), "s.x", Some("other")).unwrap();
    assert_eq!((same.as_str(), again.as_str()), (text.as_str(), "x"));
    let (_, v) = offer_output(&text, Some("spring"), "s.v", Some("speed")).unwrap();
    assert_eq!(v, "speed");
    let none = offer_output(&text, Some("spring"), "q.x", None).unwrap_err();
    assert!(none.contains("no block 'q'"), "{none}");
}
