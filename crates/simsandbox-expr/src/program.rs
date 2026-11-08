//! Expressions compiled to a compact instruction list (ADR 0060, Q35).
//!
//! A [`Program`] runs a list of assignments, `variable = expr`, over one
//! buffer of registers: the caller's values first (time, states, inputs,
//! parameters, results), then constants, then temporaries. A variable is
//! one register or, for arrays, a run of them ([`Span`]). Each instruction
//! reads and writes registers by index, so evaluating costs one dispatch
//! per operation instead of a walk over a tree of boxes, and an operation
//! on arrays loops over every cell inside one instruction. Parts without
//! variables are folded to constants when the program is built.
//!
//! A program gives the same numbers as [`Expr::eval`], operation for
//! operation, apart from `x^2`, which is computed as `x*x`.

use crate::cells::{CellOp, Kind, Operand, Outside, Phase, Step};
use crate::{Along, ArrayOp, Edge, Expr, Func, Line, Op, MAX_AXES};

/// Where a variable's values are in the register buffer: `len` registers
/// from `at` (1 for a scalar).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub at: usize,
    pub len: usize,
}

/// One scalar instruction: `d` is the register written, `a` and `b` those
/// read. `Arr` runs an operation on arrays, by its place in the program's
/// list of them.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Ins {
    Copy(u32, u32),
    Neg(u32, u32),
    Add(u32, u32, u32),
    Sub(u32, u32, u32),
    Mul(u32, u32, u32),
    Div(u32, u32, u32),
    Pow(u32, u32, u32),
    /// A comparison (ADR 0070): 1 or 0.
    Cmp(Op, u32, u32, u32),
    Sqr(u32, u32),
    Call1(Func, u32, u32),
    Call2(Func, u32, u32, u32),
    Arr(u32),
}

/// An operand of an array instruction: its first register, and whether it
/// is an array (step 1 from cell to cell) or a scalar read for every cell.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Src {
    at: usize,
    array: bool,
}

