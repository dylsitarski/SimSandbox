//! Lowering a checked syntax tree to an expression with its axes (ADR 0080).

use super::functions::{linear, meshes, reduce, tables, Function};
use super::shapes::{ast_name, fit, union, Shapes};
use super::Axis;
use simsandbox_expr::{Along, Ast, Expr};

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
            Centre => meshes::expansion_lower(self, args, meshes::centre_expansion),
            Measure => meshes::expansion_lower(self, args, meshes::measure_expansion),
            Normal => meshes::expansion_lower(self, args, meshes::normal_expansion),
            Inner => meshes::cell_lower(self, args, 0),
            Outer => meshes::cell_lower(self, args, 1),
            Across => meshes::cell_lower(self, args, 3),
            Boundary => meshes::boundary_lower(self, args),
            Solve => linear::solve_lower(self, args),
            Rcond => linear::rcond_lower(self, args),
            Sum => reduce::along_lower(self, args, Along::Sum),
            Mean => reduce::along_lower(self, args, Along::Mean),
            MinOf => reduce::along_lower(self, args, Along::MinOf),
            MaxOf => reduce::along_lower(self, args, Along::MaxOf),
            ArgMinOf => reduce::along_lower(self, args, Along::ArgMinOf),
            ArgMaxOf => reduce::along_lower(self, args, Along::ArgMaxOf),
            At => reduce::at_lower(self, args),
            _ => self.old_apply(name, args),
        }
    }

    fn old_apply(&self, name: &str, args: &[Ast]) -> Lowered {
        let (grid, shapes, stencil) = (self.grid, self.shapes, self.stencil);
        let low = |a: &Ast| self.low(a);
        let axis_named = |a: &Ast| self.axis_named(a);
        Ok({
            match name {
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
                "on" => {
                    let (e, from) = low(&args[0])?;
                    let (a, b) = (axis_named(&args[1]), axis_named(&args[2]));
                    let mut to: Vec<usize> =
                        from.iter().map(|&x| if x == a { b } else { x }).collect();
                    to.sort_unstable();
                    (fit(e, &from, &to, Some((a, b)), grid), to)
                }
                _ => unreachable!(),
            }
        })
    }
}
