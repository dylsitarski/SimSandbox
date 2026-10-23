//! Subsystems and declared parameters (ADR 0043): files with them load as
//! the flat model a person would write by hand, report problems where they
//! are written, and save back what changed.

use simsandbox_core::registry::Registry;
use simsandbox_core::sim::Simulator;
use simsandbox_file::{load, FileError, ModelFile, ParamValue};

fn registry() -> Registry {
    let mut r = Registry::new();
    simsandbox_eq::register(&mut r).unwrap();
    simsandbox_agents::register(&mut r).unwrap();
    r
}

fn invalid_path(text: &str) -> String {
    match load(text, &registry()) {
        Err(FileError::Invalid { path, .. }) => path,
        Err(e) => panic!("expected an Invalid error, got {e}"),
        Ok(_) => panic!("expected an error"),
    }
}

/// A lag `dx/dt = (u - x)/T` with `T` from the parameter `tau`, used as
/// `a` (tau 1) and `b` (tau = 2*slow), in series after a ramp.
const LAGS: &str = r#"{
  "format": 1,
  "run": {"dt": 0.01},
  "params": {"slow": {"value": 1.5, "description": "How slow b is"}},
  "subsystems": {
    "lag": {
      "params": {"tau": {"value": 1.0, "unit": "s"}},
      "inputs": [{"name": "u", "to": ["f.u"]}],
      "outputs": [{"name": "y", "from": "f.x"}],
      "blocks": [
        {"name": "f", "type": "eq.equations", "method": {"name": "rk4"},
         "equations": ["input u", "dx/dt = (u - x)/T"], "params": {"T": "tau"}}
      ]
    }
  },
  "blocks": [
    {"name": "ramp", "type": "eq.equations", "method": {"name": "rk4"}, "equations": ["dr/dt = 1"]},
    {"name": "a", "type": "lag"},
    {"name": "b", "type": "lag", "params": {"tau": "2*slow"}}
  ],
  "connections": [
    {"from": "ramp.r", "to": "a.u"},
    {"from": "a.y", "to": "b.u"}
  ]
}"#;

/// The same model written by hand without subsystems.
const LAGS_FLAT: &str = r#"{
  "format": 1,
  "run": {"dt": 0.01},
  "blocks": [
    {"name": "ramp", "type": "eq.equations", "method": {"name": "rk4"}, "equations": ["dr/dt = 1"]},
    {"name": "a", "type": "eq.equations", "method": {"name": "rk4"},
     "equations": ["input u", "dx/dt = (u - x)/T"], "params": {"T": 1.0}},
    {"name": "b", "type": "eq.equations", "method": {"name": "rk4"},
     "equations": ["input u", "dx/dt = (u - x)/T"], "params": {"T": 3.0}}
  ],
  "connections": [
    {"from": "ramp.r", "to": "a.u"},
    {"from": "a.x", "to": "b.u"}
  ]
}"#;

fn run_to(text: &str, port: &str, steps: usize) -> f64 {
    let loaded = load(text, &registry()).unwrap();
    let (b, p) = port.rsplit_once('.').unwrap();
    let id = loaded.ids[b];
    let mut sim = Simulator::new(loaded.model, 0.0).unwrap();
    for _ in 0..steps {
        sim.step(0.01).unwrap();
    }
    sim.output(id, p).unwrap()[0]
}

#[test]
fn instances_flatten_into_the_model_written_by_hand() {
    let loaded = load(LAGS, &registry()).unwrap();
    let names: Vec<&str> = loaded.flat.blocks.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["ramp", "a/f", "b/f"]);
    let wires: Vec<(&str, &str)> = loaded
        .flat
        .connections
        .iter()
        .map(|c| (c.from.as_str(), c.to.as_str()))
        .collect();
    assert_eq!(wires, [("ramp.r", "a/f.u"), ("a/f.x", "b/f.u")]);
    assert_eq!(loaded.model.param(loaded.ids["a/f"], "T").unwrap(), 1.0);
    assert_eq!(loaded.model.param(loaded.ids["b/f"], "T").unwrap(), 3.0);
    // Fused like the hand-written model, and the same numbers bit for bit.
    assert_eq!(loaded.groups.len(), 1);
    assert_eq!(run_to(LAGS, "b/f.x", 300), run_to(LAGS_FLAT, "b.x", 300));
    // The file as written is kept for saving and the canvas.
    assert_eq!(loaded.file.blocks.len(), 3);
}

