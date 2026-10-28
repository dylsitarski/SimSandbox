//! Implicit one-step methods for stiff systems (ADR 0060, Q17): backward
//! Euler and TR-BDF2. Both are L-stable and keep no history between
//! steps, so snapshots, rewind and restarts work as for explicit methods.
//!
//! Each step solves its equations by Newton's method with a matrix
//! `I − c·h·J`. The Jacobian `J` is found by finite differences at the
//! start of every communication step: columns that share no row are
//! perturbed together (a colouring of the Jacobian's structure), so a
//! grid needs a handful of evaluations, not one per cell. The matrix is
//! factorised by a sparse LU (`faer`).
//!
//! Nothing numeric is carried from one step to the next: a step's result
//! depends only on its start, so a run replays bit for bit. Only the
//! structure is kept ([`Workspace`]): given by the system (typed
//! equations know it exactly), or else found once by perturbing each
//! state alone.

use crate::integrators::Rhs;
use faer::linalg::solvers::Solve;
use faer::sparse::linalg::solvers::{Lu, SymbolicLu};
use faer::sparse::{SparseColMat, SymbolicSparseColMat, Triplet};
use simsandbox_core::{Result, SimError};

/// Which implicit method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    /// First order: `x₁ = x₀ + h·f(x₁)`.
    BackwardEuler,
    /// Second order: a trapezoidal stage to `t + γh`, then BDF2 to
    /// `t + h`, with γ = 2 − √2 so both stages share one matrix.
    TrBdf2,
}

/// Newton's iterations per stage before giving up.
const MAX_ITERATIONS: usize = 50;
/// A stage has converged when the last correction is this small,
/// relative to the state (and absolutely, near 0).
const TOLERANCE: f64 = 1e-10;

/// The Jacobian's structure and how it is coloured and ordered; kept
/// between steps, because it never changes the numbers a step gives.
#[derive(Default)]
pub struct Workspace {
    structure: Option<Structure>,
}

struct Structure {
    /// For each column, the rows it appears in (the diagonal included).
    rows_of: Vec<Vec<usize>>,
    /// Columns perturbed together.
    colours: Vec<Vec<usize>>,
    /// The matrix's pattern with its fill-reducing ordering.
    symbolic: SymbolicLu<usize>,
    pattern: SymbolicSparseColMat<usize>,
}

impl Workspace {
    /// Forget the structure (after the system changes).
    pub fn reset(&mut self) {
        self.structure = None;
    }
}

impl Structure {
    /// From each row's columns (`deps[i]`: the states rate `i` reads).
    fn new(n: usize, deps: &[Vec<usize>]) -> Result<Self> {
        let mut rows_of: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (i, cols) in deps.iter().enumerate() {
            for &j in cols {
                rows_of[j].push(i);
            }
        }
        for (j, rows) in rows_of.iter_mut().enumerate() {
            rows.push(j);
            rows.sort_unstable();
            rows.dedup();
        }
        // Greedy colouring: a column takes the first colour none of whose
        // columns shares a row with it.
        let mut colours: Vec<Vec<usize>> = Vec::new();
        let mut used: Vec<Vec<bool>> = Vec::new();
        for (j, rows) in rows_of.iter().enumerate() {
            let c = (0..colours.len())
                .find(|&c| rows.iter().all(|&i| !used[c][i]))
                .unwrap_or_else(|| {
                    colours.push(Vec::new());
                    used.push(vec![false; n]);
                    colours.len() - 1
                });
            colours[c].push(j);
            for &i in rows {
                used[c][i] = true;
            }
        }
        let entries: Vec<Triplet<usize, usize, f64>> = rows_of
            .iter()
            .enumerate()
            .flat_map(|(j, rows)| rows.iter().map(move |&i| Triplet::new(i, j, 1.0)))
            .collect();
        let fail = |e: String| SimError::Block(format!("implicit method: {e}"));
        let matrix = SparseColMat::<usize, f64>::try_new_from_triplets(n, n, &entries)
            .map_err(|e| fail(format!("{e:?}")))?;
        let pattern = matrix
            .symbolic()
            .to_owned()
            .map_err(|e| fail(format!("{e:?}")))?;
        let symbolic = SymbolicLu::try_new(pattern.as_ref()).map_err(|e| fail(format!("{e:?}")))?;
        Ok(Self {
            rows_of,
            colours,
            symbolic,
            pattern,
        })
    }

    /// The structure of `f` found by perturbing each state alone at `x`:
    /// a rate that changes depends on that state.
    fn probe(f: &Rhs<'_>, t: f64, x: &[f64]) -> Vec<Vec<usize>> {
        let n = x.len();
        let mut f0 = vec![0.0; n];
        f(t, x, &mut f0);
        let mut deps: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut xp = x.to_vec();
        let mut f1 = vec![0.0; n];
        for j in 0..n {
            let delta = 1e-3 * x[j].abs().max(1.0);
            xp[j] = x[j] + delta;
            f(t, &xp, &mut f1);
            xp[j] = x[j];
            for i in 0..n {
                if f1[i] != f0[i] {
                    deps[i].push(j);
                }
            }
        }
        deps
    }
}

