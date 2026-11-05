//! Parameters driven by wires (ADR 0075): checked against an exact
//! solution, with their feedthrough, loops and starting values.

use simsandbox_core::graph::{BlockId, Model};
use simsandbox_core::registry::{BlockConfig, MethodSettings, Registry};
use simsandbox_core::sim::Simulator;
use simsandbox_eq::integrators::GroupIntegrator;
use simsandbox_eq::Integrator;

fn registry() -> Registry {
    let mut r = Registry::new();
    simsandbox_eq::registry::register(&mut r).unwrap();
    r
}

/// A typed-equations block, by its lines, with RK4 for one with states.
fn typed(model: &mut Model, name: &str, lines: &[&str], rk4: bool) -> BlockId {
    let config = BlockConfig {
        definition: lines.iter().map(|l| l.to_string()).collect(),
        method: if rk4 {
            MethodSettings::new("rk4").with("substeps", 1.0)
        } else {
            MethodSettings::default()
        },
        ..Default::default()
    };
    model.add(registry().build("eq.equations", name, &config).unwrap())
}

/// `dx/dt = -k·x` with `k = a·t` driven by a clock: `x = x0·exp(-a·t²/2)`.
fn decay(fused: bool, dt: f64) -> f64 {
    let a = 0.5;
    let mut m = Model::new();
    let clock = typed(
        &mut m,
        "clock",
        &[
            "param a [1/s^2] = 0.5",
            "state tau [s] = 0",
            "dtau/dt = 1",
            "k = a*tau [1/s]",
        ],
        true,
    );
    let decay = typed(
        &mut m,
        "decay",
        &["param k [1/s] = 100", "state x = 1", "dx/dt = -k*x"],
        true,
    );
    m.drive((clock, "k"), (decay, "k")).unwrap();
    if fused {
        m.fuse(
            &[clock, decay],
            Box::new(GroupIntegrator::new(Integrator::Rk4 { substeps: 1 })),
        )
        .unwrap();
    }
    let mut sim = Simulator::new(m, 0.0).unwrap();
    let mut err = 0.0f64;
    for _ in 0..(2.0 / dt).round() as usize {
        sim.step(dt).unwrap();
        let t = sim.time();
        let x = sim.output(decay, "x").unwrap()[0];
        err = err.max((x - (-a * t * t / 2.0).exp()).abs());
    }
    err
}

#[test]
fn a_driven_parameter_follows_its_signal_exactly_when_fused() {
    // Fused, `k` is read at every right-hand-side evaluation: RK4's order.
    let (e1, e2) = (decay(true, 0.02), decay(true, 0.01));
    assert!(e1 < 1e-7, "{e1}");
    assert!(e1 / e2 > 12.0, "fourth order: {e1} / {e2}");
    // Exchanged once per step, it is held over the step, as an input
    // from outside would be: first order.
    let (h1, h2) = (decay(false, 0.02), decay(false, 0.01));
    assert!(h1 < 0.02, "{h1}");
    let ratio = h1 / h2;
    assert!((1.7..2.3).contains(&ratio), "first order: {h1} / {h2}");
}

#[test]
fn driven_parameters_feed_through_only_where_outputs_read_them() {
    // y reads c at the same instant: driving c by y is a loop, solved.
    let mut m = Model::new();
    let p = typed(&mut m, "p", &["param c = 1", "y = 0.5*c + 1"], false);
    m.drive((p, "y"), (p, "c")).unwrap();
    let sim = Simulator::new(m, 0.0).unwrap();
    assert_eq!(sim.loops(), vec![vec!["p.y".to_string()]]);
    assert!((sim.output(p, "y").unwrap()[0] - 2.0).abs() < 1e-9);
    // An output reading other parameters and the state does not read it
    // at the same instant: no loop.
    let mut m = Model::new();
    let s = typed(
        &mut m,
        "s",
        &[
            "param c [1/s] = 1",
            "param rate [1/s] = 1",
            "state x = 1",
            "dx/dt = -c*x",
            "r = x*rate",
        ],
        true,
    );
    m.drive((s, "r"), (s, "c")).unwrap();
    let sim = Simulator::new(m, 0.0).unwrap();
    assert!(sim.loops().is_empty(), "{:?}", sim.loops());
    // Refused under run.loops 'refuse'.
    let mut m = Model::new();
    let p = typed(&mut m, "p", &["param c = 1", "y = 0.5*c + 1"], false);
    m.drive((p, "y"), (p, "c")).unwrap();
    m.set_loop_method(simsandbox_core::loops::LoopMethod::Refuse);
    assert!(Simulator::new(m, 0.0).is_err());
}

