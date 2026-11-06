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

use crate::cells::{CellOp, Kind, Operand, Phase, Step};
use crate::{ArrayOp, Edge, Expr, Func, Line, Op};

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
                let cells = &r[a..a + n];
                r[d] = match op {
                    Reduce::Sum => cells.iter().sum(),
                    Reduce::Mean => cells.iter().sum::<f64>() / n as f64,
                    Reduce::Min => cells.iter().copied().fold(f64::INFINITY, f64::min),
                    Reduce::Max => cells.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                    Reduce::ArgMin => arg_extreme(cells, false),
                    Reduce::ArgMax => arg_extreme(cells, true),
                };
            }
            ArrIns::Pick { d, a, n, i } => {
                let k = r[i].round();
                r[d] = if k >= 0.0 && k < n as f64 {
                    r[a + k as usize]
                } else {
                    f64::NAN
                };
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
    /// For arrays large enough to be worth several cores: the code as
    /// phases, stencils and single instructions (ADR 0062 step 3); empty
    /// otherwise.
    steps: Vec<Step>,
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
        let width = vars.iter().map(|s| s.len).max().unwrap_or(1).max(1);
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
        // Steps only when a phase is large enough to share.
        let mut steps = plan(&b.code, &b.arrays);
        let worth = steps.iter().any(|s| match s {
            Step::Cells(p) => p.work() >= crate::cells::WORTH,
            Step::One(_) => false,
        });
        if !worth {
            steps.clear();
        }
        Self {
            code: b.code,
            arrays: b.arrays,
            template,
            steps,
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
        if !self.steps.is_empty() && crate::cells::cores() > 1 {
            self.run_steps(&self.steps, r, None);
            return;
        }
        self.run_in_order(r);
    }

    /// Run by `steps`: phases of cells split over the cores, as they
    /// choose or in `chunks` chunks.
    fn run_steps(&self, steps: &[Step], r: &mut [f64], chunks: Option<usize>) {
        for step in steps {
            match step {
                Step::One(i) => self.one(self.code[*i], r),
                Step::Cells(phase) => {
                    for &i in &phase.first {
                        self.one(self.code[i], r);
                    }
                    match chunks {
                        Some(c) => phase.run_in(r, c),
                        None => phase.run(r),
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
        // out1 = lap_x(u) + lap_y(u) - u*v*v + k*(1 - u) + sin(v)
        let reaction = bin(Op::Mul, bin(Op::Mul, var(u), var(v)), var(v));
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
            p.run_steps(&steps, &mut split, Some(chunks));
            assert_eq!(bits(&split), bits(&in_order), "{chunks} chunks");
        }
        assert!(in_order[spans[total].at].is_finite());
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
