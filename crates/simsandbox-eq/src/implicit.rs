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
//! structure is kept ([`Workspace`]), and it decides which entries of the
//! Jacobian are kept, so it must not depend on where it was first found
//! (ADR 0104): a run restored into a fresh workspace finds it again at
//! another state. It is
//! - given by the system (typed equations know it exactly), or else
//! - dense, up to [`DENSE_UP_TO`] states: exact whatever the state;
//! - beyond that, found once by perturbing each state alone just beside
//!   the state and at two states offset from it, and taking the union.
//!   An entry that is zero at all three (a dependence on a velocity that
//!   starts at 0 is not: the offsets move it) is missed, and the Jacobian
//!   is then wrong in it, which changes the bits Newton's method
//!   converges to. Large systems should give their structure.

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

/// Systems up to this many states that do not give their structure are
/// taken as dense: `n` evaluations per Jacobian instead of a handful,
/// but no entry can be missed.
pub const DENSE_UP_TO: usize = 128;

/// The Jacobian's structure and how it is coloured and ordered; kept
/// between steps. It is the same wherever it is found (see the module's
/// notes), so keeping it never changes the numbers a step gives.
#[derive(Default)]
pub struct Workspace {
    structure: Option<Structure>,
    /// Explicit methods' working arrays, kept between steps instead of
    /// allocated at each.
    buffers: Vec<Vec<f64>>,
    /// The implicit methods' working arrays, likewise.
    arrays: Arrays,
}

/// The implicit methods' working arrays, kept between steps. Each is set
/// (filled, or zeroed where a fresh one was) where one was allocated
/// before, so a step computes the same numbers.
#[derive(Default)]
struct Arrays {
    /// [`factorise`]'s perturbed state and rates, the matrix's values and
    /// a colour's perturbations.
    xp: Vec<f64>,
    f1: Vec<f64>,
    values: Vec<f64>,
    deltas: Vec<f64>,
    /// Newton's rates and corrections, and the state a stage starts from.
    fy: Vec<f64>,
    g: Vec<f64>,
    from: Vec<f64>,
    /// A substep's right side and stage.
    r: Vec<f64>,
    y: Vec<f64>,
    /// A step's rates at its start, and its start.
    f0: Vec<f64>,
    start: Vec<f64>,
    /// The LU's working arrays, and a factor's storage given back.
    lu: crate::sparse_lu::Work,
    spare: Option<crate::sparse_lu::Numeric>,
}

/// `v` as `n` zeros, as a fresh `vec![0.0; n]`.
fn zeros(v: &mut Vec<f64>, n: usize) {
    v.clear();
    v.resize(n, 0.0);
}

