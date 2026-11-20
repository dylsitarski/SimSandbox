//! Functions along grids (ADR 0060, ADR 0080): the stencils `lap`, `dd`,
//! `ddx`, `prev` and `next`, an axis's `spacing`, and `on`, which reads
//! one axis as another. Each with its axes, its unit and its
//! instructions; the stencils' instructions need the registers and are
//! built by the program (`Lowering::stencil` in program.rs).

use super::{Axes, Check, Function};
use crate::symbolic::lower::{Lower, Lowered};
use crate::symbolic::shapes::{ast_name, fit};
use crate::symbolic::units::{ArrayDims, DimResult};
use simsandbox_expr::units::{dim_div, dim_pow, Dim, DIM_TOO_HIGH, NO_DIM};
use simsandbox_expr::Ast;

/// `lap(T)`: over T's axes.
pub(in crate::symbolic) fn lap_axes(c: &Check) -> Axes {
    c.count(c.args.len() == 1, "1 argument")?;
    c.no_rows(c.named_array()?)
}

/// Along the array's own axes (ADR 0080), which need one unit: T's over
/// the axis's squared.
pub(in crate::symbolic) fn lap_dim(
    dims: &ArrayDims,
    args: &[(&Ast, Dim)],
    col: usize,
) -> DimResult {
    let grid = dims.arrays.grid;
    let d = ArrayDims::first(args);
    let own = match args[0].0 {
        Ast::Name(n, _) => dims.arrays.of(n),
        _ => Vec::new(),
    };
    let Some(&first) = own.first() else {
        return Ok(d);
    };
    if own
        .iter()
        .any(|&a| dims.axis_dims[a] != dims.axis_dims[first])
    {
        let names: Vec<&str> = own.iter().map(|&a| grid[a].name.as_str()).collect();
        return Err((
            col,
            format!(
                "lap adds second derivatives along {}, so the axes need the same unit",
                names.join(" and ")
            ),
        ));
    }
    dim_pow(dims.axis_dims[first], 2)
        .and_then(|h2| dim_div(d, h2))
        .ok_or_else(|| (col, DIM_TOO_HIGH.to_string()))
}

/// `prev(T)`, `next(T, x)`: over T's axes; the axis named when T has
/// several.
pub(in crate::symbolic) fn shift_axes(c: &Check) -> Axes {
    let (name, args, names) = (c.name, c.args, &c.names);
    c.count(matches!(args.len(), 1 | 2), "an array and an axis")?;
    let axes = c.no_rows(c.named_array()?)?;
    // Over two axes, which one is named (ADR 0080).
    if args.len() == 1 && axes.len() > 1 {
        let ways: Vec<String> = axes
            .iter()
            .map(|&a| format!("{name}({}, {})", ast_name(&args[0]), names[a]))
            .collect();
        return c.fail(format!(
            "{} is over {}: name the axis, {}",
            ast_name(&args[0]),
            c.list(&axes),
            ways.join(" or ")
        ));
    }
    if let Some(a) = args.get(1) {
        match c.axis_named(a) {
            Some(i) if axes.contains(&i) => {}
            _ => {
                return c.fail(format!(
                    "'{name}' runs along an axis of the array ({}): {name}(v, {})",
                    c.list(&axes),
                    names[axes[0]]
                ))
            }
        }
    }
    Ok(axes)
}

/// `dd(T, x)`: the derivative along a named axis (ADR 0080), over T's
/// axes.
pub(in crate::symbolic) fn dd_axes(c: &Check) -> Axes {
    let (args, names) = (c.args, &c.names);
    c.count(args.len() == 2, "an array and an axis, e.g. dd(T, x)")?;
    c.no_rows(c.of(&args[0])?)?;
    let Some(i) = c.axis_named(&args[1]) else {
        return c.fail(format!(
            "'dd(T, {})' differentiates along an axis of the grid ({})",
            names[0],
            names.join(", ")
        ));
    };
    let axes = c.named_array()?;
    if !axes.contains(&i) {
        return c.not_over(&format!("'dd' of {}", ast_name(&args[0])), &axes, names[i]);
    }
    Ok(axes)
}

