//! Reductions and one cell (ADR 0070, ADR 0080): `sum`, `mean`, `minof`,
//! `maxof`, `argminof` and `argmaxof` over all cells or along one axis,
//! and `at`. Each with its axes, its unit and its instructions.

use super::{Axes, Check};
use crate::symbolic::lower::{Lower, Lowered};
use crate::symbolic::shapes::{line_of, number};
use crate::symbolic::units::{ArrayDims, DimResult};
use simsandbox_expr::units::{Dim, NO_DIM};
use simsandbox_expr::{Along, ArrayOp, Ast, Expr, MAX_AXES};

/// A reduction: over all cells, or along one axis (ADR 0080), over the
/// axes left.
pub(in crate::symbolic) fn along_axes(c: &Check) -> Axes {
    let (name, args) = (c.name, c.args);
    c.count(
        matches!(args.len(), 1 | 2),
        "an array, and an axis to work along",
    )?;
    let axes = c.of(&args[0])?;
    if axes.is_empty() {
        return c.fail(format!("'{name}' takes an array, e.g. {name}(T)"));
    }
    let Some(a) = args.get(1) else {
        return Ok(Vec::new());
    };
    match c.axis_named(a) {
        Some(i) if axes.contains(&i) => Ok(axes.into_iter().filter(|&x| x != i).collect()),
        Some(i) => c.not_over(&format!("'{name}''s array"), &axes, c.names[i]),
        None => c.fail(format!(
            "'{name}(T, {})' works along an axis of T ({})",
            c.names[axes[0]],
            c.list(&axes)
        )),
    }
}

/// A reduction `of` as instructions: along the axis named second, or
/// over all cells.
pub(in crate::symbolic) fn along_lower(l: &Lower, args: &[Ast], of: Along) -> Lowered {
    let grid = l.grid;
    let (first, axes) = l.low(&args[0])?;
    let Some(a) = args.get(1) else {
        let op = match of {
            Along::Sum => ArrayOp::Sum,
            Along::Mean => ArrayOp::Mean,
            Along::MinOf => ArrayOp::MinOf,
            Along::MaxOf => ArrayOp::MaxOf,
            Along::ArgMinOf => ArrayOp::ArgMinOf,
            Along::ArgMaxOf => ArrayOp::ArgMaxOf,
        };
        return Ok((Expr::Array(op, vec![first]), Vec::new()));
    };
    let axis = l.axis_named(a);
    let mut dims = [1; MAX_AXES];
    for (k, &x) in axes.iter().enumerate() {
        dims[k] = grid[x].cells;
    }
    let op = ArrayOp::Along {
        of,
        rank: axes.len(),
        dims,
        axis: axes.iter().position(|&x| x == axis).expect("checked"),
    };
    let rest = axes.into_iter().filter(|&x| x != axis).collect();
    Ok((Expr::Array(op, vec![first]), rest))
}

/// `at(T, i, j)` with a whole number along each of T's axes, or `at(T,
/// k)` with one computed cell number (ADR 0070): one value.
pub(in crate::symbolic) fn at_axes(c: &Check) -> Axes {
    let (args, grid) = (c.args, c.grid);
    let axes = if args.is_empty() {
        Vec::new()
    } else {
        c.of(&args[0])?
    };
    if axes.is_empty() {
        return c.fail(
            "'at' takes an array and a cell number along each of its axes, e.g. at(T, 0)".into(),
        );
    }
    if args.len() == 2 && number(&args[1]).is_none() {
        if !c.of(&args[1])?.is_empty() {
            return c.fail("a computed cell number is one value, e.g. at(T, argmaxof(T))".into());
        }
        return Ok(Vec::new());
    }
    if args.len() != 1 + axes.len() {
        let cells = vec!["0"; axes.len()].join(", ");
        return c.fail(format!(
            "'at' takes {} argument(s), the array and a cell number along each of its axes ({}): at(T, {cells})",
            1 + axes.len(),
            c.list(&axes)
        ));
    }
    for (&a, i) in axes.iter().zip(&args[1..]) {
        match i {
            Ast::Num(i) if i.fract() == 0.0 && *i >= 0.0 && (*i as usize) < grid[a].cells => {}
            _ => {
                return c.fail(format!(
                    "a cell number along {} is a whole number from 0 to {}",
                    c.names[a],
                    grid[a].cells - 1
                ))
            }
        }
    }
    Ok(Vec::new())
}

/// T's unit; a cell number has none.
pub(in crate::symbolic) fn at_dim(args: &[(&Ast, Dim)], col: usize) -> DimResult {
    if args[1..].iter().any(|a| a.1 != NO_DIM) {
        return Err((col, "a cell number has no unit".into()));
    }
    Ok(ArrayDims::first(args))
}

pub(in crate::symbolic) fn at_lower(l: &Lower, args: &[Ast]) -> Lowered {
    let (first, axes) = l.low(&args[0])?;
    if args.len() == 2 && number(&args[1]).is_none() {
        // at(T, k), k computed: cell k in storage order.
        let (k, _) = l.low(&args[1])?;
        return Ok((Expr::Array(ArrayOp::Cell, vec![first, k]), Vec::new()));
    }
    // at(T, i, j): cell i + j·(T's first axis), and so on.
    let place = args[1..]
        .iter()
        .zip(&axes)
        .map(|(a, &axis)| match a {
            Ast::Num(i) => *i as usize * line_of(&axes, axis, l.grid).stride,
            _ => unreachable!("checked: a whole number"),
        })
        .sum();
    Ok((Expr::Array(ArrayOp::At(place), vec![first]), Vec::new()))
}
