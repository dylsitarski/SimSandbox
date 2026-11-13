//! Activation of used models (ADR 0088 step 2): a used model whose state
//! counts time since it started, `dx/dt = 1`, activated by a gate high at
//! t = 3, 4, 5 (steps of 1 s): held under `while`, started again on edges
//! and on a rising `restart`.

use serde_json::{json, Value};
use simsandbox_session::compose::run_composition;
use simsandbox_session::examples::registry;
use simsandbox_session::session::Session;

/// The model: `src` gives the gate (and runs in time); `c` uses `counter`
/// on its parent's clock, with `use_extra` set on the use and `wires`
/// from the gate.
fn model(use_extra: Value, wires: &[&str]) -> String {
    let mut c = json!({"name": "c", "model": "counter", "clock": "shared"});
    for (k, v) in use_extra.as_object().unwrap() {
        c[k] = v.clone();
    }
    let connections: Vec<Value> = wires
        .iter()
        .map(|to| json!({"from": "src.gate", "to": to}))
        .collect();
    json!({"format": 1, "run": {"dt": 1, "t_end": 8},
        "models": {"counter": {"run": {"dt": 1}, "blocks": [
            {"name": "x", "type": "eq.equations", "method": {"name": "euler"},
             "equations": ["state x = 0", "dx/dt = 1"]}]}},
        "blocks": [
            {"name": "src", "type": "eq.equations",
             "equations": ["state clock = 0", "dclock/dt = 1", "gate = (t > 2.5)*(t < 5.5)"]},
            c],
        "connections": connections})
    .to_string()
}

fn counted(text: &str) -> Vec<f64> {
    let c = run_composition(text, &registry(), None).unwrap();
    c.values("c/x.x").unwrap().to_vec()
}

#[test]
fn each_mode_holds_or_starts_the_used_model_again() {
    let activated = |mode: &str| model(json!({"activate": mode}), &["c/activate"]);
    // Not activated: it counts the whole run.
    assert_eq!(
        counted(&model(json!({}), &[])),
        [0., 1., 2., 3., 4., 5., 6., 7., 8.]
    );
    // Steps only from instants where the gate is high; held after.
    assert_eq!(
        counted(&activated("while")),
        [0., 0., 0., 0., 1., 2., 3., 3., 3.]
    );
    // Starts again at the rise (the sample at t = 3 is before it), the
    // fall, or both.
    assert_eq!(
        counted(&activated("rise")),
        [0., 1., 2., 3., 1., 2., 3., 4., 5.]
    );
    assert_eq!(
        counted(&activated("fall")),
        [0., 1., 2., 3., 4., 5., 6., 1., 2.]
    );
    assert_eq!(
        counted(&activated("change")),
        [0., 1., 2., 3., 1., 2., 3., 1., 2.]
    );
    // A restart input alone starts it again on its rise.
    let restart = model(json!({"restart": true}), &["c/restart"]);
    assert_eq!(counted(&restart), [0., 1., 2., 3., 1., 2., 3., 4., 5.]);
    // Switched on and started again by the same signal: from its start
    // each time it is switched on, held while off.
    let both = model(
        json!({"activate": "while", "restart": true}),
        &["c/activate", "c/restart"],
    );
    assert_eq!(counted(&both), [0., 0., 0., 0., 1., 2., 3., 3., 3.]);
}

#[test]
fn a_session_runs_it_alike_and_rewinds_exactly() {
    let text = model(json!({"activate": "change"}), &["c/activate"]);
    let mut s = Session::load("m", &text, &registry()).unwrap();
    s.set_recorder(Some(Default::default()));
    s.run_until(8.0).unwrap();
    s.flush(true);
    let values = |s: &Session| -> Vec<u64> {
        let h = s.signals();
        h.values(h.resolve("c/x.x").unwrap())
            .iter()
            .map(|v| v.to_bits())
            .collect()
    };
    let first = values(&s);
    let headless: Vec<u64> = counted(&text).iter().map(|v| v.to_bits()).collect();
    assert_eq!(first, headless);
    // Back into a stretch after a start, and on again.
    for at in [4.0, 6.0, 1.0] {
        s.rewind_to(at);
        s.run_until(8.0).unwrap();
        s.flush(true);
        assert_eq!(values(&s), first, "rewound to {at}");
    }
    // It started again at the rise and the fall; a rewind takes the count
    // back with it, and running on counts them again.
    assert_eq!(s.activations(), [("c".to_string(), false, 2)]);
    s.rewind_to(1.0);
    assert_eq!(s.activations(), [("c".to_string(), false, 0)]);
    s.run_until(8.0).unwrap();
    assert_eq!(s.activations(), [("c".to_string(), false, 2)]);
}

