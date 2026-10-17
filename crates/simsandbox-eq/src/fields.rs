//! Built-in systems whose outputs are fields (ADR 0016): a grid of values
//! and a set of points.

use crate::ode::OdeSystem;
use crate::systems::{initial, param};
use simsandbox_core::block::{BlockSpec, Domain, Equations, PortSpec, Shape, TimeSemantics};
use simsandbox_core::units::Unit;

/// Heat flowing along a 1 m rod, ∂T/∂t = α ∂²T/∂x², with the ends held at
/// fixed temperatures. The rod is cut into [`HeatRod::CELLS`] equal cells
/// (second-order central differences, the method of lines); the output is
/// the cell temperatures as a one-row grid.
///
/// Explicit methods are only stable while the step stays below about
/// h²/(2α) (Euler) or 0.7 h²/α (RK4), where h is the cell width; use
/// substeps for larger α.
pub struct HeatRod {
    spec: BlockSpec,
}

impl HeatRod {
    pub const CELLS: usize = 50;

    pub fn new(name: &str) -> Self {
        let n = Self::CELLS;
        let k = Unit::KELVIN;
        Self {
            spec: BlockSpec {
                name: name.into(),
                domain: Domain::Equations,
                ports: vec![PortSpec::output(
                    "temperature",
                    k,
                    Shape::Grid { nx: n, ny: 1 },
                )],
                params: vec![
                    param(
                        "alpha",
                        Unit::parse("m^2/s").expect("valid unit"),
                        0.01,
                        0.0,
                        0.05,
                        "Thermal diffusivity",
                    ),
                    param(
                        "left",
                        k,
                        0.0,
                        -100.0,
                        100.0,
                        "Temperature held at the left end",
                    ),
                    param(
                        "right",
                        k,
                        0.0,
                        -100.0,
                        100.0,
                        "Temperature held at the right end",
                    ),
                    initial(
                        "peak",
                        k,
                        100.0,
                        -100.0,
                        100.0,
                        "Initial hot spot at the middle, above the line between the ends",
                    ),
                    initial(
                        "width",
                        Unit::METER,
                        0.1,
                        0.01,
                        0.5,
                        "Initial hot spot width",
                    ),
                ],
                states: (0..n).map(|i| format!("T{i}")).collect(),
                time: TimeSemantics::Continuous,
            },
        }
    }

    /// Cell width: the cells are the interior points of n + 1 equal
    /// intervals, with the held ends at x = 0 and x = 1.
    fn h() -> f64 {
        1.0 / (Self::CELLS + 1) as f64
    }
}

impl Equations for HeatRod {
    fn rhs(&self, _t: f64, x: &[f64], _u: &[f64], p: &[f64], dx: &mut [f64]) {
        let (alpha, left, right) = (p[0], p[1], p[2]);
        let c = alpha / (Self::h() * Self::h());
        let n = x.len();
        for i in 0..n {
            let l = if i == 0 { left } else { x[i - 1] };
            let r = if i + 1 == n { right } else { x[i + 1] };
            dx[i] = c * (l - 2.0 * x[i] + r);
        }
    }

    fn outputs(&self, _t: f64, x: &[f64], _p: &[f64], y: &mut [f64]) {
        y.copy_from_slice(x);
    }
}

impl OdeSystem for HeatRod {
    fn spec(&self) -> &BlockSpec {
        &self.spec
    }

    fn initial_state(&self, p: &[f64], x0: &mut [f64]) {
        let (left, right, peak, width) = (p[1], p[2], p[3], p[4]);
        for (i, v) in x0.iter_mut().enumerate() {
            let x = (i + 1) as f64 * Self::h();
            let bump = ((x - 0.5) / width).powi(2);
            *v = left + (right - left) * x + peak * (-bump).exp();
        }
    }
}

/// Test particles orbiting a central mass in a plane: each feels only the
/// centre, r'' = −GM r / (|r|² + ε²)^(3/2), with softening ε. They start
/// spread over a disc between `r_min` and `r_max`, moving at `speed` times
/// the circular-orbit speed; below 1 the orbits are ellipses that precess
/// at different rates, winding the disc into spirals.
///
/// The output is the positions as a point set, `[x0, y0, x1, y1, …]`.
pub struct Orbits {
    spec: BlockSpec,
}

impl Orbits {
    pub const PARTICLES: usize = 200;

    pub fn new(name: &str) -> Self {
        let one = Unit::DIMENSIONLESS;
        Self {
            spec: BlockSpec {
                name: name.into(),
                domain: Domain::Equations,
                ports: vec![PortSpec::output(
                    "positions",
                    one,
                    Shape::Points {
                        count: Self::PARTICLES,
                        dim: 2,
                    },
                )],
                params: vec![
                    param("GM", one, 1.0, 0.0, 5.0, "Central mass times G"),
                    param(
                        "eps",
                        one,
                        0.05,
                        0.0,
                        0.5,
                        "Softening length: keeps close passes finite",
                    ),
                    initial("r_min", one, 0.5, 0.05, 2.0, "Inner edge of the disc"),
                    initial("r_max", one, 2.0, 0.1, 5.0, "Outer edge of the disc"),
                    initial(
                        "speed",
                        one,
                        0.8,
                        0.0,
                        1.5,
                        "Starting speed as a fraction of the circular-orbit speed",
                    ),
                ],
                states: (0..Self::PARTICLES)
                    .flat_map(|i| {
                        [
                            format!("x{i}"),
                            format!("y{i}"),
                            format!("vx{i}"),
                            format!("vy{i}"),
                        ]
                    })
                    .collect(),
                time: TimeSemantics::Continuous,
            },
        }
    }
}