#[test]
fn declared_parameters_feed_formulas_and_can_be_changed() {
    let mut plan = load(LAGS, &registry()).unwrap().params;
    let keys: Vec<&str> = plan.declared.iter().map(|d| d.key.as_str()).collect();
    assert_eq!(keys, ["slow", "a.tau", "b.tau"]);
    assert!(plan.declared("a.tau").unwrap().editable);
    assert!(!plan.declared("b.tau").unwrap().editable, "a formula");
    let set = plan.set("slow", 2.0).unwrap();
    let t = |b: &str| set.iter().find(|(x, p, _)| x == b && p == "T").unwrap().2;
    assert_eq!((t("a/f"), t("b/f")), (1.0, 4.0));
    assert_eq!(plan.declared("b.tau").unwrap().value, 4.0);
    let set = plan.set("a.tau", 0.5).unwrap();
    assert_eq!(set.iter().find(|(b, _, _)| b == "a/f").unwrap().2, 0.5);
    assert!(plan.set("b.tau", 1.0).unwrap_err().contains("2*slow"));
    assert!(plan.set("nope", 1.0).is_err());
}

#[test]
fn shared_model_parameters_set_several_blocks() {
    // Q30: the grazing field size, once.
    let e = include_str!("../../../examples/grazing.ssbx.json");
    let (mut file, _) = ModelFile::from_json(e).unwrap();
    file.params.insert(
        "size".into(),
        simsandbox_file::ParamDecl {
            value: 60.0,
            unit: "m".into(),
            description: "Side of the field".into(),
            range: Some([5.0, 200.0]),
        },
    );
    for b in &mut file.blocks {
        b.params
            .insert("size".into(), ParamValue::Formula("size".into()));
    }
    let loaded = load(&file.to_json(), &registry()).unwrap();
    for b in ["grass", "sheep"] {
        assert_eq!(loaded.model.param(loaded.ids[b], "size").unwrap(), 60.0);
    }
    // Sliders may name declared parameters.
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
}

#[test]
fn subsystems_nest() {
    let text = LAGS
        .replace(
            r#""lag": {"#,
            r#""pair": {
      "params": {"tau": {"value": 2.0}},
      "inputs": [{"name": "u", "to": ["p.u"]}],
      "outputs": [{"name": "y", "from": "q.y"}],
      "blocks": [{"name": "p", "type": "lag", "params": {"tau": "tau"}}, {"name": "q", "type": "lag", "params": {"tau": "tau/2"}}],
      "connections": [{"from": "p.y", "to": "q.u"}]
    },
    "lag": {"#,
        )
        .replace(r#"{"name": "b", "type": "lag", "params": {"tau": "2*slow"}}"#, r#"{"name": "b", "type": "pair"}"#);
    let loaded = load(&text, &registry()).unwrap();
    let names: Vec<&str> = loaded.flat.blocks.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["ramp", "a/f", "b/p/f", "b/q/f"]);
    assert_eq!(loaded.model.param(loaded.ids["b/q/f"], "T").unwrap(), 1.0);
    let d = loaded.params.declared("b/q.tau").unwrap();
    assert!(!d.editable, "inside another subsystem");
    assert!(loaded
        .flat
        .connections
        .iter()
        .any(|c| c.from == "b/p/f.x" && c.to == "b/q/f.u"));
}

