//! How fast typed equations run against the same system built in (Q35,
//! ADR 0060). Run with
//! `cargo run --release -p simsandbox-eq --example typed_speed`.

use simsandbox_core::graph::Model;
use simsandbox_core::sim::Simulator;
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
    let cases: [(&str, Make<'_>, Make<'_>); 2] = [
        (
            "Lorenz",
            &|k| OdeBlock::new(Lorenz::new(&format!("b{k}")), rk4()),
            &|k| typed(&format!("b{k}"), LORENZ),
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
}
