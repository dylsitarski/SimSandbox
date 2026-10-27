//! # simsandbox-expr
//!
//! A small expression language for typed equations and, later, derived
//! signals (ADR 0017). No dependencies.
//!
//! ```text
//! sigma*(y - x)     x*(rho - z) - y     -g/L*sin(theta)     2^-k
//! ```
//!
//! - Numbers (`3`, `0.5`, `1e-3`), names (`x`, `rho`, `theta_1`), `+ - * / ^`
//!   (`^` is right-associative and binds tighter than a leading minus, so
//!   `-x^2` is `-(x^2)`), parentheses, and the functions in [`Func`].
//! - `pi` is the constant π.
//! - [`parse`] gives an [`Ast`] that still uses names; [`Ast::resolve`] maps
//!   names to slots in a value buffer and gives an [`Expr`] that evaluates
//!   quickly. Errors carry a 1-based column.

pub mod program;
pub mod units;

pub use program::Program;

use std::fmt;

/// A parse or name-resolution error at a 1-based character column.
#[derive(Debug, Clone, PartialEq)]
pub struct Error {
    pub column: usize,
    pub message: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "column {}: {}", self.column, self.message)
    }
}

impl std::error::Error for Error {}

fn err<T>(column: usize, message: impl Into<String>) -> Result<T, Error> {
    Err(Error {
        column,
        message: message.into(),
    })
}

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

/// Built-in functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Func {
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Atan2,
    Sinh,
    Cosh,
    Tanh,
    Exp,
    Ln,
    Log10,
    Sqrt,
    Abs,
    Sign,
    Floor,
    Ceil,
    Min,
    Max,
    Pow,
}

impl Func {
    /// Function names as written, with their functions.
    pub const ALL: &'static [(&'static str, Func)] = &[
        ("sin", Func::Sin),
        ("cos", Func::Cos),
        ("tan", Func::Tan),
        ("asin", Func::Asin),
        ("acos", Func::Acos),
        ("atan", Func::Atan),
        ("atan2", Func::Atan2),
        ("sinh", Func::Sinh),
        ("cosh", Func::Cosh),
        ("tanh", Func::Tanh),
        ("exp", Func::Exp),
        ("ln", Func::Ln),
        ("log", Func::Ln),
        ("log10", Func::Log10),
        ("sqrt", Func::Sqrt),
        ("abs", Func::Abs),
        ("sign", Func::Sign),
        ("floor", Func::Floor),
        ("ceil", Func::Ceil),
        ("min", Func::Min),
        ("max", Func::Max),
        ("pow", Func::Pow),
    ];

    pub fn by_name(name: &str) -> Option<Func> {
        Self::ALL.iter().find(|(n, _)| *n == name).map(|(_, f)| *f)
    }

    pub fn arity(self) -> usize {
        match self {
            Func::Atan2 | Func::Min | Func::Max | Func::Pow => 2,
            _ => 1,
        }
    }

    fn apply(self, a: f64, b: f64) -> f64 {
        match self {
            Func::Sin => a.sin(),
            Func::Cos => a.cos(),
            Func::Tan => a.tan(),
            Func::Asin => a.asin(),
            Func::Acos => a.acos(),
            Func::Atan => a.atan(),
            Func::Atan2 => a.atan2(b),
            Func::Sinh => a.sinh(),
            Func::Cosh => a.cosh(),
            Func::Tanh => a.tanh(),
            Func::Exp => a.exp(),
            Func::Ln => a.ln(),
            Func::Log10 => a.log10(),
            Func::Sqrt => a.sqrt(),
            Func::Abs => a.abs(),
            Func::Sign => {
                if a > 0.0 {
                    1.0
                } else if a < 0.0 {
                    -1.0
                } else {
                    0.0
                }
            }
            Func::Floor => a.floor(),
            Func::Ceil => a.ceil(),
            Func::Min => a.min(b),
            Func::Max => a.max(b),
            Func::Pow => a.powf(b),
        }
    }
}

/// Names that cannot be used for variables: `pi` and the function names.
pub fn is_reserved(name: &str) -> bool {
    name == "pi" || Func::by_name(name).is_some()
}

/// Operations on arrays (ADR 0060), written like functions: stencils,
/// reductions, one element, an axis's spacing. A derivative along an
/// axis is `dd` and the axis name (`ddx`). Only typed equations with a
/// grid give them a meaning ([`Ast::resolve_with`]).
pub const ARRAY_OPS: &[&str] = &["lap", "sum", "mean", "minof", "maxof", "at", "spacing"];

