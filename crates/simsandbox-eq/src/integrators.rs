//! Methods for the equation domain, each with speed and accuracy labels.

use crate::implicit::{Scheme, Workspace};
use simsandbox_core::block::{AccuracyClass, MethodLabels, MethodSpec, SpeedClass};
use simsandbox_core::registry::MethodSettings;
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
    /// Backward (implicit) Euler, `substeps` per communication step.
    /// First order, L-stable: for stiff systems (ADR 0060).
    BackwardEuler { substeps: usize },
    /// TR-BDF2, `substeps` per communication step. Second order,
    /// L-stable: for stiff systems such as diffusion on fine grids.
    TrBdf2 { substeps: usize },
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
            // A fixed amount of work per step (one factorisation), so
            // real-time for systems of moderate size; first order is
            // qualitative, as for explicit Euler.
            Integrator::BackwardEuler { .. } => MethodLabels {
                speed: SpeedClass::RealTime,
                accuracy: AccuracyClass::Qualitative,
            },
            Integrator::TrBdf2 { .. } => MethodLabels {
                speed: SpeedClass::RealTime,
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
            Integrator::BackwardEuler { substeps } => {
                format!("Backward Euler ({substeps} substeps)")
            }
            Integrator::TrBdf2 { substeps } => format!("TR-BDF2 ({substeps} substeps)"),
        };
        MethodSpec {
            name,
            labels: self.labels(),
        }
    }

    /// Advance `x` from `t` to `t + dt` in place.
    /// Build from file settings: `euler`, `rk4`, `backward_euler` or
    /// `trbdf2` with optional `substeps` (default 1), or `dopri5` with
    /// optional `rtol` (default 1e-8) and `atol` (default 1e-10).
    pub fn from_settings(m: &MethodSettings) -> Result<Self> {
        let allowed: &[&str] = match m.name.as_str() {
            "euler" | "rk4" | "backward_euler" | "trbdf2" => &["substeps"],
            "dopri5" => &["rtol", "atol"],
            other => {
                return Err(SimError::InvalidMethod(format!(
                "unknown method '{other}' (expected euler, rk4, dopri5, backward_euler or trbdf2)"
            )))
            }
        };
        if let Some(k) = m.options.keys().find(|k| !allowed.contains(&k.as_str())) {
            return Err(SimError::InvalidMethod(format!(
                "'{}' has no option '{k}'",
                m.name
            )));
        }
        let substeps = || -> Result<usize> {
            match m.options.get("substeps") {
                None => Ok(1),
                Some(&n) if n >= 1.0 && n.fract() == 0.0 && n <= 1e6 => Ok(n as usize),
                Some(n) => Err(SimError::InvalidMethod(format!(
                    "substeps must be a whole number from 1 to 1e6, got {n}"
                ))),
            }
        };
        let tol = |key: &str, default: f64| -> Result<f64> {
            match m.options.get(key) {
                None => Ok(default),
                Some(&v) if v.is_finite() && v > 0.0 => Ok(v),
                Some(v) => Err(SimError::InvalidMethod(format!(
                    "{key} must be positive, got {v}"
                ))),
            }
        };
        Ok(match m.name.as_str() {
            "euler" => Integrator::Euler {
                substeps: substeps()?,
            },
            "rk4" => Integrator::Rk4 {
                substeps: substeps()?,
            },
            "backward_euler" => Integrator::BackwardEuler {
                substeps: substeps()?,
            },
            "trbdf2" => Integrator::TrBdf2 {
                substeps: substeps()?,
            },
            _ => Integrator::Dopri5 {
                rtol: tol("rtol", 1e-8)?,
                atol: tol("atol", 1e-10)?,
            },
        })
    }

    /// Settings that rebuild this integrator with [`Integrator::from_settings`].
    pub fn settings(&self) -> MethodSettings {
        match *self {
            Integrator::Euler { substeps } => {
                MethodSettings::new("euler").with("substeps", substeps as f64)
            }
            Integrator::Rk4 { substeps } => {
                MethodSettings::new("rk4").with("substeps", substeps as f64)
            }
            Integrator::Dopri5 { rtol, atol } => MethodSettings::new("dopri5")
                .with("rtol", rtol)
                .with("atol", atol),
            Integrator::BackwardEuler { substeps } => {
                MethodSettings::new("backward_euler").with("substeps", substeps as f64)
            }
            Integrator::TrBdf2 { substeps } => {
                MethodSettings::new("trbdf2").with("substeps", substeps as f64)
            }
        }
    }

    /// Advance `x` from `t` to `t + dt` in place. Implicit methods find
    /// the Jacobian's structure afresh each call; see
    /// [`Integrator::advance_in`] to keep it.
    pub fn advance(&self, f: &Rhs<'_>, t: f64, dt: f64, x: &mut [f64]) -> Result<()> {
        self.advance_in(f, t, dt, x, &mut Workspace::default(), None)
    }

    /// As [`Integrator::advance`], keeping what implicit methods learn of
    /// the system's structure in `ws`. `structure`, if given, tells it
    /// (for each rate, the states it reads) instead of probing.
    pub fn advance_in(
        &self,
        f: &Rhs<'_>,
        t: f64,
        dt: f64,
        x: &mut [f64],
        ws: &mut Workspace,
        structure: Option<&dyn Fn() -> Option<Vec<Vec<usize>>>>,
    ) -> Result<()> {
        match *self {
            Integrator::BackwardEuler { substeps } => crate::implicit::advance(
                (Scheme::BackwardEuler, substeps),
                f,
                t,
                dt,
                x,
                ws,
                structure,
            ),
            Integrator::TrBdf2 { substeps } => {
                crate::implicit::advance((Scheme::TrBdf2, substeps), f, t, dt, x, ws, structure)
            }
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

/// An [`Integrator`] advancing a fused group (ADR 0023).
pub struct GroupIntegrator {
    integrator: Integrator,
    method: MethodSpec,
    /// What an implicit method learns of the group's structure.
    workspace: Workspace,
}

impl GroupIntegrator {
    pub fn new(integrator: Integrator) -> Self {
        Self {
            method: integrator.method_spec(),
            integrator,
            workspace: Workspace::default(),
        }
    }
}

impl simsandbox_core::fusion::GroupSolver for GroupIntegrator {
    fn method(&self) -> &MethodSpec {
        &self.method
    }

    fn advance(
        &mut self,
        rhs: &simsandbox_core::fusion::GroupRhs<'_>,
        t: f64,
        dt: f64,
        x: &mut [f64],
    ) -> Result<()> {
        self.integrator
            .advance_in(rhs, t, dt, x, &mut self.workspace, None)
    }
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

    #[test]
    fn backward_euler_is_first_order_and_trbdf2_second() {
        let be = |n| error(Integrator::BackwardEuler { substeps: 1 }, n);
        let ratio = be(100) / be(200);
        assert!((1.8..2.2).contains(&ratio), "backward Euler ratio {ratio}");
        let tr = |n| error(Integrator::TrBdf2 { substeps: 1 }, n);
        let ratio = tr(20) / tr(40);
        assert!((3.6..4.4).contains(&ratio), "TR-BDF2 ratio {ratio}");
        // Substeps are steps: 2 substeps of 1/20 match 40 steps of 1/40.
        let sub = error(Integrator::TrBdf2 { substeps: 2 }, 20);
        assert!((sub - tr(40)).abs() < 1e-12 * tr(40).max(1e-30) + 1e-15);
    }

    #[test]
    fn implicit_methods_take_stiff_steps_explicit_ones_cannot() {
        // x' = -1000 (x - cos t): x follows cos t closely; steps of 0.1
        // are 50 times the explicit limit (about 0.0028 for RK4).
        let stiff = |t: f64, x: &[f64], dx: &mut [f64]| dx[0] = -1000.0 * (x[0] - t.cos());
        let run = |m: Integrator| {
            let mut x = [0.0];
            for i in 0..20 {
                m.advance(&stiff, i as f64 * 0.1, 0.1, &mut x).unwrap();
            }
            x[0]
        };
        let exact = 2f64.cos();
        for m in [
            Integrator::BackwardEuler { substeps: 1 },
            Integrator::TrBdf2 { substeps: 1 },
        ] {
            let x = run(m);
            assert!((x - exact).abs() < 2e-3, "{m:?}: {x} vs {exact}");
        }
        let blown = run(Integrator::Rk4 { substeps: 1 });
        assert!(
            !blown.is_finite() || blown.abs() > 1e10,
            "RK4 stayed bounded: {blown}"
        );
        // A system whose Newton iteration fails is an error, not a guess.
        let wild = |_t: f64, x: &[f64], dx: &mut [f64]| dx[0] = x[0].exp() * 1e6;
        let mut x = [1.0];
        let r = Integrator::BackwardEuler { substeps: 1 }.advance(&wild, 0.0, 1.0, &mut x);
        assert!(r.is_err());
    }

    #[test]
    fn implicit_methods_are_read_and_written_as_settings() {
        for name in ["backward_euler", "trbdf2"] {
            let m = MethodSettings::new(name).with("substeps", 4.0);
            let i = Integrator::from_settings(&m).unwrap();
            assert_eq!(i.settings(), m);
            assert!(
                Integrator::from_settings(&MethodSettings::new(name).with("rtol", 1.0)).is_err()
            );
        }
    }
}
