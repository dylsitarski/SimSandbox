//! Which axes an expression is over: the checks before lowering, and the
//! helpers for grids, cells and meshes they use.

use super::Axis;
use simsandbox_expr::{ArrayOp, Ast, Edge, Expr, Line as GridLine, MAX_AXES};
/// The variables `e` reads, edge values of its stencils included.
pub(super) fn reads(e: &Expr, out: &mut Vec<usize>) {
    match e {
        Expr::Num(_) => {}
        Expr::Var(i) => out.push(*i),
        Expr::Neg(a) => reads(a, out),
        Expr::Bin(_, a, b) => {
            reads(a, out);
            reads(b, out);
        }
        Expr::Call(_, args) => args.iter().for_each(|a| reads(a, out)),
        Expr::Array(op, args) => {
            if let ArrayOp::Second { edges, .. }
            | ArrayOp::First { edges, .. }
            | ArrayOp::Shift { edges, .. } = op
            {
                for e in edges {
                    match e {
                        Edge::Value(v) | Edge::Gradient(Some(v)) => out.push(*v),
                        Edge::Gradient(None) | Edge::Periodic => {}
                    }
                }
            }
            args.iter().for_each(|a| reads(a, out));
        }
    }
}

/// Only the steps needed for the variables `wanted`, in order: the rates
/// need neither the cell centres nor quantities only shown as outputs,
/// unless they use them.
pub(super) fn needed<'a>(
    steps: Vec<(usize, &'a Expr)>,
    wanted: &[usize],
) -> Vec<(usize, &'a Expr)> {
    let mut want = wanted.to_vec();
    let mut keep = vec![false; steps.len()];
    for (k, (target, e)) in steps.iter().enumerate().rev() {
        if want.contains(target) {
            keep[k] = true;
            reads(e, &mut want);
        }
    }
    steps
        .into_iter()
        .zip(keep)
        .filter_map(|(s, k)| k.then_some(s))
        .collect()
}

/// The value of a plain number, `4` or `-4`.
pub(super) fn number(ast: &Ast) -> Option<f64> {
    match ast {
        Ast::Num(v) => Some(*v),
        Ast::Neg(a) => number(a).map(|v| -v),
        _ => None,
    }
}

/// Axes by name (ADR 0080): each quantity's axes, places in the grid
/// lines, in their order. A scalar has none and need not be listed.
pub(super) type Shapes = std::collections::HashMap<String, Vec<usize>>;

