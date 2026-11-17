//! Functions of tables and edges (ADR 0079, ADR 0085): `id`, `rows`,
//! `of`, `from_end`, `to_end`, `corner` and `outflow`, each with its
//! axes, its unit and its instructions.

use super::{Axes, Check};
use crate::symbolic::lower::{Lower, Lowered};
use crate::symbolic::units::{ArrayDims, DimResult};
use simsandbox_expr::units::{Dim, NO_DIM};
use simsandbox_expr::{ArrayOp, Ast, Expr, Op};

/// `id(tbl)`, `rows(tbl)`: the table named.
fn table(c: &Check) -> std::result::Result<usize, super::Fail> {
    let name = c.name;
    match c
        .args
        .first()
        .and_then(|a| c.table_named(a))
        .filter(|_| c.args.len() == 1)
    {
        Some(i) => Ok(i),
        None => c.fail(format!("'{name}' takes a table, e.g. {name}(pts)")),
    }
}

/// `id(tbl)`: over the table's rows.
pub(in crate::symbolic) fn id_axes(c: &Check) -> Axes {
    Ok(vec![table(c)?])
}

/// `rows(tbl)`: one value.
pub(in crate::symbolic) fn rows_axes(c: &Check) -> Axes {
    table(c)?;
    Ok(Vec::new())
}

/// Each row's id (ADR 0079).
pub(in crate::symbolic) fn id_lower(l: &Lower, args: &[Ast]) -> Lowered {
    let i = l.axis_named(&args[0]);
    Ok((Expr::Var(l.ids[i].expect("checked: a table")), vec![i]))
}

/// The number of rows (ADR 0079).
pub(in crate::symbolic) fn rows_lower(l: &Lower, args: &[Ast]) -> Lowered {
    Ok((
        Expr::Num(l.grid[l.axis_named(&args[0])].cells as f64),
        Vec::new(),
    ))
}

/// `from_end(T, e)`, `to_end(T, e)` and `corner(T, c, k)`: T at each
/// cell's corner (ADR 0085; an edge's ends, ADR 0079 stage 2), over the
/// cells.
pub(in crate::symbolic) fn ends_axes(c: &Check, corner: bool) -> Axes {
    let (name, args, grid) = (c.name, c.args, c.grid);
    let usage = if corner {
        "a quantity over a table of points, cells over it and a corner's number, e.g. corner(x, tri, 0)".to_string()
    } else {
        format!("a quantity over a table and its edges, e.g. {name}(x, segs)")
    };
    c.count(args.len() == if corner { 3 } else { 2 }, &usage)?;
    let a = c.of(&args[0])?;
    let e = c
        .axis_named(&args[1])
        .filter(|&e| grid[e].ends_of.is_some());
    let Some(e) = e else {
        return c.fail(format!(
            "'{name}' takes a table of cells, sides or edges second: {usage}"
        ));
    };
    let corners = grid[e].shape.map_or(2, |s| s.corners());
    if corner {
        match &args[2] {
            Ast::Num(k) if k.fract() == 0.0 && *k >= 0.0 && (*k as usize) < corners => {}
            _ => {
                return c.fail(format!(
                    "{}'s corners are numbered 0 to {}: give one as a number",
                    grid[e].name,
                    corners - 1
                ))
            }
        }
    }
    match &a[..] {
        [m] if grid[e].ends_of == Some(*m) => Ok(vec![e]),
        _ => c.fail(format!(
            "'{name}' takes a quantity over {}, whose rows {} joins",
            grid[grid[e].ends_of.expect("edges")].name,
            grid[e].name
        )),
    }
}

/// T at each edge's end (ADR 0079 stage 2): T's cells at the places of
/// the edges' `from` (`k` 0) or `to` (`k` 1) rows.
pub(in crate::symbolic) fn end_lower(l: &Lower, args: &[Ast], k: usize) -> Lowered {
    let (t, _) = l.low(&args[0])?;
    let e = l.axis_named(&args[1]);
    take(l, t, e, k)
}

/// T at corner k of each cell (ADR 0085).
pub(in crate::symbolic) fn corner_lower(l: &Lower, args: &[Ast]) -> Lowered {
    let (t, _) = l.low(&args[0])?;
    let e = l.axis_named(&args[1]);
    let k = match &args[2] {
        Ast::Num(k) => *k as usize,
        _ => unreachable!("checked: a corner's number"),
    };
    take(l, t, e, k)
}

/// `t` at corner `k` of each row of `e`.
fn take(l: &Lower, t: Expr, e: usize, k: usize) -> Lowered {
    let places = l.ends[e].as_ref().expect("checked: cells")[k];
    Ok((
        Expr::Array(ArrayOp::Take, vec![t, Expr::Var(places)]),
        vec![e],
    ))
}