/// The matrix `I − c·h·J` factorised, `J` by finite differences at
/// `(t, x)` where the rates are `f0`.
fn factorise(
    s: &Structure,
    f: &Rhs<'_>,
    t: f64,
    x: &[f64],
    f0: &[f64],
    ch: f64,
) -> Result<Lu<usize, f64>> {
    let n = x.len();
    // Values in the pattern's order: column by column, rows ascending.
    let mut values = Vec::with_capacity(s.rows_of.iter().map(Vec::len).sum());
    let mut xp = x.to_vec();
    let mut f1 = vec![0.0; n];
    let mut jac: Vec<Vec<f64>> = s.rows_of.iter().map(|r| vec![0.0; r.len()]).collect();
    for colour in &s.colours {
        let deltas: Vec<f64> = colour
            .iter()
            .map(|&j| 1.5e-8 * x[j].abs().max(1.0))
            .collect();
        for (&j, d) in colour.iter().zip(&deltas) {
            xp[j] = x[j] + d;
        }
        f(t, &xp, &mut f1);
        for (&j, d) in colour.iter().zip(&deltas) {
            xp[j] = x[j];
            // xp[j] + d − x[j] is the step actually taken, in floating point.
            let taken = (x[j] + d) - x[j];
            for (k, &i) in s.rows_of[j].iter().enumerate() {
                jac[j][k] = (f1[i] - f0[i]) / taken;
            }
        }
    }
    for (j, rows) in s.rows_of.iter().enumerate() {
        for (k, &i) in rows.iter().enumerate() {
            let identity = if i == j { 1.0 } else { 0.0 };
            values.push(identity - ch * jac[j][k]);
        }
    }
    let matrix = SparseColMat::<usize, f64>::new(s.pattern.clone(), values);
    Lu::try_new_with_symbolic(s.symbolic.clone(), matrix.as_ref())
        .map_err(|e| SimError::Block(format!("implicit method: the matrix is singular ({e:?})")))
}

/// Solve `y − c·h·f(t, y) = r` for `y` by Newton's method from `y`.
fn stage(
    f: &Rhs<'_>,
    lu: &Lu<usize, f64>,
    t: f64,
    ch: f64,
    r: &[f64],
    y: &mut [f64],
) -> Result<()> {
    let n = y.len();
    let mut fy = vec![0.0; n];
    let mut g = faer::Mat::<f64>::zeros(n, 1);
    for _ in 0..MAX_ITERATIONS {
        f(t, y, &mut fy);
        for i in 0..n {
            g[(i, 0)] = y[i] - ch * fy[i] - r[i];
        }
        lu.solve_in_place(g.as_mut());
        let mut size = 0.0f64;
        let mut scale = 1.0f64;
        for i in 0..n {
            y[i] -= g[(i, 0)];
            size = size.max(g[(i, 0)].abs());
            scale = scale.max(y[i].abs());
        }
        if !size.is_finite() {
            break;
        }
        if size <= TOLERANCE * scale {
            return Ok(());
        }
    }
    Err(SimError::Block(
        "an implicit step did not converge (Newton's method): take a shorter step or more substeps"
            .into(),
    ))
}

/// Advance `x` from `t` to `t + dt` in `substeps` steps of `scheme`.
pub fn advance(
    (scheme, substeps): (Scheme, usize),
    f: &Rhs<'_>,
    t: f64,
    dt: f64,
    x: &mut [f64],
    ws: &mut Workspace,
    structure: Option<&dyn Fn() -> Option<Vec<Vec<usize>>>>,
) -> Result<()> {
    let n = x.len();
    if n == 0 {
        return Ok(());
    }
    if ws.structure.as_ref().is_none_or(|s| s.rows_of.len() != n) {
        let deps = structure
            .and_then(|s| s())
            .unwrap_or_else(|| Structure::probe(f, t, x));
        ws.structure = Some(Structure::new(n, &deps)?);
    }
    let s = ws.structure.as_ref().expect("set above");
    let steps = substeps.max(1);
    let h = dt / steps as f64;
    let gamma = 2.0 - std::f64::consts::SQRT_2;
    let c = match scheme {
        Scheme::BackwardEuler => 1.0,
        Scheme::TrBdf2 => gamma / 2.0,
    };
    // One Jacobian and factorisation per communication step.
    let mut f0 = vec![0.0; n];
    f(t, x, &mut f0);
    let lu = factorise(s, f, t, x, &f0, c * h)?;
    let mut r = vec![0.0; n];
    for k in 0..steps {
        let t0 = t + k as f64 * h;
        match scheme {
            Scheme::BackwardEuler => {
                r.copy_from_slice(x);
                stage(f, &lu, t0 + h, h, &r, x)?;
            }
            Scheme::TrBdf2 => {
                // Trapezoid to t + γh: y − (γh/2)f(y) = x + (γh/2)f(x).
                if k > 0 {
                    f(t0, x, &mut f0);
                }
                for i in 0..n {
                    r[i] = x[i] + c * h * f0[i];
                }
                let mut y = x.to_vec();
                stage(f, &lu, t0 + gamma * h, c * h, &r, &mut y)?;
                // BDF2 to t + h: z − c·h·f(z) = (y − (1−γ)²·x) / (γ(2−γ)).
                let w = gamma * (2.0 - gamma);
                for i in 0..n {
                    r[i] = (y[i] - (1.0 - gamma).powi(2) * x[i]) / w;
                }
                let mut z = y;
                stage(f, &lu, t0 + h, c * h, &r, &mut z)?;
                x.copy_from_slice(&z);
            }
        }
    }
    Ok(())
}
