//! Methods for the equation domain, each with speed and accuracy labels.

use simsandbox_core::block::{AccuracyClass, MethodLabels, MethodSpec, SpeedClass};
use simsandbox_core::{Result, SimError};

/// Right-hand side as seen by an integrator: `f(t, x, dx)`.
pub type Rhs<'a> = dyn Fn(f64, &[f64], &mut [f64]) + 'a;

/// An ODE integration method.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Integrator {
    /// Explicit Euler with `substeps` per communication step. First order.
    Euler { substeps: usize },
    /// Classic fourth-order Runge–Kutta, fixed step.
    Rk4 { substeps: usize },
    /// Dormand–Prince 5(4) with adaptive step-size control.
    Dopri5 { rtol: f64, atol: f64 },
}

impl Integrator {
    pub fn labels(&self) -> MethodLabels {
        match self {
            Integrator::Euler { .. } => MethodLabels {
                speed: SpeedClass::RealTime,
                accuracy: AccuracyClass::Qualitative,
            },
            Integrator::Rk4 { .. } => MethodLabels {
                speed: SpeedClass::RealTime,
                accuracy: AccuracyClass::Quantitative,
            },
            Integrator::Dopri5 { .. } => MethodLabels {
                speed: SpeedClass::Responsive,
                accuracy: AccuracyClass::Quantitative,
            },
        }
    }

    pub fn method_spec(&self) -> MethodSpec {
        let name = match self {
            Integrator::Euler { substeps } => format!("Explicit Euler ({substeps} substeps)"),
            Integrator::Rk4 { substeps } => format!("RK4 ({substeps} substeps)"),
            Integrator::Dopri5 { rtol, atol } => {
                format!("Dormand–Prince 5(4), rtol={rtol:e}, atol={atol:e}")
            }
        };
        MethodSpec {
            name,
            labels: self.labels(),
        }
    }

    /// Advance `x` from `t` to `t + dt` in place.
    pub fn advance(&self, f: &Rhs<'_>, t: f64, dt: f64, x: &mut [f64]) -> Result<()> {
        match *self {
            Integrator::Euler { substeps } => {
                let h = dt / substeps.max(1) as f64;
                let mut dx = vec![0.0; x.len()];
                for i in 0..substeps.max(1) {
                    f(t + i as f64 * h, x, &mut dx);
                    x.iter_mut().zip(&dx).for_each(|(xi, di)| *xi += h * di);
                }
                Ok(())
            }
            Integrator::Rk4 { substeps } => {
                let h = dt / substeps.max(1) as f64;
                for i in 0..substeps.max(1) {
                    rk4_step(f, t + i as f64 * h, h, x);
                }
                Ok(())
            }
            Integrator::Dopri5 { rtol, atol } => dopri5(f, t, dt, x, rtol, atol),
        }
    }
}

fn rk4_step(f: &Rhs<'_>, t: f64, h: f64, x: &mut [f64]) {
    let n = x.len();
    let (mut k1, mut k2, mut k3, mut k4) = (vec![0.0; n], vec![0.0; n], vec![0.0; n], vec![0.0; n]);
    let mut tmp = vec![0.0; n];
    f(t, x, &mut k1);
    axpy(&mut tmp, x, &[(0.5 * h, &k1)]);
    f(t + 0.5 * h, &tmp, &mut k2);
    axpy(&mut tmp, x, &[(0.5 * h, &k2)]);
    f(t + 0.5 * h, &tmp, &mut k3);
    axpy(&mut tmp, x, &[(h, &k3)]);
    f(t + h, &tmp, &mut k4);
    for i in 0..n {
        x[i] += h / 6.0 * (k1[i] + 2.0 * k2[i] + 2.0 * k3[i] + k4[i]);
    }
}

/// out = x + Σ c_j k_j
fn axpy(out: &mut [f64], x: &[f64], terms: &[(f64, &Vec<f64>)]) {
    for i in 0..x.len() {
        out[i] = x[i] + terms.iter().map(|(c, k)| c * k[i]).sum::<f64>();
    }
}