impl Src {
    fn get(self, r: &[f64], k: usize) -> f64 {
        if self.array {
            r[self.at + k]
        } else {
            r[self.at]
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Reduce {
    Sum,
    Mean,
    Min,
    Max,
    ArgMin,
    ArgMax,
}

/// Up to this many cells a sum adds them one after another; past it, in
/// chunks of this many, each added in four interleaved lanes, the chunks'
/// sums then added in order (ADR 0077). The rule depends only on the
/// number of cells, so a sum is the same bits on any machine and any
/// number of cores.
pub const SUM_CHUNK: usize = 4096;

/// From this many cells a sum's chunks are added on several cores.
const SUM_SPLIT: usize = 1 << 20;

/// The sum of `cells` (ADR 0077).
pub fn sum(cells: &[f64]) -> f64 {
    sum_split(cells, cells.len() >= SUM_SPLIT)
}

/// [`sum`], its chunks added on several cores when `split`: the same bits.
fn sum_split(cells: &[f64], split: bool) -> f64 {
    if cells.len() <= SUM_CHUNK {
        return cells.iter().sum();
    }
    // Four lanes: cell i to lane i mod 4, so the additions of a chunk run
    // side by side (vector instructions) instead of one waiting for the last.
    let chunk = |c: &[f64]| -> f64 {
        let mut lanes = [0.0f64; 4];
        let mut quads = c.chunks_exact(4);
        for q in &mut quads {
            lanes[0] += q[0];
            lanes[1] += q[1];
            lanes[2] += q[2];
            lanes[3] += q[3];
        }
        for (lane, &x) in lanes.iter_mut().zip(quads.remainder()) {
            *lane += x;
        }
        (lanes[0] + lanes[1]) + (lanes[2] + lanes[3])
    };
    if split && simsandbox_core::parallel::threads() > 1 {
        let chunks: Vec<&[f64]> = cells.chunks(SUM_CHUNK).collect();
        simsandbox_core::parallel::each(chunks, true, chunk)
            .into_iter()
            .sum()
    } else {
        cells.chunks(SUM_CHUNK).map(chunk).sum()
    }
}

/// The most cells any part of `e` has, over variables laid out as `vars`.
fn longest(e: &Expr, vars: &[Span]) -> usize {
    let own = match e {
        Expr::Num(_) => 1,
        Expr::Var(i) => vars[*i].len,
        Expr::Neg(_) | Expr::Bin(..) | Expr::Call(..) => 1,
        Expr::Array(op, args) => match op {
            ArrayOp::Axis { cells, .. } => *cells,
            ArrayOp::Gather { rank, dims, .. } => dims[..*rank].iter().product(),
            ArrayOp::Along {
                rank, dims, axis, ..
            } => (0..*rank).filter(|k| k != axis).map(|k| dims[k]).product(),
            ArrayOp::Take => args.get(1).map_or(1, |a| longest(a, vars)),
            _ => args.first().map_or(1, |a| longest(a, vars)),
        },
    };
    let inner = match e {
        Expr::Num(_) | Expr::Var(_) => 1,
        Expr::Neg(a) => longest(a, vars),
        Expr::Bin(_, a, b) => longest(a, vars).max(longest(b, vars)),
        Expr::Call(_, args) | Expr::Array(_, args) => {
            args.iter().map(|a| longest(a, vars)).max().unwrap_or(1)
        }
    };
    own.max(inner)
}

/// `op` of `cells`.
fn reduce(op: Reduce, cells: &[f64]) -> f64 {
    match op {
        Reduce::Sum => sum(cells),
        Reduce::Mean => sum(cells) / cells.len() as f64,
        Reduce::Min => cells.iter().copied().fold(f64::INFINITY, f64::min),
        Reduce::Max => cells.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        Reduce::ArgMin => arg_extreme(cells, false),
        Reduce::ArgMax => arg_extreme(cells, true),
    }
}

/// The first cell holding the least (`greatest` false) or greatest value,
/// NaN cells skipped; NaN if all are.
fn arg_extreme(cells: &[f64], greatest: bool) -> f64 {
    let mut best: Option<(usize, f64)> = None;
    for (k, &v) in cells.iter().enumerate() {
        if v.is_nan() {
            continue;
        }
        let better = match best {
            None => true,
            Some((_, b)) => (greatest && v > b) || (!greatest && v < b),
        };
        if better {
            best = Some((k, v));
        }
    }
    best.map_or(f64::NAN, |(k, _)| k as f64)
}

/// An edge with its registers resolved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum EdgeAt {
    Value(usize),
    Gradient(Option<usize>),
    Periodic,
}

/// An operation over `n` cells.
#[derive(Debug, Clone, PartialEq)]
enum ArrIns {
    Copy {
        d: usize,
        a: Src,
        n: usize,
    },
    Neg {
        d: usize,
        a: Src,
        n: usize,
    },
    Bin {
        op: Op,
        d: usize,
        a: Src,
        b: Src,
        n: usize,
    },
    Call1 {
        f: Func,
        d: usize,
        a: Src,
        n: usize,
    },
    Call2 {
        f: Func,
        d: usize,
        a: Src,
        b: Src,
        n: usize,
    },
    /// Cell centres `lo + (j + 1/2)·h`, `j` the place along `line`.
    Axis {
        d: usize,
        lo: usize,
        h: usize,
        n: usize,
        line: Line,
    },
    /// Second (`second`) or first derivative along `line`.
    Stencil {
        second: bool,
        d: usize,
        a: usize,
        h: usize,
        n: usize,
        line: Line,
        edges: [EdgeAt; 2],
    },
    /// The neighbour before or after (`up`) along `line`.
    Shift {
        up: bool,
        d: usize,
        a: usize,
        h: usize,
        n: usize,
        line: Line,
        edges: [EdgeAt; 2],
    },
    /// A reduction to one value.
    Reduce {
        op: Reduce,
        d: usize,
        a: usize,
        n: usize,
    },
    /// `r[d]` = cell `r[i]` (rounded) of the `n` cells at `a`, NaN outside.
    Pick {
        d: usize,
        a: usize,
        n: usize,
        i: usize,
    },
    /// `r[d + k]` = cell `r[i + k]` (rounded) of the `n` cells at `a`, NaN
    /// outside, for each of the `m` places at `i`.
    Take {
        d: usize,
        a: usize,
        n: usize,
        i: usize,
        m: usize,
    },
    /// [`ArrayOp::Gather`] from the array at `a` (`n` cells written).
    Gather {
        d: usize,
        a: usize,
        n: usize,
        rank: usize,
        dims: [usize; MAX_AXES],
        strides: [usize; MAX_AXES],
    },
    /// [`ArrayOp::Rcond`] of the matrix at `a` into `r[d]`.
    Rcond {
        d: usize,
        a: usize,
        n: usize,
        rows: usize,
        cols: usize,
    },
    /// [`ArrayOp::Solve`] of the matrix at `a` and the vector at `b`.
    Solve {
        d: usize,
        a: usize,
        b: usize,
        n: usize,
        rows: usize,
        cols: usize,
    },
    /// [`ArrayOp::Along`] of the array at `a` (`n` cells written).
    Along {
        op: Reduce,
        d: usize,
        a: usize,
        n: usize,
        rank: usize,
        dims: [usize; MAX_AXES],
        axis: usize,
    },
}

/// The solution of `a x = b` into `x` (ADR 0080): `a` row by row, `n` by
/// `n`, both overwritten. Gaussian elimination with partial pivoting, each
/// step in a fixed order, so the same bits on any machine; NaN throughout
/// when a pivot is zero or a value is not finite.
fn solve_into(n: usize, a: &mut [f64], b: &mut [f64], x: &mut [f64]) {
    if a.iter().chain(b.iter()).any(|v| !v.is_finite()) {
        x.fill(f64::NAN);
        return;
    }
    for k in 0..n {
        // The first row from k down with the largest |a[i][k]|.
        let mut p = k;
        for i in k + 1..n {
            if a[i * n + k].abs() > a[p * n + k].abs() {
                p = i;
            }
        }
        if a[p * n + k] == 0.0 {
            x.fill(f64::NAN);
            return;
        }
        if p != k {
            for j in 0..n {
                a.swap(p * n + j, k * n + j);
            }
            b.swap(p, k);
        }
        let pivot = a[k * n + k];
        for i in k + 1..n {
            let f = a[i * n + k] / pivot;
            for j in k + 1..n {
                a[i * n + j] -= f * a[k * n + j];
            }
            b[i] -= f * b[k];
        }
    }
    for i in (0..n).rev() {
        let mut s = b[i];
        for j in i + 1..n {
            s -= a[i * n + j] * x[j];
        }
        x[i] = s / a[i * n + i];
    }
}

/// The reciprocal condition number of `a` (row by row, `n` by `n`) in the
/// 1-norm: `1/(‖a‖₁‖a⁻¹‖₁)`, `a⁻¹` column by column with [`solve_into`]
/// (fine for the small systems of strategies). 0 when singular, NaN when a
/// value is not finite.
fn rcond(n: usize, a: &[f64]) -> f64 {
    if a.iter().any(|v| !v.is_finite()) {
        return f64::NAN;
    }
    let norm = |m: &dyn Fn(usize, usize) -> f64| -> f64 {
        (0..n)
            .map(|j| (0..n).map(|i| m(i, j).abs()).sum::<f64>())
            .fold(0.0, f64::max)
    };
    let a_norm = norm(&|i, j| a[i * n + j]);
    let mut inverse = vec![0.0; n * n];
    for j in 0..n {
        let mut m = a.to_vec();
        let mut e = vec![0.0; n];
        e[j] = 1.0;
        let mut x = vec![0.0; n];
        solve_into(n, &mut m, &mut e, &mut x);
        if x.iter().any(|v| !v.is_finite()) {
            return 0.0;
        }
        for i in 0..n {
            inverse[i * n + j] = x[i];
        }
    }
    let inv_norm = norm(&|i, j| inverse[i * n + j]);
    if a_norm == 0.0 || inv_norm == 0.0 {
        return 0.0;
    }
    1.0 / (a_norm * inv_norm)
}

/// The source place of each cell of a [`ArrayOp::Gather`], in order.
fn gather_places(rank: usize, dims: &[usize; MAX_AXES], strides: &[usize; MAX_AXES]) -> Vec<usize> {
    let n: usize = dims[..rank].iter().product();
    let mut at = [0usize; MAX_AXES];
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push((0..rank).map(|k| at[k] * strides[k]).sum());
        for k in 0..rank {
            at[k] += 1;
            if at[k] < dims[k] {
                break;
            }
            at[k] = 0;
        }
    }
    out
}

/// For each cell of an [`ArrayOp::Along`]'s result, in order, the places of
/// the argument's cells it reduces, in order along the axis.
fn along_lines(rank: usize, dims: &[usize; MAX_AXES], axis: usize) -> Vec<Vec<usize>> {
    let stride: usize = dims[..axis].iter().product();
    let count = dims[axis];
    let mut rest = [1usize; MAX_AXES];
    let mut rest_strides = [0usize; MAX_AXES];
    let mut m = 0;
    let mut s = 1;
    for (k, &d) in dims[..rank].iter().enumerate() {
        if k != axis {
            rest[m] = d;
            rest_strides[m] = s;
            m += 1;
        }
        s *= d;
    }
    gather_places(m, &rest, &rest_strides)
        .into_iter()
        .map(|base| (0..count).map(|j| base + j * stride).collect())
        .collect()
}

/// `r[d..d+n] = f(a, b)` cell by cell, a scalar operand read for every
/// cell; one loop per case so each is a plain pass over slices.
fn zip(r: &mut [f64], d: usize, a: Src, b: Src, n: usize, f: impl Fn(f64, f64) -> f64) {
    match (a.array, b.array) {
        (true, true) => (0..n).for_each(|k| r[d + k] = f(r[a.at + k], r[b.at + k])),
        (true, false) => {
            let y = r[b.at];
            (0..n).for_each(|k| r[d + k] = f(r[a.at + k], y));
        }
        (false, true) => {
            let x = r[a.at];
            (0..n).for_each(|k| r[d + k] = f(x, r[b.at + k]));
        }
        (false, false) => {
            let v = f(r[a.at], r[b.at]);
            r[d..d + n].fill(v);
        }
    }
}

/// The `n` cells from `a`, to read, and the `n` from `d`, to write; they
/// do not overlap.
fn split(r: &mut [f64], a: usize, d: usize, n: usize) -> (&[f64], &mut [f64]) {
    if a < d {
        let (lo, hi) = r.split_at_mut(d);
        (&lo[a..a + n], &mut hi[..n])
    } else {
        let (lo, hi) = r.split_at_mut(a);
        (&hi[..n], &mut lo[d..d + n])
    }
}

/// An edge with its value read.
#[derive(Debug, Clone, Copy)]
pub(crate) enum EdgeValue {
    Value(f64),
    Gradient(f64),
    Periodic,
}

/// The value beyond an end cell holding `end`, the line's far end
/// holding `far`, with spacing `h`.
pub(crate) fn ghost(edge: EdgeValue, end: f64, far: f64, h: f64) -> f64 {
    match edge {
        // The edge value is the mean of the end cell and the ghost.
        EdgeValue::Value(v) => 2.0 * v - end,
        EdgeValue::Gradient(g) => end + g * h,
        EdgeValue::Periodic => far,
    }
}

impl ArrIns {
    fn run(&self, r: &mut [f64]) {
        match *self {
            ArrIns::Copy { d, a, n } => (0..n).for_each(|k| r[d + k] = a.get(r, k)),
            ArrIns::Neg { d, a, n } => (0..n).for_each(|k| r[d + k] = -a.get(r, k)),
            ArrIns::Bin { op, d, a, b, n } => match op {
                Op::Add => zip(r, d, a, b, n, |x, y| x + y),
                Op::Sub => zip(r, d, a, b, n, |x, y| x - y),
                Op::Mul => zip(r, d, a, b, n, |x, y| x * y),
                Op::Div => zip(r, d, a, b, n, |x, y| x / y),
                Op::Pow => zip(r, d, a, b, n, f64::powf),
                op => zip(r, d, a, b, n, |x, y| op.apply(x, y)),
            },
            ArrIns::Call1 { f, d, a, n } => {
                (0..n).for_each(|k| r[d + k] = f.apply(a.get(r, k), 0.0))
            }
            ArrIns::Call2 { f, d, a, b, n } => {
                (0..n).for_each(|k| r[d + k] = f.apply(a.get(r, k), b.get(r, k)))
            }
            ArrIns::Axis { d, lo, h, n, line } => {
                let (lo, h) = (r[lo], r[h]);
                (0..n).for_each(|k| {
                    let j = (k / line.stride) % line.count;
                    r[d + k] = lo + (j as f64 + 0.5) * h;
                });
            }
            ArrIns::Stencil {
                second,
                d,
                a,
                h,
                n,
                line,
                edges,
            } => {
                let h = r[h];
                let edges = edges.map(|e| match e {
                    EdgeAt::Value(v) => EdgeValue::Value(r[v]),
                    EdgeAt::Gradient(g) => EdgeValue::Gradient(g.map_or(0.0, |g| r[g])),
                    EdgeAt::Periodic => EdgeValue::Periodic,
                });
                let (src, dst) = split(r, a, d, n);
                let (s, c) = (line.stride, line.count);
                for l in 0..n / c {
                    // The cells of one line: base, base + s, ...
                    let base = (l / s) * s * c + l % s;
                    let at = |j: usize| base + j * s;
                    let low = ghost(edges[0], src[at(0)], src[at(c - 1)], h);
                    let high = ghost(edges[1], src[at(c - 1)], src[at(0)], h);
                    let below = |j: usize| if j == 0 { low } else { src[at(j - 1)] };
                    let above = |j: usize| if j + 1 == c { high } else { src[at(j + 1)] };
                    if second {
                        let h2 = h * h;
                        for j in 1..c.saturating_sub(1) {
                            dst[at(j)] = (src[at(j - 1)] - 2.0 * src[at(j)] + src[at(j + 1)]) / h2;
                        }
                        for j in [0, c - 1] {
                            dst[at(j)] = (below(j) - 2.0 * src[at(j)] + above(j)) / h2;
                        }
                    } else {
                        let h2 = 2.0 * h;
                        for j in 1..c.saturating_sub(1) {
                            dst[at(j)] = (src[at(j + 1)] - src[at(j - 1)]) / h2;
                        }
                        for j in [0, c - 1] {
                            dst[at(j)] = (above(j) - below(j)) / h2;
                        }
                    }
                }
            }
            ArrIns::Shift {
                up,
                d,
                a,
                h,
                n,
                line,
                edges,
            } => {
                let h = r[h];
                // Beyond an end: the value itself, not mirrored as for a
                // stencil (ADR 0069).
                let beyond = |e: EdgeAt, end: f64, far: f64, r: &[f64]| match e {
                    EdgeAt::Value(v) => r[v],
                    EdgeAt::Gradient(g) => end + g.map_or(0.0, |g| r[g]) * h,
                    EdgeAt::Periodic => far,
                };
                let (s, c) = (line.stride, line.count);
                let mut out = vec![0.0; n];
                for l in 0..n / c {
                    let base = (l / s) * s * c + l % s;
                    let at = |j: usize| a + base + j * s;
                    for j in 0..c {
                        let v = match (up, j) {
                            (false, 0) => beyond(edges[0], r[at(0)], r[at(c - 1)], r),
                            (false, j) => r[at(j - 1)],
                            (true, j) if j + 1 == c => beyond(edges[1], r[at(c - 1)], r[at(0)], r),
                            (true, j) => r[at(j + 1)],
                        };
                        out[base + j * s] = v;
                    }
                }
                r[d..d + n].copy_from_slice(&out);
            }
            ArrIns::Reduce { op, d, a, n } => {
                r[d] = reduce(op, &r[a..a + n]);
            }
            ArrIns::Gather {
                d,
                a,
                rank,
                ref dims,
                ref strides,
                ..
            } => {
                for (k, at) in gather_places(rank, dims, strides).into_iter().enumerate() {
                    r[d + k] = r[a + at];
                }
            }
            ArrIns::Rcond {
                d,
                a,
                n,
                rows,
                cols,
            } => {
                let m: Vec<f64> = (0..n * n)
                    .map(|k| r[a + (k / n) * rows + (k % n) * cols])
                    .collect();
                r[d] = rcond(n, &m);
            }
            ArrIns::Solve {
                d,
                a,
                b,
                n,
                rows,
                cols,
            } => {
                let mut m: Vec<f64> = (0..n * n)
                    .map(|k| r[a + (k / n) * rows + (k % n) * cols])
                    .collect();
                let mut v = r[b..b + n].to_vec();
                let mut x = vec![0.0; n];
                solve_into(n, &mut m, &mut v, &mut x);
                r[d..d + n].copy_from_slice(&x);
            }
            ArrIns::Along {
                op,
                d,
                a,
                rank,
                ref dims,
                axis,
                ..
            } => {
                let mut cells = Vec::with_capacity(dims[axis]);
                for (k, line) in along_lines(rank, dims, axis).into_iter().enumerate() {
                    cells.clear();
                    cells.extend(line.iter().map(|&at| r[a + at]));
                    r[d + k] = reduce(op, &cells);
                }
            }
            ArrIns::Pick { d, a, n, i } => {
                let k = r[i].round();
                r[d] = if k >= 0.0 && k < n as f64 {
                    r[a + k as usize]
                } else {
                    f64::NAN
                };
            }
            ArrIns::Take { d, a, n, i, m } => {
                // Read every place first: the result may share registers
                // with them.
                let picked: Vec<f64> = (0..m)
                    .map(|k| {
                        let p = r[i + k].round();
                        if p >= 0.0 && p < n as f64 {
                            r[a + p as usize]
                        } else {
                            f64::NAN
                        }
                    })
                    .collect();
                r[d..d + m].copy_from_slice(&picked);
            }
        }
    }
}

impl ArrIns {
    /// As one operation of a phase (ADR 0062 step 3), if it fills its
    /// result cell by cell: `(n, kind, d, a, b, regs)`, operands as
    /// (register, is an array), with the scalar registers it reads.
    fn as_cell_op(&self) -> Option<(usize, Kind, usize, Src, Src, Vec<usize>)> {
        let scalar = |at: usize| Src { at, array: false };
        let edge_regs = |edges: [EdgeAt; 2]| -> Vec<usize> {
            edges
                .iter()
                .filter_map(|e| match *e {
                    EdgeAt::Value(v) => Some(v),
                    EdgeAt::Gradient(g) => g,
                    EdgeAt::Periodic => None,
                })
                .collect()
        };
        Some(match *self {
            ArrIns::Copy { d, a, n } => (n, Kind::Copy, d, a, a, Vec::new()),
            ArrIns::Neg { d, a, n } => (n, Kind::Neg, d, a, a, Vec::new()),
            ArrIns::Bin { op, d, a, b, n } => (n, Kind::Bin(op), d, a, b, Vec::new()),
            ArrIns::Call1 { f, d, a, n } => (n, Kind::Call1(f), d, a, a, Vec::new()),
            ArrIns::Call2 { f, d, a, b, n } => (n, Kind::Call2(f), d, a, b, Vec::new()),
            ArrIns::Axis { d, lo, h, n, line } => {
                (n, Kind::Axis(line), d, scalar(lo), scalar(h), Vec::new())
            }
            ArrIns::Stencil {
                second,
                d,
                a,
                h,
                n,
                line,
                edges,
            } => (
                n,
                Kind::Stencil {
                    second,
                    line,
                    edges,
                },
                d,
                Src { at: a, array: true },
                scalar(h),
                edge_regs(edges),
            ),
            ArrIns::Shift {
                up,
                d,
                a,
                h,
                n,
                line,
                edges,
            } => (
                n,
                Kind::Shift { up, line, edges },
                d,
                Src { at: a, array: true },
                scalar(h),
                edge_regs(edges),
            ),
            _ => return None,
        })
    }
}

impl ArrIns {
    /// The runs of registers it reads or writes, as (first, count).
    fn touches(&self) -> Vec<(usize, usize)> {
        let src = |s: Src, n: usize| (s.at, if s.array { n } else { 1 });
        let edge_regs = |edges: [EdgeAt; 2]| {
            edges.into_iter().filter_map(|e| match e {
                EdgeAt::Value(v) => Some((v, 1)),
                EdgeAt::Gradient(g) => g.map(|g| (g, 1)),
                EdgeAt::Periodic => None,
            })
        };
        match *self {
            ArrIns::Copy { d, a, n } | ArrIns::Neg { d, a, n } | ArrIns::Call1 { d, a, n, .. } => {
                vec![(d, n), src(a, n)]
            }
            ArrIns::Bin { d, a, b, n, .. } | ArrIns::Call2 { d, a, b, n, .. } => {
                vec![(d, n), src(a, n), src(b, n)]
            }
            ArrIns::Axis { d, lo, h, n, .. } => vec![(d, n), (lo, 1), (h, 1)],
            ArrIns::Stencil {
                d, a, h, n, edges, ..
            }
            | ArrIns::Shift {
                d, a, h, n, edges, ..
            } => {
                let mut all = vec![(d, n), (a, n), (h, 1)];
                all.extend(edge_regs(edges));
                all
            }
            ArrIns::Reduce { d, a, n, .. } => vec![(d, 1), (a, n)],
            ArrIns::Pick { d, a, n, i } => vec![(d, 1), (a, n), (i, 1)],
            ArrIns::Take { d, a, n, i, m } => vec![(d, m), (a, n), (i, m)],
            ArrIns::Gather {
                d,
                a,
                n,
                rank,
                ref dims,
                ref strides,
            } => {
                let reach = gather_places(rank, dims, strides)
                    .into_iter()
                    .max()
                    .unwrap_or(0)
                    + 1;
                vec![(d, n), (a, reach)]
            }
            ArrIns::Along {
                d,
                a,
                n,
                rank,
                ref dims,
                ..
            } => vec![(d, n), (a, dims[..rank].iter().product())],
            ArrIns::Solve {
                d,
                a,
                b,
                n,
                rows,
                cols,
            } => vec![(d, n), (a, (n - 1) * (rows + cols) + 1), (b, n)],
            ArrIns::Rcond {
                d,
                a,
                n,
                rows,
                cols,
            } => vec![(d, 1), (a, (n - 1) * (rows + cols) + 1)],
        }
    }
}

/// The registers a scalar instruction writes and reads.
fn scalar_regs(ins: Ins) -> Option<(usize, Vec<usize>)> {
    let u = |x: u32| x as usize;
    Some(match ins {
        Ins::Copy(d, a) | Ins::Neg(d, a) | Ins::Sqr(d, a) | Ins::Call1(_, d, a) => {
            (u(d), vec![u(a)])
        }
        Ins::Add(d, a, b)
        | Ins::Sub(d, a, b)
        | Ins::Mul(d, a, b)
        | Ins::Div(d, a, b)
        | Ins::Pow(d, a, b)
        | Ins::Cmp(_, d, a, b)
        | Ins::Call2(_, d, a, b) => (u(d), vec![u(a), u(b)]),
        Ins::Arr(_) => return None,
    })
}

/// A phase being gathered: its cells, scalar instructions run first, the
/// arrays it writes and those it only reads (by first register), the
/// scalar registers its operations read, and its operations.
struct Gathering {
    n: usize,
    first: Vec<usize>,
    written: Vec<usize>,
    read: Vec<usize>,
    scalars: Vec<usize>,
    ops: Vec<(Kind, usize, Src, Src)>,
}

impl Gathering {
    fn new(n: usize) -> Self {
        Self {
            n,
            first: Vec::new(),
            written: Vec::new(),
            read: Vec::new(),
            scalars: Vec::new(),
            ops: Vec::new(),
        }
    }

