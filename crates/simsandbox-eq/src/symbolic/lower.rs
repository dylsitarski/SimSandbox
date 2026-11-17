//! Lowering a checked syntax tree to an expression with its axes (ADR 0080).

use super::functions::{tables, Function};
use super::shapes::{ast_name, fit, line_of, mesh_expansion, number, union, Shapes};
use super::Axis;
use simsandbox_expr::{Along, ArrayOp, Ast, Expr, MAX_AXES};

/// An expression with its axes, or the mistake found lowering it.
pub(super) type Lowered = std::result::Result<(Expr, Vec<usize>), simsandbox_expr::Error>;

/// What lowering needs: the grid, every quantity's axes and where its
/// values are.
pub(super) struct Lower<'a> {
    pub(super) grid: &'a [Axis],
    shapes: &'a Shapes,
    slot: &'a dyn Fn(&str) -> Option<usize>,
    /// Builds `lap`, `dd…`, `prev`, `next` and `spacing`.
    pub(super) stencil: &'a dyn Fn(&Function, &[Ast]) -> Expr,
    /// Each table's ids, by axis.
    pub(super) ids: &'a [Option<usize>],
    /// Each edge, cell or side table's places, by axis.
    pub(super) ends: &'a [Option<Vec<usize>>],
}

/// Lower `ast` to an expression (ADR 0080), with its axes: operands over
/// different axes are spread over all of them, reductions along an axis
/// and `on` become array instructions. `stencil` builds `lap`, `dd…`,
/// `prev`, `next` and `spacing` from their arguments. Checked by
/// [`super::shapes::axes_of`] first.
pub(super) fn lower(
    ast: &Ast,
    grid: &[Axis],
    shapes: &Shapes,
    slot: &dyn Fn(&str) -> Option<usize>,
    stencil: &dyn Fn(&Function, &[Ast]) -> Expr,
    ids: &[Option<usize>],
    ends: &[Option<Vec<usize>>],
) -> Lowered {
    Lower {
        grid,
        shapes,
        slot,
        stencil,
        ids,
        ends,
    }
    .low(ast)
}

impl Lower<'_> {
    /// `ast` lowered, with its axes.
    pub(super) fn low(&self, ast: &Ast) -> Lowered {
        let grid = self.grid;
        let low = |a: &Ast| self.low(a);
        Ok(match ast {
            Ast::Num(v) => (Expr::Num(*v), Vec::new()),
            Ast::Name(n, c) => match (self.slot)(n) {
                Some(i) => (
                    Expr::Var(i),
                    self.shapes.get(n).cloned().unwrap_or_default(),
                ),
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
            Ast::Apply(name, args, _) => return self.apply(name, args),
        })
    }

    /// The axis an argument names: checked to be one.
    pub(super) fn axis_named(&self, a: &Ast) -> usize {
        match a {
            Ast::Name(n, _) => self
                .grid
                .iter()
                .position(|g| &g.name == n)
                .expect("checked: an axis"),
            _ => unreachable!("checked: an axis"),
        }
    }

    /// A call of an array function.
    fn apply(&self, name: &str, args: &[Ast]) -> Lowered {
        use Function::*;
        let f = Function::by_name(name).expect("the parser calls only array functions");
        match f {
            Id => tables::id_lower(self, args),
            Rows => tables::rows_lower(self, args),
            FromEnd => tables::end_lower(self, args, 0),
            ToEnd => tables::end_lower(self, args, 1),
            Corner => tables::corner_lower(self, args),
            Outflow => tables::outflow_lower(self, args),
            Of => tables::of_lower(self, args),
            _ => self.old_apply(name, args),
        }
    }

    fn old_apply(&self, name: &str, args: &[Ast]) -> Lowered {
        let (grid, shapes, stencil) = (self.grid, self.shapes, self.stencil);
        let low = |a: &Ast| self.low(a);
        let axis_named = |a: &Ast| self.axis_named(a);
        let ends = self.ends;
        Ok({
            match name {
                // A side's cells' values, and whether on the boundary.
                "inner" | "outer" | "across" | "boundary" => {
                    let f = axis_named(args.last().expect("checked: a sides table"));
                    let v = ends[f].as_ref().expect("checked: sides");
                    let n = v.len() - 4;
                    match name {
                        "boundary" => (Expr::Var(v[n + 2]), vec![f]),
                        _ => {
                            let (t, _) = low(&args[0])?;
                            // Across: the outer cell, or on the boundary
                            // the inner one (ADR 0085 step 3).
                            let at = v[n + match name {
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
                "lap" | "prev" | "next" | "spacing" => {
                    let axes = match name {
                        "spacing" => Vec::new(),
                        _ => shapes[ast_name(&args[0])].clone(),
                    };
                    (
                        stencil(&Function::by_name(name).expect("checked"), args),
                        axes,
                    )
                }
                d if d.starts_with("dd") => (
                    stencil(&Function::by_name(name).expect("checked"), args),
                    shapes[ast_name(&args[0])].clone(),
                ),
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
                        let of = match name {
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
                        let op = match name {
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
        })
    }
}