/// Whether `name(...)` is an array operation: one of [`ARRAY_OPS`] or
/// `dd` followed by a name.
pub fn is_array_op(name: &str) -> bool {
    ARRAY_OPS.contains(&name)
        || name
            .strip_prefix("dd")
            .is_some_and(|a| a.starts_with(|c: char| c.is_alphabetic() || c == '_'))
}

/// How a stencil finds the values beyond an end of a grid (ADR 0060).
/// Registers are variables of the program.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Edge {
    /// Held at a value (Dirichlet), at the face of the end cell.
    Value(usize),
    /// The outward derivative (Neumann); `None` is 0 (insulated).
    Gradient(Option<usize>),
    /// The far end continues the near one.
    Periodic,
}

/// An array operation in an [`Expr`], resolved by typed equations.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ArrayOp {
    /// The cell centres of an axis of `n` cells; arguments the low end
    /// and the spacing.
    Axis(usize),
    /// The second derivative along a line of cells (the Laplacian in 1D);
    /// arguments the array and the spacing; edges low and high.
    Lap([Edge; 2]),
    /// The first derivative along a line (central differences);
    /// arguments as for `Lap`.
    Deriv([Edge; 2]),
    Sum,
    Mean,
    MinOf,
    MaxOf,
    /// One element, 0-based.
    At(usize),
}

/// A parsed expression that still refers to variables by name.
#[derive(Debug, Clone, PartialEq)]
pub enum Ast {
    Num(f64),
    /// A name and the column where it appears.
    Name(String, usize),
    Neg(Box<Ast>),
    Bin(Op, Box<Ast>, Box<Ast>),
    Call(Func, Vec<Ast>),
    /// An array operation ([`is_array_op`]) by name, with its arguments
    /// and the column of its name.
    Apply(String, Vec<Ast>, usize),
}

/// What an array operation means ([`Ast::resolve_with`]): from its name,
/// its arguments as written and resolved, and its column.
pub type ApplyOp<'a> = &'a dyn Fn(&str, &[Ast], Vec<Expr>, usize) -> Result<Expr, Error>;

/// An expression with variables resolved to slots; evaluate with
/// [`Expr::eval`], or as part of a [`Program`] when it uses arrays.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Num(f64),
    Var(usize),
    Neg(Box<Expr>),
    Bin(Op, Box<Expr>, Box<Expr>),
    Call(Func, Vec<Expr>),
    Array(ArrayOp, Vec<Expr>),
}

impl Ast {
    /// Variable names in order of first appearance, with their columns.
    pub fn names(&self) -> Vec<(String, usize)> {
        let mut out: Vec<(String, usize)> = Vec::new();
        self.collect_names(&mut out);
        out
    }

    fn collect_names(&self, out: &mut Vec<(String, usize)>) {
        match self {
            Ast::Num(_) => {}
            Ast::Name(n, c) => {
                if !out.iter().any(|(m, _)| m == n) {
                    out.push((n.clone(), *c));
                }
            }
            Ast::Neg(a) => a.collect_names(out),
            Ast::Bin(_, a, b) => {
                a.collect_names(out);
                b.collect_names(out);
            }
            Ast::Call(_, args) | Ast::Apply(_, args, _) => {
                args.iter().for_each(|a| a.collect_names(out))
            }
        }
    }

    /// Map each name to a slot with `slot`; a name it does not know is an
    /// error at that name's column. Array operations are an error: only
    /// typed equations with a grid have arrays.
    pub fn resolve(&self, slot: &impl Fn(&str) -> Option<usize>) -> Result<Expr, Error> {
        self.resolve_with(slot, &|name, _, _, column| {
            err(
                column,
                format!("'{name}' works on arrays, which only typed equations with a grid have"),
            )
        })
    }

