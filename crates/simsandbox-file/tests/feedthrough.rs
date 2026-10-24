//! Direct feedthrough and algebraic loops (ADR 0040), through model files
//! with typed equations: outputs agree with their inputs at every
//! communication point, fused groups carry no lag, and loops are found,
//! named and solved as `run.loops` says.

use simsandbox_core::graph::BlockId;
use simsandbox_core::registry::Registry;
use simsandbox_core::sim::Simulator;
use simsandbox_core::SimError;
use simsandbox_file::{load, FileError};

fn registry() -> Registry {
    let mut r = Registry::new();
    simsandbox_eq::register(&mut r).unwrap();
    r
}

/// A model of typed blocks `(name, method, equations)`, connections
/// `(from, to)` and extra top-level JSON.
fn model(blocks: &[(&str, &str, &str)], connections: &[(&str, &str)], extra: &str) -> String {
    let blocks: Vec<String> = blocks
        .iter()
        .map(|(name, method, eqs)| {
            let lines: Vec<String> = eqs.lines().map(|l| format!("{l:?}")).collect();
            format!(
                r#"{{"name": "{name}", "type": "eq.equations", "method": {{"name": "{method}"}}, "equations": [{}]}}"#,
                lines.join(", ")
            )
        })
        .collect();
    let connections: Vec<String> = connections
        .iter()
        .map(|(f, t)| format!(r#"{{"from": "{f}", "to": "{t}"}}"#))
        .collect();
    format!(
        r#"{{"format": 1, {extra} "blocks": [{}], "connections": [{}]}}"#,
        blocks.join(", "),
        connections.join(", ")
    )
}

fn start(text: &str) -> Result<(Simulator, std::collections::BTreeMap<String, BlockId>), String> {
    let loaded = load(text, &registry()).map_err(|e| e.to_string())?;
    let sim = Simulator::new(loaded.model, 0.0).map_err(|e| e.to_string())?;
    Ok((sim, loaded.ids))
}

/// The error starting a model, which must fail.
fn fails(text: &str) -> String {
    match start(text) {
        Ok(_) => panic!("expected an error"),
        Err(e) => e,
    }
}

fn out(sim: &Simulator, ids: &std::collections::BTreeMap<String, BlockId>, key: &str) -> f64 {
    let (b, p) = key.split_once('.').unwrap();
    sim.output(ids[b], p).unwrap()[0]
}

#[test]
fn feedthrough_outputs_agree_with_their_inputs_at_every_step() {
    // A clock and a gain: the gain's output is 3 × the clock now, not a
    // step late.
    let text = model(
        &[
            ("clock", "rk4", "dx/dt = 1\nx0 = 0"),
            ("gain", "evaluate", "input s\ny = 3*s"),
        ],
        &[("clock.x", "gain.s")],
        r#""run": {"dt": 0.1},"#,
    );
    let text = text.replace("\"x0 = 0\"", "\"\"");
    let (mut sim, ids) = start(&text).unwrap();
    assert!(sim.loops().is_empty());
    for _ in 0..20 {
        assert_eq!(out(&sim, &ids, "gain.y"), 3.0 * out(&sim, &ids, "clock.x"));
        sim.step(0.1).unwrap();
    }
}

/// x(t) for the plant dx/dt = −x + u under u = kp (1 − x), from x = 0.
fn closed_loop(kp: f64, t: f64) -> f64 {
    kp / (1.0 + kp) * (1.0 - (-(1.0 + kp) * t).exp())
}

#[test]
fn a_fused_controller_and_plant_follow_the_closed_loop_exactly() {
    let blocks = [
        ("plant", "rk4", "input u\ndx/dt = -x + u"),
        ("control", "rk4", "input y\ndI/dt = 1 - y\nu = kp*(1 - y)"),
    ];
    let wires = [("plant.x", "control.y"), ("control.u", "plant.u")];
    let run = |extra: &str, dt: f64| {
        let text = model(&blocks, &wires, extra);
        let mut loaded = load(&text, &registry()).unwrap();
        let ids = loaded.ids.clone();
        loaded.model.set_param(ids["plant"], "x0", 0.0).unwrap();
        loaded.model.set_param(ids["control"], "kp", 4.0).unwrap();
        let fused = loaded.groups.len();
        let mut sim = Simulator::new(loaded.model, 0.0).unwrap();
        // During the rise, before it settles.
        for _ in 0..(0.3 / dt).round() as usize {
            sim.step(dt).unwrap();
        }
        let err = (out(&sim, &ids, "plant.x") - closed_loop(4.0, 0.3)).abs();
        (fused, err)
    };
    // Fused (the default): the controller's feedthrough is resolved inside
    // every evaluation, so the error is RK4's own: halving the step divides
    // it by about 16.
    let (groups, a) = run("", 0.02);
    let (_, b) = run("", 0.01);
    assert_eq!(groups, 1);
    assert!(b < 1e-7 && (12.0..20.0).contains(&(a / b)), "fused {a} {b}");
    // Co-simulated, the plant holds u over each step: first order, much
    // larger.
    let off = r#""fusion": "off","#;
    let (groups, c) = run(off, 0.02);
    let (_, d) = run(off, 0.01);
    assert_eq!(groups, 0);
    assert!(
        d > 1e3 * b && (1.6..2.4).contains(&(c / d)),
        "co-simulated {c} {d}"
    );
}

/// Two formula blocks in a loop: a = gain·fb + c, b = 1 − x, a → x, b → fb.
fn loop_model(a: &str, extra: &str) -> String {
    model(
        &[
            ("A", "evaluate", &format!("input fb\na = {a}")),
            ("B", "evaluate", "input x\nb = 1 - x"),
        ],
        &[("A.a", "B.x"), ("B.b", "A.fb")],
        extra,
    )
}

#[test]
fn loops_are_named_and_solved_by_newton_by_default() {
    let (mut sim, ids) = start(&loop_model("0.5*fb + 1", "")).unwrap();
    let loops = sim.loops();
    assert_eq!(loops.len(), 1);
    let mut names = loops[0].clone();
    names.sort();
    assert_eq!(names, ["A.a", "B.b"]);
    // a = 0.5 (1 − a) + 1, so a = 1 and b = 0, from the start and after
    // every step.
    for _ in 0..3 {
        assert!((out(&sim, &ids, "A.a") - 1.0).abs() < 1e-12);
        assert!(out(&sim, &ids, "B.b").abs() < 1e-12);
        sim.step(0.1).unwrap();
    }
    // Nonlinear: a = cos(1 − (1 − a)) = cos(a): the Dottie number.
    let (sim, ids) = start(&loop_model("cos(1 - fb)", "")).unwrap();
    assert!((out(&sim, &ids, "A.a") - 0.739_085_133_215_160_6).abs() < 1e-10);
}

#[test]
fn the_loop_method_is_part_of_the_model() {
    // Fixed point settles when the loop gain is below one...
    let fixed = r#""run": {"loops": "fixed_point"},"#;
    let (sim, ids) = start(&loop_model("0.5*fb + 1", fixed)).unwrap();
    assert!((out(&sim, &ids, "A.a") - 1.0).abs() < 1e-9);
    // ...and fails, naming the loop, when it is not; Newton solves it.
    let err = fails(&loop_model("2*fb + 1", fixed));
    assert!(err.contains("A.a") && err.contains("fixed-point"), "{err}");
    let (sim, ids) = start(&loop_model("2*fb + 1", "")).unwrap();
    assert!((out(&sim, &ids, "A.a") - 1.0).abs() < 1e-10);
    // Refuse: the model does not run, and the error names the loop.
    let err = fails(&loop_model("0.5*fb + 1", r#""run": {"loops": "refuse"},"#));
    assert!(err.contains("loop") && err.contains("B.b"), "{err}");
    // Unknown methods are load errors at their path.
    let bad = loop_model("fb", r#""run": {"loops": "magic"},"#);
    assert!(matches!(
        load(&bad, &registry()),
        Err(FileError::Invalid { path, .. }) if path == "run.loops"
    ));
}

#[test]
fn loops_inside_fused_groups_are_solved_at_every_evaluation() {
    let text = model(
        &[
            ("A", "rk4", "input fb\ndz/dt = -z\na = 0.5*fb + z"),
            ("B", "rk4", "input x\ndw/dt = 0\nb = 1 - x"),
        ],
        &[("A.a", "B.x"), ("B.b", "A.fb")],
        "",
    );
    let loaded = load(&text, &registry()).unwrap();
    assert_eq!(loaded.groups.len(), 1, "the two blocks fuse");
    let ids = loaded.ids.clone();
    let mut sim = Simulator::new(loaded.model, 0.0).unwrap();
    assert_eq!(sim.loops().len(), 1);
    for _ in 0..10 {
        sim.step(0.1).unwrap();
        let z = out(&sim, &ids, "A.z");
        // a = 0.5 (1 − a) + z, to the solver's tolerance.
        assert!((out(&sim, &ids, "A.a") - (0.5 + z) / 1.5).abs() < 1e-10);
    }
    assert!((out(&sim, &ids, "A.z") - (-1.0f64).exp()).abs() < 1e-6);
}

#[test]
fn typed_inputs_are_ports_and_only_quantities_using_them_feed_through() {
    let set = simsandbox_eq::EquationSet::parse(
        "m",
        &[
            "input u, F".into(),
            "dx/dt = -x + u".into(),
            "e = x^2".into(),
            "p = F*e".into(),
            "q = e + 1".into(),
        ],
    )
    .unwrap();
    let spec = simsandbox_eq::OdeSystem::spec(&set);
    let inputs: Vec<&str> = spec.inputs().map(|p| p.name.as_str()).collect();
    assert_eq!(inputs, ["u", "F"]);
    let ft = |name: &str| {
        spec.outputs()
            .find(|p| p.name == name)
            .unwrap()
            .feedthrough
            .clone()
    };
    assert!(ft("x").is_empty());
    assert!(ft("e").is_empty());
    assert_eq!(ft("p"), ["F"]);
    assert!(ft("q").is_empty());
    assert!(spec.params.iter().all(|p| p.name != "u" && p.name != "F"));
    // Inputs cannot clash with other names.
    assert!(
        simsandbox_eq::EquationSet::parse("m", &["input x".into(), "dx/dt = 1".into()]).is_err()
    );
    assert!(matches!(
        simsandbox_eq::EquationSet::parse("m", &["input 2u".into(), "y = 1".into()]),
        Err(SimError::InvalidDefinition(_))
    ));
}
