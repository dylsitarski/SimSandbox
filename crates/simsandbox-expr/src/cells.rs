//! Large arrays on several cores (ADR 0062 step 3).
//!
//! A [`Program`](crate::program::Program) whose arrays are large runs by
//! [`Step`]s. A **phase** gathers consecutive operations over arrays of one
//! length that each fill their result cell by cell: arithmetic, functions,
//! axes, and stencils and shifts of arrays the phase does not write (read
//! whole, so a cell may look at its neighbours). Scalar instructions among
//! them run first. The phase then runs its whole list on chunks of cells
//! at once, one chunk per core. Anything else (reductions, single cells)
//! runs in order between phases. Every cell is computed by the same
//! operations as one after another, so the results are the same bits in
//! any split; reductions keep their order.
//!
//! Whether sharing a phase among the cores is faster depends on its size
//! and the machine (threads cost time to start and to share memory): each
//! phase measures itself both ways and keeps the faster, trying the other
//! again now and then.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::program::{ghost, EdgeAt, EdgeValue};
use crate::{Func, Line, Op};

/// Below this many cell operations (a stencil counts four) a phase runs
/// on one core, without measuring: starting threads and sharing memory
/// between cores cost more than smaller phases gain (2026-11-05, a
/// 4-core virtual machine: a 128 × 128 Gray–Scott grid, about 2^17 per
/// phase, ran 10% slower on four cores, 256 × 256 the same, 512 × 512
/// 1.7 times faster).
pub(crate) const WORTH: usize = 1 << 20;

/// Below this many cell operations in every phase a program runs its
/// instructions in order, without phases: they would cost more to set up
/// than they save.
pub(crate) const PLAN: usize = 1 << 14;

/// How often a phase tries the way it does not use, by calls.
const RETRY: u64 = 64;

/// The cores available.
pub(crate) fn cores() -> usize {
    simsandbox_core::parallel::threads()
}

/// An operand: an array the phase writes (by its place among them), one it
/// only reads (read whole), or a scalar register (read once at the start:
/// nothing in the phase writes it).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Operand {
    Written(usize),
    Read(usize),
    Scalar(usize),
}

/// What an operation computes into its result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Kind {
    Copy,
    Neg,
    Bin(Op),
    Call1(Func),
    Call2(Func),
    /// Cell centres `lo + (j + 1/2)·h` along `line`; `a` is `lo`, `b` `h`.
    Axis(Line),
    /// The second (`second`) or first derivative of `a` (read whole) along
    /// `line`; `b` the spacing.
    Stencil {
        second: bool,
        line: Line,
        edges: [EdgeAt; 2],
    },
    /// The neighbour of `a` (read whole) before or after (`up`) along
    /// `line`; `b` the spacing.
    Shift {
        up: bool,
        line: Line,
        edges: [EdgeAt; 2],
    },
}

/// One operation: written array `d` from `a` and `b`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CellOp {
    pub kind: Kind,
    pub d: usize,
    pub a: Operand,
    pub b: Operand,
}

/// Operations over arrays of `n` cells: those written and those only read
/// (by first register), apart from each other.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Phase {
    pub n: usize,
    /// Scalar instructions run first, by their place in the code.
    pub first: Vec<usize>,
    pub written: Vec<usize>,
    pub read: Vec<usize>,
    pub ops: Vec<CellOp>,
    /// Which written and which read arrays lie [`Outside`] the registers
    /// when the program runs with them there.
    pub written_out: Vec<bool>,
    pub read_out: Vec<bool>,
    pub choice: Choice,
}

/// Arrays kept by the caller instead of in the registers: registers from
/// `reads_at` are read from `reads`, those from `writes_at` written to
/// `writes`, for the arrays the program placed there
/// ([`Program::place_outside`](crate::Program::place_outside)). Saves
/// copying large states in and rates out at every evaluation.
pub struct Outside<'a> {
    pub reads: &'a [f64],
    pub reads_at: usize,
    pub writes: &'a mut [f64],
    pub writes_at: usize,
}