/// `outflow(F, f)`: a sum over each cell's sides (ADR 0085 step 3), over
/// the cells; or over each row's edges, from `from` to `to` (ADR 0069
/// step 3), over the rows they join.
pub(in crate::symbolic) fn outflow_axes(c: &Check) -> Axes {
    let (args, grid) = (c.args, c.grid);
    let usage = "a quantity over sides or edges and those, e.g. outflow(F, f) or outflow(Q, pipes)";
    c.count(args.len() == 2, usage)?;
    let edges = |f: usize| {
        grid[f].sides_of.is_none()
            && grid[f].ends_of.is_some()
            && grid[f].shape == Some(crate::mesh::CellShape::Segment)
    };
    let Some(f) = c
        .axis_named(&args[1])
        .filter(|&f| grid[f].sides_of.is_some() || edges(f))
    else {
        return c.fail(format!("'outflow' takes {usage}"));
    };
    let a = c.of(&args[0])?;
    if !a.is_empty() && a != [f] {
        return c.fail(format!(
            "'outflow' takes a quantity over the {} {}",
            if grid[f].sides_of.is_some() {
                "sides"
            } else {
                "edges"
            },
            grid[f].name
        ));
    }
    Ok(vec![grid[f]
        .sides_of
        .or(grid[f].ends_of)
        .expect("sides or edges")])
}

/// Each cell's sum over its sides of F, outwards: + where it is the
/// inner cell, − where the outer (ADR 0085 step 3).
pub(in crate::symbolic) fn outflow_lower(l: &Lower, args: &[Ast]) -> Lowered {
    let grid = l.grid;
    let (t, _) = l.low(&args[0])?;
    let f = l.axis_named(&args[1]);
    let c = grid[f].sides_of.or(grid[f].ends_of).expect("checked");
    let v = l.ends[f].as_ref().expect("checked: sides or edges");
    // Edges: + at `from`, − at `to` (their first and second places);
    // sides: at the inner and the outer cell.
    let n = if grid[f].sides_of.is_some() {
        v.len() - 4
    } else {
        0
    };
    let cells = grid[c].cells;
    let to = |at: usize| {
        Expr::Array(
            ArrayOp::Scatter { cells },
            vec![t.clone(), Expr::Var(v[n + at])],
        )
    };
    Ok((
        Expr::Bin(Op::Sub, Box::new(to(0)), Box::new(to(1))),
        vec![c],
    ))
}

/// `of(T, i)`: T's value in the row with id i, NaN if none; one value.
pub(in crate::symbolic) fn of_axes(c: &Check) -> Axes {
    let (args, grid) = (c.args, c.grid);
    c.count(
        args.len() == 2,
        "a quantity over a table and an id, e.g. of(kp, 3)",
    )?;
    let a = c.of(&args[0])?;
    if !(a.len() == 1 && grid[a[0]].table.is_some()) {
        return c.fail("'of(T, i)' takes T over a table's rows".into());
    }
    if !c.of(&args[1])?.is_empty() {
        return c.fail("'of(T, i)' takes one id, e.g. of(kp, 3)".into());
    }
    Ok(Vec::new())
}

/// T's unit; an id has none.
pub(in crate::symbolic) fn of_dim(args: &[(&Ast, Dim)], col: usize) -> DimResult {
    if args.get(1).is_some_and(|a| a.1 != NO_DIM) {
        return Err((col, "a row's id has no unit".into()));
    }
    Ok(ArrayDims::first(args))
}

/// The cell whose id is i: at(T, k), k the first place where id(pts) is
/// i, NaN if none.
pub(in crate::symbolic) fn of_lower(l: &Lower, args: &[Ast]) -> Lowered {
    let (t, axes) = l.low(&args[0])?;
    let (i, _) = l.low(&args[1])?;
    let id = || Box::new(Expr::Var(l.ids[axes[0]].expect("checked: a table")));
    let near = |op: Op, by: f64| {
        Expr::Bin(
            op,
            id(),
            Box::new(Expr::Bin(
                Op::Add,
                Box::new(i.clone()),
                Box::new(Expr::Num(by)),
            )),
        )
    };
    let is = Expr::Bin(
        Op::Mul,
        Box::new(near(Op::Gt, -0.5)),
        Box::new(near(Op::Lt, 0.5)),
    );
    let place = Expr::Bin(
        Op::Add,
        Box::new(Expr::Array(ArrayOp::ArgMaxOf, vec![is.clone()])),
        Box::new(Expr::Bin(
            Op::Div,
            Box::new(Expr::Num(0.0)),
            Box::new(Expr::Array(ArrayOp::MaxOf, vec![is])),
        )),
    );
    Ok((Expr::Array(ArrayOp::Cell, vec![t, place]), Vec::new()))
}
