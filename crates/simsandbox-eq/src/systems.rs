//! Built-in example systems.

use crate::ode::OdeSystem;
use simsandbox_core::block::{
    BlockSpec, Domain, Equations, ParamSpec, PortSpec, Shape, TimeSemantics,
};
use simsandbox_core::units::Unit;

/// A parameter with a suggested `[min, max]` slider range.
fn param(name: &str, unit: Unit, default: f64, min: f64, max: f64, description: &str) -> ParamSpec {
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
fn initial(
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
