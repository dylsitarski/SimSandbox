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

use crate::{ArrayOp, Edge, Expr, Func, Op};

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
}

/// An edge with its registers resolved.
#[derive(Debug, Clone, Copy, PartialEq)]
enum EdgeAt {
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
    /// Cell centres `lo + (k + 1/2)·h`.
    Axis {
        d: usize,
        lo: usize,
        h: usize,
        n: usize,
    },
    /// Second (`second`) or first derivative along a line of cells.
    Stencil {
        second: bool,
        d: usize,
        a: usize,
        h: usize,
        n: usize,
        edges: [EdgeAt; 2],
    },
    /// A reduction to one value.
    Reduce {
        op: Reduce,
        d: usize,
        a: usize,
        n: usize,
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

/// The value beyond the low (`high` false) or high end of the line of `n`
/// cells from `a`, with spacing `h`.
fn ghost(edge: EdgeAt, r: &[f64], a: usize, n: usize, h: f64, high: bool) -> f64 {
    let (end, far) = if high {
        (r[a + n - 1], r[a])
    } else {
        (r[a], r[a + n - 1])
    };
    match edge {
        // The edge value is the mean of the end cell and the ghost.
        EdgeAt::Value(v) => 2.0 * r[v] - end,
        EdgeAt::Gradient(g) => end + g.map_or(0.0, |g| r[g]) * h,
        EdgeAt::Periodic => far,
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
            },
            ArrIns::Call1 { f, d, a, n } => {
                (0..n).for_each(|k| r[d + k] = f.apply(a.get(r, k), 0.0))
            }
            ArrIns::Call2 { f, d, a, b, n } => {
                (0..n).for_each(|k| r[d + k] = f.apply(a.get(r, k), b.get(r, k)))
            }
            ArrIns::Axis { d, lo, h, n } => {
                let (lo, h) = (r[lo], r[h]);
                (0..n).for_each(|k| r[d + k] = lo + (k as f64 + 0.5) * h);
            }
            ArrIns::Stencil {
                second,
                d,
                a,
                h,
                n,
                edges,
            } => {
                let h = r[h];
                let low = ghost(edges[0], r, a, n, h, false);
                let high = ghost(edges[1], r, a, n, h, true);
                // Neighbours below and above each cell, the ends from the
                // ghosts; the interior straight from the cells.
                let below = |r: &[f64], k: usize| if k == 0 { low } else { r[a + k - 1] };
                let above = |r: &[f64], k: usize| if k + 1 == n { high } else { r[a + k + 1] };
                let (src, dst) = split(r, a, d, n);
                if second {
                    let h2 = h * h;
                    for k in 1..n.saturating_sub(1) {
                        dst[k] = (src[k - 1] - 2.0 * src[k] + src[k + 1]) / h2;
                    }
                    for k in [0, n - 1] {
                        r[d + k] = (below(r, k) - 2.0 * r[a + k] + above(r, k)) / h2;
                    }
                } else {
                    let h2 = 2.0 * h;
                    for k in 1..n.saturating_sub(1) {
                        dst[k] = (src[k + 1] - src[k - 1]) / h2;
                    }
                    for k in [0, n - 1] {
                        r[d + k] = (above(r, k) - below(r, k)) / h2;
                    }
                }
            }
            ArrIns::Reduce { op, d, a, n } => {
                let cells = &r[a..a + n];
                r[d] = match op {
                    Reduce::Sum => cells.iter().sum(),
                    Reduce::Mean => cells.iter().sum::<f64>() / n as f64,
                    Reduce::Min => cells.iter().copied().fold(f64::INFINITY, f64::min),
                    Reduce::Max => cells.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                };
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
}

struct Builder<'a> {
    vars: &'a [Span],
    /// Where constants start, then where temporaries start; each temporary
    /// takes `width` registers, room for the longest array.
    consts_at: usize,
    temps_at: usize,
    width: usize,
    code: Vec<Ins>,
    arrays: Vec<ArrIns>,
    consts: Vec<f64>,
    /// Temporaries in use, and the most ever in use.
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
                ArrayOp::Axis(n) => *n,
                ArrayOp::Lap(_) | ArrayOp::Deriv(_) => self.len(&args[0]),
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
        let at = self.temps_at + self.temps * self.width;
        self.temps += 1;
        self.max_temps = self.max_temps.max(self.temps);
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
        let mark = self.temps;
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
                    ArrayOp::Axis(cells) => self.array(ArrIns::Axis {
                        d,
                        lo: one(0),
                        h: one(1),
                        n: cells,
                    }),
                    ArrayOp::Lap(e) | ArrayOp::Deriv(e) => {
                        assert_ne!(srcs[0].at, d, "a stencil writes elsewhere than it reads");
                        self.array(ArrIns::Stencil {
                            second: matches!(op, ArrayOp::Lap(_)),
                            d,
                            a: one(0),
                            h: one(1),
                            n: len,
                            edges: [edge(e[0], self.vars), edge(e[1], self.vars)],
                        })
                    }
                    ArrayOp::At(i) => {
                        let at = one(0) + i.min(len - 1);
                        let a = Src { at, array: false };
                        if scalar {
                            self.code.push(Ins::Copy(dr, reg(at)));
                        } else {
                            self.array(ArrIns::Copy { d, a, n });
                        }
                    }
                    ArrayOp::Sum | ArrayOp::Mean | ArrayOp::MinOf | ArrayOp::MaxOf => {
                        let op = match op {
                            ArrayOp::Sum => Reduce::Sum,
                            ArrayOp::Mean => Reduce::Mean,
                            ArrayOp::MinOf => Reduce::Min,
                            _ => Reduce::Max,
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
        self.temps = mark;
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
        let build = |consts: Vec<f64>| {
            let mut b = Builder {
                vars,
                consts_at: size,
                temps_at: size + consts.len(),
                width,
                code: Vec::new(),
                arrays: Vec::new(),
                consts,
                temps: 0,
                max_temps: 0,
            };
            for (target, e) in steps {
                let s = vars[*target];
                b.emit(e, s.at, s.len);
            }
            b
        };
        // First to find the constants, then with them in place so the
        // temporaries can follow.
        let consts = build(Vec::new()).consts;
        let n = consts.len();
        let b = build(consts);
        assert_eq!(b.consts.len(), n, "the same constants both times");
        let mut template = vec![0.0; size];
        template.extend_from_slice(&b.consts);
        template.resize(b.temps_at + b.max_temps * width, 0.0);
        Self {
            code: b.code,
            arrays: b.arrays,
            template,
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

    /// Run every step on `r`, a buffer from [`Program::registers`] (or
    /// one at least as long, with the same constants).
    pub fn run(&self, r: &mut [f64]) {
        for ins in &self.code {
            match *ins {
                Ins::Copy(d, a) => r[d as usize] = r[a as usize],
                Ins::Neg(d, a) => r[d as usize] = -r[a as usize],
                Ins::Add(d, a, b) => r[d as usize] = r[a as usize] + r[b as usize],
                Ins::Sub(d, a, b) => r[d as usize] = r[a as usize] - r[b as usize],
                Ins::Mul(d, a, b) => r[d as usize] = r[a as usize] * r[b as usize],
                Ins::Div(d, a, b) => r[d as usize] = r[a as usize] / r[b as usize],
                Ins::Pow(d, a, b) => r[d as usize] = r[a as usize].powf(r[b as usize]),
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
        let axis = Expr::Array(ArrayOp::Axis(4), vec![Expr::Var(0), Expr::Var(1)]);
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
        let lap = |edges| Expr::Array(ArrayOp::Lap(edges), vec![Expr::Var(5), Expr::Var(1)]);
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
            ArrayOp::Deriv([Edge::Gradient(Some(2)), Edge::Gradient(Some(2))]),
            vec![Expr::Var(5), Expr::Var(1)],
        );
        let r = run_on(&[(6, &d)], a);
        assert_eq!(
            &r[12..16],
            [(2.0 - 6.0) / 1.0, 3.0, 6.0, (13.0 - 4.0) / 1.0]
        );
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
}