    /// As [`Ast::resolve`], with `apply` giving the meaning of an array
    /// operation: its name, its arguments as written and resolved, and its
    /// column.
    pub fn resolve_with(
        &self,
        slot: &impl Fn(&str) -> Option<usize>,
        apply: ApplyOp<'_>,
    ) -> Result<Expr, Error> {
        Ok(match self {
            Ast::Num(v) => Expr::Num(*v),
            Ast::Name(n, c) => match slot(n) {
                Some(i) => Expr::Var(i),
                None => return err(*c, format!("unknown name '{n}'")),
            },
            Ast::Neg(a) => Expr::Neg(Box::new(a.resolve_with(slot, apply)?)),
            Ast::Bin(op, a, b) => Expr::Bin(
                *op,
                Box::new(a.resolve_with(slot, apply)?),
                Box::new(b.resolve_with(slot, apply)?),
            ),
            Ast::Call(f, args) => Expr::Call(
                *f,
                args.iter()
                    .map(|a| a.resolve_with(slot, apply))
                    .collect::<Result<_, _>>()?,
            ),
            Ast::Apply(name, args, column) => {
                let resolved = args
                    .iter()
                    .map(|a| a.resolve_with(slot, apply))
                    .collect::<Result<_, _>>()?;
                apply(name, args, resolved, *column)?
            }
        })
    }
}

impl Expr {
    /// Evaluate with variable `i` taken from `vars[i]`.
    pub fn eval(&self, vars: &[f64]) -> f64 {
        match self {
            Expr::Num(v) => *v,
            Expr::Var(i) => vars[*i],
            Expr::Neg(a) => -a.eval(vars),
            Expr::Bin(op, a, b) => {
                let (a, b) = (a.eval(vars), b.eval(vars));
                match op {
                    Op::Add => a + b,
                    Op::Sub => a - b,
                    Op::Mul => a * b,
                    Op::Div => a / b,
                    Op::Pow => a.powf(b),
                }
            }
            Expr::Call(f, args) => {
                let a = args[0].eval(vars);
                let b = args.get(1).map_or(0.0, |x| x.eval(vars));
                f.apply(a, b)
            }
            // Arrays are evaluated by programs only.
            Expr::Array(..) => f64::NAN,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Name(String),
    Sym(char),
    End,
}

/// `dotted`: names may continue with `.name` segments and end with `[i]`,
/// as signal and parameter names do (`a.v`, `lorenz.xyz[0]`).
fn lex(src: &str, dotted: bool) -> Result<Vec<(Tok, usize)>, Error> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let col = i + 1;
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit()
            || (c == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit()))
        {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            // Exponent: 1e-3, 2.5E4.
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                let mut j = i + 1;
                if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
                    j += 1;
                }
                if j < chars.len() && chars[j].is_ascii_digit() {
                    i = j;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            let text: String = chars[start..i].iter().collect();
            match text.parse::<f64>() {
                Ok(v) => out.push((Tok::Num(v), col)),
                Err(_) => return err(col, format!("'{text}' is not a number")),
            }
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            if dotted {
                let starts_name =
                    |j: usize| chars.get(j).is_some_and(|c| c.is_alphabetic() || *c == '_');
                while chars.get(i) == Some(&'.') && starts_name(i + 1) {
                    i += 1;
                    while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                        i += 1;
                    }
                }
                // An element index such as [0].
                if chars.get(i) == Some(&'[') {
                    let mut j = i + 1;
                    while chars.get(j).is_some_and(|c| c.is_ascii_digit()) {
                        j += 1;
                    }
                    if j > i + 1 && chars.get(j) == Some(&']') {
                        i = j + 1;
                    }
                }
            }
            out.push((Tok::Name(chars[start..i].iter().collect()), col));
        } else if "+-*/^(),".contains(c) {
            out.push((Tok::Sym(c), col));
            i += 1;
        } else {
            return err(col, format!("unexpected '{c}'"));
        }
    }
    out.push((Tok::End, chars.len() + 1));
    Ok(out)
}