/// What a phase took on one core and on several, smoothed over its recent
/// calls (nanoseconds as `f64` bits; 0 until measured), and its calls.
#[derive(Debug, Default)]
pub(crate) struct Choice {
    calls: AtomicU64,
    one: AtomicU64,
    many: AtomicU64,
}

impl Choice {
    /// Whether the next call runs on several cores.
    fn several(&self) -> bool {
        let call = self.calls.fetch_add(1, Ordering::Relaxed);
        let one = f64::from_bits(self.one.load(Ordering::Relaxed));
        let many = f64::from_bits(self.many.load(Ordering::Relaxed));
        if one == 0.0 || many == 0.0 {
            return one != 0.0;
        }
        let best = many < one;
        if call % RETRY == 0 {
            !best
        } else {
            best
        }
    }

    fn record(&self, several: bool, nanos: f64) {
        let slot = if several { &self.many } else { &self.one };
        let old = f64::from_bits(slot.load(Ordering::Relaxed));
        let new = if old == 0.0 {
            nanos
        } else {
            0.7 * old + 0.3 * nanos
        };
        slot.store(new.to_bits(), Ordering::Relaxed);
    }
}

/// Measurements are not part of what a phase is.
impl Clone for Choice {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl PartialEq for Choice {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// How a large program runs.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Step {
    /// One instruction, by its place in the code, as in order.
    One(usize),
    /// A phase.
    Cells(Phase),
}

/// An operand as the chunks read it.
#[derive(Clone, Copy)]
enum Value {
    Mine(usize),
    Whole(usize),
    Scalar(f64),
}

/// An operation as the chunks run it, its scalars read.
#[derive(Clone, Copy)]
struct Ready {
    kind: Kind,
    d: usize,
    a: Value,
    b: Value,
    edges: [EdgeValue; 2],
}

impl Phase {
    /// Place arrays outside the registers: those `read_out` says (read
    /// only, so moved to the read arrays) and those `write_out` says
    /// (written, never read).
    pub fn place(&mut self, read_out: impl Fn(usize) -> bool, write_out: impl Fn(usize) -> bool) {
        let moved: Vec<bool> = self.written.iter().map(|&at| read_out(at)).collect();
        if moved.iter().any(|&m| m) {
            let mut new_index = vec![Operand::Written(0); self.written.len()];
            let (mut written, mut read) = (Vec::new(), self.read.clone());
            for (k, &at) in self.written.iter().enumerate() {
                new_index[k] = if moved[k] {
                    read.push(at);
                    Operand::Read(read.len() - 1)
                } else {
                    written.push(at);
                    Operand::Written(written.len() - 1)
                };
            }
            let remap = |o: Operand| match o {
                Operand::Written(k) => new_index[k],
                o => o,
            };
            for op in &mut self.ops {
                let Operand::Written(d) = new_index[op.d] else {
                    unreachable!("an array read outside is not written")
                };
                op.d = d;
                op.a = remap(op.a);
                op.b = remap(op.b);
            }
            self.written = written;
            self.read = read;
        }
        self.written_out = self.written.iter().map(|&at| write_out(at)).collect();
        self.read_out = self.read.iter().map(|&at| read_out(at)).collect();
    }

    /// The work it does: cells times operations, a stencil as four.
    pub fn work(&self) -> usize {
        let weight = |op: &CellOp| match op.kind {
            Kind::Stencil { .. } => 4,
            _ => 1,
        };
        self.n * self.ops.iter().map(weight).sum::<usize>()
    }

    /// Run on the registers `r` (after the scalar instructions `first`),
    /// on several cores when large enough and faster so.
    pub fn run(&self, r: &mut [f64], out: Option<&mut Outside>) {
        if self.work() < WORTH || cores() < 2 {
            return self.run_in(r, 1, out);
        }
        let several = self.choice.several();
        let start = std::time::Instant::now();
        self.run_in(r, if several { cores() } else { 1 }, out);
        self.choice
            .record(several, start.elapsed().as_secs_f64() * 1e9);
    }