/// `v` as a copy of `x`, as a fresh `x.to_vec()`.
fn copy(v: &mut Vec<f64>, x: &[f64]) {
    v.clear();
    v.extend_from_slice(x);
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
    /// The matrix's rows, column after column, where each column starts
    /// among them, and its LU's structure.
    row_idx: Vec<usize>,
    col_ptr: Vec<usize>,
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
    fn solve(&self, s: &Structure, x: &mut [f64], work: &mut crate::sparse_lu::Work) {
        match self {
            Factor::Own(f) => s.lu.solve_with(f, x, work),
            Factor::Pivoted(lu) => {
                let mut g = faer::Mat::<f64>::from_fn(x.len(), 1, |i, _| x[i]);
                lu.solve_in_place(g.as_mut());
                simsandbox_compiled::vector_state::clear_upper();
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
            col_ptr,
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

    /// The structure of `f` when the system does not give it: dense up
    /// to [`DENSE_UP_TO`] states, else probed around `x`.
    fn undeclared(f: &Rhs<'_>, t: f64, x: &[f64]) -> Vec<Vec<usize>> {
        let n = x.len();
        if n <= DENSE_UP_TO {
            return vec![(0..n).collect(); n];
        }
        Self::probe(f, t, x)
    }

    /// The structure of `f` found by perturbing each state alone, beside
    /// `x` and at two states offset from it, taking the union: a rate
    /// that changes at any of them depends on that state. At `x` itself a
    /// product hides what it depends on wherever its other factor is 0 (a
    /// velocity starting at rest, a reaction `k·a·b` with no `b`), and
    /// equal states hide what their difference multiplies; the offsets
    /// move every state by a quarter to three quarters of its size plus
    /// the state's scale, up at one and down at the other (alternating
    /// from state to state). Steps and offsets are relative to the
    /// state's scale (ADR 0097), so other units probe alike. A rate that
    /// is not a number counts as depending on the state moved, which is
    /// safe.
    fn probe(f: &Rhs<'_>, t: f64, x: &[f64]) -> Vec<Vec<usize>> {
        let n = x.len();
        let floor = perturbation_floor(x);
        // A fixed sequence, the same for every run: no two states alike.
        let golden = |j: usize| (j as f64 * 0.618_033_988_749_895).fract();
        let mut deps: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut f0 = vec![0.0; n];
        let mut f1 = vec![0.0; n];
        for k in 0..3usize {
            let mut xp: Vec<f64> = x
                .iter()
                .enumerate()
                .map(|(j, &v)| {
                    if k == 0 {
                        // Just beside `x`: between 1 and 2 thousandths.
                        return v + 1e-3 * v.abs().max(floor) * (1.0 + golden(j));
                    }
                    let r = 0.25 + 0.5 * golden(j + 1);
                    let sign = if (j + k) % 2 == 0 { 1.0 } else { -1.0 };
                    v + sign * r * (floor + v.abs())
                })
                .collect();
            f(t, &xp, &mut f0);
            for j in 0..n {
                let kept = xp[j];
                xp[j] = kept + 1e-3 * kept.abs().max(floor) * (1.0 + golden(j));
                f(t, &xp, &mut f1);
                xp[j] = kept;
                for i in 0..n {
                    let finite = f0[i].is_finite() && f1[i].is_finite();
                    if !finite || f1[i] != f0[i] {
                        deps[i].push(j);
                    }
                }
            }
        }
        for row in &mut deps {
            row.sort_unstable();
            row.dedup();
        }
        deps
    }
}

/// The matrix `I − c·h·J` factorised, `J` by finite differences at
/// `(t, x)` where the rates are `f0`. faer's matrix products leave the
/// upper halves of the vector registers in use, which slows every later
/// SSE instruction on this thread on Intel processors: they are cleared
/// before returning (`simsandbox_compiled::vector_state`).
fn factorise(
    s: &Structure,
    f: &Rhs<'_>,
    point: (f64, &[f64], &[f64]),
    ch: f64,
    a: &mut Arrays,
) -> Result<Factor> {
    let factor = factorise_with_faer(s, f, point, ch, a);
    simsandbox_compiled::vector_state::clear_upper();
    factor
}

fn factorise_with_faer(
    s: &Structure,
    f: &Rhs<'_>,
    (t, x, f0): (f64, &[f64], &[f64]),
    ch: f64,
    a: &mut Arrays,
) -> Result<Factor> {
    let n = x.len();
    let Arrays {
        xp,
        f1,
        values,
        deltas,
        lu,
        spare,
        ..
    } = a;
    // Values in the pattern's order: column by column, rows ascending;
    // each column is in one colour, so every value is written.
    values.clear();
    values.resize(s.row_idx.len(), 0.0);
    copy(xp, x);
    zeros(f1, n);
    let floor = perturbation_floor(x);
    for colour in &s.colours {
        deltas.clear();
        deltas.extend(colour.iter().map(|&j| PERTURBATION * x[j].abs().max(floor)));
        for (&j, d) in colour.iter().zip(deltas.iter()) {
            xp[j] = x[j] + d;
        }
        f(t, xp, f1);
        for (&j, d) in colour.iter().zip(deltas.iter()) {
            xp[j] = x[j];
            // xp[j] + d − x[j] is the step actually taken, in floating point.
            let taken = (x[j] + d) - x[j];
            let at = s.col_ptr[j];
            for (k, &i) in s.rows_of[j].iter().enumerate() {
                let identity = if i == j { 1.0 } else { 0.0 };
                let jac = (f1[i] - f0[i]) / taken;
                values[at + k] = identity - ch * jac;
            }
        }
    }
    if let Some(f) = s.lu.factorise_with(values, spare.take(), lu) {
        return Ok(Factor::Own(f));
    }
    // A pivot too small without row interchanges: with them.
    let (pattern, symbolic) = s.pivoted()?;
    let matrix = SparseColMat::<usize, f64>::new(pattern.clone(), values.clone());
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
    a: &mut Arrays,
) -> std::result::Result<(), Failure> {
    let n = y.len();
    let Arrays {
        fy, g, lu: work, ..
    } = a;
    zeros(fy, n);
    zeros(g, n);
    let right = r.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    for _ in 0..MAX_ITERATIONS {
        f(t, y, fy);
        for i in 0..n {
            g[i] = y[i] - ch * fy[i] - r[i];
        }
        lu.solve(s, g, work);
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
fn stage(
    f: &Rhs<'_>,
    solver: &mut Solver<'_>,
    t: f64,
    r: &[f64],
    y: &mut [f64],
    a: &mut Arrays,
) -> Result<()> {
    copy(&mut a.from, y);
    let failure = match iterate(f, (solver.s, &solver.lu), t, solver.ch, r, y, a) {
        Ok(()) => return Ok(()),
        Err(failure) => failure,
    };
    if failure == Failure::NotANumber {
        y.copy_from_slice(&a.from);
    }
    let n = y.len();
    let right = r.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    // Taken out while `factorise` uses the rest.
    let mut fy = std::mem::take(&mut a.fy);
    let mut g = std::mem::take(&mut a.g);
    zeros(&mut fy, n);
    zeros(&mut g, n);
    let done = (|| {
        for _ in 0..MAX_FRESH {
            f(t, y, &mut fy);
            if fy.iter().any(|v| !v.is_finite()) {
                return Err(Failure::NotANumber.error());
            }
            let fresh = factorise(solver.s, f, (t, y, &fy), solver.ch, a)?;
            if let Factor::Own(old) = std::mem::replace(&mut solver.lu, fresh) {
                a.spare = Some(old);
            }
            for i in 0..n {
                g[i] = y[i] - solver.ch * fy[i] - r[i];
            }
            solver.lu.solve(solver.s, &mut g, &mut a.lu);
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
    })();
    a.fy = fy;
    a.g = g;
    done
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
    a: &mut Arrays,
) -> Result<()> {
    let n = x.len();
    let gamma = 2.0 - std::f64::consts::SQRT_2;
    // Taken out while the stages use the rest.
    let mut r = std::mem::take(&mut a.r);
    let mut y = std::mem::take(&mut a.y);
    zeros(&mut r, n);
    let done = (|| {
        match scheme {
            Scheme::BackwardEuler => {
                r.copy_from_slice(x);
                stage(f, solver, t0 + h, &r, x, a)?;
            }
            Scheme::TrBdf2 => {
                let ch = solver.ch;
                // Trapezoid to t + γh: y − (γh/2)f(y) = x + (γh/2)f(x).
                for i in 0..n {
                    r[i] = x[i] + ch * f0[i];
                }
                copy(&mut y, x);
                stage(f, solver, t0 + gamma * h, &r, &mut y, a)?;
                // BDF2 to t + h: z − c·h·f(z) = (y − (1−γ)²·x) / (γ(2−γ)),
                // from z = y.
                let w = gamma * (2.0 - gamma);
                for i in 0..n {
                    r[i] = (y[i] - (1.0 - gamma).powi(2) * x[i]) / w;
                }
                stage(f, solver, t0 + h, &r, &mut y, a)?;
                x.copy_from_slice(&y);
            }
        }
        Ok(())
    })();
    a.r = r;
    a.y = y;
    done
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
            .unwrap_or_else(|| Structure::undeclared(f, t, x));
        ws.structure = Some(Structure::new(n, &deps)?);
    }
    let Workspace {
        structure, arrays, ..
    } = ws;
    let s = structure.as_ref().expect("set above");
    let steps = substeps.max(1);
    let h = dt / steps as f64;
    let c = match scheme {
        Scheme::BackwardEuler => 1.0,
        Scheme::TrBdf2 => (2.0 - std::f64::consts::SQRT_2) / 2.0,
    };
    // One Jacobian and factorisation per communication step, unless a
    // stage needs another.
    let mut f0 = std::mem::take(&mut arrays.f0);
    let mut start = std::mem::take(&mut arrays.start);
    zeros(&mut f0, n);
    copy(&mut start, x);
    let done = run(
        scheme,
        f,
        (s, c * h),
        (t, h, steps),
        (&mut f0, &start),
        x,
        arrays,
    );
    arrays.f0 = f0;
    arrays.start = start;
    done
}

/// [`advance`]'s steps, with its rates at the start `f0` and the state it
/// was given `start`.
fn run(
    scheme: Scheme,
    f: &Rhs<'_>,
    (s, ch): (&Structure, f64),
    (t, h, steps): (f64, f64, usize),
    (f0, start): (&mut [f64], &[f64]),
    x: &mut [f64],
    arrays: &mut Arrays,
) -> Result<()> {
    f(t, x, f0);
    let lu = factorise(s, f, (t, x, f0), ch, arrays)?;
    let mut solver = Solver { s, lu, ch };
    for k in 0..steps {
        let t0 = t + k as f64 * h;
        if k > 0 && scheme == Scheme::TrBdf2 {
            f(t0, x, f0);
        }
        if let Err(e) = substep(scheme, f, &mut solver, (t0, h), f0, x, arrays) {
            x.copy_from_slice(start);
            return Err(e);
        }
    }
    // The factor's storage, for the next step's.
    if let Factor::Own(f) = solver.lu {
        arrays.spare = Some(f);
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

    /// Two oscillators coupled through their velocities, `x₁' = v₁,
    /// v₁' = −x₁ + v₁v₂, x₂' = v₂, v₂' = −x₂ − v₁v₂` (as a Coriolis term),
    /// in `groups` copies: at rest, perturbing one velocity alone leaves
    /// the product 0, so its entries vanish there.
    fn coupled(groups: usize) -> impl Fn(f64, &[f64], &mut [f64]) {
        move |_: f64, x: &[f64], dx: &mut [f64]| {
            for k in 0..groups {
                let s = &x[4 * k..4 * k + 4];
                let d = &mut dx[4 * k..4 * k + 4];
                let q = s[1] * s[3];
                d[0] = s[1];
                d[1] = -s[0] + q;
                d[2] = s[3];
                d[3] = -s[2] - q;
            }
        }
    }

    /// At rest, `groups` copies.
    fn at_rest(groups: usize) -> Vec<f64> {
        (0..groups)
            .flat_map(|k| [0.5 + 0.001 * k as f64, 0.0, 0.3, 0.0])
            .collect()
    }

    /// Run `steps` steps of TR-BDF2 from `x` in `ws`.
    fn run(f: &Rhs<'_>, x: &mut [f64], from: usize, steps: usize, ws: &mut Workspace) {
        let dt = 0.05;
        for k in from..from + steps {
            advance((Scheme::TrBdf2, 1), f, k as f64 * dt, dt, x, ws, None).unwrap();
        }
    }

    /// Started at rest, snapshot, run on; the snapshot restored into a
    /// fresh workspace (which finds the structure again, away from rest)
    /// runs on to the same bits: dense (small) and probed (large).
    #[test]
    fn a_run_restored_into_a_fresh_workspace_gives_the_same_bits() {
        for groups in [1, DENSE_UP_TO / 2] {
            let f = coupled(groups);
            let mut x = at_rest(groups);
            let mut ws = Workspace::default();
            run(&f, &mut x, 0, 100, &mut ws);
            let snapshot = x.clone();
            run(&f, &mut x, 100, 100, &mut ws);
            let mut y = snapshot;
            run(&f, &mut y, 100, 100, &mut Workspace::default());
            let bits = |x: &[f64]| x.iter().map(|v| v.to_bits()).collect::<Vec<_>>();
            assert_eq!(bits(&x), bits(&y), "{groups} groups");
            assert!(x.iter().all(|v| v.is_finite() && v.abs() < 1.0), "{x:?}");
        }
    }

    /// The probe finds entries that are 0 at the state it starts from;
    /// a small system that does not give its structure is dense.
    #[test]
    fn the_probe_finds_entries_that_vanish_at_the_state() {
        let f = coupled(1);
        let x = at_rest(1);
        let deps = Structure::probe(&f, 0.0, &x);
        assert_eq!(deps, vec![vec![1], vec![0, 1, 3], vec![3], vec![1, 2, 3]]);
        assert_eq!(
            Structure::undeclared(&f, 0.0, &x),
            vec![(0..4).collect::<Vec<_>>(); 4]
        );
    }

    /// A dense system, factorised with faer's matrix products: the upper
    /// halves of the vector registers are left unused (performance notes,
    /// 2026-11-19). Without clearing them, a 200-link chain stepped
    /// afterwards on this thread ran 17 times slower.
    #[test]
    fn factorising_leaves_the_upper_vector_registers_unused() {
        use simsandbox_compiled::vector_state::{clear_upper, upper_in_use};
        // faer's product of two 64 × 64 matrices, as the LU's dense
        // blocks: whether it leaves them in use here (it does on x86-64
        // with AVX, faer 0.24).
        clear_upper();
        let a = faer::Mat::<f64>::from_fn(64, 64, |i, j| (i + 2 * j) as f64);
        let mut c = faer::Mat::<f64>::zeros(64, 64);
        faer::linalg::matmul::matmul(
            c.as_mut(),
            faer::Accum::Add,
            a.as_ref(),
            a.as_ref(),
            1.0,
            faer::Par::Seq,
        );
        let dirtied = upper_in_use();
        let n = 64;
        let f = move |_: f64, x: &[f64], dx: &mut [f64]| {
            let total: f64 = x.iter().sum();
            for i in 0..n {
                dx[i] = -3.0 * x[i] + 0.01 * total;
            }
        };
        let mut x = vec![1.0; n];
        let mut ws = Workspace::default();
        advance((Scheme::TrBdf2, 2), &f, 0.0, 0.1, &mut x, &mut ws, None).unwrap();
        if dirtied == Some(true) {
            assert_eq!(upper_in_use(), Some(false));
        }
    }
}