    fn inside(&self, reg: usize, arrays: &[usize]) -> bool {
        arrays.iter().any(|&x| reg >= x && reg < x + self.n)
    }

    /// Take an operation if it fits. Arrays read cell by cell are split
    /// into chunks with those written (each chunk reads its own cells);
    /// a stencil's or shift's source is read whole by every chunk, so the
    /// phase must not write it. All arrays must be the phase's own or
    /// apart from them, and no scalar read may lie in a chunked array.
    fn take(&mut self, n: usize, kind: Kind, d: usize, a: Src, b: Src, regs: &[usize]) -> bool {
        if n != self.n {
            return false;
        }
        let whole = matches!(kind, Kind::Stencil { .. } | Kind::Shift { .. });
        let mut chunked = self.written.clone();
        let mut read = self.read.clone();
        let add = |list: &mut Vec<usize>, at: usize| {
            if !list.contains(&at) {
                list.push(at);
            }
        };
        add(&mut chunked, d);
        for (k, s) in [a, b].iter().enumerate().filter(|(_, s)| s.array) {
            if whole && k == 0 {
                add(&mut read, s.at);
            } else {
                add(&mut chunked, s.at);
            }
        }
        if chunked.iter().any(|x| read.contains(x)) {
            return false;
        }
        let all: Vec<usize> = chunked.iter().chain(&read).copied().collect();
        let apart = all
            .iter()
            .all(|&x| all.iter().all(|&y| x == y || x.abs_diff(y) >= n));
        let new_scalars: Vec<usize> = [a, b]
            .iter()
            .filter(|s| !s.array)
            .map(|s| s.at)
            .chain(regs.iter().copied())
            .collect();
        let clear = self
            .scalars
            .iter()
            .chain(&new_scalars)
            .all(|&reg| !self.inside(reg, &chunked));
        if !apart || !clear {
            return false;
        }
        self.written = chunked;
        self.read = read;
        self.scalars.extend(new_scalars);
        self.ops.push((kind, d, a, b));
        true
    }

    /// Take scalar instruction `i` (writing `d`, reading `reads`) to run
    /// before the phase, if no operation already taken reads `d` (it would
    /// see the new value) and nothing it touches lies in the phase's
    /// arrays.
    fn take_first(&mut self, i: usize, d: usize, reads: &[usize]) -> bool {
        let all: Vec<usize> = self.written.iter().chain(&self.read).copied().collect();
        if self.scalars.contains(&d)
            || self.inside(d, &all)
            || reads.iter().any(|&r| self.inside(r, &self.written))
        {
            return false;
        }
        self.first.push(i);
        true
    }

    /// Take instruction `i`, a reduction or a picked cell writing the
    /// scalar `d` from the runs of registers `reads`, to run before the
    /// phase: if no operation taken reads `d`, `d` lies outside the
    /// phase's arrays, and no operation taken writes what it reads (it
    /// would read them as they were before the phase).
    fn take_first_array(&mut self, i: usize, d: usize, reads: &[(usize, usize)]) -> bool {
        let all: Vec<usize> = self.written.iter().chain(&self.read).copied().collect();
        let n = self.n;
        let overlaps =
            |(a, m): (usize, usize)| self.ops.iter().any(|&(_, w, _, _)| a < w + n && w < a + m);
        if self.scalars.contains(&d) || self.inside(d, &all) || reads.iter().any(|&r| overlaps(r)) {
            return false;
        }
        self.first.push(i);
        true
    }