struct Parser {
    toks: Vec<(Tok, usize)>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].0
    }

    fn col(&self) -> usize {
        self.toks[self.pos].1
    }

    fn next(&mut self) -> (Tok, usize) {
        let t = self.toks[self.pos].clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn expect(&mut self, c: char) -> Result<(), Error> {
        match self.next() {
            (Tok::Sym(s), _) if s == c => Ok(()),
            (t, col) => err(col, format!("expected '{c}', found {}", describe(&t))),
        }
    }

    /// Sums: term (('+' | '-') term)*
    fn expr(&mut self) -> Result<Ast, Error> {
        let mut lhs = self.term()?;
        while let Tok::Sym(c @ ('+' | '-')) = *self.peek() {
            self.next();
            let rhs = self.term()?;
            let op = if c == '+' { Op::Add } else { Op::Sub };
            lhs = Ast::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    /// Products: unary (('*' | '/') unary)*
    fn term(&mut self) -> Result<Ast, Error> {
        let mut lhs = self.unary()?;
        loop {
            match *self.peek() {
                Tok::Sym(c @ ('*' | '/')) => {
                    self.next();
                    let rhs = self.unary()?;
                    let op = if c == '*' { Op::Mul } else { Op::Div };
                    lhs = Ast::Bin(op, Box::new(lhs), Box::new(rhs));
                }
                // Two operands side by side, e.g. "2x" or "x y".
                Tok::Num(_) | Tok::Name(_) | Tok::Sym('(') => {
                    return err(self.col(), "missing operator (write 2*x, not 2x)");
                }
                _ => return Ok(lhs),
            }
        }
    }

    /// Signs: ('-' | '+') unary | power
    fn unary(&mut self) -> Result<Ast, Error> {
        match *self.peek() {
            Tok::Sym('-') => {
                self.next();
                Ok(Ast::Neg(Box::new(self.unary()?)))
            }
            Tok::Sym('+') => {
                self.next();
                self.unary()
            }
            _ => self.power(),
        }
    }

    /// Powers, right-associative: atom ('^' unary)?
    fn power(&mut self) -> Result<Ast, Error> {
        let base = self.atom()?;
        if *self.peek() == Tok::Sym('^') {
            self.next();
            let exp = self.unary()?;
            return Ok(Ast::Bin(Op::Pow, Box::new(base), Box::new(exp)));
        }
        Ok(base)
    }

    fn atom(&mut self) -> Result<Ast, Error> {
        match self.next() {
            (Tok::Num(v), _) => Ok(Ast::Num(v)),
            (Tok::Sym('('), _) => {
                let inner = self.expr()?;
                self.expect(')')?;
                Ok(inner)
            }
            (Tok::Name(n), col) => {
                if *self.peek() == Tok::Sym('(') && Func::by_name(&n).is_none() && is_array_op(&n) {
                    self.next();
                    let mut args = vec![self.expr()?];
                    while *self.peek() == Tok::Sym(',') {
                        self.next();
                        args.push(self.expr()?);
                    }
                    self.expect(')')?;
                    return Ok(Ast::Apply(n, args, col));
                }
                if *self.peek() == Tok::Sym('(') {
                    let Some(f) = Func::by_name(&n) else {
                        return err(col, format!("unknown function '{n}'"));
                    };
                    self.next();
                    let mut args = vec![self.expr()?];
                    while *self.peek() == Tok::Sym(',') {
                        self.next();
                        args.push(self.expr()?);
                    }
                    self.expect(')')?;
                    if args.len() != f.arity() {
                        return err(
                            col,
                            format!("'{n}' takes {} argument(s), got {}", f.arity(), args.len()),
                        );
                    }
                    Ok(Ast::Call(f, args))
                } else if n == "pi" {
                    Ok(Ast::Num(std::f64::consts::PI))
                } else if Func::by_name(&n).is_some() {
                    err(col, format!("'{n}' is a function; write {n}(...)"))
                } else {
                    Ok(Ast::Name(n, col))
                }
            }
            (t, col) => err(
                col,
                format!("expected a number, name or '(', found {}", describe(&t)),
            ),
        }
    }
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Num(v) => format!("number {v}"),
        Tok::Name(n) => format!("'{n}'"),
        Tok::Sym(c) => format!("'{c}'"),
        Tok::End => "the end".into(),
    }
}

/// Parse one expression.
pub fn parse(src: &str) -> Result<Ast, Error> {
    parse_with(src, false)
}

/// Parse an expression over signal names, which may be dotted and indexed:
/// `0.5*(a.v^2 + b.v^2)`, `lorenz.xyz[0] - lorenz.xyz[1]`. Used for derived
/// signals (ADR 0015).
pub fn parse_signals(src: &str) -> Result<Ast, Error> {
    parse_with(src, true)
}

fn parse_with(src: &str, dotted: bool) -> Result<Ast, Error> {
    let mut p = Parser {
        toks: lex(src, dotted)?,
        pos: 0,
    };
    if p.peek() == &Tok::End {
        return err(1, "empty expression");
    }
    let ast = p.expr()?;
    match p.next() {
        (Tok::End, _) => Ok(ast),
        (Tok::Sym(')'), col) => err(col, "unmatched ')'"),
        (t, col) => err(col, format!("unexpected {}", describe(&t))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Evaluate with x = 2, y = 3.
    fn eval(src: &str) -> f64 {
        let names = ["x", "y"];
        parse(src)
            .unwrap()
            .resolve(&|n| names.iter().position(|m| *m == n))
            .unwrap()
            .eval(&[2.0, 3.0])
    }

    fn error(src: &str) -> Error {
        parse(src).unwrap_err()
    }

    #[test]
    fn precedence_and_associativity() {
        assert_eq!(eval("1 + 2*3"), 7.0);
        assert_eq!(eval("(1 + 2)*3"), 9.0);
        assert_eq!(eval("10 - 4 - 3"), 3.0);
        assert_eq!(eval("12/2/3"), 2.0);
        assert_eq!(eval("2^3^2"), 512.0);
        assert_eq!(eval("-x^2"), -4.0);
        assert_eq!(eval("2^-1"), 0.5);
        assert_eq!(eval("--x"), 2.0);
        assert_eq!(eval("x*(y - x)"), 2.0);
        assert_eq!(eval("1e-3*1000 + .5 + 2.5E1"), 26.5);
    }

    #[test]
    fn functions_and_constants() {
        assert_eq!(eval("sin(0) + cos(0)"), 1.0);
        assert!((eval("pi") - std::f64::consts::PI).abs() < 1e-15);
        assert_eq!(eval("max(x, y) - min(x, y)"), 1.0);
        assert_eq!(eval("pow(x, y)"), 8.0);
        assert!((eval("atan2(1, 1)") - std::f64::consts::FRAC_PI_4).abs() < 1e-15);
        assert_eq!(eval("ln(exp(y))"), 3.0);
        assert_eq!(eval("log(exp(y))"), 3.0);
        assert_eq!(eval("sqrt(abs(-16))"), 4.0);
        assert_eq!(eval("sign(-x) + floor(2.7) + ceil(0.2)"), 2.0);
    }

    #[test]
    fn errors_point_at_the_problem() {
        assert_eq!(error("x + ").column, 5);
        assert_eq!(error("2x").column, 2);
        assert!(error("2x").message.contains("2*x"));
        assert_eq!(error("(x + 1").column, 7);
        assert_eq!(error("x + 1)").column, 6);
        assert_eq!(error("x $ y").column, 3);
        assert!(error("foo(x)").message.contains("unknown function"));
        assert!(error("atan2(x)").message.contains("2 argument"));
        assert!(error("sin + 1").message.contains("is a function"));
        assert_eq!(error("").message, "empty expression");
        let unknown = parse("x + zeta")
            .unwrap()
            .resolve(&|n| (n == "x").then_some(0));
        assert_eq!(unknown.unwrap_err().column, 5);
    }

    #[test]
    fn signal_expressions_take_dotted_and_indexed_names() {
        let ast = parse_signals("0.5*a.v^2 + lorenz.xyz[2] - t").unwrap();
        let names: Vec<_> = ast.names().into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["a.v", "lorenz.xyz[2]", "t"]);
        // Numbers with decimals are not names; plain parsing rejects dots.
        assert_eq!(parse_signals("x.y*1.5").unwrap().names()[0].0, "x.y");
        assert!(parse("a.v").is_err());
    }

    #[test]
    fn names_in_order_of_first_appearance() {
        let ast = parse("sigma*(y - x) + y*sigma").unwrap();
        let names: Vec<_> = ast.names().into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["sigma", "y", "x"]);
        assert!(is_reserved("pi") && is_reserved("sin") && !is_reserved("x"));
    }

    #[test]
    fn array_operations_parse_and_need_typed_equations() {
        let ast = parse("alpha*lap(T) - ddx(T) + at(T, 0)/sum(T)").unwrap();
        let names: Vec<_> = ast.names().into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["alpha", "T"]);
        assert!(matches!(&ast, Ast::Bin(..)));
        assert!(is_array_op("ddr") && is_array_op("spacing") && !is_array_op("dd"));
        // Elsewhere (parameter formulas, filters) they are an error.
        let e = parse("2*lap(T)")
            .unwrap()
            .resolve(&|_| Some(0))
            .unwrap_err();
        assert_eq!(e.column, 3);
        assert!(e.message.contains("only typed equations with a grid"));
        // Unknown functions are still refused when parsing.
        assert!(error("foo(x)").message.contains("unknown function"));
    }
}
