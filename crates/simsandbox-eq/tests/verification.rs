//! Verification against analytic solutions. These are the first entries of
//! the benchmark suite that every method must pass.

use simsandbox_core::block::{AccuracyClass, SpeedClass};
use simsandbox_core::graph::Model;
use simsandbox_core::sim::Simulator;
use simsandbox_core::units::Unit;
use simsandbox_core::SimError;
use simsandbox_eq::systems::{CoupledOscillator, Lorenz};
use simsandbox_eq::{EquationGroup, Integrator, OdeBlock};

const T_END: f64 = 10.0;
const DT: f64 = 0.01;

/// Exact x1(t) for two coupled oscillators with ω0² = 1, κ = 0.5,
/// x1(0) = 1, x2(0) = 0, both at rest.
fn exact_x1(t: f64) -> f64 {
    let w2 = (1.0f64 + 2.0 * 0.5).sqrt();
    0.5 * (t.cos() + (w2 * t).cos())
}

fn run(sim: &mut Simulator, id: simsandbox_core::graph::BlockId, port: &str) -> f64 {
    let steps = (T_END / DT).round() as usize;
    let mut max_err = 0.0f64;
    for _ in 0..steps {
        sim.step(DT).unwrap();
        let x1 = sim.output(id, port).unwrap()[0];
        max_err = max_err.max((x1 - exact_x1(sim.time())).abs());
    }
    max_err
}

fn fused_error(integrator: Integrator) -> f64 {
    let mut g = EquationGroup::new("pair");
    let a = g.add(CoupledOscillator::new("a", 1.0));
    let b = g.add(CoupledOscillator::new("b", 0.0));
    g.connect((a, "x"), (b, "neighbor_x")).unwrap();
    g.connect((b, "x"), (a, "neighbor_x")).unwrap();
    let mut model = Model::new();
    let id = model.add(Box::new(OdeBlock::new(g.build().unwrap(), integrator)));
    let mut sim = Simulator::new(model, 0.0).unwrap();
    run(&mut sim, id, "a.x")
}

fn cosim_error() -> f64 {
    let rk4 = Integrator::Rk4 { substeps: 4 };
    let mut model = Model::new();
    let a = model.add(Box::new(OdeBlock::new(
        CoupledOscillator::new("a", 1.0),
        rk4,
    )));
    let b = model.add(Box::new(OdeBlock::new(
        CoupledOscillator::new("b", 0.0),
        rk4,
    )));
    model.connect((a, "x"), (b, "neighbor_x")).unwrap();
    model.connect((b, "x"), (a, "neighbor_x")).unwrap();
    let mut sim = Simulator::new(model, 0.0).unwrap();
    run(&mut sim, a, "x")
}

#[test]
fn fused_rk4_matches_exact_solution() {
    let e = fused_error(Integrator::Rk4 { substeps: 1 });
    assert!(e < 1e-7, "fused RK4 error {e}");
}

#[test]
fn fused_dopri5_matches_exact_solution() {
    let e = fused_error(Integrator::Dopri5 {
        rtol: 1e-10,
        atol: 1e-12,
    });
    assert!(e < 1e-8, "fused Dopri5 error {e}");
}

/// The reason for the layered contract: the same equations split across two
/// co-simulated Blocks pick up coupling error that fusion avoids.
#[test]
fn cosimulation_adds_coupling_error() {
    let fused = fused_error(Integrator::Rk4 { substeps: 1 });
    let cosim = cosim_error();
    assert!(cosim > 100.0 * fused, "co-sim {cosim} vs fused {fused}");
    assert!(
        cosim < 0.05,
        "co-sim should still be roughly right, got {cosim}"
    );
}

#[test]
fn rejects_unit_mismatch() {
    let mut model = Model::new();
    let osc = model.add(Box::new(OdeBlock::new(
        CoupledOscillator::new("a", 1.0),
        Integrator::Rk4 { substeps: 1 },
    )));
    let other = model.add(Box::new(OdeBlock::new(
        CoupledOscillator::new("b", 0.0),
        Integrator::Rk4 { substeps: 1 },
    )));
    // velocity [m/s] into a position input [m]
    let err = model
        .connect((osc, "v"), (other, "neighbor_x"))
        .unwrap_err();
    assert!(matches!(err, SimError::UnitMismatch { .. }));
    assert!(Unit::parse("m/s")
        .unwrap()
        .conversion_to(&Unit::METER)
        .is_none());
}

#[test]
fn rejects_unconnected_input() {
    let mut model = Model::new();
    model.add(Box::new(OdeBlock::new(
        CoupledOscillator::new("a", 1.0),
        Integrator::Rk4 { substeps: 1 },
    )));
    assert!(matches!(
        Simulator::new(model, 0.0),
        Err(SimError::UnconnectedInput(_))
    ));
}

#[test]
fn labels_follow_slowest_and_least_accurate_method() {
    let mut model = Model::new();
    model.add(Box::new(OdeBlock::new(
        Lorenz::new("fast"),
        Integrator::Euler { substeps: 1 },
    )));
    model.add(Box::new(OdeBlock::new(
        Lorenz::new("careful"),
        Integrator::Dopri5 {
            rtol: 1e-8,
            atol: 1e-10,
        },
    )));
    let labels = Simulator::new(model, 0.0).unwrap().labels().unwrap();
    assert_eq!(labels.speed, SpeedClass::Responsive);
    assert_eq!(labels.accuracy, AccuracyClass::Qualitative);
}

#[test]
fn snapshot_and_rewind_reproduce_the_run() {
    let mut model = Model::new();
    let id = model.add(Box::new(OdeBlock::new(
        Lorenz::new("l"),
        Integrator::Rk4 { substeps: 1 },
    )));
    let mut sim = Simulator::new(model, 0.0).unwrap();
    for _ in 0..100 {
        sim.step(DT).unwrap();
    }
    let snap = sim.snapshot();
    for _ in 0..100 {
        sim.step(DT).unwrap();
    }
    let first = sim.state(id).to_vec();
    sim.restore(&snap);
    for _ in 0..100 {
        sim.step(DT).unwrap();
    }
    assert_eq!(first, sim.state(id));
}