// Dormand–Prince 5(4) tableau.
const C: [f64; 7] = [0.0, 1.0 / 5.0, 3.0 / 10.0, 4.0 / 5.0, 8.0 / 9.0, 1.0, 1.0];
const A: [&[f64]; 7] = [
    &[],
    &[1.0 / 5.0],
    &[3.0 / 40.0, 9.0 / 40.0],
    &[44.0 / 45.0, -56.0 / 15.0, 32.0 / 9.0],
    &[
        19372.0 / 6561.0,
        -25360.0 / 2187.0,
        64448.0 / 6561.0,
        -212.0 / 729.0,
    ],
    &[
        9017.0 / 3168.0,
        -355.0 / 33.0,
        46732.0 / 5247.0,
        49.0 / 176.0,
        -5103.0 / 18656.0,
    ],
    &[
        35.0 / 384.0,
        0.0,
        500.0 / 1113.0,
        125.0 / 192.0,
        -2187.0 / 6784.0,
        11.0 / 84.0,
    ],
];
/// Difference between the 5th- and 4th-order weights (error estimate).
const E: [f64; 7] = [
    71.0 / 57600.0,
    0.0,
    -71.0 / 16695.0,
    71.0 / 1920.0,
    -17253.0 / 339200.0,
    22.0 / 525.0,
    -1.0 / 40.0,
];

fn dopri5(f: &Rhs<'_>, t0: f64, dt: f64, x: &mut [f64], rtol: f64, atol: f64) -> Result<()> {
    let n = x.len();
    let t_end = t0 + dt;
    let mut t = t0;
    let mut h = dt;
    let mut k: Vec<Vec<f64>> = vec![vec![0.0; n]; 7];
    let mut stage = vec![0.0; n];
    let mut x_new = vec![0.0; n];
    let mut rejected = 0usize;

    while t < t_end - 1e-14 * dt.abs().max(1.0) {
        h = h.min(t_end - t);
        f(t, x, &mut k[0]);
        for s in 1..7 {
            for i in 0..n {
                stage[i] = x[i]
                    + h * A[s]
                        .iter()
                        .enumerate()
                        .map(|(j, a)| a * k[j][i])
                        .sum::<f64>();
            }
            let (_, rest) = k.split_at_mut(s);
            f(t + C[s] * h, &stage, &mut rest[0]);
        }
        // The 7th stage is evaluated at the 5th-order solution (FSAL).
        x_new.copy_from_slice(&stage);

        let mut err = 0.0f64;
        for i in 0..n {
            let e = h * (0..7).map(|j| E[j] * k[j][i]).sum::<f64>();
            let scale = atol + rtol * x[i].abs().max(x_new[i].abs());
            err += (e / scale).powi(2);
        }
        let err = (err / n.max(1) as f64).sqrt();

        if err <= 1.0 {
            t += h;
            x.copy_from_slice(&x_new);
        } else {
            rejected += 1;
            if rejected > 10_000 || h < 1e-14 * dt.abs().max(1.0) {
                return Err(SimError::Block("Dopri5: step size underflow".into()));
            }
        }
        let factor = if err == 0.0 {
            5.0
        } else {
            0.9 * err.powf(-0.2)
        };
        h *= factor.clamp(0.2, 5.0);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// x' = -x, x(0) = 1  =>  x(t) = exp(-t)
    fn decay(_t: f64, x: &[f64], dx: &mut [f64]) {
        dx[0] = -x[0];
    }

    fn error(m: Integrator, steps: usize) -> f64 {
        let mut x = [1.0];
        let dt = 1.0 / steps as f64;
        for i in 0..steps {
            m.advance(&decay, i as f64 * dt, dt, &mut x).unwrap();
        }
        (x[0] - (-1.0f64).exp()).abs()
    }

    #[test]
    fn euler_is_first_order() {
        let ratio = error(Integrator::Euler { substeps: 1 }, 100)
            / error(Integrator::Euler { substeps: 1 }, 200);
        assert!((1.8..2.2).contains(&ratio), "ratio {ratio}");
    }

    #[test]
    fn rk4_is_fourth_order() {
        let ratio =
            error(Integrator::Rk4 { substeps: 1 }, 20) / error(Integrator::Rk4 { substeps: 1 }, 40);
        assert!((14.0..18.0).contains(&ratio), "ratio {ratio}");
    }

    #[test]
    fn dopri5_meets_tolerance() {
        let e = error(
            Integrator::Dopri5 {
                rtol: 1e-10,
                atol: 1e-12,
            },
            1,
        );
        assert!(e < 1e-8, "error {e}");
    }
}
