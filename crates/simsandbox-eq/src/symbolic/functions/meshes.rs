//! Functions of meshes' cells and sides (ADR 0085): `centre`, `measure`
//! and `normal`, written as expressions of corners; `inner`, `outer`,
//! `across` and `boundary`, read through a side's cells. Each with its
//! axes, its unit and its instructions.

use super::{Axes, Check};
use crate::symbolic::lower::{Lower, Lowered};
use crate::symbolic::units::{ArrayDims, DimResult};
use crate::symbolic::Axis;
use simsandbox_expr::units::{dim_pow, Dim, DIM_TOO_HIGH, NO_DIM};
use simsandbox_expr::{ArrayOp, Ast, Expr};

/// The cells or sides table an argument names, if it is one.
fn table(a: &Ast, grid: &[Axis]) -> Option<usize> {
    match a {
        Ast::Name(n, _) => grid.iter().position(|g| &g.name == n && g.shape.is_some()),
        _ => None,
    }
}

/// `centre(T, c)` as an expression of corners.
pub(in crate::symbolic) fn centre_expansion(
    args: &[Ast],
    grid: &[Axis],
) -> std::result::Result<Ast, String> {
    let (Some(t), Some(c)) = (args.first(), args.get(1).and_then(|a| table(a, grid))) else {
        return Err(
            "'centre' takes a quantity over points and cells over them, e.g. centre(x, tri)".into(),
        );
    };
    if args.len() != 2 {
        return Err("'centre' takes 2 arguments, e.g. centre(x, tri)".into());
    }
    Ok(crate::mesh::centre(
        t,
        &grid[c].name,
        grid[c].shape.expect("cells"),
    ))
}

/// `measure(c, x[, y[, z]])` as an expression of corners.
pub(in crate::symbolic) fn measure_expansion(
    args: &[Ast],
    grid: &[Axis],
) -> std::result::Result<Ast, String> {
    let Some(c) = args.first().and_then(|a| table(a, grid)) else {
        return Err(
            "'measure' takes cells or sides and their points' coordinates, e.g. measure(tri, x, y)"
                .into(),
        );
    };
    if args.len() < 2 {
        return Err("'measure' takes the points' coordinates too, e.g. measure(tri, x, y)".into());
    }
    crate::mesh::measure(&grid[c].name, grid[c].shape.expect("cells"), &args[1..])
}

/// `normal(f, k, x[, y[, z]])` as an expression of corners.
pub(in crate::symbolic) fn normal_expansion(
    args: &[Ast],
    grid: &[Axis],
) -> std::result::Result<Ast, String> {
    let Some(f) = args
        .first()
        .and_then(|a| table(a, grid))
        .filter(|&f| grid[f].sides_of.is_some())
    else {
        return Err(
            "'normal' takes sides, a component and the coordinates, e.g. normal(f, 0, x, y)".into(),
        );
    };
    let Some(Ast::Num(k)) = args.get(1) else {
        return Err("'normal' takes a component's number second, e.g. normal(f, 0, x, y)".into());
    };
    if k.fract() != 0.0 || *k < 0.0 {
        return Err(format!(
            "a normal's component is a whole number from 0, not {k}: normal(f, 0, x, y)"
        ));
    }
    let c = grid[f].sides_of.expect("sides");
    crate::mesh::normal(
        &grid[f].name,
        grid[f].shape.expect("sides"),
        &grid[c].name,
        grid[c].shape.expect("cells"),
        *k as usize,
        &args[2..],
    )
}

/// `centre(T, c)`'s axes: `c`'s rows, of a quantity over the points they
/// join. Checked here, before the expansion, so a mistake is told of
/// `centre`'s own arguments, not of the `corner` calls it stands for.
pub(in crate::symbolic) fn centre_axes(c: &Check) -> Axes {
    let e = centre_expansion(c.args, c.grid).map_err(|m| (c.col, m))?;
    let cells = table(&c.args[1], c.grid).expect("checked: a table of cells");
    let points = c.grid[cells].ends_of.expect("cells join points");
    if c.of(&c.args[0])? != [points] {
        return Err((
            c.col,
            format!(
                "'centre' takes a quantity over {}, whose rows {} joins, e.g. centre(x, {})",
                c.grid[points].name, c.grid[cells].name, c.grid[cells].name
            ),
        ));
    }
    c.of(&e)
}