#[test]
fn a_driven_parameter_is_not_set_by_hand_and_is_checked_as_a_wire() {
    let mut m = Model::new();
    let src = typed(&mut m, "src", &["param a [m] = 3", "y = 2*a"], false);
    let dst = typed(&mut m, "dst", &["param g [m] = 1", "z = g*1"], false);
    let other = typed(&mut m, "other", &["param h [s] = 1", "w = h*1"], false);
    // Units must match, and a parameter is driven once.
    assert!(m.drive((src, "y"), (other, "h")).is_err());
    m.drive((src, "y"), (dst, "g")).unwrap();
    assert!(m.drive((src, "y"), (dst, "g")).is_err());
    assert!(m.drive((src, "y"), (dst, "nope")).is_err());
    let mut sim = Simulator::new(m, 0.0).unwrap();
    assert_eq!(sim.output(dst, "z").unwrap()[0], 6.0);
    assert!(sim.set_param(dst, "g", 4.0).is_err());
    // The driver's change reaches it in the next step.
    sim.set_param(src, "a", 5.0).unwrap();
    sim.step(0.1).unwrap();
    assert_eq!(sim.output(dst, "z").unwrap()[0], 10.0);
    // Rewound, it is as it was.
    let snap = sim.snapshot();
    sim.set_param(src, "a", 7.0).unwrap();
    sim.step(0.1).unwrap();
    assert_eq!(sim.output(dst, "z").unwrap()[0], 14.0);
    sim.restore(&snap);
    assert_eq!(sim.output(dst, "z").unwrap()[0], 10.0);
    assert_eq!(sim.param(dst, "g").unwrap(), 10.0);
}

#[test]
fn driven_starting_values_are_read_when_the_block_starts() {
    let mut m = Model::new();
    let src = typed(&mut m, "src", &["param a = 3", "y = 2*a"], false);
    let mid = typed(
        &mut m,
        "mid",
        &["state u = 0", "du/dt = 0", "w = u + 1"],
        true,
    );
    let end = typed(&mut m, "end", &["state v = 0", "dv/dt = 0"], true);
    // A chain: end starts from mid, which starts from src.
    m.drive((mid, "w"), (end, "v0")).unwrap();
    m.drive((src, "y"), (mid, "u0")).unwrap();
    let mut sim = Simulator::new(m, 0.0).unwrap();
    assert_eq!(sim.output(mid, "u").unwrap()[0], 6.0);
    assert_eq!(sim.output(end, "v").unwrap()[0], 7.0);
    // While running, a starting value does not move the state...
    sim.set_param(src, "a", 5.0).unwrap();
    sim.step(0.1).unwrap();
    assert_eq!(sim.output(end, "v").unwrap()[0], 7.0);
    // ...until the run starts again.
    sim.restart(0.0).unwrap();
    assert_eq!(sim.output(end, "v").unwrap()[0], 11.0);
}

#[test]
fn driven_starting_values_in_a_cycle_are_refused() {
    let mut m = Model::new();
    let a = typed(&mut m, "a", &["state v = 0", "dv/dt = 0"], true);
    m.drive((a, "v"), (a, "v0")).unwrap();
    let e = Simulator::new(m, 0.0).err().expect("refused");
    assert!(e.to_string().contains("cycle"), "{e}");
}
