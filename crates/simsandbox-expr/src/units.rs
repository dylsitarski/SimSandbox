//! Units of expressions (ADR 0045): the dimension an expression has given
//! the dimensions of its names, and evaluating in SI. Used by typed
//! equations and by parameter formulas, so both follow the same rules.

use crate::{Ast, Expr};
use simsandbox_core::units::Unit;

/// A dimension: exponents of the seven SI base units.
pub type Dim = [i8; 7];
pub const NO_DIM: Dim = [0; 7];

pub fn dim_text(d: Dim) -> String {
    Unit::new(d, 1.0).to_string()
}

/// The dimension of an expression, given each name's; an error says
/// what does not balance, with the column of a name involved. Array
/// operations are an error ([`dimension_with`] gives them meaning).
pub fn dimension(ast: &Ast, of: &dyn Fn(&str) -> Dim) -> std::result::Result<Dim, (usize, String)> {
    dimension_with(ast, of, &|name, _, column| {
        Err((
            column,
            format!("'{name}' works on arrays, which only typed equations with a grid have"),
        ))
    })
}

/// The dimension of an array operation, from its name, its arguments as
/// written with their dimensions, and its column.
pub type ApplyDim<'a> =
    &'a dyn Fn(&str, &[(&Ast, Dim)], usize) -> std::result::Result<Dim, (usize, String)>;

/// As [`dimension`], with `apply` giving array operations their
/// dimension (typed equations with a grid, ADR 0060).
pub fn dimension_with(
    ast: &Ast,
    of: &dyn Fn(&str) -> Dim,
    apply: ApplyDim<'_>,
) -> std::result::Result<Dim, (usize, String)> {
    let first_column = |a: &Ast| a.names().first().map_or(1, |(_, c)| *c);
    let dimensionless = |a: &Ast, what: &str| -> std::result::Result<(), (usize, String)> {
        let d = dimension_with(a, of, apply)?;
        if d == NO_DIM {
            Ok(())
        } else {
            Err((
                first_column(a),
                format!("{what} must be dimensionless, not {}", dim_text(d)),
            ))
        }
    };
    let constant = |a: &Ast| -> Option<f64> {
        match a {
            Ast::Num(v) => Some(*v),
            Ast::Neg(b) => match &**b {
                Ast::Num(v) => Some(-v),
                _ => None,
            },
            _ => None,
        }
    };
    let power = |base: &Ast, exp: &Ast| -> std::result::Result<Dim, (usize, String)> {
        let d = dimension_with(base, of, apply)?;
        if d == NO_DIM {
            dimensionless(exp, "an exponent")?;
            return Ok(NO_DIM);
        }
        match constant(exp) {
            Some(n) if n.fract() == 0.0 && n.abs() < 64.0 => Ok(d.map(|e| e * n as i8)),
            Some(n) if (2.0 * n).fract() == 0.0 && d.iter().all(|e| e % 2 == 0) => {
                Ok(d.map(|e| (e / 2) * (2.0 * n) as i8))
            }
            _ => Err((
                first_column(base),
                format!(
                    "{} can only be raised to a constant whole power",
                    dim_text(d)
                ),
            )),
        }
    };
    Ok(match ast {
        Ast::Num(_) => NO_DIM,
        Ast::Apply(name, args, column) => {
            let dims = args
                .iter()
                .map(|a| dimension_with(a, of, apply).map(|d| (a, d)))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            apply(name, &dims, *column)?
        }
        Ast::Name(n, _) => of(n),
        Ast::Neg(a) => dimension_with(a, of, apply)?,
        Ast::Bin(op, a, b) => {
            use crate::Op;
            match op {
                Op::Add | Op::Sub => {
                    let (da, db) = (dimension_with(a, of, apply)?, dimension_with(b, of, apply)?);
                    if da != db {
                        let verb = if *op == Op::Add { "add" } else { "subtract" };
                        return Err((
                            first_column(b),
                            format!("cannot {verb} {} and {}", dim_text(da), dim_text(db)),
                        ));
                    }
                    da
                }
                Op::Mul => {
                    let (da, db) = (dimension_with(a, of, apply)?, dimension_with(b, of, apply)?);
                    std::array::from_fn(|i| da[i] + db[i])
                }
                Op::Div => {
                    let (da, db) = (dimension_with(a, of, apply)?, dimension_with(b, of, apply)?);
                    std::array::from_fn(|i| da[i] - db[i])
                }
                Op::Pow => power(a, b)?,
            }
        }
        Ast::Call(f, args) => {
            use crate::Func;
            match f {
                Func::Sqrt => {
                    let d = dimension_with(&args[0], of, apply)?;
                    if d.iter().any(|e| e % 2 != 0) {
                        return Err((
                            first_column(&args[0]),
                            format!("sqrt of {} has no unit", dim_text(d)),
                        ));
                    }
                    d.map(|e| e / 2)
                }
                Func::Abs | Func::Floor | Func::Ceil => dimension_with(&args[0], of, apply)?,
                Func::Min | Func::Max | Func::Atan2 => {
                    let (da, db) = (
                        dimension_with(&args[0], of, apply)?,
                        dimension_with(&args[1], of, apply)?,
                    );
                    if da != db {
                        return Err((
                            first_column(&args[1]),
                            format!(
                                "the two arguments are in {} and {}",
                                dim_text(da),
                                dim_text(db)
                            ),
                        ));
                    }
                    if *f == Func::Atan2 {
                        NO_DIM
                    } else {
                        da
                    }
                }
                Func::Sign => {
                    dimension_with(&args[0], of, apply)?;
                    NO_DIM
                }
                Func::Pow => power(&args[0], &args[1])?,
                _ => {
                    dimensionless(&args[0], "a function's argument")?;
                    NO_DIM
                }
            }
        }
    })
}

