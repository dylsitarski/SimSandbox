//! Implicit one-step methods for stiff systems (ADR 0060, Q17): backward
//! Euler and TR-BDF2. Both are L-stable and keep no history between
//! steps, so snapshots, rewind and restarts work as for explicit methods.
//!
//! Each step solves its equations by Newton's method with a matrix
//! `I − c·h·J`. The Jacobian `J` is found by finite differences at the
//! start of every communication step: columns that share no row are
//! perturbed together (a colouring of the Jacobian's structure), so a
//! grid needs a handful of evaluations, not one per cell. The matrix is
//! factorised by a sparse LU on its own symmetric pattern without row
//! interchanges ([`crate::sparse_lu`], ADR 0087), or, when a pivot is too
//! small for that, by `faer`'s LU with partial pivoting.
//!
//! A stage that fails with the step's Jacobian (the iteration creeps, or
//! runs off to values that are not numbers) goes on with the Jacobian
//! taken afresh at every iteration: from where it got to, or from its
//! start. If that fails too, the step is an error and the state is left
//! as it was before the step. A stage that converges with the step's
//! first Jacobian never takes another, so such runs give the same
//! numbers as before this was added.
//!
//! Tolerances and perturbations are relative to the size of the state
//! (its largest value), not to 1, so the same system written in other
//! units (amperes or nanoamperes) converges the same way.
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

/// Newton's iterations per stage with one Jacobian.
const MAX_ITERATIONS: usize = 50;
/// Newton's iterations of a stage that failed with the step's Jacobian,
/// each with the Jacobian taken afresh.
const MAX_FRESH: usize = 20;
/// A stage has converged when the last correction is this small,
/// relative to the largest value of the stage's solution and of its
/// right side (the state the stage starts from): to the size of the
/// system's numbers, whatever its units, and still meaningful for a value
/// passing through 0.
const TOLERANCE: f64 = 1e-10;
/// A state is perturbed by this much of its size to find the Jacobian
/// (about the square root of the rounding error), or of the floor below
/// when it is smaller.
const PERTURBATION: f64 = 1.5e-8;

/// The least size a state is perturbed relative to: 1, or the largest
/// state when all are smaller (a system in small units), or 1 when all
/// are 0. A state at or near 0 beside others cannot be perturbed relative
/// to itself: the change in the rates would be lost in their rounding.
fn perturbation_floor(x: &[f64]) -> f64 {
    let largest = x.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    if largest > 0.0 && largest < 1.0 {
        largest
    } else {
        1.0
    }
}

/// Why a stage gave no solution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Failure {
    /// Newton's corrections did not shrink to the tolerance.
    NotConverged,
    /// A correction was NaN or infinite.
    NotANumber,
}

impl Failure {
    fn error(self) -> SimError {
        SimError::Block(
            match self {
                Failure::NotConverged => {
                    "an implicit step did not converge (Newton's method, also with the Jacobian \
                     taken afresh at every iteration): take a shorter step or more substeps"
                }
                Failure::NotANumber => {
                    "an implicit step gave values that are not numbers (NaN or infinite): the \
                     equations cannot be evaluated near this state, or the step is too long"
                }
            }
            .into(),
        )
    }
}

/// The Jacobian's structure and how it is coloured and ordered; kept
/// between steps, because it never changes the numbers a step gives.
#[derive(Default)]
pub struct Workspace {
    structure: Option<Structure>,
    /// Explicit methods' working arrays, kept between steps instead of
    /// allocated at each.
    buffers: Vec<Vec<f64>>,
}

impl Workspace {
    /// `count` working arrays of `n` values each. They hold what the last
    /// step left: every value is written before it is read, since a
    /// right-hand side writes all of `dx` (`Equations::rhs`).
    pub(crate) fn buffers(&mut self, count: usize, n: usize) -> &mut [Vec<f64>] {
        if self.buffers.len() < count {
            self.buffers.resize_with(count, Vec::new);
        }
        for b in &mut self.buffers[..count] {
            if b.len() != n {
                b.clear();
                b.resize(n, 0.0);
            }
        }
        &mut self.buffers[..count]
    }
}