/// r² + ε², raised to −3/2.
fn inverse_cube(r2: f64, eps: f64) -> f64 {
    (r2 + eps * eps).powf(-1.5)
}

impl Equations for Orbits {
    fn rhs(&self, _t: f64, x: &[f64], _u: &[f64], p: &[f64], dx: &mut [f64]) {
        let (gm, eps) = (p[0], p[1]);
        for (s, d) in x.chunks_exact(4).zip(dx.chunks_exact_mut(4)) {
            let a = -gm * inverse_cube(s[0] * s[0] + s[1] * s[1], eps);
            d[0] = s[2];
            d[1] = s[3];
            d[2] = a * s[0];
            d[3] = a * s[1];
        }
    }

    fn outputs(&self, _t: f64, x: &[f64], _p: &[f64], y: &mut [f64]) {
        for (s, out) in x.chunks_exact(4).zip(y.chunks_exact_mut(2)) {
            out.copy_from_slice(&s[..2]);
        }
    }
}

impl OdeSystem for Orbits {
    fn spec(&self) -> &BlockSpec {
        &self.spec
    }

    fn initial_state(&self, p: &[f64], x0: &mut [f64]) {
        let (gm, eps, r_min, r_max, speed) = (p[0], p[1], p[2], p[3], p[4]);
        // The golden angle spreads the particles evenly without lining up.
        let golden = std::f64::consts::PI * (3.0 - 5f64.sqrt());
        let n = Self::PARTICLES as f64;
        for (i, s) in x0.chunks_exact_mut(4).enumerate() {
            let r = r_min + (r_max - r_min) * (i as f64 + 0.5) / n;
            let (sin, cos) = (i as f64 * golden).sin_cos();
            // Circular orbit: v²/r = GM r / (r² + ε²)^(3/2).
            let v = speed * (gm * r * r * inverse_cube(r * r, eps)).sqrt();
            s.copy_from_slice(&[r * cos, r * sin, -v * sin, v * cos]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Integrator, OdeBlock};
    use simsandbox_core::graph::{BlockId, Model};
    use simsandbox_core::sim::Simulator;

    /// Run one system; returns the simulator and the block's id.
    fn run(
        system: impl OdeSystem + 'static,
        params: &[(&str, f64)],
        dt: f64,
        steps: usize,
    ) -> (Simulator, BlockId) {
        let mut model = Model::new();
        let id = model.add(Box::new(OdeBlock::new(
            system,
            Integrator::Rk4 { substeps: 1 },
        )));
        let mut sim = Simulator::new(model, 0.0).unwrap();
        for &(name, v) in params {
            sim.set_param(id, name, v).unwrap();
        }
        sim.restart(0.0).unwrap();
        for _ in 0..steps {
            sim.step(dt).unwrap();
        }
        (sim, id)
    }

    #[test]
    fn a_sine_mode_of_the_rod_decays_at_its_discrete_rate() {
        // With both ends at 0, T_i = sin(π x_i) is an eigenvector of the
        // differenced equation, with rate α (4/h²) sin²(π h / 2).
        let rod = HeatRod::new("rod");
        let h = HeatRod::h();
        let alpha = 0.01;
        let p = [alpha, 0.0, 0.0, 0.0, 0.1];
        let x: Vec<f64> = (0..HeatRod::CELLS)
            .map(|i| (std::f64::consts::PI * (i + 1) as f64 * h).sin())
            .collect();
        let mut dx = vec![0.0; x.len()];
        rod.rhs(0.0, &x, &[], &p, &mut dx);
        let rate = alpha * 4.0 / (h * h) * (std::f64::consts::PI * h / 2.0).sin().powi(2);
        for (d, v) in dx.iter().zip(&x) {
            assert!((d + rate * v).abs() < 1e-12, "{d} vs {}", -rate * v);
        }
        // And it is close to the continuous rate α π².
        assert!((rate / (alpha * std::f64::consts::PI.powi(2)) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn the_rod_settles_to_a_straight_line_between_its_ends() {
        let (sim, id) = run(
            HeatRod::new("rod"),
            &[("alpha", 0.05), ("left", 20.0), ("right", 80.0)],
            0.002,
            50_000,
        );
        let t = sim.output(id, "temperature").unwrap();
        for (i, v) in t.iter().enumerate() {
            let x = (i + 1) as f64 * HeatRod::h();
            assert!((v - (20.0 + 60.0 * x)).abs() < 1e-6, "cell {i}: {v}");
        }
    }

    #[test]
    fn circular_orbits_keep_their_radius() {
        let dt = 0.002;
        let steps = 5000;
        let (start, id) = run(Orbits::new("o"), &[("speed", 1.0)], dt, 0);
        let (end, _) = run(Orbits::new("o"), &[("speed", 1.0)], dt, steps);
        let radius = |p: &[f64]| {
            p.chunks_exact(2)
                .map(|c| c[0].hypot(c[1]))
                .collect::<Vec<_>>()
        };
        let r0 = radius(start.output(id, "positions").unwrap());
        let r1 = radius(end.output(id, "positions").unwrap());
        assert_eq!(r0.len(), Orbits::PARTICLES);
        for (a, b) in r0.iter().zip(&r1) {
            assert!((a - b).abs() < 1e-6 * a, "{a} became {b}");
        }
        // The innermost particle has gone round more than once.
        let period = 2.0 * std::f64::consts::PI * r0[0]
            / (r0[0] * r0[0] * inverse_cube(r0[0] * r0[0], 0.05)).sqrt();
        assert!(dt * steps as f64 > period);
    }
}