/// The expression with each variable converted to SI (`v * scale`).
pub fn in_si(e: Expr, scales: &[f64]) -> Expr {
    use crate::Op;
    match e {
        Expr::Var(i) if scales[i] != 1.0 => Expr::Bin(
            Op::Mul,
            Box::new(Expr::Var(i)),
            Box::new(Expr::Num(scales[i])),
        ),
        Expr::Neg(a) => Expr::Neg(Box::new(in_si(*a, scales))),
        Expr::Bin(op, a, b) => {
            Expr::Bin(op, Box::new(in_si(*a, scales)), Box::new(in_si(*b, scales)))
        }
        Expr::Call(f, args) => Expr::Call(f, args.into_iter().map(|a| in_si(a, scales)).collect()),
        // A stencil reads its array and spacing as they are (in their units,
        // with the array's edges) and is converted as a whole: its result is
        // in the array's unit per spacing unit (squared for `Lap`).
        // Reductions and elements take their argument in SI.
        Expr::Array(op, args) => {
            use crate::ArrayOp;
            let scale = |k: usize| match args.get(k) {
                Some(Expr::Var(i)) => scales[*i],
                _ => 1.0,
            };
            let (factor, args) = match op {
                ArrayOp::Lap(_) => (scale(0) / (scale(1) * scale(1)), args),
                ArrayOp::Deriv(_) => (scale(0) / scale(1), args),
                ArrayOp::Axis(_) => (1.0, args),
                _ => (1.0, args.into_iter().map(|a| in_si(a, scales)).collect()),
            };
            let e = Expr::Array(op, args);
            if factor == 1.0 {
                e
            } else {
                Expr::Bin(Op::Mul, Box::new(e), Box::new(Expr::Num(factor)))
            }
        }
        other => other,
    }
}

/// An SI value expressed in a unit of scale `scale`.
pub fn from_si(e: Expr, scale: f64) -> Expr {
    if scale == 1.0 {
        e
    } else {
        Expr::Bin(crate::Op::Div, Box::new(e), Box::new(Expr::Num(scale)))
    }
}
