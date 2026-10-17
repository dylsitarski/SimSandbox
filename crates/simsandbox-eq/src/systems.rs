//! Built-in example systems.

use crate::ode::OdeSystem;
use simsandbox_core::block::{
    BlockSpec, Domain, Equations, ParamSpec, PortSpec, Shape, TimeSemantics,
};
use simsandbox_core::units::Unit;

/// A parameter with a suggested `[min, max]` slider range.
pub(crate) fn param(
    name: &str,
    unit: Unit,
    default: f64,
    min: f64,
    max: f64,
    description: &str,
) -> ParamSpec {
    ParamSpec {
        name: name.into(),
        unit,
        default,
        suggested_min: min,
        suggested_max: max,
        description: description.into(),
        initial_condition: false,
    }
}

/// A parameter that sets initial state.
pub(crate) fn initial(
    name: &str,
    unit: Unit,
    default: f64,
    min: f64,
    max: f64,
    description: &str,
) -> ParamSpec {
    ParamSpec {
        initial_condition: true,
        ..param(name, unit, default, min, max, description)
    }
}

/// The Lorenz system: dx/dt = σ(y − x), dy/dt = x(ρ − z) − y, dz/dt = xy − βz.
pub struct Lorenz {
    spec: BlockSpec,
}

impl Lorenz {
    pub fn new(name: &str) -> Self {
        let one = Unit::DIMENSIONLESS;
        Self {
            spec: BlockSpec {
                name: name.into(),
                domain: Domain::Equations,
                ports: vec![PortSpec::output("xyz", one, Shape::Vector(3))],
                params: vec![
                    param("sigma", one, 10.0, 0.0, 50.0, "Prandtl number"),
                    param("rho", one, 28.0, 0.0, 100.0, "Rayleigh number"),
                    param("beta", one, 8.0 / 3.0, 0.0, 10.0, "Geometric factor"),
                    initial("x0", one, 1.0, -30.0, 30.0, "Initial x"),
                    initial("y0", one, 1.0, -30.0, 30.0, "Initial y"),
                    initial("z0", one, 1.0, 0.0, 60.0, "Initial z"),
                ],
                states: vec!["x".into(), "y".into(), "z".into()],
                time: TimeSemantics::Continuous,
            },
        }
    }
}

impl Equations for Lorenz {
    fn rhs(&self, _t: f64, x: &[f64], _u: &[f64], p: &[f64], dx: &mut [f64]) {
        let (sigma, rho, beta) = (p[0], p[1], p[2]);
        dx[0] = sigma * (x[1] - x[0]);
        dx[1] = x[0] * (rho - x[2]) - x[1];
        dx[2] = x[0] * x[1] - beta * x[2];
    }

    fn outputs(&self, _t: f64, x: &[f64], _p: &[f64], y: &mut [f64]) {
        y.copy_from_slice(&x[..3]);
    }
}

impl OdeSystem for Lorenz {
    fn spec(&self) -> &BlockSpec {
        &self.spec
    }

    fn initial_state(&self, p: &[f64], x0: &mut [f64]) {
        x0.copy_from_slice(&p[3..6]);
    }
}

/// A unit-mass oscillator tied to a wall (ω0²) and to a neighbour (κ):
/// x'' = −ω0² x + κ (x_neighbour − x).
///
/// Two of these connected to each other have the exact solution, for
/// x1(0) = 1, x2(0) = 0, v(0) = 0:
/// x1(t) = (cos ω0 t + cos ω2 t) / 2, with ω2 = sqrt(ω0² + 2κ).
pub struct CoupledOscillator {
    spec: BlockSpec,
}

impl CoupledOscillator {
    pub fn new(name: &str, x0: f64) -> Self {
        let per_s2 = Unit::parse("1/s^2").expect("valid unit");
        Self {
            spec: BlockSpec {
                name: name.into(),
                domain: Domain::Equations,
                ports: vec![
                    PortSpec::input("neighbor_x", Unit::METER, Shape::Scalar),
                    PortSpec::output("x", Unit::METER, Shape::Scalar),
                    PortSpec::output("v", Unit::parse("m/s").expect("valid unit"), Shape::Scalar),
                ],
                params: vec![
                    param(
                        "omega0_sq",
                        per_s2,
                        1.0,
                        0.0,
                        100.0,
                        "Wall stiffness per unit mass",
                    ),
                    param(
                        "kappa",
                        per_s2,
                        0.5,
                        0.0,
                        100.0,
                        "Coupling stiffness per unit mass",
                    ),
                    initial("x0", Unit::METER, x0, -10.0, 10.0, "Initial position"),
                    initial(
                        "v0",
                        Unit::parse("m/s").expect("valid unit"),
                        0.0,
                        -10.0,
                        10.0,
                        "Initial velocity",
                    ),
                ],
                states: vec!["x".into(), "v".into()],
                time: TimeSemantics::Continuous,
            },
        }
    }
}

impl Equations for CoupledOscillator {
    fn rhs(&self, _t: f64, x: &[f64], u: &[f64], p: &[f64], dx: &mut [f64]) {
        let (w0sq, kappa) = (p[0], p[1]);
        dx[0] = x[1];
        dx[1] = -w0sq * x[0] + kappa * (u[0] - x[0]);
    }

    fn outputs(&self, _t: f64, x: &[f64], _p: &[f64], y: &mut [f64]) {
        y[0] = x[0];
        y[1] = x[1];
    }
}

impl OdeSystem for CoupledOscillator {
    fn spec(&self) -> &BlockSpec {
        &self.spec
    }

    fn initial_state(&self, p: &[f64], x0: &mut [f64]) {
        x0[0] = p[2];
        x0[1] = p[3];
    }
}

