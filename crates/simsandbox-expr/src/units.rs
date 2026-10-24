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
/// what does not balance, with the column of a name involved.
pub fn dimension(ast: &Ast, of: &dyn Fn(&str) -> Dim) -> std::result::Result<Dim, (usize, String)> {
    let first_column = |a: &Ast| a.names().first().map_or(1, |(_, c)| *c);
    let dimensionless = |a: &Ast, what: &str| -> std::result::Result<(), (usize, String)> {
        let d = dimension(a, of)?;
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
        let d = dimension(base, of)?;
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
        Ast::Name(n, _) => of(n),
        Ast::Neg(a) => dimension(a, of)?,
        Ast::Bin(op, a, b) => {
            use crate::Op;
            match op {
                Op::Add | Op::Sub => {
                    let (da, db) = (dimension(a, of)?, dimension(b, of)?);
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
                    let (da, db) = (dimension(a, of)?, dimension(b, of)?);
                    std::array::from_fn(|i| da[i] + db[i])
                }
                Op::Div => {
                    let (da, db) = (dimension(a, of)?, dimension(b, of)?);
                    std::array::from_fn(|i| da[i] - db[i])
                }
                Op::Pow => power(a, b)?,
            }
        }
        Ast::Call(f, args) => {
            use crate::Func;
            match f {
                Func::Sqrt => {
                    let d = dimension(&args[0], of)?;
                    if d.iter().any(|e| e % 2 != 0) {
                        return Err((
                            first_column(&args[0]),
                            format!("sqrt of {} has no unit", dim_text(d)),
                        ));
                    }
                    d.map(|e| e / 2)
                }
                Func::Abs | Func::Floor | Func::Ceil => dimension(&args[0], of)?,
                Func::Min | Func::Max | Func::Atan2 => {
                    let (da, db) = (dimension(&args[0], of)?, dimension(&args[1], of)?);
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
                    dimension(&args[0], of)?;
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
