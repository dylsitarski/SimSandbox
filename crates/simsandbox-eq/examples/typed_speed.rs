//! How fast typed equations run against the same system built in (Q35,
//! ADR 0060). Run with
//! `cargo run --release -p simsandbox-eq --example typed_speed`.

use simsandbox_core::graph::Model;
use simsandbox_core::sim::Simulator;
use simsandbox_eq::fields::HeatRod;
use simsandbox_eq::integrators::Integrator;
use simsandbox_eq::ode::OdeBlock;
use simsandbox_eq::symbolic::EquationSet;
use simsandbox_eq::systems::{Lorenz, Pendulum};
use std::time::Instant;

const LORENZ: &[&str] = &[
    "dx/dt = sigma*(y - x)",
    "dy/dt = x*(rho - z) - y",
    "dz/dt = x*y - beta*z",
];

const PENDULUM: &[&str] = &[
    "dtheta/dt = omega",
    "domega/dt = -g/L*sin(theta) - c*omega",
    "x = L*sin(theta)",
    "y = -L*cos(theta)",
];

/// Makes the `k`-th block of a case.
type Make<'a> = &'a dyn Fn(usize) -> OdeBlock;

/// The heat rod of `eq.heat_rod` as typed equations on a grid (ADR 0060).
const HEAT_ROD: &[&str] = &[
    "grid x [m] = 0 .. length, 50",
    "param length [m] = 1",
    "param alpha [m^2/s] = 0.01",
    "param peak [K] = 100",
    "param width [m] = 0.1",
    "param left [K] = 0",
    "param right [K] = 0",
    "state T[x] [K] = peak*exp(-((x - length/2)/width)^2)",
    "edge T: x- = left, x+ = right",
    "dT/dt = alpha*lap(T)",
];

fn typed(name: &str, lines: &[&str]) -> OdeBlock {
    let lines: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    OdeBlock::new(
        EquationSet::parse(name, &lines).expect("valid equations"),
        Integrator::Rk4 { substeps: 1 },
    )
}

/// Microseconds per step of `blocks` copies, the best of three runs.
fn time(make: Make<'_>, blocks: usize) -> f64 {
    const STEPS: usize = 200_000;
    let mut best = f64::INFINITY;
    for _ in 0..3 {
        let mut m = Model::new();
        for k in 0..blocks {
            m.add(Box::new(make(k)));
        }
        let mut s = Simulator::new(m, 0.0).expect("model builds");
        let start = Instant::now();
        for _ in 0..STEPS {
            s.step(0.001).expect("steps");
        }
        best = best.min(start.elapsed().as_secs_f64() * 1e6 / STEPS as f64);
    }
    best
}

fn main() {
    let rk4 = || Integrator::Rk4 { substeps: 1 };
    let cases: [(&str, Make<'_>, Make<'_>); 3] = [
        (
            "Lorenz",
            &|k| OdeBlock::new(Lorenz::new(&format!("b{k}")), rk4()),
            &|k| typed(&format!("b{k}"), LORENZ),
        ),
        (
            "heat rod",
            &|k| OdeBlock::new(HeatRod::new(&format!("b{k}")), rk4()),
            &|k| typed(&format!("b{k}"), HEAT_ROD),
        ),
        (
            "pendulum",
            &|k| OdeBlock::new(Pendulum::new(&format!("b{k}")), rk4()),
            &|k| typed(&format!("b{k}"), PENDULUM),
        ),
    ];
    println!("RK4, dt = 0.001, µs per step (best of 3 × 200 000 steps)");
    for (name, built_in, typed) in cases {
        for n in [1, 10] {
            let (b, t) = (time(built_in, n), time(typed, n));
            println!(
                "{name:9} × {n:2}: built in {b:6.3}  typed {t:6.3}  ratio {:.2}",
                t / b
            );
        }
    }
    stiff();
}

/// Stiff steps (ADR 0060): the typed rod at 0.05 s a step, by RK4 with
/// the substeps it needs to stay stable, and by TR-BDF2 in one step.
fn stiff() {
    println!("\nStiff: typed rod, 0.05 s steps to t = 2 s, ms per step (best of 3)");
    for cells in [50usize, 200, 800] {
        let lines: Vec<String> = HEAT_ROD
            .iter()
            .map(|l| l.replace(", 50", &format!(", {cells}")))
            .collect();
        let h = 1.0 / cells as f64;
        // RK4 is stable below about 0.7 h²/α; keep a margin.
        let substeps = (0.05 / (0.5 * h * h / 0.01)).ceil() as usize;
        let time = |integrator: Integrator| {
            let mut best = f64::INFINITY;
            for _ in 0..3 {
                let mut m = Model::new();
                let set = EquationSet::parse("rod", &lines).expect("valid equations");
                m.add(Box::new(OdeBlock::new(set, integrator)));
                let mut s = Simulator::new(m, 0.0).expect("model builds");
                let start = Instant::now();
                for _ in 0..40 {
                    s.step(0.05).expect("steps");
                }
                best = best.min(start.elapsed().as_secs_f64() * 1e3 / 40.0);
            }
            best
        };
        let rk4 = time(Integrator::Rk4 { substeps });
        let tr = time(Integrator::TrBdf2 { substeps: 1 });
        println!(
            "{cells:4} cells: RK4 × {substeps:5} substeps {rk4:8.3}  TR-BDF2 × 1 {tr:7.3}  ratio {:.1}",
            rk4 / tr
        );
    }
}