/// A pendulum: a bob on a rigid, massless rod of length L, swinging under
/// gravity with linear damping, θ'' = −(g/L) sin θ − c θ'. θ = 0 hangs
/// straight down.
///
/// Outputs the angle, the angular velocity and the bob's position with the
/// pivot at the origin: x = L sin θ, y = −L cos θ.
pub struct Pendulum {
    spec: BlockSpec,
}

impl Pendulum {
    pub fn new(name: &str) -> Self {
        let rad = Unit::DIMENSIONLESS;
        let per_s = Unit::parse("1/s").expect("valid unit");
        Self {
            spec: BlockSpec {
                name: name.into(),
                domain: Domain::Equations,
                ports: vec![
                    PortSpec::output("angle", rad, Shape::Scalar),
                    PortSpec::output("omega", per_s, Shape::Scalar),
                    PortSpec::output("x", Unit::METER, Shape::Scalar),
                    PortSpec::output("y", Unit::METER, Shape::Scalar),
                ],
                params: vec![
                    param(
                        "g",
                        Unit::parse("m/s^2").expect("valid unit"),
                        9.81,
                        0.0,
                        30.0,
                        "Gravitational acceleration",
                    ),
                    param("L", Unit::METER, 1.0, 0.1, 5.0, "Rod length"),
                    param("c", per_s, 0.0, 0.0, 2.0, "Damping rate"),
                    initial(
                        "theta0",
                        rad,
                        2.0,
                        -std::f64::consts::PI,
                        std::f64::consts::PI,
                        "Initial angle from straight down (rad)",
                    ),
                    initial(
                        "omega0",
                        per_s,
                        0.0,
                        -10.0,
                        10.0,
                        "Initial angular velocity",
                    ),
                ],
                states: vec!["theta".into(), "omega".into()],
                time: TimeSemantics::Continuous,
            },
        }
    }
}

impl Equations for Pendulum {
    fn rhs(&self, _t: f64, x: &[f64], _u: &[f64], p: &[f64], dx: &mut [f64]) {
        let (g, l, c) = (p[0], p[1], p[2]);
        dx[0] = x[1];
        dx[1] = -(g / l) * x[0].sin() - c * x[1];
    }

    fn outputs(&self, _t: f64, x: &[f64], p: &[f64], y: &mut [f64]) {
        let l = p[1];
        y[0] = x[0];
        y[1] = x[1];
        y[2] = l * x[0].sin();
        y[3] = -l * x[0].cos();
    }
}

impl OdeSystem for Pendulum {
    fn spec(&self) -> &BlockSpec {
        &self.spec
    }

    fn initial_state(&self, p: &[f64], x0: &mut [f64]) {
        x0[0] = p[3];
        x0[1] = p[4];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Integrator, OdeBlock};
    use simsandbox_core::graph::Model;
    use simsandbox_core::sim::Simulator;

    fn pendulum(theta0: f64, c: f64) -> (Simulator, simsandbox_core::graph::BlockId) {
        let mut model = Model::new();
        let id = model.add(Box::new(OdeBlock::new(
            Pendulum::new("p"),
            Integrator::Rk4 { substeps: 1 },
        )));
        let mut sim = Simulator::new(model, 0.0).unwrap();
        sim.set_param(id, "theta0", theta0).unwrap();
        sim.set_param(id, "c", c).unwrap();
        sim.restart(0.0).unwrap();
        (sim, id)
    }

    fn out(sim: &Simulator, id: simsandbox_core::graph::BlockId, port: &str) -> f64 {
        sim.output(id, port).unwrap()[0]
    }

    #[test]
    fn small_swings_have_the_period_2_pi_sqrt_l_over_g() {
        let (mut sim, id) = pendulum(0.01, 0.0);
        let period = 2.0 * std::f64::consts::PI * (1.0f64 / 9.81).sqrt();
        // Time the first upward crossing of θ = 0 after one full swing from
        // the right: θ goes 0.01 → 0 (T/4) → −0.01 → 0 (3T/4).
        let dt = 1e-4;
        let mut last = out(&sim, id, "angle");
        let mut crossings = Vec::new();
        while crossings.len() < 2 {
            sim.step(dt).unwrap();
            let now = out(&sim, id, "angle");
            if last.signum() != now.signum() {
                // Interpolate the crossing within the step.
                crossings.push(sim.time() - dt * now / (now - last));
            }
            last = now;
        }
        let half = crossings[1] - crossings[0];
        // The finite amplitude lengthens the period by about θ0²/16.
        assert!((2.0 * half / period - 1.0).abs() < 1e-5, "{half}");
    }

    #[test]
    fn without_damping_the_energy_stays_constant() {
        let (mut sim, id) = pendulum(2.0, 0.0);
        let energy = |sim: &Simulator| {
            let w = out(sim, id, "omega");
            0.5 * w * w + 9.81 * out(sim, id, "y")
        };
        let e0 = energy(&sim);
        for _ in 0..10_000 {
            sim.step(0.001).unwrap();
            assert!((energy(&sim) - e0).abs() < 1e-8 * e0.abs());
        }
        let (x, y) = (out(&sim, id, "x"), out(&sim, id, "y"));
        assert!((x.hypot(y) - 1.0).abs() < 1e-12, "the bob stays on the rod");
    }

    #[test]
    fn damping_brings_it_to_rest() {
        let (mut sim, id) = pendulum(1.0, 1.0);
        for _ in 0..3000 {
            sim.step(0.01).unwrap();
        }
        assert!(out(&sim, id, "angle").abs() < 1e-5);
    }
}