/// `ddx(T)`: the derivative along `axis`, named after `dd`; over T's
/// axes.
pub(in crate::symbolic) fn dd_along_axes(c: &Check, axis: &str) -> Axes {
    let (d, args, names) = (c.name, c.args, &c.names);
    c.count(args.len() == 1, "1 argument")?;
    c.no_rows(c.of(&args[0])?)?;
    let Some(i) = names.iter().position(|n| *n == axis) else {
        return c.fail(format!(
            "'{d}' is a derivative along '{axis}', which is not an axis of the grid ({}): write dd{} or dd(T, {})",
            names.join(", "),
            names[0],
            names[0]
        ));
    };
    let axes = c.named_array()?;
    if !axes.contains(&i) {
        return c.not_over(&format!("'{d}' of {}", ast_name(&args[0])), &axes, names[i]);
    }
    Ok(axes)
}

/// A derivative along `axis`: T's unit over the axis's.
pub(in crate::symbolic) fn dd_dim(
    dims: &ArrayDims,
    args: &[(&Ast, Dim)],
    axis: &str,
    col: usize,
) -> DimResult {
    let a = dims.axis_dim_of(axis);
    dim_div(ArrayDims::first(args), a).ok_or_else(|| (col, DIM_TOO_HIGH.to_string()))
}

/// The axis `dd(T, x)` names second, as written.
pub(in crate::symbolic) fn dd_axis<'a>(args: &[(&'a Ast, Dim)]) -> &'a str {
    args.get(1).map_or("", |a| ast_name(a.0))
}

/// `spacing(x)`: one value, of a grid's axis.
pub(in crate::symbolic) fn spacing_axes(c: &Check) -> Axes {
    let (args, names) = (c.args, &c.names);
    let axis = args
        .first()
        .and_then(|a| c.axis_named(a))
        .filter(|_| args.len() == 1);
    match axis {
        Some(a) if c.grid[a].table.is_some() => c.fail(format!(
            "'spacing' takes a grid's axis; {} is a table, whose rows have no spacing",
            names[a]
        )),
        Some(_) => Ok(Vec::new()),
        None => c.fail(format!("'spacing' takes an axis: spacing({})", names[0])),
    }
}

/// The axis's unit.
pub(in crate::symbolic) fn spacing_dim(dims: &ArrayDims, args: &[(&Ast, Dim)]) -> Dim {
    match args[0].0 {
        Ast::Name(n, _) => dims.axis_dim_of(n),
        _ => NO_DIM,
    }
}

/// A stencil (`lap`, `dd`, `ddx`, `prev`, `next`) over its array's axes,
/// or `spacing`, one value: built by the program.
pub(in crate::symbolic) fn stencil_lower(l: &Lower, f: &Function, args: &[Ast]) -> Lowered {
    let axes = match f {
        Function::Spacing => Vec::new(),
        _ => l.shapes[ast_name(&args[0])].clone(),
    };
    Ok(((l.stencil)(f, args), axes))
}

/// `on(T, a, b)`: T with its axis a read as b, of the same length.
pub(in crate::symbolic) fn on_axes(c: &Check) -> Axes {
    let (args, grid, names) = (c.args, c.grid, &c.names);
    c.count(args.len() == 3, "a quantity and two axes, e.g. on(J, p, q)")?;
    let axes = c.no_rows(c.of(&args[0])?)?;
    let (Some(a), Some(b)) = (c.axis_named(&args[1]), c.axis_named(&args[2])) else {
        return c.fail("'on(T, a, b)' names two axes: T's axis a, read as b".into());
    };
    if !axes.contains(&a) {
        return c.not_over("the quantity", &axes, names[a]);
    }
    if a != b && axes.contains(&b) {
        return c.fail(format!("the quantity is over {} already", names[b]));
    }
    if grid[a].cells != grid[b].cells {
        return c.fail(format!(
            "{} has {} cells and {} has {}: 'on' needs the same number",
            names[a], grid[a].cells, names[b], grid[b].cells
        ));
    }
    let mut out: Vec<usize> = axes
        .into_iter()
        .map(|x| if x == a { b } else { x })
        .collect();
    out.sort_unstable();
    Ok(out)
}

pub(in crate::symbolic) fn on_lower(l: &Lower, args: &[Ast]) -> Lowered {
    let (e, from) = l.low(&args[0])?;
    let (a, b) = (l.axis_named(&args[1]), l.axis_named(&args[2]));
    let mut to: Vec<usize> = from.iter().map(|&x| if x == a { b } else { x }).collect();
    to.sort_unstable();
    Ok((fit(e, &from, &to, Some((a, b)), l.grid), to))
}
