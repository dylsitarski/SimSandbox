//! Which axes an expression is over: the checks before lowering, and the
//! helpers for grids, cells and meshes they use.

use super::functions::{grids, linear, meshes, reduce, tables, Check, Function};
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

/// The axes of `ast`'s value (ADR 0080), checking every array operation:
/// known, with the right arguments (each function's rule is in
/// [`super::functions`]). Names not in `shapes` are scalars.
pub(super) fn axes_of(
    ast: &Ast,
    grid: &[Axis],
    shapes: &Shapes,
) -> std::result::Result<Vec<usize>, (usize, String)> {
    let of = |a: &Ast| axes_of(a, grid, shapes);
    let (name, args, col) = match ast {
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
        Ast::Apply(name, args, col) => (name, args, *col),
    };
    // The parser makes calls only of array functions, each with an
    // argument; a call without one is a split's or merge's transfer
    // (`share`), which is never a quantity's value. Refused here, the
    // first pass, so no later one meets it.
    if args.is_empty() {
        return Err((
            col,
            format!("'{name}' alone is a split's or merge's transfer, not a value"),
        ));
    }
    if grid.is_empty() {
        return Err((
            col,
            format!("'{name}' works on arrays: declare a grid with 'grid x [m] = 0 .. length, 50'"),
        ));
    }
    use Function::*;
    let c = Check::new(name, args, col, grid, shapes);
    match Function::by_name(name).expect("the parser calls only array functions") {
        Lap => grids::lap_axes(&c),
        Dd => grids::dd_axes(&c),
        DdAlong(axis) => grids::dd_along_axes(&c, &axis),
        Prev | Next => grids::shift_axes(&c),
        Spacing => grids::spacing_axes(&c),
        On => grids::on_axes(&c),
        Sum | Mean | MinOf | MaxOf | ArgMinOf | ArgMaxOf => reduce::along_axes(&c),
        At => reduce::at_axes(&c),
        Solve => linear::solve_axes(&c),
        Rcond => linear::rcond_axes(&c),
        Id => tables::id_axes(&c),
        Rows => tables::rows_axes(&c),
        Of => tables::of_axes(&c),
        FromEnd | ToEnd => tables::ends_axes(&c, false),
        Corner => tables::ends_axes(&c, true),
        Outflow => tables::outflow_axes(&c),
        Centre => meshes::centre_axes(&c),
        Measure => meshes::expansion_axes(&c, meshes::measure_expansion),
        Normal => meshes::expansion_axes(&c, meshes::normal_expansion),
        Inner | Outer | Across => meshes::sides_axes(&c, false),
        Boundary => meshes::sides_axes(&c, true),
    }
}

/// A name as written, for messages.
pub(super) fn ast_name(a: &Ast) -> &str {
    match a {
        Ast::Name(n, _) => n,
        _ => "the array",
    }
}
