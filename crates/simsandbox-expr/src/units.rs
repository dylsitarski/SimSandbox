//! Units of expressions (ADR 0045): the dimension an expression has given
//! the dimensions of its names, and evaluating in SI. Used by typed
//! equations and by parameter formulas, so both follow the same rules.

use crate::{Ast, Expr};
use simsandbox_core::units::Unit;

/// A dimension: exponents of the seven SI base units.
pub type Dim = [i8; 7];
pub const NO_DIM: Dim = [0; 7];

/// The dimension of a product; `None` when a power of a base unit leaves
/// what a dimension keeps (±127).
pub fn dim_mul(a: Dim, b: Dim) -> Option<Dim> {
    let mut out = a;
    for (o, b) in out.iter_mut().zip(b) {
        *o = o.checked_add(b)?;
    }
    Some(out)
}

/// The dimension of a quotient; `None` as [`dim_mul`].
pub fn dim_div(a: Dim, b: Dim) -> Option<Dim> {
    let mut out = a;
    for (o, b) in out.iter_mut().zip(b) {
        *o = o.checked_sub(b)?;
    }
    Some(out)
}

/// The dimension of a whole power; `None` as [`dim_mul`].
pub fn dim_pow(a: Dim, n: i8) -> Option<Dim> {
    let mut out = a;
    for o in &mut out {
        *o = o.checked_mul(n)?;
    }
    Some(out)
}

/// What is said when a power of a base unit leaves ±127.
pub const DIM_TOO_HIGH: &str =
    "the unit here has a power of a base unit beyond ±127, which is not kept";

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
            Some(n) if n.fract() == 0.0 && n.abs() < 64.0 => {
                dim_pow(d, n as i8).ok_or_else(|| (first_column(base), DIM_TOO_HIGH.into()))
            }
            Some(n)
                if (2.0 * n).fract() == 0.0 && n.abs() < 64.0 && d.iter().all(|e| e % 2 == 0) =>
            {
                dim_pow(d.map(|e| e / 2), (2.0 * n) as i8)
                    .ok_or_else(|| (first_column(base), DIM_TOO_HIGH.into()))
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
                Op::Lt | Op::Le | Op::Gt | Op::Ge => {
                    let (da, db) = (dimension_with(a, of, apply)?, dimension_with(b, of, apply)?);
                    if da != db {
                        return Err((
                            first_column(b),
                            format!("cannot compare {} and {}", dim_text(da), dim_text(db)),
                        ));
                    }
                    // 1 or 0.
                    NO_DIM
                }
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
                    dim_mul(da, db).ok_or_else(|| (first_column(b), DIM_TOO_HIGH.into()))?
                }
                Op::Div => {
                    let (da, db) = (dimension_with(a, of, apply)?, dimension_with(b, of, apply)?);
                    dim_div(da, db).ok_or_else(|| (first_column(b), DIM_TOO_HIGH.into()))?
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
        // in the array's unit per spacing unit (squared for `Second`).
        // Reductions and elements take their argument in SI.
        Expr::Array(op, args) => {
            use crate::ArrayOp;
            let scale = |k: usize| match args.get(k) {
                Some(Expr::Var(i)) => scales[*i],
                _ => 1.0,
            };
            let (factor, args) = match op {
                ArrayOp::Second { .. } => (scale(0) / (scale(1) * scale(1)), args),
                ArrayOp::First { .. } => (scale(0) / scale(1), args),
                ArrayOp::Shift { .. } => (scale(0), args),
                ArrayOp::Axis { .. } => (1.0, args),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A power of a base unit beyond what a dimension keeps (±127) is an
    /// error where it arises, not a wrapped dimension that could balance
    /// by accident.
    #[test]
    fn powers_beyond_what_is_kept_are_refused() {
        let mut length = NO_DIM;
        length[0] = 100;
        let of = |n: &str| if n == "a" { length } else { NO_DIM };
        let dim = |src: &str| dimension(&crate::parse(src).unwrap(), &of);
        assert_eq!(dim("a").unwrap()[0], 100);
        assert_eq!(dim("a/a").unwrap(), NO_DIM);
        for src in ["a*a", "1/a/a", "a^2", "pow(a, 2)", "a^-2", "x*(a*a)"] {
            let (_, message) = dim(src).unwrap_err();
            assert_eq!(message, DIM_TOO_HIGH, "{src}");
        }
        assert_eq!(dim_mul([127; 7], [1; 7]), None);
        assert_eq!(dim_div([-128; 7], [1; 7]), None);
        assert_eq!(dim_pow([64; 7], 2), None);
        assert_eq!(dim_pow([2; 7], -3), Some([-6; 7]));
    }
}