    fn finish(mut self) -> Phase {
        self.written.sort_unstable();
        self.read.sort_unstable();
        let (written, read) = (&self.written, &self.read);
        let operand = |s: Src| {
            if !s.array {
                Operand::Scalar(s.at)
            } else if let Some(k) = written.iter().position(|&x| x == s.at) {
                Operand::Written(k)
            } else {
                Operand::Read(read.iter().position(|&x| x == s.at).expect("read"))
            }
        };
        let ops = self
            .ops
            .iter()
            .map(|&(kind, d, a, b)| CellOp {
                kind,
                d: written.iter().position(|&x| x == d).expect("written"),
                a: operand(a),
                b: operand(b),
            })
            .collect();
        Phase {
            n: self.n,
            first: self.first,
            written_out: vec![false; self.written.len()],
            read_out: vec![false; self.read.len()],
            written: self.written,
            read: self.read,
            ops,
            choice: crate::cells::Choice::default(),
        }
    }
}

/// The code as steps for several cores (ADR 0062 step 3).
fn plan(code: &[Ins], arrays: &[ArrIns]) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut open: Option<Gathering> = None;
    let close = |open: &mut Option<Gathering>, steps: &mut Vec<Step>| {
        if let Some(g) = open.take() {
            steps.push(Step::Cells(g.finish()));
        }
    };
    for (i, &ins) in code.iter().enumerate() {
        // A scalar instruction: before the open phase if it can be.
        if let Some((d, reads)) = scalar_regs(ins) {
            if !open.as_mut().is_some_and(|g| g.take_first(i, d, &reads)) {
                close(&mut open, &mut steps);
                steps.push(Step::One(i));
            }
            continue;
        }
        let Ins::Arr(k) = ins else {
            unreachable!("scalars above")
        };
        let Some((n, kind, d, a, b, regs)) = arrays[k as usize].as_cell_op() else {
            // A reduction or a picked cell: before the open phase if it
            // can be (a mean of a state, say, need not split the phase).
            let ins = &arrays[k as usize];
            let scalar_result = matches!(ins, ArrIns::Reduce { .. } | ArrIns::Pick { .. });
            let touches = ins.touches();
            if scalar_result
                && open
                    .as_mut()
                    .is_some_and(|g| g.take_first_array(i, touches[0].0, &touches[1..]))
            {
                continue;
            }
            close(&mut open, &mut steps);
            steps.push(Step::One(i));
            continue;
        };
        if open
            .as_mut()
            .is_some_and(|g| g.take(n, kind, d, a, b, &regs))
        {
            continue;
        }
        close(&mut open, &mut steps);
        let mut g = Gathering::new(n);
        if g.take(n, kind, d, a, b, &regs) {
            open = Some(g);
        } else {
            steps.push(Step::One(i));
        }
    }
    close(&mut open, &mut steps);
    steps
}

/// The sorted union of sorted index lists.
fn union(sets: &[&Vec<usize>]) -> Vec<usize> {
    let mut out: Vec<usize> = sets.iter().flat_map(|s| s.iter().copied()).collect();
    out.sort_unstable();
    out.dedup();
    out
}

impl ArrIns {
    /// As [`ArrIns::run`], on the dependencies of each register.
    fn dependencies(&self, deps: &mut [Vec<usize>]) {
        let src = |deps: &[Vec<usize>], s: Src, k: usize| -> Vec<usize> {
            deps[if s.array { s.at + k } else { s.at }].clone()
        };
        match *self {
            ArrIns::Copy { d, a, n } | ArrIns::Neg { d, a, n } | ArrIns::Call1 { d, a, n, .. } => {
                for k in 0..n {
                    deps[d + k] = src(deps, a, k);
                }
            }
            ArrIns::Bin { d, a, b, n, .. } | ArrIns::Call2 { d, a, b, n, .. } => {
                for k in 0..n {
                    deps[d + k] = union(&[&src(deps, a, k), &src(deps, b, k)]);
                }
            }
            ArrIns::Axis { d, lo, h, n, .. } => {
                let both = union(&[&deps[lo], &deps[h]]);
                for k in 0..n {
                    deps[d + k] = both.clone();
                }
            }
            ArrIns::Stencil {
                d,
                a,
                h,
                n,
                line,
                edges,
                ..
            } => {
                let (s, c) = (line.stride, line.count);
                let mut cells: Vec<Vec<usize>> = vec![Vec::new(); n];
                for l in 0..n / c {
                    let base = (l / s) * s * c + l % s;
                    let at = |j: usize| a + base + j * s;
                    let ghost = |edge: EdgeAt, end: usize, far: usize| -> Vec<usize> {
                        match edge {
                            EdgeAt::Value(v) => union(&[&deps[v], &deps[end]]),
                            EdgeAt::Gradient(g) => {
                                let g = g.map(|g| deps[g].clone()).unwrap_or_default();
                                union(&[&deps[end], &deps[h], &g])
                            }
                            EdgeAt::Periodic => deps[far].clone(),
                        }
                    };
                    let low = ghost(edges[0], at(0), at(c - 1));
                    let high = ghost(edges[1], at(c - 1), at(0));
                    for j in 0..c {
                        let below = if j == 0 { &low } else { &deps[at(j - 1)] };
                        let above = if j + 1 == c { &high } else { &deps[at(j + 1)] };
                        cells[base + j * s] = union(&[below, &deps[at(j)], above, &deps[h]]);
                    }
                }
                for (k, c) in cells.into_iter().enumerate() {
                    deps[d + k] = c;
                }
            }
            ArrIns::Shift {
                up,
                d,
                a,
                h,
                n,
                line,
                edges,
            } => {
                let (s, c) = (line.stride, line.count);
                let mut cells: Vec<Vec<usize>> = vec![Vec::new(); n];
                let beyond = |e: EdgeAt, end: usize, far: usize| -> Vec<usize> {
                    match e {
                        EdgeAt::Value(v) => deps[v].clone(),
                        EdgeAt::Gradient(g) => {
                            let g = g.map(|g| deps[g].clone()).unwrap_or_default();
                            union(&[&deps[end], &deps[h], &g])
                        }
                        EdgeAt::Periodic => deps[far].clone(),
                    }
                };
                for l in 0..n / c {
                    let base = (l / s) * s * c + l % s;
                    let at = |j: usize| a + base + j * s;
                    for j in 0..c {
                        cells[base + j * s] = match (up, j) {
                            (false, 0) => beyond(edges[0], at(0), at(c - 1)),
                            (false, j) => deps[at(j - 1)].clone(),
                            (true, j) if j + 1 == c => beyond(edges[1], at(c - 1), at(0)),
                            (true, j) => deps[at(j + 1)].clone(),
                        };
                    }
                }
                for (k, c) in cells.into_iter().enumerate() {
                    deps[d + k] = c;
                }
            }
            ArrIns::Reduce { d, a, n, .. } => {
                let all: Vec<&Vec<usize>> = deps[a..a + n].iter().collect();
                deps[d] = union(&all);
            }
            ArrIns::Pick { d, a, n, i } => {
                let mut all: Vec<&Vec<usize>> = deps[a..a + n].iter().collect();
                all.push(&deps[i]);
                deps[d] = union(&all);
            }
            ArrIns::Take { d, a, n, i, m } => {
                let cells: Vec<Vec<usize>> = (0..m)
                    .map(|k| {
                        let mut all: Vec<&Vec<usize>> = deps[a..a + n].iter().collect();
                        all.push(&deps[i + k]);
                        union(&all)
                    })
                    .collect();
                for (k, c) in cells.into_iter().enumerate() {
                    deps[d + k] = c;
                }
            }
            ArrIns::Gather {
                d,
                a,
                rank,
                ref dims,
                ref strides,
                ..
            } => {
                let from: Vec<Vec<usize>> = gather_places(rank, dims, strides)
                    .into_iter()
                    .map(|at| deps[a + at].clone())
                    .collect();
                for (k, f) in from.into_iter().enumerate() {
                    deps[d + k] = f;
                }
            }
            ArrIns::Rcond {
                d,
                a,
                n,
                rows,
                cols,
            } => {
                let all: Vec<&Vec<usize>> = (0..n * n)
                    .map(|k| &deps[a + (k / n) * rows + (k % n) * cols])
                    .collect();
                deps[d] = union(&all);
            }
            ArrIns::Solve {
                d,
                a,
                b,
                n,
                rows,
                cols,
            } => {
                let mut all: Vec<&Vec<usize>> = (0..n * n)
                    .map(|k| &deps[a + (k / n) * rows + (k % n) * cols])
                    .collect();
                all.extend(&deps[b..b + n]);
                let one = union(&all);
                for k in 0..n {
                    deps[d + k] = one.clone();
                }
            }
            ArrIns::Along {
                d,
                a,
                rank,
                ref dims,
                axis,
                ..
            } => {
                let from: Vec<Vec<usize>> = along_lines(rank, dims, axis)
                    .into_iter()
                    .map(|line| union(&line.iter().map(|&at| &deps[a + at]).collect::<Vec<_>>()))
                    .collect();
                for (k, f) in from.into_iter().enumerate() {
                    deps[d + k] = f;
                }
            }
        }
    }
}

/// Assignments compiled to instructions; see the module documentation.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    code: Vec<Ins>,
    arrays: Vec<ArrIns>,
    /// The registers, constants filled in; copied by [`Program::registers`].
    template: Vec<f64>,
    /// For large arrays: the code as phases of cells and single
    /// instructions (ADR 0062 step 3); empty otherwise.
    steps: Vec<Step>,
    /// Whether a phase is large enough to share among cores.
    split: bool,
    /// Whether some arrays were placed outside the registers.
    outside: bool,
}

struct Builder<'a> {
    vars: &'a [Span],
    /// Where constants start, then where scalar temporaries start, then
    /// array temporaries, each taking `width` registers, room for the
    /// longest array. Scalars kept apart from arrays let a phase of cells
    /// compute its scalars first (ADR 0062 step 3).
    consts_at: usize,
    scalars_at: usize,
    temps_at: usize,
    width: usize,
    code: Vec<Ins>,
    arrays: Vec<ArrIns>,
    consts: Vec<f64>,
    /// Temporaries in use and the most ever in use: scalars (not reused
    /// when there are arrays, so a phase of cells can compute those it
    /// reads first), arrays.
    scalars: usize,
    max_scalars: usize,
    temps: usize,
    max_temps: usize,
}