struct Structure {
    /// For each column, the rows it appears in (the diagonal included).
    rows_of: Vec<Vec<usize>>,
    /// Columns perturbed together.
    colours: Vec<Vec<usize>>,
    /// The matrix's rows, column after column, and its LU's structure.
    row_idx: Vec<usize>,
    lu: crate::sparse_lu::Symbolic,
    /// `faer`'s, made when a pivot first refuses the LU above.
    fallback: std::sync::OnceLock<std::result::Result<Pivoting, String>>,
}

/// `faer`'s pattern and structure for an LU with partial pivoting.
type Pivoting = (SymbolicSparseColMat<usize>, SymbolicLu<usize>);

/// A factorised matrix.
enum Factor {
    Own(crate::sparse_lu::Numeric),
    Pivoted(Box<Lu<usize, f64>>),
}

impl Factor {
    /// Solve in place.
    fn solve(&self, s: &Structure, x: &mut [f64]) {
        match self {
            Factor::Own(f) => s.lu.solve(f, x),
            Factor::Pivoted(lu) => {
                let mut g = faer::Mat::<f64>::from_fn(x.len(), 1, |i, _| x[i]);
                lu.solve_in_place(g.as_mut());
                for (i, v) in x.iter_mut().enumerate() {
                    *v = g[(i, 0)];
                }
            }
        }
    }
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
        let mut col_ptr = vec![0];
        let mut row_idx = Vec::new();
        for rows in &rows_of {
            row_idx.extend(rows);
            col_ptr.push(row_idx.len());
        }
        let lu = crate::sparse_lu::Symbolic::new(n, &col_ptr, &row_idx);
        Ok(Self {
            rows_of,
            colours,
            row_idx,
            lu,
            fallback: std::sync::OnceLock::new(),
        })
    }

    /// `faer`'s structure for the LU with partial pivoting.
    fn pivoted(&self) -> Result<&Pivoting> {
        let made = self.fallback.get_or_init(|| {
            let n = self.rows_of.len();
            let entries: Vec<Triplet<usize, usize, f64>> = self
                .rows_of
                .iter()
                .enumerate()
                .flat_map(|(j, rows)| rows.iter().map(move |&i| Triplet::new(i, j, 1.0)))
                .collect();
            let matrix = SparseColMat::<usize, f64>::try_new_from_triplets(n, n, &entries)
                .map_err(|e| format!("{e:?}"))?;
            let pattern = matrix.symbolic().to_owned().map_err(|e| format!("{e:?}"))?;
            let symbolic = SymbolicLu::try_new(pattern.as_ref()).map_err(|e| format!("{e:?}"))?;
            Ok((pattern, symbolic))
        });
        made.as_ref()
            .map_err(|e| SimError::Block(format!("implicit method: {e}")))
    }

    /// The structure of `f` found by perturbing each state alone: a rate
    /// that changes depends on that state. Not at `x` itself but beside
    /// it, every state moved a little and each by a different amount: at
    /// `x` a product hides what it depends on wherever its other factor
    /// is 0 (a reaction `k·a·b` starting with no `b`), and equal states
    /// hide what their difference multiplies. A rate that is not a number
    /// beside `x` counts as depending on everything, which is safe.
    fn probe(f: &Rhs<'_>, t: f64, x: &[f64]) -> Vec<Vec<usize>> {
        let n = x.len();
        let floor = perturbation_floor(x);
        // Between 1 and 2 thousandths of each state's size, by the
        // golden ratio's multiples: no two alike.
        let delta: Vec<f64> = (0..n)
            .map(|j| {
                1e-3 * x[j].abs().max(floor) * (1.0 + (j as f64 * 0.618_033_988_749_895).fract())
            })
            .collect();
        let mut xp: Vec<f64> = x.iter().zip(&delta).map(|(v, d)| v + d).collect();
        let mut f0 = vec![0.0; n];
        f(t, &xp, &mut f0);
        let mut deps: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut f1 = vec![0.0; n];
        for j in 0..n {
            let kept = xp[j];
            xp[j] = kept + delta[j];
            f(t, &xp, &mut f1);
            xp[j] = kept;
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
fn factorise(s: &Structure, f: &Rhs<'_>, t: f64, x: &[f64], f0: &[f64], ch: f64) -> Result<Factor> {
    let n = x.len();
    // Values in the pattern's order: column by column, rows ascending.
    let mut values = Vec::with_capacity(s.rows_of.iter().map(Vec::len).sum());
    let mut xp = x.to_vec();
    let mut f1 = vec![0.0; n];
    let mut jac: Vec<Vec<f64>> = s.rows_of.iter().map(|r| vec![0.0; r.len()]).collect();
    let floor = perturbation_floor(x);
    for colour in &s.colours {
        let deltas: Vec<f64> = colour
            .iter()
            .map(|&j| PERTURBATION * x[j].abs().max(floor))
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
    debug_assert_eq!(values.len(), s.row_idx.len());
    if let Some(f) = s.lu.factorise(&values) {
        return Ok(Factor::Own(f));
    }
    // A pivot too small without row interchanges: with them.
    let (pattern, symbolic) = s.pivoted()?;
    let matrix = SparseColMat::<usize, f64>::new(pattern.clone(), values);
    Lu::try_new_with_symbolic(symbolic.clone(), matrix.as_ref())
        .map(|lu| Factor::Pivoted(Box::new(lu)))
        .map_err(|e| SimError::Block(format!("implicit method: the matrix is singular ({e:?})")))
}

/// Newton's iterations for `y − c·h·f(t, y) = r` from `y` with one
/// factorised matrix.
fn iterate(
    f: &Rhs<'_>,
    (s, lu): (&Structure, &Factor),
    t: f64,
    ch: f64,
    r: &[f64],
    y: &mut [f64],
) -> std::result::Result<(), Failure> {
    let n = y.len();
    let mut fy = vec![0.0; n];
    let mut g = vec![0.0; n];
    let right = r.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    for _ in 0..MAX_ITERATIONS {
        f(t, y, &mut fy);
        for i in 0..n {
            g[i] = y[i] - ch * fy[i] - r[i];
        }
        lu.solve(s, &mut g);
        let mut size = 0.0f64;
        let mut scale = right;
        for i in 0..n {
            y[i] -= g[i];
            size = size.max(g[i].abs());
            scale = scale.max(y[i].abs());
        }
        // `max` passes over NaN, so each correction is looked at.
        if !(size.is_finite() && g.iter().all(|v| v.is_finite())) {
            return Err(Failure::NotANumber);
        }
        if size <= TOLERANCE * scale {
            return Ok(());
        }
    }
    Err(Failure::NotConverged)
}

/// What solves a stage: the structure, the factorised matrix `I − c·h·J`
/// (replaced when a stage takes the Jacobian afresh, and then used for
/// the rest of the step) and `c·h`.
struct Solver<'a> {
    s: &'a Structure,
    lu: Factor,
    ch: f64,
}

/// Solve `y − c·h·f(t, y) = r` for `y` by Newton's method from `y`:
/// with the step's matrix, and if that fails with the Jacobian taken
/// afresh at every iteration, from where the first got to, or from the
/// start again if it ran off to values that are not numbers.
fn stage(f: &Rhs<'_>, solver: &mut Solver<'_>, t: f64, r: &[f64], y: &mut [f64]) -> Result<()> {
    let from = y.to_vec();
    let failure = match iterate(f, (solver.s, &solver.lu), t, solver.ch, r, y) {
        Ok(()) => return Ok(()),
        Err(failure) => failure,
    };
    if failure == Failure::NotANumber {
        y.copy_from_slice(&from);
    }
    let n = y.len();
    let right = r.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let mut fy = vec![0.0; n];
    let mut g = vec![0.0; n];
    for _ in 0..MAX_FRESH {
        f(t, y, &mut fy);
        if fy.iter().any(|v| !v.is_finite()) {
            return Err(Failure::NotANumber.error());
        }
        solver.lu = factorise(solver.s, f, t, y, &fy, solver.ch)?;
        for i in 0..n {
            g[i] = y[i] - solver.ch * fy[i] - r[i];
        }
        solver.lu.solve(solver.s, &mut g);
        let mut size = 0.0f64;
        let mut scale = right;
        for i in 0..n {
            y[i] -= g[i];
            size = size.max(g[i].abs());
            scale = scale.max(y[i].abs());
        }
        if !(size.is_finite() && g.iter().all(|v| v.is_finite())) {
            return Err(Failure::NotANumber.error());
        }
        if size <= TOLERANCE * scale {
            return Ok(());
        }
    }
    Err(failure.error())
}

/// One step of `scheme` from `t0` to `t0 + h`, replacing `x` by the
/// result. `f0` is `f(t0, x)` (TR-BDF2 reads it).
fn substep(
    scheme: Scheme,
    f: &Rhs<'_>,
    solver: &mut Solver<'_>,
    (t0, h): (f64, f64),
    f0: &[f64],
    x: &mut [f64],
) -> Result<()> {
    let n = x.len();
    let gamma = 2.0 - std::f64::consts::SQRT_2;
    let mut r = vec![0.0; n];
    match scheme {
        Scheme::BackwardEuler => {
            r.copy_from_slice(x);
            stage(f, solver, t0 + h, &r, x)?;
        }
        Scheme::TrBdf2 => {
            let ch = solver.ch;
            // Trapezoid to t + γh: y − (γh/2)f(y) = x + (γh/2)f(x).
            for i in 0..n {
                r[i] = x[i] + ch * f0[i];
            }
            let mut y = x.to_vec();
            stage(f, solver, t0 + gamma * h, &r, &mut y)?;
            // BDF2 to t + h: z − c·h·f(z) = (y − (1−γ)²·x) / (γ(2−γ)).
            let w = gamma * (2.0 - gamma);
            for i in 0..n {
                r[i] = (y[i] - (1.0 - gamma).powi(2) * x[i]) / w;
            }
            let mut z = y;
            stage(f, solver, t0 + h, &r, &mut z)?;
            x.copy_from_slice(&z);
        }
    }
    Ok(())
}

/// Advance `x` from `t` to `t + dt` in `substeps` steps of `scheme`. On an
/// error `x` is as it was given.
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
    let c = match scheme {
        Scheme::BackwardEuler => 1.0,
        Scheme::TrBdf2 => (2.0 - std::f64::consts::SQRT_2) / 2.0,
    };
    // One Jacobian and factorisation per communication step, unless a
    // stage needs another.
    let mut f0 = vec![0.0; n];
    f(t, x, &mut f0);
    let mut solver = Solver {
        s,
        lu: factorise(s, f, t, x, &f0, c * h)?,
        ch: c * h,
    };
    let start = x.to_vec();
    for k in 0..steps {
        let t0 = t + k as f64 * h;
        if k > 0 && scheme == Scheme::TrBdf2 {
            f(t0, x, &mut f0);
        }
        if let Err(e) = substep(scheme, f, &mut solver, (t0, h), &f0, x) {
            x.copy_from_slice(&start);
            return Err(e);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_matrix_needing_row_interchanges_falls_back_and_stays_exact() {
        // x' = w·y, y' = −w·x with w·h = 1000: I − h·J = [[1, −1000],
        // [1000, 1]], whose first pivot is far too small without row
        // interchanges. Backward Euler solves that linear system exactly.
        let w = 1000.0;
        let f = move |_: f64, x: &[f64], dx: &mut [f64]| {
            dx[0] = w * x[1];
            dx[1] = -w * x[0];
        };
        let mut x = [1.0, 0.0];
        let mut ws = Workspace::default();
        advance(
            (Scheme::BackwardEuler, 1),
            &f,
            0.0,
            1.0,
            &mut x,
            &mut ws,
            None,
        )
        .unwrap();
        let s = ws.structure.as_ref().unwrap();
        assert!(s.fallback.get().is_some(), "faer's LU was used");
        // (1 + w²)·x₁ = x₀, y₁ = −w·x₁.
        let x1 = 1.0 / (1.0 + w * w);
        assert!(
            (x[0] - x1).abs() < 1e-15 && (x[1] + w * x1).abs() < 1e-12,
            "{x:?}"
        );
        // A diagonally dominant system never needs it.
        let g = |_: f64, x: &[f64], dx: &mut [f64]| {
            dx[0] = -2.0 * x[0] + x[1];
            dx[1] = x[0] - 2.0 * x[1];
        };
        let mut y = [1.0, 0.0];
        let mut ws = Workspace::default();
        advance((Scheme::TrBdf2, 2), &g, 0.0, 0.1, &mut y, &mut ws, None).unwrap();
        assert!(ws.structure.as_ref().unwrap().fallback.get().is_none());
    }
}
