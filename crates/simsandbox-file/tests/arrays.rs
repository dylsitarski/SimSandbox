//! Arrays in typed equations (ADR 0060), through model files: array
//! ports connect between typed blocks and to built-in grid blocks, fuse,
//! and are checked for shape and unit.

use simsandbox_core::registry::Registry;
use simsandbox_core::sim::Simulator;
use simsandbox_file::load;

fn registry() -> Registry {
    let mut r = Registry::new();
    simsandbox_eq::register(&mut r).unwrap();
    r
}

fn typed(name: &str, lines: &[&str]) -> String {
    let lines: Vec<String> = lines.iter().map(|l| format!("{l:?}")).collect();
    format!(
        r#"{{"name": "{name}", "type": "eq.equations", "equations": [{}]}}"#,
        lines.join(", ")
    )
}

fn model(blocks: &[String], connections: &[(&str, &str)]) -> String {
    let connections: Vec<String> = connections
        .iter()
        .map(|(f, t)| format!(r#"{{"from": "{f}", "to": "{t}"}}"#))
        .collect();
    format!(
        r#"{{"format": 1, "blocks": [{}], "connections": [{}]}}"#,
        blocks.join(", "),
        connections.join(", ")
    )
}

/// A heat source on a 1 m grid of 50 cells, peaked at the middle.
const SOURCE: &[&str] = &[
    "grid x [m] = 0 .. 1, 50",
    "param power [K/s] = 2",
    "param c [m] = 0.5",
    "param w [m] = 0.1",
    "heat = power*exp(-((x - c)/w)^2)",
];

#[test]
fn array_ports_connect_fuse_and_run() {
    // A heat source feeds a typed rod; the rod's temperature feeds a
    // typed probe on the same grid, which lags it.
    let rod = typed(
        "rod",
        &[
            "grid x [m] = 0 .. 1, 50",
            "input q[x] [K/s]",
            "param alpha [m^2/s] = 0.01",
            "state T[x] [K] = 0",
            "edge T: x- = 0, x+ = 0",
            "dT/dt = alpha*lap(T) + q",
        ],
    );
    let probe = typed(
        "probe",
        &[
            "grid x [m] = 0 .. 1, 50",
            "input T[x] [K]",
            "param tau [s] = 0.1",
            "state S[x] [K] = 0",
            "dS/dt = (T - S)/tau",
            "hottest = maxof(T) [K]",
        ],
    );
    let text = model(
        &[typed("source", SOURCE), rod, probe],
        &[("source.heat", "rod.q"), ("rod.T", "probe.T")],
    );
    let loaded = load(&text, &registry()).expect("loads");
    // All three fuse: two have states, and the source's formulas join them.
    assert_eq!(loaded.groups.len(), 1, "{:?}", loaded.groups);
    assert_eq!(loaded.groups[0].blocks.len(), 3);
    let ids = loaded.ids.clone();
    let mut sim = Simulator::new(loaded.model, 0.0).unwrap();
    for _ in 0..100 {
        sim.step(0.01).unwrap();
    }
    let t = sim.output(ids["rod"], "T").unwrap().to_vec();
    let hottest = sim.output(ids["probe"], "hottest").unwrap()[0];
    assert_eq!(hottest, t.iter().copied().fold(f64::MIN, f64::max));
    // On an infinite rod a Gaussian source P·exp(-(x/w)^2) heats the
    // middle by ∫ P·w/√(w² + 4αs) ds over 0..t: with P = 2, w = 0.1,
    // α = 0.01 and t = 1 that is 0.2·50·(√0.05 − √0.01) = 1.236 K. The
    // ends, 5 widths away, barely matter; 50 cells resolve it to ~0.3%.
    let exact = 0.2 * 50.0 * (0.05f64.sqrt() - 0.01f64.sqrt());
    assert!(
        (hottest - exact).abs() < 0.005 * exact,
        "{hottest} vs {exact}"
    );
}

#[test]
fn array_ports_need_the_same_shape_and_unit() {
    let rod = |cells: usize, unit: &str| {
        typed(
            "rod",
            &[
                &format!("grid x [m] = 0 .. 1, {cells}"),
                &format!("input q[x] [{unit}]"),
                "state T[x] [K] = 0",
                "dT/dt = q",
            ],
        )
    };
    let source = typed("source", SOURCE);
    let short = model(
        &[source.clone(), rod(40, "K/s")],
        &[("source.heat", "rod.q")],
    );
    let err = load(&short, &registry()).err().expect("shapes differ");
    assert!(err.to_string().contains("shape"), "{err}");
    let wrong = model(&[source, rod(50, "K/h")], &[("source.heat", "rod.q")]);
    let err = load(&wrong, &registry()).err().expect("units differ");
    assert!(err.to_string().contains("unit"), "{err}");
}

#[test]
fn a_typed_rod_reads_the_built_in_rods_temperature() {
    let built_in =
        r#"{"name": "hot", "type": "eq.heat_rod", "method": {"name": "rk4", "substeps": 1.0}}"#;
    let probe = typed(
        "probe",
        &[
            "grid x [m] = 0 .. 1, 50",
            "input T[x] [K]",
            "average = mean(T) [K]",
        ],
    );
    let text = model(
        &[built_in.to_string(), probe],
        &[("hot.temperature", "probe.T")],
    );
    let loaded = load(&text, &registry()).expect("loads");
    let ids = loaded.ids.clone();
    let sim = Simulator::new(loaded.model, 0.0).unwrap();
    let t = sim.output(ids["hot"], "temperature").unwrap();
    let mean = t.iter().sum::<f64>() / t.len() as f64;
    assert!((sim.output(ids["probe"], "average").unwrap()[0] - mean).abs() < 1e-12);
}