#[test]
fn problems_are_reported_where_they_are_written() {
    let bad = |from: &str, to: &str| invalid_path(&LAGS.replacen(from, to, 1));
    assert_eq!(
        bad(r#""params": {"tau": "2*slow"}"#, r#""params": {"tao": 2}"#),
        "blocks[2].params.tao"
    );
    assert_eq!(
        bad(r#""tau": "2*slow""#, r#""tau": "2*fast""#),
        "blocks[2].params.tau"
    );
    // A port the inner block lacks.
    assert_eq!(
        bad(r#""from": "f.x""#, r#""from": "f.z""#),
        "subsystems.lag.outputs[0].from"
    );
    assert_eq!(
        bad(r#""from": "f.x""#, r#""from": "g.x""#),
        "subsystems.lag.outputs[0].from"
    );
    assert_eq!(
        bad(r#""dx/dt = (u - x)/T""#, r#""dx/dt = (u - x/T""#),
        "subsystems.lag.blocks[0].equations"
    );
    assert_eq!(
        bad(
            r#"{"from": "a.y", "to": "b.u"}"#,
            r#"{"from": "a.y", "to": "b.v"}"#
        ),
        "connections[1].to"
    );
    assert_eq!(
        bad(r#""T": "tau""#, r#""T": "tau*k""#),
        "subsystems.lag.blocks[0].params.T"
    );
    assert_eq!(
        bad(
            r#"{"name": "a", "type": "lag"}"#,
            r#"{"name": "a", "type": "lag", "method": {"name": "rk4"}}"#
        ),
        "blocks[1].method"
    );
    // A subsystem that contains itself.
    let looped = LAGS.replacen(
        r#""type": "eq.equations", "method": {"name": "rk4"},
         "equations""#,
        r#""type": "lag", "equations""#,
        1,
    );
    assert!(
        invalid_path(&looped).ends_with(".type") || invalid_path(&looped).contains("equations")
    );
    // Unused subsystems are still checked.
    let unused = LAGS
        .replace(
            r#"{"name": "b", "type": "lag", "params": {"tau": "2*slow"}}"#,
            r#"{"name": "b", "type": "eq.lorenz", "method": {"name": "rk4"}}"#,
        )
        .replace(
            r#"{"from": "a.y", "to": "b.u"}"#,
            r#"{"from": "a.y", "to": "a.u"}"#,
        )
        .replace(r#"{"from": "ramp.r", "to": "a.u"},"#, "")
        .replace(r#"{"name": "a", "type": "lag"},"#, "")
        .replace(r#"{"from": "a.y", "to": "a.u"}"#, "");
    let unused = unused.replace(r#""from": "f.x""#, r#""from": "f.nope""#);
    assert!(load(&unused, &registry()).is_err());
}

#[test]
fn saving_writes_back_only_what_changed() {
    let mut loaded = load(LAGS, &registry()).unwrap();
    // Running values: a's tau, an inner value of b, the model's slow.
    for (b, p, v) in loaded.params.set("a.tau", 0.25).unwrap() {
        loaded.model.set_param(loaded.ids[&b], &p, v).unwrap();
    }
    loaded.params.set("slow", 1.0).unwrap();
    let ramp = loaded.ids["ramp"];
    loaded.model.set_param(ramp, "r0", 5.0).unwrap();
    let mut flat = loaded.flat.clone();
    flat.capture_params(&loaded.model, &loaded.ids).unwrap();
    let mut file = loaded.file.clone();
    file.capture_from(&flat, &loaded.params);
    assert_eq!(file.params["slow"].value, 1.0);
    let a = &file.blocks[1].params;
    assert_eq!(a.get("tau"), Some(&ParamValue::Number(0.25)));
    assert_eq!(a.len(), 1, "nothing else of a changed: {a:?}");
    let b = &file.blocks[2].params;
    assert_eq!(b.get("tau"), Some(&ParamValue::Formula("2*slow".into())));
    assert_eq!(b.len(), 1, "{b:?}");
    assert_eq!(file.blocks[0].params["r0"], ParamValue::Number(5.0));
    // And it loads to the same values.
    let again = load(&file.to_json(), &registry()).unwrap();
    assert_eq!(again.model.param(again.ids["a/f"], "T").unwrap(), 0.25);
    assert_eq!(again.model.param(again.ids["b/f"], "T").unwrap(), 2.0);
}