fn has_var(e: &Expr) -> bool {
    match e {
        Expr::Num(_) => false,
        Expr::Var(_) => true,
        Expr::Neg(a) => has_var(a),
        Expr::Bin(_, a, b) => has_var(a) || has_var(b),
        Expr::Call(_, args) => args.iter().any(has_var),
        Expr::Array(..) => true,
    }
}

fn reg(i: usize) -> u32 {
    u32::try_from(i).expect("fewer than 2^32 registers")
}

impl Builder<'_> {
    /// How many values `e` has: 1, or the length of the arrays it uses.
    fn len(&self, e: &Expr) -> usize {
        match e {
            Expr::Num(_) => 1,
            Expr::Var(i) => self.vars[*i].len,
            Expr::Neg(a) => self.len(a),
            Expr::Bin(_, a, b) => self.len(a).max(self.len(b)),
            Expr::Call(_, args) => args.iter().map(|a| self.len(a)).max().unwrap_or(1),
            Expr::Array(op, args) => match op {
                ArrayOp::Axis { cells, .. } => *cells,
                ArrayOp::Second { .. } | ArrayOp::First { .. } | ArrayOp::Shift { .. } => {
                    self.len(&args[0])
                }
                ArrayOp::Gather { rank, ref dims, .. } => dims[..*rank].iter().product(),
                ArrayOp::Solve { n, .. } => *n,
                ArrayOp::Take => self.len(&args[1]),
                ArrayOp::Along {
                    rank,
                    ref dims,
                    axis,
                    ..
                } => (0..*rank).filter(|k| k != axis).map(|k| dims[k]).product(),
                _ => 1,
            },
        }
    }

    fn constant(&mut self, v: f64) -> Src {
        let k = match self.consts.iter().position(|c| c.to_bits() == v.to_bits()) {
            Some(k) => k,
            None => {
                self.consts.push(v);
                self.consts.len() - 1
            }
        };
        Src {
            at: self.consts_at + k,
            array: false,
        }
    }

    /// Where the values of `e` can be read: a variable or constant as it
    /// is, else a temporary they are computed into.
    fn operand(&mut self, e: &Expr) -> Src {
        if !has_var(e) {
            return self.constant(e.eval(&[]));
        }
        if let Expr::Var(i) = e {
            let s = self.vars[*i];
            return Src {
                at: s.at,
                array: s.len > 1,
            };
        }
        let n = self.len(e);
        let at = if n == 1 {
            self.scalars += 1;
            self.max_scalars = self.max_scalars.max(self.scalars);
            self.scalars_at + self.scalars - 1
        } else {
            self.temps += 1;
            self.max_temps = self.max_temps.max(self.temps);
            self.temps_at + (self.temps - 1) * self.width
        };
        self.emit(e, at, n);
        Src { at, array: n > 1 }
    }

    /// A one-cell result at `d` spread over the `n` cells written there.
    fn spread(&mut self, d: usize, cells: usize, n: usize) {
        if cells == 1 && n > 1 {
            let a = Src {
                at: d,
                array: false,
            };
            self.array(ArrIns::Copy { d, a, n });
        }
    }

    fn array(&mut self, ins: ArrIns) {
        self.arrays.push(ins);
        self.code.push(Ins::Arr(reg(self.arrays.len() - 1)));
    }

    /// Compute `e` into the `n` registers from `d` (a scalar `e` fills all
    /// `n`). Temporaries taken for its operands are free again after.
    fn emit(&mut self, e: &Expr, d: usize, n: usize) {
        let mark = (self.scalars, self.temps);
        let scalar = n == 1;
        let dr = reg(d);
        match e {
            _ if !has_var(e) || matches!(e, Expr::Var(_)) => {
                let a = self.operand(e);
                if scalar {
                    self.code.push(Ins::Copy(dr, reg(a.at)));
                } else {
                    self.array(ArrIns::Copy { d, a, n });
                }
            }
            Expr::Neg(a) => {
                let a = self.operand(a);
                if scalar {
                    self.code.push(Ins::Neg(dr, reg(a.at)));
                } else {
                    self.array(ArrIns::Neg { d, a, n });
                }
            }
            Expr::Bin(op, a, b) => {
                let square = *op == Op::Pow && matches!(**b, Expr::Num(v) if v == 2.0);
                let a = self.operand(a);
                let (op, b) = if square {
                    (Op::Mul, a)
                } else {
                    (*op, self.operand(b))
                };
                if scalar {
                    let (a, b) = (reg(a.at), reg(b.at));
                    self.code.push(match op {
                        _ if square => Ins::Sqr(dr, a),
                        Op::Add => Ins::Add(dr, a, b),
                        Op::Sub => Ins::Sub(dr, a, b),
                        Op::Mul => Ins::Mul(dr, a, b),
                        Op::Div => Ins::Div(dr, a, b),
                        Op::Pow => Ins::Pow(dr, a, b),
                        op => Ins::Cmp(op, dr, a, b),
                    });
                } else {
                    self.array(ArrIns::Bin { op, d, a, b, n });
                }
            }
            Expr::Call(f, args) => {
                let a = self.operand(&args[0]);
                match args.get(1) {
                    Some(b) => {
                        let b = self.operand(b);
                        if scalar {
                            self.code.push(Ins::Call2(*f, dr, reg(a.at), reg(b.at)));
                        } else {
                            self.array(ArrIns::Call2 { f: *f, d, a, b, n });
                        }
                    }
                    None if scalar => self.code.push(Ins::Call1(*f, dr, reg(a.at))),
                    None => self.array(ArrIns::Call1 { f: *f, d, a, n }),
                }
            }
            Expr::Array(op, args) => {
                let srcs: Vec<Src> = args.iter().map(|a| self.operand(a)).collect();
                let len = self.len(&args[0]);
                let edge = |e: Edge, vars: &[Span]| match e {
                    Edge::Value(v) => EdgeAt::Value(vars[v].at),
                    Edge::Gradient(g) => EdgeAt::Gradient(g.map(|g| vars[g].at)),
                    Edge::Periodic => EdgeAt::Periodic,
                };
                let one = |k: usize| -> usize { srcs[k].at };
                match *op {
                    ArrayOp::Axis { line, cells } => self.array(ArrIns::Axis {
                        d,
                        lo: one(0),
                        h: one(1),
                        n: cells,
                        line,
                    }),
                    ArrayOp::Second { line, edges: e } | ArrayOp::First { line, edges: e } => {
                        assert_ne!(srcs[0].at, d, "a stencil writes elsewhere than it reads");
                        self.array(ArrIns::Stencil {
                            second: matches!(op, ArrayOp::Second { .. }),
                            d,
                            a: one(0),
                            h: one(1),
                            n: len,
                            line,
                            edges: [edge(e[0], self.vars), edge(e[1], self.vars)],
                        })
                    }
                    ArrayOp::Shift { line, edges: e, up } => self.array(ArrIns::Shift {
                        up,
                        d,
                        a: one(0),
                        h: one(1),
                        n: len,
                        line,
                        edges: [edge(e[0], self.vars), edge(e[1], self.vars)],
                    }),
                    ArrayOp::At(i) => {
                        let at = one(0) + i.min(len - 1);
                        let a = Src { at, array: false };
                        if scalar {
                            self.code.push(Ins::Copy(dr, reg(at)));
                        } else {
                            self.array(ArrIns::Copy { d, a, n });
                        }
                    }
                    ArrayOp::Cell => {
                        // Into a register of its own, then copied where it
                        // goes.
                        self.array(ArrIns::Pick {
                            d,
                            a: one(0),
                            n: len,
                            i: one(1),
                        });
                        if !scalar {
                            let a = Src {
                                at: d,
                                array: false,
                            };
                            self.array(ArrIns::Copy { d, a, n });
                        }
                    }
                    ArrayOp::Take => {
                        let m = self.len(&args[1]);
                        self.array(ArrIns::Take {
                            d,
                            a: one(0),
                            n: len,
                            i: one(1),
                            m,
                        });
                        self.spread(d, m, n);
                    }
                    ArrayOp::Gather {
                        rank,
                        dims,
                        strides,
                    } => {
                        let cells: usize = dims[..rank].iter().product();
                        self.array(ArrIns::Gather {
                            d,
                            a: one(0),
                            n: cells,
                            rank,
                            dims,
                            strides,
                        });
                        self.spread(d, cells, n);
                    }
                    ArrayOp::Rcond { n: m, rows, cols } => {
                        self.array(ArrIns::Rcond {
                            d,
                            a: one(0),
                            n: m,
                            rows,
                            cols,
                        });
                        self.spread(d, 1, n);
                    }
                    ArrayOp::Solve {
                        n: cells,
                        rows,
                        cols,
                    } => {
                        self.array(ArrIns::Solve {
                            d,
                            a: one(0),
                            b: one(1),
                            n: cells,
                            rows,
                            cols,
                        });
                        self.spread(d, cells, n);
                    }
                    ArrayOp::Along {
                        of,
                        rank,
                        dims,
                        axis,
                    } => {
                        let op = match of {
                            Along::Sum => Reduce::Sum,
                            Along::Mean => Reduce::Mean,
                            Along::MinOf => Reduce::Min,
                            Along::MaxOf => Reduce::Max,
                            Along::ArgMinOf => Reduce::ArgMin,
                            Along::ArgMaxOf => Reduce::ArgMax,
                        };
                        let cells: usize =
                            (0..rank).filter(|&k| k != axis).map(|k| dims[k]).product();
                        self.array(ArrIns::Along {
                            op,
                            d,
                            a: one(0),
                            n: cells,
                            rank,
                            dims,
                            axis,
                        });
                        self.spread(d, cells, n);
                    }
                    ArrayOp::Sum
                    | ArrayOp::Mean
                    | ArrayOp::MinOf
                    | ArrayOp::MaxOf
                    | ArrayOp::ArgMinOf
                    | ArrayOp::ArgMaxOf => {
                        let op = match op {
                            ArrayOp::Sum => Reduce::Sum,
                            ArrayOp::Mean => Reduce::Mean,
                            ArrayOp::MinOf => Reduce::Min,
                            ArrayOp::MaxOf => Reduce::Max,
                            ArrayOp::ArgMinOf => Reduce::ArgMin,
                            _ => Reduce::ArgMax,
                        };
                        self.array(ArrIns::Reduce {
                            op,
                            d,
                            a: one(0),
                            n: len,
                        });
                        if !scalar {
                            let a = Src {
                                at: d,
                                array: false,
                            };
                            self.array(ArrIns::Copy { d, a, n });
                        }
                    }
                }
            }
            Expr::Num(_) | Expr::Var(_) => unreachable!("copied above"),
        }
        // Scalar temporaries are reused only without arrays, where no
        // phase of cells can form.
        if self.width == 1 {
            self.scalars = mark.0;
        }
        self.temps = mark.1;
    }
}