    /// Run in `chunks` chunks of cells, on as many cores as there are,
    /// with the arrays placed outside in `out` if given.
    pub fn run_in(&self, r: &mut [f64], chunks: usize, out: Option<&mut Outside>) {
        let value = |o: Operand, r: &[f64]| match o {
            Operand::Written(k) => Value::Mine(k),
            Operand::Read(k) => Value::Whole(k),
            Operand::Scalar(at) => Value::Scalar(r[at]),
        };
        let edge = |e: EdgeAt, r: &[f64]| match e {
            EdgeAt::Value(v) => EdgeValue::Value(r[v]),
            EdgeAt::Gradient(g) => EdgeValue::Gradient(g.map_or(0.0, |g| r[g])),
            EdgeAt::Periodic => EdgeValue::Periodic,
        };
        let ops: Vec<Ready> = self
            .ops
            .iter()
            .map(|op| Ready {
                kind: op.kind,
                d: op.d,
                a: value(op.a, r),
                b: value(op.b, r),
                edges: match op.kind {
                    Kind::Stencil { edges, .. } | Kind::Shift { edges, .. } => {
                        edges.map(|e| edge(e, r))
                    }
                    _ => [EdgeValue::Periodic; 2],
                },
            })
            .collect();
        // The arrays' registers, apart: those written to be chunked, those
        // read shared whole; in the registers or outside them.
        let n = self.n;
        let mut written: Vec<Option<&mut [f64]>> = (0..self.written.len()).map(|_| None).collect();
        let mut read: Vec<Option<&[f64]>> = (0..self.read.len()).map(|_| None).collect();
        let (out_reads, out_writes) = match out {
            Some(o) => (
                Some((o.reads, o.reads_at)),
                Some((&mut *o.writes, o.writes_at)),
            ),
            None => (None, None),
        };
        let away = |outside: &[bool], k: usize, there: bool| there && outside[k];
        let mut here: Vec<(usize, bool, usize)> = Vec::new();
        let mut there: Vec<(usize, usize)> = Vec::new();
        for (k, &at) in self.written.iter().enumerate() {
            if away(&self.written_out, k, out_writes.is_some()) {
                there.push((at, k));
            } else {
                here.push((at, true, k));
            }
        }
        for (k, &at) in self.read.iter().enumerate() {
            match out_reads {
                Some((cells, from)) if self.read_out[k] => {
                    read[k] = Some(&cells[at - from..at - from + n]);
                }
                _ => here.push((at, false, k)),
            }
        }
        here.sort_unstable();
        let mut rest = r;
        let mut taken = 0;
        for (at, writes, k) in here {
            let (_, tail) = rest.split_at_mut(at - taken);
            let (cells, after) = tail.split_at_mut(n);
            if writes {
                written[k] = Some(cells);
            } else {
                read[k] = Some(&*cells);
            }
            rest = after;
            taken = at + n;
        }
        if let Some((cells, from)) = out_writes {
            there.sort_unstable();
            let mut rest = cells;
            let mut taken = from;
            for (at, k) in there {
                let (_, tail) = rest.split_at_mut(at - taken);
                let (mine, after) = tail.split_at_mut(n);
                written[k] = Some(mine);
                rest = after;
                taken = at + n;
            }
        }
        let read: Vec<&[f64]> = read.into_iter().map(|c| c.expect("split")).collect();
        let size = self.n.div_ceil(chunks.max(1)).max(1);
        let mut work: Vec<(usize, Vec<&mut [f64]>)> = (0..self.n.div_ceil(size))
            .map(|c| (c * size, Vec::with_capacity(written.len())))
            .collect();
        for part in written {
            for (c, cells) in part.expect("split").chunks_mut(size).enumerate() {
                work[c].1.push(cells);
            }
        }
        let read = &read;
        let go = |(lo, mut mine): (usize, Vec<&mut [f64]>)| {
            for op in &ops {
                run_op(op, lo, &mut mine, read);
            }
        };
        simsandbox_core::parallel::each(work, chunks > 1, go);
    }
}

/// One operation on the chunk starting at cell `lo`: `mine` its part of
/// each written array, `read` the read arrays whole.
fn run_op(op: &Ready, lo: usize, mine: &mut [&mut [f64]], read: &[&[f64]]) {
    // The result, taken out while the operands are read.
    let dst = std::mem::take(&mut mine[op.d]);
    let z = |dst: &mut [f64], a: Value, b: Value, f: &dyn Fn(f64, f64) -> f64| {
        zip(dst, op.d, a, b, lo, mine, read, f)
    };
    match op.kind {
        Kind::Copy => zip(&mut *dst, op.d, op.a, op.a, lo, mine, read, |x, _| x),
        Kind::Neg => zip(&mut *dst, op.d, op.a, op.a, lo, mine, read, |x, _| -x),
        Kind::Bin(Op::Add) => zip(&mut *dst, op.d, op.a, op.b, lo, mine, read, |x, y| x + y),
        Kind::Bin(Op::Sub) => zip(&mut *dst, op.d, op.a, op.b, lo, mine, read, |x, y| x - y),
        Kind::Bin(Op::Mul) => zip(&mut *dst, op.d, op.a, op.b, lo, mine, read, |x, y| x * y),
        Kind::Bin(Op::Div) => zip(&mut *dst, op.d, op.a, op.b, lo, mine, read, |x, y| x / y),
        Kind::Bin(Op::Pow) => zip(&mut *dst, op.d, op.a, op.b, lo, mine, read, f64::powf),
        Kind::Bin(o) => z(&mut *dst, op.a, op.b, &|x, y| o.apply(x, y)),
        Kind::Call1(f) => z(&mut *dst, op.a, op.a, &|x, _| f.apply(x, 0.0)),
        Kind::Call2(f) => z(&mut *dst, op.a, op.b, &|x, y| f.apply(x, y)),
        Kind::Axis(line) => {
            let (Value::Scalar(start), Value::Scalar(h)) = (op.a, op.b) else {
                unreachable!("an axis's ends are scalars")
            };
            let mut place = Place::new(lo, line);
            for v in dst.iter_mut() {
                *v = start + (place.j as f64 + 0.5) * h;
                place.next();
            }
        }
        Kind::Stencil { second, line, .. } => {
            let (Value::Whole(a), Value::Scalar(h)) = (op.a, op.b) else {
                unreachable!("a stencil reads a whole array")
            };
            let src = read[a];
            let (s, c) = (line.stride, line.count);
            let edges = op.edges;
            let mut place = Place::new(lo, line);
            for (i, v) in dst.iter_mut().enumerate() {
                let (j, base) = (place.j, lo + i - place.j * s);
                let at = |j: usize| base + j * s;
                let below = |j: usize| {
                    if j == 0 {
                        ghost(edges[0], src[at(0)], src[at(c - 1)], h)
                    } else {
                        src[at(j - 1)]
                    }
                };
                let above = |j: usize| {
                    if j + 1 == c {
                        ghost(edges[1], src[at(c - 1)], src[at(0)], h)
                    } else {
                        src[at(j + 1)]
                    }
                };
                let edge = j == 0 || j + 1 == c;
                *v = match (second, edge) {
                    (true, false) => (src[at(j - 1)] - 2.0 * src[at(j)] + src[at(j + 1)]) / (h * h),
                    (true, true) => (below(j) - 2.0 * src[at(j)] + above(j)) / (h * h),
                    (false, false) => (src[at(j + 1)] - src[at(j - 1)]) / (2.0 * h),
                    (false, true) => (above(j) - below(j)) / (2.0 * h),
                };
                place.next();
            }
        }
        Kind::Shift { up, line, .. } => {
            let (Value::Whole(a), Value::Scalar(h)) = (op.a, op.b) else {
                unreachable!("a shift reads a whole array")
            };
            let src = read[a];
            let (s, c) = (line.stride, line.count);
            // Beyond an end: the value itself, not mirrored (ADR 0069).
            let beyond = |e: EdgeValue, end: f64, far: f64| match e {
                EdgeValue::Value(v) => v,
                EdgeValue::Gradient(g) => end + g * h,
                EdgeValue::Periodic => far,
            };
            let edges = op.edges;
            let mut place = Place::new(lo, line);
            for (i, v) in dst.iter_mut().enumerate() {
                let (j, base) = (place.j, lo + i - place.j * s);
                let at = |j: usize| base + j * s;
                *v = match (up, j) {
                    (false, 0) => beyond(edges[0], src[at(0)], src[at(c - 1)]),
                    (false, j) => src[at(j - 1)],
                    (true, j) if j + 1 == c => beyond(edges[1], src[at(c - 1)], src[at(0)]),
                    (true, j) => src[at(j + 1)],
                };
                place.next();
            }
        }
    }
    mine[op.d] = dst;
}

/// A cell's place along a line, `j`, kept by counting from cell `lo`
/// instead of dividing at every cell.
struct Place {
    j: usize,
    within: usize,
    line: Line,
}

impl Place {
    fn new(lo: usize, line: Line) -> Self {
        Self {
            j: (lo / line.stride) % line.count,
            within: lo % line.stride,
            line,
        }
    }

