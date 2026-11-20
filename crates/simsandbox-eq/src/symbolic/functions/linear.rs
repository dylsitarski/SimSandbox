//! Linear algebra on arrays (ADR 0080): `solve` and `rcond`, each with
//! its axes, its unit and its instructions.

use super::{Axes, Check};
use crate::symbolic::lower::{Lower, Lowered};
use crate::symbolic::shapes::line_of;
use crate::symbolic::units::{ArrayDims, DimResult};
use simsandbox_expr::units::{dim_div, Dim, DIM_TOO_HIGH, NO_DIM};
use simsandbox_expr::{ArrayOp, Ast, Expr};

/// A is over two axes of the same length.
fn square(c: &Check, a: &[usize], call: &str) -> std::result::Result<(), super::Fail> {
    if a.len() != 2 || c.grid[a[0]].cells != c.grid[a[1]].cells {
        return c.fail(format!(
            "{call} needs A over two axes of the same length, not over {}",
            c.list_or_none(a)
        ));
    }
    Ok(())
}

/// `rcond(A)`: one value, how far A is from singular (ADR 0080).
pub(in crate::symbolic) fn rcond_axes(c: &Check) -> Axes {
    c.count(c.args.len() == 1, "a matrix, e.g. rcond(A)")?;
    let a = c.of(&c.args[0])?;
    square(c, &a, "rcond(A)")?;
    Ok(Vec::new())
}

pub(in crate::symbolic) fn rcond_lower(l: &Lower, args: &[Ast]) -> Lowered {
    let (ea, sa) = l.low(&args[0])?;
    let op = ArrayOp::Rcond {
        n: l.grid[sa[0]].cells,
        rows: line_of(&sa, sa[0], l.grid).stride,
        cols: line_of(&sa, sa[1], l.grid).stride,
    };
    Ok((Expr::Array(op, vec![ea]), Vec::new()))
}

/// `solve(A, b)`: A over two axes of one length, b over one of them; the
/// result over the other (ADR 0080).
pub(in crate::symbolic) fn solve_axes(c: &Check) -> Axes {
    let args = c.args;
    c.count(args.len() == 2, "a matrix and a vector, e.g. solve(A, b)")?;
    let (a, b) = (c.of(&args[0])?, c.of(&args[1])?);
    square(c, &a, "solve(A, b)")?;
    match b[..] {
        [r] if a.contains(&r) => Ok(a.into_iter().filter(|&x| x != r).collect()),
        _ => c.fail(format!(
            "solve(A, b) needs b over one of A's axes ({}), not over {}",
            c.list(&a),
            c.list_or_none(&b)
        )),
    }
}

/// A x = b: x in b's unit over A's.
pub(in crate::symbolic) fn solve_dim(args: &[(&Ast, Dim)], col: usize) -> DimResult {
    let b = args.get(1).map_or(NO_DIM, |a| a.1);
    dim_div(b, ArrayDims::first(args)).ok_or_else(|| (col, DIM_TOO_HIGH.to_string()))
}

pub(in crate::symbolic) fn solve_lower(l: &Lower, args: &[Ast]) -> Lowered {
    let grid = l.grid;
    let ((ea, sa), (eb, sb)) = (l.low(&args[0])?, l.low(&args[1])?);
    let (r, c) = if sb[0] == sa[0] {
        (sa[0], sa[1])
    } else {
        (sa[1], sa[0])
    };
    let op = ArrayOp::Solve {
        n: grid[r].cells,
        rows: line_of(&sa, r, grid).stride,
        cols: line_of(&sa, c, grid).stride,
    };
    Ok((Expr::Array(op, vec![ea, eb]), vec![c]))
}
