//! Expressions compiled to a compact instruction list (ADR 0060, Q35).
//!
//! A [`Program`] runs a list of assignments, `register[target] = expr`,
//! over one buffer of registers: the caller's values first (time, states,
//! inputs, parameters, results), then temporaries, then constants. Each
//! instruction reads and writes registers by index, so evaluating costs one
//! dispatch per operation instead of a walk over a tree of boxes. Parts
//! without variables are folded to constants when the program is built.
//!
//! A program gives the same numbers as [`Expr::eval`], operation for
//! operation, apart from `x^2`, which is computed as `x*x`.

use crate::{Expr, Func, Op};

/// One instruction: `d` is the register written, `a` and `b` those read.
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
}

/// A register while compiling, before temporaries and constants are
/// placed after the caller's values.
#[derive(Debug, Clone, Copy)]
enum Reg {
    Var(usize),
    Temp(usize),
    Const(usize),
}

/// Assignments compiled to instructions; see the module documentation.
#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    code: Vec<Ins>,
    /// The registers, constants filled in; copied by [`Program::registers`].
    template: Vec<f64>,
}

struct Builder {
    code: Vec<(Ins, [Reg; 3])>,
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
    }
}

impl Builder {
    fn constant(&mut self, v: f64) -> Reg {
        let k = match self.consts.iter().position(|c| c.to_bits() == v.to_bits()) {
            Some(k) => k,
            None => {
                self.consts.push(v);
                self.consts.len() - 1
            }
        };
        Reg::Const(k)
    }

    /// Where the value of `e` can be read: a variable or constant as it
    /// is, else a temporary it is computed into.
    fn operand(&mut self, e: &Expr) -> Reg {
        if !has_var(e) {
            return self.constant(e.eval(&[]));
        }
        if let Expr::Var(i) = e {
            return Reg::Var(*i);
        }
        let t = Reg::Temp(self.temps);
        self.temps += 1;
        self.max_temps = self.max_temps.max(self.temps);
        self.emit(e, t);
        t
    }

    /// Compute `e` into `d`. Temporaries taken for its operands are free
    /// again once it is computed.
    fn emit(&mut self, e: &Expr, d: Reg) {
        let mark = self.temps;
        let z = Reg::Var(0);
        let ins = match e {
            Expr::Num(_) | Expr::Var(_) => {
                let a = self.operand(e);
                (Ins::Copy(0, 0), [d, a, z])
            }
            _ if !has_var(e) => {
                let a = self.operand(e);
                (Ins::Copy(0, 0), [d, a, z])
            }
            Expr::Neg(a) => {
                let a = self.operand(a);
                (Ins::Neg(0, 0), [d, a, z])
            }
            Expr::Bin(Op::Pow, a, b) if matches!(**b, Expr::Num(n) if n == 2.0) => {
                let a = self.operand(a);
                (Ins::Sqr(0, 0), [d, a, z])
            }
            Expr::Bin(op, a, b) => {
                let (a, b) = (self.operand(a), self.operand(b));
                let ins = match op {
                    Op::Add => Ins::Add(0, 0, 0),
                    Op::Sub => Ins::Sub(0, 0, 0),
                    Op::Mul => Ins::Mul(0, 0, 0),
                    Op::Div => Ins::Div(0, 0, 0),
                    Op::Pow => Ins::Pow(0, 0, 0),
                };
                (ins, [d, a, b])
            }
            Expr::Call(f, args) => {
                let a = self.operand(&args[0]);
                match args.get(1) {
                    Some(b) => {
                        let b = self.operand(b);
                        (Ins::Call2(*f, 0, 0, 0), [d, a, b])
                    }
                    None => (Ins::Call1(*f, 0, 0), [d, a, z]),
                }
            }
        };
        self.code.push(ins);
        self.temps = mark;
    }
}

impl Program {
    /// Compile `steps`, run in order: each writes its expression into the
    /// register `target`, one of the first `vars` registers, where later
    /// steps can read it. Expressions read registers below `vars`.
    pub fn new(vars: usize, steps: &[(usize, &Expr)]) -> Self {
        let mut b = Builder {
            code: Vec::new(),
            consts: Vec::new(),
            temps: 0,
            max_temps: 0,
        };
        for (target, e) in steps {
            assert!(*target < vars, "a program writes only its variables");
            b.emit(e, Reg::Var(*target));
        }
        let first_const = vars + b.max_temps;
        let at = |r: Reg| -> u32 {
            let i = match r {
                Reg::Var(i) => i,
                Reg::Temp(k) => vars + k,
                Reg::Const(k) => first_const + k,
            };
            u32::try_from(i).expect("fewer than 2^32 registers")
        };
        let code = b
            .code
            .into_iter()
            .map(|(ins, [d, a, c])| {
                let (d, a, c) = (at(d), at(a), at(c));
                match ins {
                    Ins::Copy(..) => Ins::Copy(d, a),
                    Ins::Neg(..) => Ins::Neg(d, a),
                    Ins::Sqr(..) => Ins::Sqr(d, a),
                    Ins::Add(..) => Ins::Add(d, a, c),
                    Ins::Sub(..) => Ins::Sub(d, a, c),
                    Ins::Mul(..) => Ins::Mul(d, a, c),
                    Ins::Div(..) => Ins::Div(d, a, c),
                    Ins::Pow(..) => Ins::Pow(d, a, c),
                    Ins::Call1(f, ..) => Ins::Call1(f, d, a),
                    Ins::Call2(f, ..) => Ins::Call2(f, d, a, c),
                }
            })
            .collect();
        let mut template = vec![0.0; first_const];
        template.extend_from_slice(&b.consts);
        Self { code, template }
    }

    /// A register buffer for [`Program::run`]: the variables (all 0, for
    /// the caller to fill), temporaries and constants.
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
}