    fn next(&mut self) {
        self.within += 1;
        if self.within == self.line.stride {
            self.within = 0;
            self.j += 1;
            if self.j == self.line.count {
                self.j = 0;
            }
        }
    }
}

/// `dst = f(a, b)` cell by cell over the chunk from `lo`; `dst` is written
/// array `d`, taken out of `mine`, which an operand may name too (then it
/// reads `dst` as it was).
#[allow(clippy::too_many_arguments)]
fn zip(
    dst: &mut [f64],
    d: usize,
    a: Value,
    b: Value,
    lo: usize,
    mine: &[&mut [f64]],
    read: &[&[f64]],
    f: impl Fn(f64, f64) -> f64,
) {
    let len = dst.len();
    // Each operand as a slice over the chunk's cells, or a scalar; the
    // result itself when an operand is it.
    enum Src<'a> {
        Cells(&'a [f64]),
        Itself,
        Scalar(f64),
    }
    let src = |v: Value| match v {
        Value::Mine(k) if k == d => Src::Itself,
        Value::Mine(k) => Src::Cells(&mine[k][..len]),
        Value::Whole(k) => Src::Cells(&read[k][lo..lo + len]),
        Value::Scalar(x) => Src::Scalar(x),
    };
    match (src(a), src(b)) {
        (Src::Scalar(x), Src::Scalar(y)) => dst.fill(f(x, y)),
        (Src::Cells(x), Src::Scalar(y)) => dst.iter_mut().zip(x).for_each(|(v, &x)| *v = f(x, y)),
        (Src::Scalar(x), Src::Cells(y)) => dst.iter_mut().zip(y).for_each(|(v, &y)| *v = f(x, y)),
        (Src::Cells(x), Src::Cells(y)) => dst
            .iter_mut()
            .zip(x.iter().zip(y))
            .for_each(|(v, (&x, &y))| *v = f(x, y)),
        (Src::Itself, Src::Scalar(y)) => dst.iter_mut().for_each(|v| *v = f(*v, y)),
        (Src::Scalar(x), Src::Itself) => dst.iter_mut().for_each(|v| *v = f(x, *v)),
        (Src::Itself, Src::Cells(y)) => dst.iter_mut().zip(y).for_each(|(v, &y)| *v = f(*v, y)),
        (Src::Cells(x), Src::Itself) => dst.iter_mut().zip(x).for_each(|(v, &x)| *v = f(x, *v)),
        (Src::Itself, Src::Itself) => dst.iter_mut().for_each(|v| *v = f(*v, *v)),
    }
}