/// The axes of `a` and `b` together, in grid order.
pub(super) fn union(a: &[usize], b: &[usize]) -> Vec<usize> {
    let mut out: Vec<usize> = a.iter().chain(b).copied().collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// How axis `axis` runs through the cells of a quantity over `axes`
/// (the first fastest).
pub(super) fn line_of(axes: &[usize], axis: usize, grid: &[Axis]) -> GridLine {
    GridLine {
        stride: axes
            .iter()
            .take_while(|&&a| a != axis)
            .map(|&a| grid[a].cells)
            .product(),
        count: grid[axis].cells,
    }
}

/// The number of cells of a quantity over `axes`.
pub(super) fn cells_of(axes: &[usize], grid: &[Axis]) -> usize {
    axes.iter().map(|&a| grid[a].cells).product()
}

/// `e`, over `from`, spread over `to` (ADR 0080): each cell of `to` takes
/// the cell of `e` at the same place along `e`'s axes, axis `a` of `e`
/// read as `b` when `rename` is `Some((a, b))`. A scalar or a quantity
/// already over `to` is left as it is.
pub(super) fn fit(
    e: Expr,
    from: &[usize],
    to: &[usize],
    rename: Option<(usize, usize)>,
    grid: &[Axis],
) -> Expr {
    if from.is_empty() || from == to && rename.is_none() {
        return e;
    }
    let mut dims = [1; MAX_AXES];
    let mut strides = [0; MAX_AXES];
    for (k, &t) in to.iter().enumerate() {
        dims[k] = grid[t].cells;
        let source = match rename {
            Some((a, b)) if t == b => a,
            _ => t,
        };
        if from.contains(&source) {
            strides[k] = line_of(from, source, grid).stride;
        }
    }
    Expr::Array(
        ArrayOp::Gather {
            rank: to.len(),
            dims,
            strides,
        },
        vec![e],
    )
}

/// `centre`, `measure` and `normal` (ADR 0085) as expressions of
/// corners: `centre(T, c)`, `measure(c, x[, y[, z]])`, `normal(f, k, x[,
/// y[, z]])`.
pub(super) fn mesh_expansion(
    name: &str,
    args: &[Ast],
    grid: &[Axis],
) -> std::result::Result<Ast, String> {
    let table = |a: &Ast| -> Option<usize> {
        match a {
            Ast::Name(n, _) => grid.iter().position(|g| &g.name == n && g.shape.is_some()),
            _ => None,
        }
    };
    match name {
        "centre" => {
            let (Some(t), Some(c)) = (args.first(), args.get(1).and_then(table)) else {
                return Err("'centre' takes a quantity over points and cells over them, e.g. centre(x, tri)".into());
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
        "measure" => {
            let Some(c) = args.first().and_then(table) else {
                return Err("'measure' takes cells or sides and their points' coordinates, e.g. measure(tri, x, y)".into());
            };
            if args.len() < 2 {
                return Err(
                    "'measure' takes the points' coordinates too, e.g. measure(tri, x, y)".into(),
                );
            }
            crate::mesh::measure(&grid[c].name, grid[c].shape.expect("cells"), &args[1..])
        }
        _ => {
            let Some(f) = args
                .first()
                .and_then(table)
                .filter(|&f| grid[f].sides_of.is_some())
            else {
                return Err("'normal' takes sides, a component and the coordinates, e.g. normal(f, 0, x, y)".into());
            };
            let Some(Ast::Num(k)) = args.get(1) else {
                return Err(
                    "'normal' takes a component's number second, e.g. normal(f, 0, x, y)".into(),
                );
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
    }
}

/// The axes of `ast`'s value (ADR 0080), checking every array operation:
/// known, with the right arguments. Names not in `shapes` are scalars.
pub(super) fn axes_of(
    ast: &Ast,
    grid: &[Axis],
    shapes: &Shapes,
) -> std::result::Result<Vec<usize>, (usize, String)> {
    let of = |a: &Ast| axes_of(a, grid, shapes);
    let args = match ast {
        Ast::Num(_) => return Ok(Vec::new()),
        Ast::Name(n, _) => return Ok(shapes.get(n).cloned().unwrap_or_default()),
        Ast::Neg(a) => return of(a),
        Ast::Bin(_, a, b) => return Ok(union(&of(a)?, &of(b)?)),
        Ast::Call(_, args) => {
            let mut out = Vec::new();
            for a in args {
                out = union(&out, &of(a)?);
            }
            return Ok(out);
        }
        Ast::Apply(_, args, _) => args,
    };
    let Ast::Apply(name, _, col) = ast else {
        unreachable!()
    };
    let col = *col;
    if grid.is_empty() {
        return Err((
            col,
            format!("'{name}' works on arrays: declare a grid with 'grid x [m] = 0 .. length, 50'"),
        ));
    }
    let names: Vec<&str> = grid.iter().map(|a| a.name.as_str()).collect();
    let axis_named = |a: &Ast| match a {
        Ast::Name(n, _) => names.iter().position(|m| m == n),
        _ => None,
    };
    let list = |axes: &[usize]| {
        axes.iter()
            .map(|&a| names[a])
            .collect::<Vec<_>>()
            .join(", ")
    };
    let count = |ok: bool, want: &str| {
        if ok {
            Ok(())
        } else {
            Err((
                col,
                format!("'{name}' takes {want}, got {} argument(s)", args.len()),
            ))
        }
    };
    // A stencil's array: a declared or defined quantity by name.
    let named_array = || -> std::result::Result<Vec<usize>, (usize, String)> {
        match &args[0] {
            Ast::Name(n, _) if !names.contains(&n.as_str()) && shapes.get(n).is_some_and(|s| !s.is_empty()) => {
                Ok(shapes[n].clone())
            }
            _ => Err((
                col,
                format!("'{name}' takes the name of an array, e.g. {name}(T); define one above (D = k*T) for an expression"),
            )),
        }
    };
    let not_over = |what: &str, axes: &[usize], a: &str| {
        Err((
            col,
            format!(
                "{what} is over {}, not {a}",
                if axes.is_empty() {
                    "no axis".to_string()
                } else {
                    list(axes)
                }
            ),
        ))
    };
    // Rows have no neighbours by place (ADR 0079: edges come with stage 2).
    let no_rows = |axes: Vec<usize>| -> std::result::Result<Vec<usize>, (usize, String)> {
        match axes.iter().find(|&&a| grid[a].table.is_some()) {
            Some(&a) => Err((
                col,
                format!(
                    "'{name}' works along a grid; {} is a table, whose rows have no neighbours by place",
                    names[a]
                ),
            )),
            None => Ok(axes),
        }
    };
    let table_named = |a: &Ast| axis_named(a).filter(|&i| grid[i].table.is_some());
    match name.as_str() {
        "id" | "rows" => {
            let Some(i) = args
                .first()
                .and_then(table_named)
                .filter(|_| args.len() == 1)
            else {
                return Err((col, format!("'{name}' takes a table, e.g. {name}(pts)")));
            };
            Ok(if name == "id" { vec![i] } else { Vec::new() })
        }
        "from_end" | "to_end" | "corner" => {
            // T at each cell's corner (ADR 0085; an edge's ends, ADR
            // 0079 stage 2): over the cells.
            let usage = match name.as_str() {
                "corner" => "a quantity over a table of points, cells over it and a corner's number, e.g. corner(x, tri, 0)".to_string(),
                _ => format!("a quantity over a table and its edges, e.g. {name}(x, segs)"),
            };
            count(args.len() == if name == "corner" { 3 } else { 2 }, &usage)?;
            let a = of(&args[0])?;
            let e = axis_named(&args[1]).filter(|&e| grid[e].ends_of.is_some());
            let Some(e) = e else {
                return Err((
                    col,
                    format!("'{name}' takes a table of cells, sides or edges second: {usage}"),
                ));
            };
            let corners = grid[e].shape.map_or(2, |s| s.corners());
            if name == "corner" {
                match &args[2] {
                    Ast::Num(k) if k.fract() == 0.0 && *k >= 0.0 && (*k as usize) < corners => {}
                    _ => {
                        return Err((
                            col,
                            format!(
                                "{}'s corners are numbered 0 to {}: give one as a number",
                                grid[e].name,
                                corners - 1
                            ),
                        ))
                    }
                }
            }
            match &a[..] {
                [m] if grid[e].ends_of == Some(*m) => Ok(vec![e]),
                _ => Err((
                    col,
                    format!(
                        "'{name}' takes a quantity over {}, whose rows {} joins",
                        grid[grid[e].ends_of.expect("edges")].name,
                        grid[e].name
                    ),
                )),
            }
        }
        "inner" | "outer" | "across" | "boundary" => {
            // A side's cells (ADR 0085): over the sides.
            let n = if name == "boundary" { 1 } else { 2 };
            let usage = match name.as_str() {
                "boundary" => "a table of sides, e.g. boundary(f)".to_string(),
                _ => format!("a quantity over cells and their sides, e.g. {name}(u, f)"),
            };
            count(args.len() == n, &usage)?;
            let Some(f) = axis_named(&args[n - 1]).filter(|&f| grid[f].sides_of.is_some()) else {
                return Err((col, format!("'{name}' takes {usage}")));
            };
            if n == 2 {
                let a = of(&args[0])?;
                let c = grid[f].sides_of.expect("sides");
                if a != [c] {
                    return Err((
                        col,
                        format!(
                            "'{name}' takes a quantity over {}, the cells {} lie between",
                            grid[c].name, grid[f].name
                        ),
                    ));
                }
            }
            Ok(vec![f])
        }
        "centre" | "measure" | "normal" => {
            let e = mesh_expansion(name, args, grid).map_err(|m| (col, m))?;
            of(&e)
        }
        "outflow" => {
            // A sum over each cell's sides (ADR 0085 step 3): over the
            // cells; or over each row's edges, from `from` to `to` (ADR
            // 0069 step 3): over the rows they join.
            let usage =
                "a quantity over sides or edges and those, e.g. outflow(F, f) or outflow(Q, pipes)";
            count(args.len() == 2, usage)?;
            let edges = |f: usize| {
                grid[f].sides_of.is_none()
                    && grid[f].ends_of.is_some()
                    && grid[f].shape == Some(crate::mesh::CellShape::Segment)
            };
            let Some(f) = axis_named(&args[1]).filter(|&f| grid[f].sides_of.is_some() || edges(f))
            else {
                return Err((col, format!("'outflow' takes {usage}")));
            };
            let a = of(&args[0])?;
            if !a.is_empty() && a != [f] {
                return Err((
                    col,
                    format!(
                        "'outflow' takes a quantity over the {} {}",
                        if grid[f].sides_of.is_some() {
                            "sides"
                        } else {
                            "edges"
                        },
                        grid[f].name
                    ),
                ));
            }
            Ok(vec![grid[f]
                .sides_of
                .or(grid[f].ends_of)
                .expect("sides or edges")])
        }
        "of" => {
            // of(T, i): T's value in the row with id i, NaN if none.
            count(
                args.len() == 2,
                "a quantity over a table and an id, e.g. of(kp, 3)",
            )?;
            let a = of(&args[0])?;
            if !(a.len() == 1 && grid[a[0]].table.is_some()) {
                return Err((col, "'of(T, i)' takes T over a table's rows".into()));
            }
            if !of(&args[1])?.is_empty() {
                return Err((col, "'of(T, i)' takes one id, e.g. of(kp, 3)".into()));
            }
            Ok(Vec::new())
        }
        "lap" => {
            count(args.len() == 1, "1 argument")?;
            no_rows(named_array()?)
        }
        "prev" | "next" => {
            count(matches!(args.len(), 1 | 2), "an array and an axis")?;
            let axes = no_rows(named_array()?)?;
            // Over two axes, which one is named (ADR 0080).
            if args.len() == 1 && axes.len() > 1 {
                let ways: Vec<String> = axes
                    .iter()
                    .map(|&a| format!("{name}({}, {})", ast_name(&args[0]), names[a]))
                    .collect();
                return Err((
                    col,
                    format!(
                        "{} is over {}: name the axis, {}",
                        ast_name(&args[0]),
                        list(&axes),
                        ways.join(" or ")
                    ),
                ));
            }
            if let Some(a) = args.get(1) {
                match axis_named(a) {
                    Some(i) if axes.contains(&i) => {}
                    _ => {
                        return Err((
                            col,
                            format!(
                                "'{name}' runs along an axis of the array ({}): {name}(v, {})",
                                list(&axes),
                                names[axes[0]]
                            ),
                        ))
                    }
                }
            }
            Ok(axes)
        }
        "dd" => {
            // dd(T, x): the derivative along a named axis (ADR 0080).
            count(args.len() == 2, "an array and an axis, e.g. dd(T, x)")?;
            no_rows(of(&args[0])?)?;
            let Some(i) = axis_named(&args[1]) else {
                return Err((
                    col,
                    format!(
                        "'dd(T, {})' differentiates along an axis of the grid ({})",
                        names[0],
                        names.join(", ")
                    ),
                ));
            };
            let axes = named_array()?;
            if !axes.contains(&i) {
                return not_over(&format!("'dd' of {}", ast_name(&args[0])), &axes, names[i]);
            }
            Ok(axes)
        }
        "rcond" => {
            // rcond(A): one value, how far A is from singular (ADR 0080).
            count(args.len() == 1, "a matrix, e.g. rcond(A)")?;
            let a = of(&args[0])?;
            if a.len() != 2 || grid[a[0]].cells != grid[a[1]].cells {
                return Err((
                    col,
                    format!(
                        "rcond(A) needs A over two axes of the same length, not over {}",
                        if a.is_empty() {
                            "no axis".to_string()
                        } else {
                            list(&a)
                        }
                    ),
                ));
            }
            Ok(Vec::new())
        }
        "solve" => {
            // solve(A, b): A over two axes of one length, b over one of
            // them; the result over the other (ADR 0080).
            count(args.len() == 2, "a matrix and a vector, e.g. solve(A, b)")?;
            let (a, b) = (of(&args[0])?, of(&args[1])?);
            if a.len() != 2 || grid[a[0]].cells != grid[a[1]].cells {
                return Err((
                    col,
                    format!(
                        "solve(A, b) needs A over two axes of the same length, not over {}",
                        if a.is_empty() {
                            "no axis".to_string()
                        } else {
                            list(&a)
                        }
                    ),
                ));
            }
            match b[..] {
                [r] if a.contains(&r) => Ok(a.into_iter().filter(|&c| c != r).collect()),
                _ => Err((
                    col,
                    format!(
                        "solve(A, b) needs b over one of A's axes ({}), not over {}",
                        list(&a),
                        if b.is_empty() {
                            "no axis".to_string()
                        } else {
                            list(&b)
                        }
                    ),
                )),
            }
        }
        d if d.starts_with("dd") => {
            count(args.len() == 1, "1 argument")?;
            no_rows(of(&args[0])?)?;
            let Some(i) = names.iter().position(|n| *n == &d[2..]) else {
                return Err((col, format!("'{d}' is a derivative along '{}', which is not an axis of the grid ({}): write dd{} or dd(T, {})", &d[2..], names.join(", "), names[0], names[0])));
            };
            let axes = named_array()?;
            if !axes.contains(&i) {
                return not_over(&format!("'{d}' of {}", ast_name(&args[0])), &axes, names[i]);
            }
            Ok(axes)
        }
        "spacing" => {
            let axis = args
                .first()
                .and_then(axis_named)
                .filter(|_| args.len() == 1);
            match axis {
                Some(a) if grid[a].table.is_some() => Err((
                    col,
                    format!(
                        "'spacing' takes a grid's axis; {} is a table, whose rows have no spacing",
                        names[a]
                    ),
                )),
                Some(_) => Ok(Vec::new()),
                None => Err((
                    col,
                    format!("'spacing' takes an axis: spacing({})", names[0]),
                )),
            }
        }
        "on" => {
            count(args.len() == 3, "a quantity and two axes, e.g. on(J, p, q)")?;
            let axes = no_rows(of(&args[0])?)?;
            let (Some(a), Some(b)) = (axis_named(&args[1]), axis_named(&args[2])) else {
                return Err((
                    col,
                    "'on(T, a, b)' names two axes: T's axis a, read as b".into(),
                ));
            };
            if !axes.contains(&a) {
                return not_over("the quantity", &axes, names[a]);
            }
            if a != b && axes.contains(&b) {
                return Err((col, format!("the quantity is over {} already", names[b])));
            }
            if grid[a].cells != grid[b].cells {
                return Err((
                    col,
                    format!(
                        "{} has {} cells and {} has {}: 'on' needs the same number",
                        names[a], grid[a].cells, names[b], grid[b].cells
                    ),
                ));
            }
            let mut out: Vec<usize> = axes
                .into_iter()
                .map(|x| if x == a { b } else { x })
                .collect();
            out.sort_unstable();
            Ok(out)
        }
        "at" => {
            let axes = if args.is_empty() {
                Vec::new()
            } else {
                of(&args[0])?
            };
            if axes.is_empty() {
                return Err((
                    col,
                    "'at' takes an array and a cell number along each of its axes, e.g. at(T, 0)"
                        .into(),
                ));
            }
            // `at(T, k)` with a computed cell number (ADR 0070).
            if args.len() == 2 && number(&args[1]).is_none() {
                if !of(&args[1])?.is_empty() {
                    return Err((
                        col,
                        "a computed cell number is one value, e.g. at(T, argmaxof(T))".into(),
                    ));
                }
                return Ok(Vec::new());
            }
            if args.len() != 1 + axes.len() {
                let cells = vec!["0"; axes.len()].join(", ");
                return Err((
                    col,
                    format!("'at' takes {} argument(s), the array and a cell number along each of its axes ({}): at(T, {cells})", 1 + axes.len(), list(&axes)),
                ));
            }
            for (&a, i) in axes.iter().zip(&args[1..]) {
                match i {
                    Ast::Num(i)
                        if i.fract() == 0.0 && *i >= 0.0 && (*i as usize) < grid[a].cells => {}
                    _ => {
                        return Err((
                            col,
                            format!(
                                "a cell number along {} is a whole number from 0 to {}",
                                names[a],
                                grid[a].cells - 1
                            ),
                        ))
                    }
                }
            }
            Ok(Vec::new())
        }
        _ => {
            // The reductions: over all cells, or along one axis (ADR 0080).
            count(
                matches!(args.len(), 1 | 2),
                "an array, and an axis to work along",
            )?;
            let axes = of(&args[0])?;
            if axes.is_empty() {
                return Err((col, format!("'{name}' takes an array, e.g. {name}(T)")));
            }
            let Some(a) = args.get(1) else {
                return Ok(Vec::new());
            };
            match axis_named(a) {
                Some(i) if axes.contains(&i) => Ok(axes.into_iter().filter(|&x| x != i).collect()),
                Some(i) => not_over(&format!("'{name}''s array"), &axes, names[i]),
                None => Err((
                    col,
                    format!(
                        "'{name}(T, {})' works along an axis of T ({})",
                        names[axes[0]],
                        list(&axes)
                    ),
                )),
            }
        }
    }
}

/// A name as written, for messages.
pub(super) fn ast_name(a: &Ast) -> &str {
    match a {
        Ast::Name(n, _) => n,
        _ => "the array",
    }
}