#[test]
fn the_node_shows_its_signals_first_and_what_the_run_says_of_it() {
    use simsandbox_session::diagram::Diagram;
    let text = model(
        json!({"activate": "while", "restart": true}),
        &["c/activate", "c/restart"],
    );
    let (mut f, _) = simsandbox_file::ModelFile::from_json(&text).unwrap();
    f.resolve_models().unwrap();
    let r = registry();
    let mut d = Diagram::new(&f, &r);
    let c = d.node("c").unwrap();
    let ports: Vec<&str> = d.nodes[c].inputs.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(ports[..2], ["activate", "restart"]);
    assert_eq!(
        d.nodes[c].settings.get("activate").map(String::as_str),
        Some("while")
    );
    // Held at the start (the gate is low), running from t = 3.
    let mut s = Session::load("m", &text, &r).unwrap();
    s.run_until(1.0).unwrap();
    d.live_activations(&s.activations());
    assert_eq!(d.nodes[c].status.as_deref(), Some("held"));
    s.run_until(4.0).unwrap();
    d.live_activations(&s.activations());
    assert_eq!(d.nodes[c].status.as_deref(), Some("started again once"));
}

#[test]
fn activation_mistakes_are_refused() {
    let r = registry();
    let err = |text: String| -> String { run_composition(&text, &r, None).err().expect("refused") };
    let e = err(model(json!({"activate": "often"}), &["c/activate"]));
    assert!(e.contains("not a mode"), "{e}");
    let e = err(model(json!({"activate": "while"}), &[]));
    assert!(e.contains("'activate' is not wired"), "{e}");
    let e = err(model(json!({"restart": true}), &[]));
    assert!(e.contains("'restart' is not wired"), "{e}");
    // A block that is not a use or a study.
    let mut v: Value = serde_json::from_str(&model(json!({}), &[])).unwrap();
    v["blocks"][0]["restart"] = json!(true);
    let e = err(v.to_string());
    assert!(e.contains("restart"), "{e}");
    // A used model with a block of the signal's name.
    let mut v: Value =
        serde_json::from_str(&model(json!({"activate": "rise"}), &["c/activate"])).unwrap();
    v["models"]["counter"]["blocks"][0]["name"] = json!("activate");
    let e = err(v.to_string());
    assert!(e.contains("named 'activate'"), "{e}");
}

#[test]
fn a_used_model_on_its_own_clock_is_activated_through_a_probe_link() {
    let mut v: Value = serde_json::from_str(&model(json!({"activate": "rise"}), &[])).unwrap();
    v["blocks"][1]["clock"] = json!({"dt": 1});
    v["connections"] = json!([{"from": "probe:p.src.gate", "to": "c/activate"}]);
    v["controls"] =
        json!({"probes": [{"name": "p", "record": ["src.gate"], "every": {"steps": 1}}]});
    let text = v.to_string();
    let expected = [0., 1., 2., 3., 1., 2., 3., 4., 5.];
    assert_eq!(counted(&text), expected);
    // In a session, as the app runs it, the same.
    let mut s = Session::load("m", &text, &registry()).unwrap();
    s.run_until(8.0).unwrap();
    s.flush(true);
    let h = s.signals();
    assert_eq!(h.values(h.resolve("c/x.x").unwrap()), expected);
}

#[test]
fn activation_is_set_and_removed_by_edits() {
    use simsandbox_session::diagram::{apply, Edit};
    let r = registry();
    let (mut f, _) = simsandbox_file::ModelFile::from_json(&model(json!({}), &[])).unwrap();
    f.resolve_models().unwrap();
    let edit = |f: &mut simsandbox_file::ModelFile, e: Edit| apply(f, &r, &e).unwrap();
    edit(
        &mut f,
        Edit::SetActivate {
            block: "c".into(),
            mode: Some("while".into()),
        },
    );
    edit(
        &mut f,
        Edit::SetRestart {
            block: "c".into(),
            on: true,
        },
    );
    // Its node shows both as ports, to wire on the canvas.
    let d = simsandbox_session::diagram::Diagram::new(&f, &r);
    let node = &d.nodes[d.node("c").unwrap()];
    let ports: Vec<&str> = node.inputs.iter().map(|p| p.name.as_str()).collect();
    assert!(
        ports.contains(&"activate") && ports.contains(&"restart"),
        "{ports:?}"
    );
    // Wired as the canvas wires them, node to port.
    for to in ["c.activate", "c.restart"] {
        edit(
            &mut f,
            Edit::Connect {
                from: "src.gate".into(),
                to: to.into(),
            },
        );
    }
    assert_eq!(counted(&f.to_json()), [0., 0., 0., 0., 1., 2., 3., 3., 3.]);
    // Off again: the inputs and their wires go; it runs throughout.
    edit(
        &mut f,
        Edit::SetActivate {
            block: "c".into(),
            mode: None,
        },
    );
    edit(
        &mut f,
        Edit::SetRestart {
            block: "c".into(),
            on: false,
        },
    );
    assert!(f.connections.is_empty());
    assert_eq!(counted(&f.to_json()), [0., 1., 2., 3., 4., 5., 6., 7., 8.]);
}