impl Program {
    /// Compile `steps`, run in order: each writes its expression into the
    /// register `target`, one of the first `vars` registers, where later
    /// steps can read it. Expressions read registers below `vars`. For
    /// scalars only; see [`Program::with_layout`] for arrays.
    pub fn new(vars: usize, steps: &[(usize, &Expr)]) -> Self {
        let spans: Vec<Span> = (0..vars).map(|at| Span { at, len: 1 }).collect();
        Self::with_layout(&spans, vars, steps)
    }

    /// Compile `steps` over variables laid out as `vars` in the first
    /// `size` registers: `Expr::Var(i)` is `vars[i]`, and each step writes
    /// the variable it names. An array target takes an array value cell by
    /// cell, or a scalar in every cell.
    pub fn with_layout(vars: &[Span], size: usize, steps: &[(usize, &Expr)]) -> Self {
        assert!(
            vars.iter().all(|s| s.at + s.len <= size),
            "variables lie in the buffer"
        );
        // Room for the longest variable or value along the way (ADR 0080:
        // an array over more axes than any variable).
        let longest = steps
            .iter()
            .map(|(_, e)| longest(e, vars))
            .max()
            .unwrap_or(1);
        let width = vars
            .iter()
            .map(|s| s.len)
            .max()
            .unwrap_or(1)
            .max(longest)
            .max(1);
        let build = |consts: Vec<f64>, scalars: usize| {
            let mut b = Builder {
                vars,
                consts_at: size,
                scalars_at: size + consts.len(),
                temps_at: size + consts.len() + scalars,
                width,
                code: Vec::new(),
                arrays: Vec::new(),
                consts,
                scalars: 0,
                max_scalars: 0,
                temps: 0,
                max_temps: 0,
            };
            for (target, e) in steps {
                let s = vars[*target];
                b.emit(e, s.at, s.len);
            }
            b
        };
        // First to find the constants and scalar temporaries, then with
        // them in place so the array temporaries can follow.
        let first = build(Vec::new(), 0);
        let (n, scalars) = (first.consts.len(), first.max_scalars);
        let b = build(first.consts, scalars);
        assert_eq!(b.consts.len(), n, "the same constants both times");
        assert_eq!(b.max_scalars, scalars, "the same scalars both times");
        let mut template = vec![0.0; size];
        template.extend_from_slice(&b.consts);
        template.resize(b.temps_at + b.max_temps * width, 0.0);
        // Steps only when a phase is large enough.
        let mut steps = plan(&b.code, &b.arrays);
        let largest = steps
            .iter()
            .map(|s| match s {
                Step::Cells(p) => p.work(),
                Step::One(_) => 0,
            })
            .max()
            .unwrap_or(0);
        if largest < crate::cells::PLAN {
            steps.clear();
        }
        Self {
            code: b.code,
            arrays: b.arrays,
            template,
            steps,
            split: largest >= crate::cells::WORTH,
            outside: false,
        }
    }

    /// A register buffer for [`Program::run`]: the variables (all 0, for
    /// the caller to fill), constants and temporaries.
    pub fn registers(&self) -> Vec<f64> {
        self.template.clone()
    }

    /// The number of instructions.
    pub fn len(&self) -> usize {
        self.code.len()
    }

    pub fn is_empty(&self) -> bool {
        self.code.is_empty()
    }

    /// Which of the registers `from` each register in `to` depends on,
    /// as offsets into `from`: the structure of the program's Jacobian
    /// (ADR 0060). Found by running the program on sets of dependencies
    /// instead of values, so it is exact for every value: a cell of a
    /// stencil depends on its neighbours (and on the far end when
    /// periodic), a reduction on every cell.
    pub fn dependencies(
        &self,
        from: std::ops::Range<usize>,
        to: std::ops::Range<usize>,
    ) -> Vec<Vec<usize>> {
        let mut deps: Vec<Vec<usize>> = vec![Vec::new(); self.template.len()];
        for (k, r) in from.clone().enumerate() {
            deps[r] = vec![k];
        }
        for ins in &self.code {
            match *ins {
                Ins::Copy(d, a) | Ins::Neg(d, a) | Ins::Sqr(d, a) | Ins::Call1(_, d, a) => {
                    deps[d as usize] = deps[a as usize].clone();
                }
                Ins::Add(d, a, b)
                | Ins::Sub(d, a, b)
                | Ins::Mul(d, a, b)
                | Ins::Div(d, a, b)
                | Ins::Pow(d, a, b)
                | Ins::Cmp(_, d, a, b)
                | Ins::Call2(_, d, a, b) => {
                    deps[d as usize] = union(&[&deps[a as usize], &deps[b as usize]]);
                }
                Ins::Arr(k) => self.arrays[k as usize].dependencies(&mut deps),
            }
        }
        to.map(|r| std::mem::take(&mut deps[r])).collect()
    }

    /// Run every step on `r`, a buffer from [`Program::registers`] (or
    /// one at least as long, with the same constants).
    pub fn run(&self, r: &mut [f64]) {
        // Large arrays: by phases, on several cores (the same bits).
        if self.split && crate::cells::cores() > 1 {
            self.run_steps(&self.steps, r, None, None);
            return;
        }
        self.run_in_order(r);
    }