/// An expansion's axes: those of the expression it stands for.
pub(in crate::symbolic) fn expansion_axes(
    c: &Check,
    expansion: fn(&[Ast], &[Axis]) -> std::result::Result<Ast, String>,
) -> Axes {
    let e = expansion(c.args, c.grid).map_err(|m| (c.col, m))?;
    c.of(&e)
}

/// An expansion lowered: the expression it stands for.
pub(in crate::symbolic) fn expansion_lower(
    l: &Lower,
    args: &[Ast],
    expansion: fn(&[Ast], &[Axis]) -> std::result::Result<Ast, String>,
) -> Lowered {
    let e = expansion(args, l.grid).expect("checked: expands");
    l.low(&e)
}

/// A length, area or volume: the coordinates' unit to the power of the
/// cells' dimension (a point counts 1).
pub(in crate::symbolic) fn measure_dim(
    dims: &ArrayDims,
    args: &[(&Ast, Dim)],
    col: usize,
) -> DimResult {
    let power = match args[0].0 {
        Ast::Name(n, _) => dims.arrays.grid.iter().find(|g| &g.name == n),
        _ => None,
    }
    .and_then(|g| g.shape)
    .map_or(0, |s| s.dim());
    let x = dims.one_unit("measure", args.get(1..).unwrap_or_default(), col)?;
    dim_pow(x, power as i8).ok_or_else(|| (col, DIM_TOO_HIGH.to_string()))
}

/// A direction: none; the coordinates in one unit.
pub(in crate::symbolic) fn normal_dim(
    dims: &ArrayDims,
    args: &[(&Ast, Dim)],
    col: usize,
) -> DimResult {
    dims.one_unit("normal", args.get(2..).unwrap_or_default(), col)?;
    Ok(NO_DIM)
}

/// `inner(u, f)`, `outer(u, f)`, `across(u, f)` and `boundary(f)`: a
/// side's cells (ADR 0085), over the sides.
pub(in crate::symbolic) fn sides_axes(c: &Check, boundary: bool) -> Axes {
    let (name, args, grid) = (c.name, c.args, c.grid);
    let n = if boundary { 1 } else { 2 };
    let usage = if boundary {
        "a table of sides, e.g. boundary(f)".to_string()
    } else {
        format!("a quantity over cells and their sides, e.g. {name}(u, f)")
    };
    c.count(args.len() == n, &usage)?;
    let Some(f) = c
        .axis_named(&args[n - 1])
        .filter(|&f| grid[f].sides_of.is_some())
    else {
        return c.fail(format!("'{name}' takes {usage}"));
    };
    if n == 2 {
        let a = c.of(&args[0])?;
        let cells = grid[f].sides_of.expect("sides");
        if a != [cells] {
            return c.fail(format!(
                "'{name}' takes a quantity over {}, the cells {} lie between",
                grid[cells].name, grid[f].name
            ));
        }
    }
    Ok(vec![f])
}

/// The sides table `f` of a call, and its places: each side's corners,
/// then its inner cell, outer cell, whether on the boundary and the cell
/// across.
fn sides<'a>(l: &'a Lower, args: &[Ast]) -> (usize, &'a [usize], usize) {
    let f = l.axis_named(args.last().expect("checked: a sides table"));
    let v = l.ends[f].as_ref().expect("checked: sides");
    (f, v, v.len() - 4)
}

/// 1 on the boundary, else 0.
pub(in crate::symbolic) fn boundary_lower(l: &Lower, args: &[Ast]) -> Lowered {
    let (f, v, n) = sides(l, args);
    Ok((Expr::Var(v[n + 2]), vec![f]))
}

/// A side's cells' values: of the inner (`at` 0) or outer (1) cell, or
/// across (3): the outer cell, or on the boundary the inner one (ADR
/// 0085 step 3).
pub(in crate::symbolic) fn cell_lower(l: &Lower, args: &[Ast], at: usize) -> Lowered {
    let (f, v, n) = sides(l, args);
    let (t, _) = l.low(&args[0])?;
    Ok((
        Expr::Array(ArrayOp::Take, vec![t, Expr::Var(v[n + at])]),
        vec![f],
    ))
}
