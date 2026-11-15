//! Lowering a checked syntax tree to an expression with its axes (ADR 0080).

use super::shapes::{ast_name, fit, line_of, mesh_expansion, number, union, Shapes};
use super::Axis;
use simsandbox_expr::{Along, ArrayOp, Ast, Expr, Op, MAX_AXES};
/// Lower `ast` to an expression (ADR 0080), with its axes: operands over
/// different axes are spread over all of them, reductions along an axis
/// and `on` become array instructions. `stencil` builds `lap`, `dd…`,
/// `prev`, `next` and `spacing` from their arguments. Checked by
/// [`axes_of`] first.
pub(super) fn lower(
    ast: &Ast,
    grid: &[Axis],
    shapes: &Shapes,
    slot: &dyn Fn(&str) -> Option<usize>,
    stencil: &dyn Fn(&str, &[Ast]) -> Expr,
    ids: &[Option<usize>],
    ends: &[Option<Vec<usize>>],
) -> std::result::Result<(Expr, Vec<usize>), simsandbox_expr::Error> {
    let low = |a: &Ast| lower(a, grid, shapes, slot, stencil, ids, ends);
    Ok(match ast {
        Ast::Num(v) => (Expr::Num(*v), Vec::new()),
        Ast::Name(n, c) => match slot(n) {
            Some(i) => (Expr::Var(i), shapes.get(n).cloned().unwrap_or_default()),
            None => {
                return Err(simsandbox_expr::Error {
                    column: *c,
                    message: format!("unknown name '{n}'"),
                })
            }
        },
        Ast::Neg(a) => {
            let (e, axes) = low(a)?;
            (Expr::Neg(Box::new(e)), axes)
        }
        Ast::Bin(op, a, b) => {
            let ((ea, aa), (eb, ab)) = (low(a)?, low(b)?);
            let to = union(&aa, &ab);
            (
                Expr::Bin(
                    *op,
                    Box::new(fit(ea, &aa, &to, None, grid)),
                    Box::new(fit(eb, &ab, &to, None, grid)),
                ),
                to,
            )
        }
        Ast::Call(f, args) => {
            let low: Vec<(Expr, Vec<usize>)> = args
                .iter()
                .map(low)
                .collect::<std::result::Result<_, _>>()?;
            let to = low.iter().fold(Vec::new(), |t, (_, a)| union(&t, a));
            let args = low
                .into_iter()
                .map(|(e, a)| fit(e, &a, &to, None, grid))
                .collect();
            (Expr::Call(*f, args), to)
        }
        Ast::Apply(name, args, _) => {
            let axis_named = |a: &Ast| match a {
                Ast::Name(n, _) => grid
                    .iter()
                    .position(|g| &g.name == n)
                    .expect("checked: an axis"),
                _ => unreachable!("checked: an axis"),
            };
            match name.as_str() {
                // Each row's id, and the number of rows (ADR 0079).
                "id" => {
                    let i = axis_named(&args[0]);
                    (Expr::Var(ids[i].expect("checked: a table")), vec![i])
                }
                "rows" => (
                    Expr::Num(grid[axis_named(&args[0])].cells as f64),
                    Vec::new(),
                ),
                // T at each edge's end (ADR 0079 stage 2): T's cells at
                // the places of the edges' `from` (or `to`) rows.
                "from_end" | "to_end" | "corner" => {
                    let (t, _) = low(&args[0])?;
                    let e = axis_named(&args[1]);
                    let k = match name.as_str() {
                        "from_end" => 0,
                        "to_end" => 1,
                        _ => match &args[2] {
                            Ast::Num(k) => *k as usize,
                            _ => unreachable!("checked: a corner's number"),
                        },
                    };
                    let places = ends[e].as_ref().expect("checked: cells")[k];
                    (
                        Expr::Array(ArrayOp::Take, vec![t, Expr::Var(places)]),
                        vec![e],
                    )
                }
                // A side's cells' values, and whether on the boundary.
                "inner" | "outer" | "across" | "boundary" => {
                    let f = axis_named(args.last().expect("checked: a sides table"));
                    let v = ends[f].as_ref().expect("checked: sides");
                    let n = v.len() - 4;
                    match name.as_str() {
                        "boundary" => (Expr::Var(v[n + 2]), vec![f]),
                        _ => {
                            let (t, _) = low(&args[0])?;
                            // Across: the outer cell, or on the boundary
                            // the inner one (ADR 0085 step 3).
                            let at = v[n + match name.as_str() {
                                "inner" => 0,
                                "outer" => 1,
                                _ => 3,
                            }];
                            (Expr::Array(ArrayOp::Take, vec![t, Expr::Var(at)]), vec![f])
                        }
                    }
                }
                "centre" | "measure" | "normal" => {
                    let e = mesh_expansion(name, args, grid).expect("checked: expands");
                    low(&e)?
                }
                // Each cell's sum over its sides of F, outwards: + where it
                // is the inner cell, − where the outer (ADR 0085 step 3).
                "outflow" => {
                    let (t, _) = low(&args[0])?;
                    let f = axis_named(&args[1]);
                    let c = grid[f].sides_of.or(grid[f].ends_of).expect("checked");
                    let v = ends[f].as_ref().expect("checked: sides or edges");
                    // Edges: + at `from`, − at `to` (their first and second
                    // places); sides: at the inner and the outer cell.
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
                    (
                        Expr::Bin(Op::Sub, Box::new(to(0)), Box::new(to(1))),
                        vec![c],
                    )
                }
                "of" => {
                    // The cell whose id is i: at(T, k), k the first place
                    // where id(pts) is i, NaN if none.
                    let (t, axes) = low(&args[0])?;
                    let (i, _) = low(&args[1])?;
                    let id = || Box::new(Expr::Var(ids[axes[0]].expect("checked: a table")));
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
                    (Expr::Array(ArrayOp::Cell, vec![t, place]), Vec::new())
                }
                "lap" | "prev" | "next" | "spacing" => {
                    let axes = match name.as_str() {
                        "spacing" => Vec::new(),
                        _ => shapes[ast_name(&args[0])].clone(),
                    };
                    (stencil(name, args), axes)
                }
                d if d.starts_with("dd") => {
                    (stencil(name, args), shapes[ast_name(&args[0])].clone())
                }
                "rcond" => {
                    let (ea, sa) = low(&args[0])?;
                    let op = ArrayOp::Rcond {
                        n: grid[sa[0]].cells,
                        rows: line_of(&sa, sa[0], grid).stride,
                        cols: line_of(&sa, sa[1], grid).stride,
                    };
                    (Expr::Array(op, vec![ea]), Vec::new())
                }
                "solve" => {
                    let ((ea, sa), (eb, sb)) = (low(&args[0])?, low(&args[1])?);
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
                    (Expr::Array(op, vec![ea, eb]), vec![c])
                }
                "on" => {
                    let (e, from) = low(&args[0])?;
                    let (a, b) = (axis_named(&args[1]), axis_named(&args[2]));
                    let mut to: Vec<usize> =
                        from.iter().map(|&x| if x == a { b } else { x }).collect();
                    to.sort_unstable();
                    (fit(e, &from, &to, Some((a, b)), grid), to)
                }
                "at" => {
                    let (first, axes) = low(&args[0])?;
                    if args.len() == 2 && number(&args[1]).is_none() {
                        // at(T, k), k computed: cell k in storage order.
                        let (k, _) = low(&args[1])?;
                        (Expr::Array(ArrayOp::Cell, vec![first, k]), Vec::new())
                    } else {
                        // at(T, i, j): cell i + j·(T's first axis), and so on.
                        let place = args[1..]
                            .iter()
                            .zip(&axes)
                            .map(|(a, &axis)| match a {
                                Ast::Num(i) => *i as usize * line_of(&axes, axis, grid).stride,
                                _ => unreachable!("checked: a whole number"),
                            })
                            .sum();
                        (Expr::Array(ArrayOp::At(place), vec![first]), Vec::new())
                    }
                }
                _ => {
                    let (first, axes) = low(&args[0])?;
                    if let Some(a) = args.get(1) {
                        let axis = axis_named(a);
                        let of = match name.as_str() {
                            "sum" => Along::Sum,
                            "mean" => Along::Mean,
                            "minof" => Along::MinOf,
                            "maxof" => Along::MaxOf,
                            "argminof" => Along::ArgMinOf,
                            _ => Along::ArgMaxOf,
                        };
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
                        (Expr::Array(op, vec![first]), rest)
                    } else {
                        let op = match name.as_str() {
                            "sum" => ArrayOp::Sum,
                            "mean" => ArrayOp::Mean,
                            "minof" => ArrayOp::MinOf,
                            "maxof" => ArrayOp::MaxOf,
                            "argminof" => ArrayOp::ArgMinOf,
                            _ => ArrayOp::ArgMaxOf,
                        };
                        (Expr::Array(op, vec![first]), Vec::new())
                    }
                }
            }
        }
    })
}