    /// Let the program read the arrays in registers `reads` and write
    /// those in `writes` where the caller keeps them, with
    /// [`Program::run_outside`], instead of in its registers: arrays only
    /// phases of cells read (from `reads`), or only they write and nothing
    /// reads (in `writes`). Returns those it placed, read and written; the
    /// caller fills the other registers and copies the other results as
    /// for [`Program::run`]. None for small arrays, without phases.
    pub fn place_outside(
        &mut self,
        reads: std::ops::Range<usize>,
        writes: std::ops::Range<usize>,
    ) -> (Vec<Span>, Vec<Span>) {
        // Registers touched other than as a phase's arrays, the arrays
        // phases write and those they read.
        let mut busy: Vec<(usize, usize)> = Vec::new();
        let mut written: Vec<usize> = Vec::new();
        let mut used: Vec<usize> = Vec::new();
        let mut arrays: Vec<Span> = Vec::new();
        let scalar = |ins: Ins, busy: &mut Vec<(usize, usize)>| {
            let (d, regs) = scalar_regs(ins).expect("a scalar instruction");
            busy.push((d, 1));
            busy.extend(regs.into_iter().map(|r| (r, 1)));
        };
        for step in &self.steps {
            match step {
                Step::One(i) => match self.code[*i] {
                    Ins::Arr(k) => busy.extend(self.arrays[k as usize].touches()),
                    ins => scalar(ins, &mut busy),
                },
                Step::Cells(p) => {
                    for &i in &p.first {
                        match self.code[i] {
                            Ins::Arr(k) => busy.extend(self.arrays[k as usize].touches()),
                            ins => scalar(ins, &mut busy),
                        }
                    }
                    for &at in p.written.iter().chain(&p.read) {
                        arrays.push(Span { at, len: p.n });
                    }
                    for op in &p.ops {
                        written.push(p.written[op.d]);
                        for o in [op.a, op.b] {
                            match o {
                                Operand::Written(k) => used.push(p.written[k]),
                                Operand::Read(k) => used.push(p.read[k]),
                                Operand::Scalar(at) => busy.push((at, 1)),
                            }
                        }
                        if let Kind::Stencil { edges, .. } | Kind::Shift { edges, .. } = op.kind {
                            for e in edges {
                                match e {
                                    EdgeAt::Value(v) | EdgeAt::Gradient(Some(v)) => {
                                        busy.push((v, 1))
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        }
        arrays.sort_unstable_by_key(|s| s.at);
        arrays.dedup();
        let free = |s: &Span| !busy.iter().any(|&(b, m)| b < s.at + s.len && s.at < b + m);
        let within =
            |s: &Span, r: &std::ops::Range<usize>| r.start <= s.at && s.at + s.len <= r.end;
        let read_out: Vec<Span> = arrays
            .iter()
            .filter(|s| within(s, &reads) && !written.contains(&s.at) && free(s))
            .copied()
            .collect();
        let write_out: Vec<Span> = arrays
            .iter()
            .filter(|s| {
                within(s, &writes) && written.contains(&s.at) && !used.contains(&s.at) && free(s)
            })
            .copied()
            .collect();
        for step in &mut self.steps {
            if let Step::Cells(p) = step {
                p.place(
                    |at| read_out.iter().any(|s| s.at == at),
                    |at| write_out.iter().any(|s| s.at == at),
                );
            }
        }
        self.outside = !read_out.is_empty() || !write_out.is_empty();
        (read_out, write_out)
    }

    /// Run as [`Program::run`], with the arrays placed outside read from
    /// and written to `out`.
    pub fn run_outside(&self, r: &mut [f64], mut out: Outside) {
        if self.outside {
            self.run_steps(&self.steps, r, None, Some(&mut out));
        } else {
            self.run(r);
        }
    }

    /// Run by `steps`: phases of cells split over the cores, as they
    /// choose or in `chunks` chunks, arrays outside in `out`.
    fn run_steps(
        &self,
        steps: &[Step],
        r: &mut [f64],
        chunks: Option<usize>,
        mut out: Option<&mut Outside>,
    ) {
        for step in steps {
            match step {
                Step::One(i) => self.one(self.code[*i], r),
                Step::Cells(phase) => {
                    for &i in &phase.first {
                        self.one(self.code[i], r);
                    }
                    match chunks {
                        Some(c) => phase.run_in(r, c, out.as_deref_mut()),
                        None => phase.run(r, out.as_deref_mut()),
                    }
                }
            }
        }
    }

    /// Run every instruction in order, on one core.
    fn run_in_order(&self, r: &mut [f64]) {
        for &ins in &self.code {
            self.one(ins, r);
        }
    }

    /// Run one instruction.
    fn one(&self, ins: Ins, r: &mut [f64]) {
        {
            match ins {
                Ins::Copy(d, a) => r[d as usize] = r[a as usize],
                Ins::Neg(d, a) => r[d as usize] = -r[a as usize],
                Ins::Add(d, a, b) => r[d as usize] = r[a as usize] + r[b as usize],
                Ins::Sub(d, a, b) => r[d as usize] = r[a as usize] - r[b as usize],
                Ins::Mul(d, a, b) => r[d as usize] = r[a as usize] * r[b as usize],
                Ins::Div(d, a, b) => r[d as usize] = r[a as usize] / r[b as usize],
                Ins::Pow(d, a, b) => r[d as usize] = r[a as usize].powf(r[b as usize]),
                Ins::Cmp(op, d, a, b) => r[d as usize] = op.apply(r[a as usize], r[b as usize]),
                Ins::Sqr(d, a) => {
                    let x = r[a as usize];
                    r[d as usize] = x * x;
                }
                Ins::Call1(f, d, a) => r[d as usize] = f.apply(r[a as usize], 0.0),
                Ins::Call2(f, d, a, b) => r[d as usize] = f.apply(r[a as usize], r[b as usize]),
                Ins::Arr(k) => self.arrays[k as usize].run(r),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    const NAMES: [&str; 4] = ["t", "x", "y", "k"];

    fn expr(src: &str) -> Expr {
        parse(src)
            .unwrap()
            .resolve(&|n| NAMES.iter().position(|m| *m == n))
            .unwrap()
    }

    /// The program's value and the tree's, for t, x, y, k = `vals`.
    fn both(src: &str, vals: [f64; 4]) -> (f64, f64) {
        let e = expr(src);
        // One result register after the names.
        let p = Program::new(5, &[(4, &e)]);
        let mut r = p.registers();
        r[..4].copy_from_slice(&vals);
        p.run(&mut r);
        (r[4], e.eval(&vals))
    }

    #[test]
    fn programs_give_the_trees_values_bit_for_bit() {
        let sources = [
            "x",
            "2.5",
            "-x",
            "x + y*k - t/3",
            "-(x - y)^3 + k^-1",
            "sin(x)*cos(y) + atan2(y, x) - max(x, min(y, k))",
            "exp(-k*t)*sqrt(abs(x*y)) + ln(2)*pi",
            "x*(y - 2*pi*k) - (1 + 2)^2*x",
            "2^3^2*x/(1 - 4/2)",
            "((x + y)*(x - y))/((k + 1)*(k - 1) + t)",
        ];
        for vals in [[0.0, 1.5, -2.25, 3.0], [1.0, -0.3, 7.0, 0.5]] {
            for src in sources {
                let (prog, tree) = both(src, vals);
                assert_eq!(prog.to_bits(), tree.to_bits(), "{src} at {vals:?}");
            }
        }
        // x^2 is x*x.
        let (prog, _) = both("x^2 + y^2", [0.0, 0.1, 3.0, 0.0]);
        assert_eq!(prog, 0.1 * 0.1 + 3.0 * 3.0);
    }

    #[test]
    fn constants_are_folded_and_steps_read_earlier_results() {
        let e = expr("(1 + 2)*(4 - 1)*x");
        let p = Program::new(5, &[(4, &e)]);
        // One multiplication by the folded 9.
        assert_eq!(p.len(), 1);
        // A defined quantity (register 3) read by the next step.
        let (q, r) = (expr("x*y"), expr("k + 1"));
        let p = Program::new(5, &[(3, &q), (4, &r)]);
        let mut regs = p.registers();
        regs[..3].copy_from_slice(&[0.0, 2.0, 5.0]);
        p.run(&mut regs);
        assert_eq!((regs[3], regs[4]), (10.0, 11.0));
        // Temporaries are reused: nine operations need five, beside the
        // five variables and five constants.
        let deep = expr("((((x + 1)*(y + 2)) + ((x + 3)*(y + 4)))*(k + 5))");
        let p = Program::new(5, &[(4, &deep)]);
        assert_eq!(p.len(), 9);
        assert_eq!(p.registers().len(), 5 + 5 + 5);
    }

    /// Variables: 0 the low end, 1 the spacing, 2 an edge value, 3 a scalar,
    /// 4 the axis (4 cells), 5 an array, 6 a result array, 7 a result
    /// scalar.
    fn layout() -> (Vec<Span>, usize) {
        let mut at = 0;
        let spans = [1, 1, 1, 1, 4, 4, 4, 1]
            .map(|len| {
                let s = Span { at, len };
                at += len;
                s
            })
            .to_vec();
        (spans, at)
    }

    fn run_on(steps: &[(usize, &Expr)], array: [f64; 4]) -> Vec<f64> {
        let (spans, size) = layout();
        let p = Program::with_layout(&spans, size, steps);
        let mut r = p.registers();
        // Low end 0, spacing 0.5, edge value 10, scalar 3.
        r[..4].copy_from_slice(&[0.0, 0.5, 10.0, 3.0]);
        r[spans[5].at..spans[5].at + 4].copy_from_slice(&array);
        p.run(&mut r);
        r[..size].to_vec()
    }

    fn var(i: usize) -> Box<Expr> {
        Box::new(Expr::Var(i))
    }

    #[test]
    fn arrays_broadcast_scalars_and_loop_over_cells() {
        let axis = Expr::Array(
            ArrayOp::Axis {
                line: Line::along(4),
                cells: 4,
            },
            vec![Expr::Var(0), Expr::Var(1)],
        );
        // result = 2*a + s*x, with the axis computed first.
        let e = Expr::Bin(
            Op::Add,
            Box::new(Expr::Bin(Op::Mul, Box::new(Expr::Num(2.0)), var(5))),
            Box::new(Expr::Bin(Op::Mul, var(3), var(4))),
        );
        let r = run_on(&[(4, &axis), (6, &e)], [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(&r[4..8], [0.25, 0.75, 1.25, 1.75]);
        assert_eq!(&r[12..16], [2.75, 6.25, 9.75, 13.25]);
        // A scalar into an array fills every cell; sin applies per cell.
        let fill = Expr::Call(Func::Sin, vec![Expr::Var(3)]);
        let r = run_on(&[(6, &fill)], [0.0; 4]);
        assert_eq!(&r[12..16], [3f64.sin(); 4]);
    }

    #[test]
    fn stencils_use_the_edges() {
        let lap = |edges| {
            Expr::Array(
                ArrayOp::Second {
                    line: Line::along(4),
                    edges,
                },
                vec![Expr::Var(5), Expr::Var(1)],
            )
        };
        let a = [1.0, 2.0, 4.0, 8.0];
        // Insulated: the ghosts repeat the end cells. h = 0.5, h^2 = 0.25.
        let r = run_on(&[(6, &lap([Edge::Gradient(None); 2]))], a);
        assert_eq!(&r[12..16], [4.0, 4.0, 8.0, -16.0]);
        // Held at 10 on both faces: ghosts 2*10 - end.
        let r = run_on(&[(6, &lap([Edge::Value(2); 2]))], a);
        assert_eq!(r[12], (19.0 - 2.0 + 2.0) / 0.25);
        assert_eq!(r[15], (4.0 - 16.0 + 12.0) / 0.25);
        // Periodic: the ends see each other.
        let r = run_on(&[(6, &lap([Edge::Periodic; 2]))], a);
        assert_eq!(r[12], (8.0 - 2.0 + 2.0) / 0.25);
        assert_eq!(r[15], (4.0 - 16.0 + 1.0) / 0.25);
        // A gradient of 10 outward: ghost = end + 10*h.
        let d = Expr::Array(
            ArrayOp::First {
                line: Line::along(4),
                edges: [Edge::Gradient(Some(2)), Edge::Gradient(Some(2))],
            },
            vec![Expr::Var(5), Expr::Var(1)],
        );
        let r = run_on(&[(6, &d)], a);
        assert_eq!(
            &r[12..16],
            [(2.0 - 6.0) / 1.0, 3.0, 6.0, (13.0 - 4.0) / 1.0]
        );
    }

    #[test]
    fn large_arrays_on_several_cores_give_the_same_bits() {
        // A 128 × 128 grid: u, v and two results, with scalars around.
        let n = 128 * 128;
        let mut at = 0;
        let spans: Vec<Span> = [1, 1, 1, n, n, n, n, 1]
            .map(|len| {
                let s = Span { at, len };
                at += len;
                s
            })
            .to_vec();
        let (h, lo, k, u, v, out1, out2, total) = (0, 1, 2, 3, 4, 5, 6, 7);
        let x = Line {
            stride: 1,
            count: 128,
        };
        let y = Line {
            stride: 128,
            count: 128,
        };
        let var = |i| Box::new(Expr::Var(i));
        let bin = |op, a: Box<Expr>, b: Box<Expr>| Box::new(Expr::Bin(op, a, b));
        let arr = |op, args: Vec<Expr>| Box::new(Expr::Array(op, args));
        let lap = |line, edges| {
            arr(
                ArrayOp::Second { line, edges },
                vec![Expr::Var(u), Expr::Var(h)],
            )
        };
        // out1 = lap_x(u) + lap_y(u) - u*v*v + k*(1 - u) + sin(v), the
        // reaction's u less its mean (a reduction run before the phase)
        let centred = bin(
            Op::Sub,
            var(u),
            Box::new(Expr::Array(ArrayOp::Mean, vec![Expr::Var(u)])),
        );
        let reaction = bin(Op::Mul, bin(Op::Mul, centred, var(v)), var(v));
        let feed = bin(
            Op::Mul,
            var(k),
            bin(Op::Sub, Box::new(Expr::Num(1.0)), var(u)),
        );
        let e1 = bin(
            Op::Add,
            bin(
                Op::Add,
                bin(
                    Op::Sub,
                    bin(
                        Op::Add,
                        lap(x, [Edge::Periodic; 2]),
                        lap(y, [Edge::Value(k), Edge::Gradient(Some(k))]),
                    ),
                    reaction,
                ),
                feed,
            ),
            Box::new(Expr::Call(Func::Sin, vec![Expr::Var(v)])),
        );
        // out2 = shifted v + d/dy(u) + the x axis + mean(out1)*v + u at 5
        let e2 = bin(
            Op::Add,
            bin(
                Op::Add,
                bin(
                    Op::Add,
                    arr(
                        ArrayOp::Shift {
                            line: y,
                            edges: [Edge::Value(k), Edge::Periodic],
                            up: true,
                        },
                        vec![Expr::Var(v), Expr::Var(h)],
                    ),
                    arr(
                        ArrayOp::First {
                            line: y,
                            edges: [Edge::Gradient(None); 2],
                        },
                        vec![Expr::Var(u), Expr::Var(h)],
                    ),
                ),
                arr(
                    ArrayOp::Axis { line: x, cells: n },
                    vec![Expr::Var(lo), Expr::Var(h)],
                ),
            ),
            bin(
                Op::Add,
                bin(Op::Mul, arr(ArrayOp::Mean, vec![Expr::Var(out1)]), var(v)),
                arr(ArrayOp::At(5), vec![Expr::Var(u)]),
            ),
        );
        let e3 = arr(ArrayOp::Sum, vec![Expr::Var(out2)]);
        let p = Program::with_layout(&spans, at, &[(out1, &e1), (out2, &e2), (total, &e3)]);
        // Planned whatever the size, run in several chunks.
        let steps = plan(&p.code, &p.arrays);
        let phases = steps.iter().filter(|s| matches!(s, Step::Cells(_))).count();
        assert!(phases >= 1 && phases < steps.len(), "{steps:?}");
        // The mean of u runs before the first phase, not between phases.
        let Some(Step::Cells(first)) = steps.iter().find(|s| matches!(s, Step::Cells(_))) else {
            unreachable!("a phase")
        };
        assert!(first
            .first
            .iter()
            .any(|&i| matches!(p.code[i], Ins::Arr(k) if matches!(p.arrays[k as usize], ArrIns::Reduce { .. }))));
        let mut r = p.registers();
        r[h] = 0.01;
        r[lo] = -0.5;
        r[k] = 0.3;
        for c in 0..n {
            r[spans[u].at + c] = (c as f64 * 0.37).sin();
            r[spans[v].at + c] = (c as f64 * 0.11).cos();
        }
        let mut in_order = r.clone();
        p.run_in_order(&mut in_order);
        let bits = |r: &[f64]| r.iter().map(|v| v.to_bits()).collect::<Vec<_>>();
        for chunks in [1, 2, 3, 7] {
            let mut split = r.clone();
            p.run_steps(&steps, &mut split, Some(chunks), None);
            assert_eq!(bits(&split), bits(&in_order), "{chunks} chunks");
        }
        assert!(in_order[spans[total].at].is_finite());
    }

    #[test]
    fn arrays_outside_the_registers_give_the_same_bits() {
        // Gray–Scott's rates: u and v read, du and dv written, each only
        // by phases of cells, so all four can stay with the caller.
        let n = 128 * 128;
        let mut at = 0;
        let spans: Vec<Span> = [1, 1, n, n, n, n, 1]
            .map(|len| {
                let s = Span { at, len };
                at += len;
                s
            })
            .to_vec();
        let (h, k, u, v, du, dv, total) = (0, 1, 2, 3, 4, 5, 6);
        let line = Line {
            stride: 128,
            count: 128,
        };
        let var = |i| Box::new(Expr::Var(i));
        let bin = |op, a: Box<Expr>, b: Box<Expr>| Box::new(Expr::Bin(op, a, b));
        let lap = |of| {
            Box::new(Expr::Array(
                ArrayOp::Second {
                    line,
                    edges: [Edge::Periodic; 2],
                },
                vec![Expr::Var(of), Expr::Var(h)],
            ))
        };
        let uvv = || bin(Op::Mul, bin(Op::Mul, var(u), var(v)), var(v));
        let feed = bin(
            Op::Mul,
            var(k),
            bin(Op::Sub, Box::new(Expr::Num(1.0)), var(u)),
        );
        let e_du = bin(Op::Add, bin(Op::Sub, lap(u), uvv()), feed);
        let e_dv = bin(
            Op::Sub,
            bin(Op::Add, lap(v), uvv()),
            bin(Op::Mul, var(k), var(v)),
        );
        // A sum of u read by a single instruction keeps u in the registers.
        let e_total = Expr::Array(ArrayOp::Sum, vec![Expr::Var(u)]);
        let p = Program::with_layout(&spans, at, &[(du, &e_du), (dv, &e_dv), (total, &e_total)]);
        let mut r = p.registers();
        r[h] = 0.01;
        r[k] = 0.04;
        for c in 0..n {
            r[spans[u].at + c] = (c as f64 * 0.37).sin();
            r[spans[v].at + c] = (c as f64 * 0.11).cos();
        }
        let mut want = r.clone();
        p.run_in_order(&mut want);

        let mut q = p.clone();
        let states = spans[u].at..spans[v].at + n;
        let rates = spans[du].at..spans[dv].at + n;
        let (read, written) = q.place_outside(states.clone(), rates.clone());
        assert_eq!(read, vec![spans[v]], "u is summed in the registers");
        assert_eq!(written, vec![spans[du], spans[dv]]);
        // The registers hold u but not v; the rates go to `out`.
        let mut regs = r.clone();
        regs[spans[v].at..spans[v].at + n].fill(f64::NAN);
        let mut out = vec![f64::NAN; 2 * n];
        q.run_outside(
            &mut regs,
            Outside {
                reads: &r[states.clone()],
                reads_at: states.start,
                writes: &mut out,
                writes_at: rates.start,
            },
        );
        let bits = |r: &[f64]| r.iter().map(|v| v.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&out), bits(&want[rates]));
        assert_eq!(
            regs[spans[total].at].to_bits(),
            want[spans[total].at].to_bits()
        );
        // Run plainly, the placed program still gives the same bits.
        let mut plain = r.clone();
        q.run(&mut plain);
        assert_eq!(bits(&plain), bits(&want));
    }

    /// A sum's definition (ADR 0077): up to `SUM_CHUNK` cells one after
    /// another; past it, chunks in four lanes, added in order; the same
    /// bits on several cores.
    #[test]
    fn sums_follow_their_fixed_rule_on_any_number_of_cores() {
        let cells: Vec<f64> = (0..(3 * SUM_CHUNK + 7))
            .map(|i| ((i as f64) * 0.618).sin() * 10f64.powi((i % 7) as i32 - 3))
            .collect();
        let small = &cells[..SUM_CHUNK];
        assert_eq!(sum(small).to_bits(), small.iter().sum::<f64>().to_bits());
        let by_rule = |c: &[f64]| -> f64 {
            c.chunks(SUM_CHUNK)
                .map(|chunk| {
                    let mut lanes = [0.0f64; 4];
                    for (i, &x) in chunk.iter().enumerate() {
                        lanes[i % 4] += x;
                    }
                    (lanes[0] + lanes[1]) + (lanes[2] + lanes[3])
                })
                .sum()
        };
        assert_eq!(sum(&cells).to_bits(), by_rule(&cells).to_bits());
        assert_eq!(
            sum_split(&cells, true).to_bits(),
            sum_split(&cells, false).to_bits()
        );
        // As accurate as one after another, here more: against the sum
        // done exactly in integers.
        let ints: Vec<f64> = (0..(5 * SUM_CHUNK))
            .map(|i| ((i * 7919) % 1000) as f64 * 0.001 + 1e6)
            .collect();
        let exact: i128 = (0..(5 * SUM_CHUNK))
            .map(|i| ((i * 7919) % 1000) as i128 + 1_000_000_000)
            .sum();
        let exact = exact as f64 / 1000.0;
        assert!((sum(&ints) - exact).abs() <= (ints.iter().sum::<f64>() - exact).abs());
    }

    #[test]
    fn reductions_and_elements_give_scalars() {
        let a = [1.0, -2.0, 4.0, 8.0];
        for (op, want) in [
            (ArrayOp::Sum, 11.0),
            (ArrayOp::Mean, 2.75),
            (ArrayOp::MinOf, -2.0),
            (ArrayOp::MaxOf, 8.0),
            (ArrayOp::At(2), 4.0),
        ] {
            let e = Expr::Array(op, vec![Expr::Var(5)]);
            assert_eq!(run_on(&[(7, &e)], a)[16], want, "{op:?}");
            // As part of a larger expression, and spread over an array.
            let e2 = Expr::Bin(Op::Mul, Box::new(e.clone()), Box::new(Expr::Num(2.0)));
            assert_eq!(run_on(&[(7, &e2)], a)[16], 2.0 * want);
            assert_eq!(&run_on(&[(6, &e)], a)[12..16], [want; 4]);
        }
    }

    #[test]
    fn dependencies_follow_stencils_edges_and_reductions() {
        // The array (variable 5) is the input; results 6 (array), 7 (scalar).
        let (spans, size) = layout();
        let a = spans[5].at;
        let lap = |edges| {
            Expr::Array(
                ArrayOp::Second {
                    line: Line::along(4),
                    edges,
                },
                vec![Expr::Var(5), Expr::Var(1)],
            )
        };
        let deps = |e: &Expr, to: usize| {
            let p = Program::with_layout(&spans, size, &[(to, e)]);
            let s = spans[to];
            p.dependencies(a..a + 4, s.at..s.at + s.len)
        };
        let held = deps(&lap([Edge::Value(2); 2]), 6);
        assert_eq!(held, [vec![0, 1], vec![0, 1, 2], vec![1, 2, 3], vec![2, 3]]);
        let periodic = deps(&lap([Edge::Periodic; 2]), 6);
        assert_eq!(periodic[0], [0, 1, 3]);
        assert_eq!(periodic[3], [0, 2, 3]);
        // Cell by cell: a cell depends on itself only.
        let scaled = Expr::Bin(Op::Mul, var(5), var(3));
        assert_eq!(deps(&scaled, 6), [vec![0], vec![1], vec![2], vec![3]]);
        // A reduction depends on every cell, and spreads that to every cell.
        let mean = Expr::Array(ArrayOp::Mean, vec![Expr::Var(5)]);
        assert_eq!(deps(&mean, 7), [vec![0, 1, 2, 3]]);
        assert_eq!(
            deps(&Expr::Bin(Op::Sub, var(5), Box::new(mean)), 6)[0],
            [0, 1, 2, 3]
        );
        assert_eq!(
            deps(&Expr::Array(ArrayOp::At(2), vec![Expr::Var(5)]), 7),
            [vec![2]]
        );
    }
}
