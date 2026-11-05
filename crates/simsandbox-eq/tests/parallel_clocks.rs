//! Clocks stepping at one instant run on several cores (ADR 0066 step 6)
//! with the same bits as one after another.

use simsandbox_core::graph::{BlockId, Clock, Model};
use simsandbox_core::parallel::Parallel;
use simsandbox_core::registry::{BlockConfig, MethodSettings, Registry};
use simsandbox_core::sim::Simulator;
use simsandbox_eq::integrators::GroupIntegrator;
use simsandbox_eq::Integrator;

fn typed(model: &mut Model, name: &str, lines: &[&str]) -> BlockId {
    let mut r = Registry::new();
    simsandbox_eq::registry::register(&mut r).unwrap();
    let config = BlockConfig {
        definition: lines.iter().map(|l| l.to_string()).collect(),
        method: MethodSettings::new("rk4").with("substeps", 2.0),
        ..Default::default()
    };
    model.add(r.build("eq.equations", name, &config).unwrap())
}

/// Two clocks (every 0.1 s and every 0.3 s), each with a fused pair and a
/// block of its own, linked both ways; with a seeded random force.
fn model() -> (Model, Vec<BlockId>) {
    let mut m = Model::new();
    let oscillator = |m: &mut Model, n: &str, k: f64| {
        typed(
            m,
            n,
            &[
                "input u",
                &format!("param k = {k}"),
                "state x = 1",
                "state v = 0",
                "dx/dt = v",
                "dv/dt = -k*x + u",
            ],
        )
    };
    let a1 = oscillator(&mut m, "a1", 1.0);
    let a2 = oscillator(&mut m, "a2", 2.0);
    let b1 = oscillator(&mut m, "b1", 3.0);
    let b2 = oscillator(&mut m, "b2", 0.5);
    let fa = typed(&mut m, "fa", &["input x", "state s = 0", "ds/dt = x - s"]);
    let fb = typed(&mut m, "fb", &["input x", "state s = 0", "ds/dt = x - s"]);
    m.connect((a1, "x"), (a2, "u")).unwrap();
    m.connect((a2, "x"), (a1, "u")).unwrap();
    m.connect((b1, "x"), (b2, "u")).unwrap();
    // Across clocks, held between their instants.
    m.connect((a1, "x"), (b1, "u")).unwrap();
    m.connect((b2, "x"), (fa, "x")).unwrap();
    m.connect((a2, "x"), (fb, "x")).unwrap();
    let solver = || Box::new(GroupIntegrator::new(Integrator::Rk4 { substeps: 2 }));
    m.fuse(&[a1, a2], solver()).unwrap();
    m.fuse(&[b1, b2], solver()).unwrap();
    let clock = |dt: f64, every: u128| Clock {
        dt,
        every,
        on_data: false,
        times: Vec::new(),
    };
    m.set_clocks(vec![clock(0.1, 1), clock(0.3, 3)], vec![0, 0, 1, 1, 0, 1])
        .unwrap();
    (m, vec![a1, a2, b1, b2, fa, fb])
}

fn run(parallel: Parallel) -> Vec<u64> {
    let (m, ids) = model();
    let mut sim = Simulator::new(m, 0.0).unwrap();
    sim.set_parallel(parallel);
    let mut bits = Vec::new();
    for _ in 0..300 {
        sim.step(0.0).unwrap();
        for &id in &ids {
            bits.extend(sim.state(id).iter().map(|v| v.to_bits()));
        }
    }
    bits
}

#[test]
fn clocks_on_several_cores_give_the_same_bits() {
    let apart = run(Parallel::Never);
    assert_eq!(run(Parallel::Always), apart);
    assert_eq!(run(Parallel::Auto), apart);
}
