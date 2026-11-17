//! The language's array functions (ADR 0060): [`Function`], the one
//! place a name becomes a function, and [`FUNCTIONS`], the table that
//! describes each for people and agents. Each family's file holds, for
//! its functions, the three rules the passes need: which axes a call is
//! over (with its argument checks), its unit, and its instructions. The
//! passes ([`super::shapes::axes_of`], `ArrayDims::apply` in
//! [`super::units`] and [`super::lower::lower`]) match [`Function`]
//! without a fallback, so a new function does not compile until each
//! handles it.

pub(super) mod meshes;
pub(super) mod tables;

use super::shapes::{axes_of, Shapes};
use super::Axis;
use simsandbox_expr::Ast;

/// An array function by what it does. `dd(T, x)` names its axis second,
/// `ddx(T)` in its own name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Function {
    // Along grids (grids.rs).
    Lap,
    Dd,
    /// `dd` and an axis's name: the axis as written, not yet looked up.
    DdAlong(String),
    Prev,
    Next,
    Spacing,
    On,
    // Over all cells or along an axis (reduce.rs).
    Sum,
    Mean,
    MinOf,
    MaxOf,
    ArgMinOf,
    ArgMaxOf,
    At,
    // Linear algebra (linear.rs).
    Solve,
    Rcond,
    // Tables and edges (tables.rs).
    Id,
    Rows,
    Of,
    FromEnd,
    ToEnd,
    Corner,
    Outflow,
    // Meshes' cells and sides (meshes.rs).
    Centre,
    Measure,
    Normal,
    Inner,
    Outer,
    Across,
    Boundary,
}

impl Function {
    /// The function `name` calls: one of [`simsandbox_expr::ARRAY_OPS`],
    /// or `dd` followed by an axis's name ([`simsandbox_expr::is_array_op`]).
    pub(super) fn by_name(name: &str) -> Option<Function> {
        use Function::*;
        Some(match name {
            "lap" => Lap,
            "dd" => Dd,
            "prev" => Prev,
            "next" => Next,
            "spacing" => Spacing,
            "on" => On,
            "sum" => Sum,
            "mean" => Mean,
            "minof" => MinOf,
            "maxof" => MaxOf,
            "argminof" => ArgMinOf,
            "argmaxof" => ArgMaxOf,
            "at" => At,
            "solve" => Solve,
            "rcond" => Rcond,
            "id" => Id,
            "rows" => Rows,
            "of" => Of,
            "from_end" => FromEnd,
            "to_end" => ToEnd,
            "corner" => Corner,
            "outflow" => Outflow,
            "centre" => Centre,
            "measure" => Measure,
            "normal" => Normal,
            "inner" => Inner,
            "outer" => Outer,
            "across" => Across,
            "boundary" => Boundary,
            _ => {
                let axis = name
                    .strip_prefix("dd")
                    .filter(|a| a.starts_with(|c: char| c.is_alphabetic() || c == '_'))?;
                DdAlong(axis.to_string())
            }
        })
    }
}

/// One array function as the reference describes it
/// (docs/reference/typed-equations.md): for `simsandbox describe`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FunctionInfo {
    /// Its name; [`DD_AXIS`] for `dd` followed by an axis's name.
    pub name: &'static str,
    /// How a call is written, as in the reference.
    pub signature: &'static str,
    /// What it gives, in one line.
    pub gives: &'static str,
    /// The unit of what it gives.
    pub unit: &'static str,
}

/// The name [`FUNCTIONS`] lists `ddx`, `ddy`, … under.
pub const DD_AXIS: &str = "dd<axis>";

const fn info(
    name: &'static str,
    signature: &'static str,
    gives: &'static str,
    unit: &'static str,
) -> FunctionInfo {
    FunctionInfo {
        name,
        signature,
        gives,
        unit,
    }
}

/// Every array function, for people and agents; held to
/// [`simsandbox_expr::ARRAY_OPS`] and to the reference by tests.
pub const FUNCTIONS: &[FunctionInfo] = &[
    // Along grids.
    info(
        "lap",
        "lap(T)",
        "The sum of second differences along each of T's axes, which need one unit",
        "T's over the axis's squared",
    ),
    info(
        DD_AXIS,
        "ddx(T)",
        "The central difference along axis x, named after dd",
        "T's over the axis's",
    ),
    info(
        "dd",
        "dd(T, x)",
        "The central difference along axis x, named second",
        "T's over the axis's",
    ),
    info(
        "prev",
        "prev(T)",
        "The previous cell's value along the axis (prev(T, x) names it when T has several)",
        "T's",
    ),
    info(
        "next",
        "next(T)",
        "The next cell's value along the axis (next(T, x) names it when T has several)",
        "T's",
    ),
    info(
        "spacing",
        "spacing(x)",
        "The width of grid axis x's cells, one value",
        "The axis's",
    ),
    info(
        "on",
        "on(e, a, b)",
        "e with its axis a read as axis b of the same length",
        "e's",
    ),
    // Over all cells, or along one axis named second.
    info(
        "sum",
        "sum(e)",
        "The sum over all cells; sum(e, x) along axis x, over the axes left",
        "e's",
    ),
    info(
        "mean",
        "mean(e)",
        "The mean over all cells; mean(e, x) along axis x",
        "e's",
    ),
    info(
        "minof",
        "minof(e)",
        "The least value over all cells; minof(e, x) along axis x",
        "e's",
    ),
    info(
        "maxof",
        "maxof(e)",
        "The greatest value over all cells; maxof(e, x) along axis x",
        "e's",
    ),
    info(
        "argminof",
        "argminof(e)",
        "The number of the least value's cell, from 0; with an axis, the place along it",
        "None",
    ),
    info(
        "argmaxof",
        "argmaxof(e)",
        "The number of the greatest value's cell, from 0; with an axis, the place along it",
        "None",
    ),
    info(
        "at",
        "at(e, i)",
        "One cell: a whole number for each axis, or one computed number",
        "e's; the cell numbers have none",
    ),
    // Linear algebra.
    info(
        "solve",
        "solve(A, b)",
        "x with A·x = b: A over two axes of one length, b over one of them; NaN when singular",
        "b's over A's",
    ),
    info(
        "rcond",
        "rcond(A)",
        "How well conditioned A is, from 0 (singular) to 1",
        "None",
    ),
    // Tables and edges.
    info("id", "id(tbl)", "Each row's id", "None"),
    info("rows", "rows(tbl)", "The number of rows now", "None"),
    info(
        "of",
        "of(T, i)",
        "T in the row whose id is i; NaN if there is none",
        "T's; i has none",
    ),
    info(
        "from_end",
        "from_end(T, e)",
        "T (over the points) at each edge's first end",
        "T's",
    ),
    info(
        "to_end",
        "to_end(T, e)",
        "T (over the points) at each edge's second end",
        "T's",
    ),
    info(
        "corner",
        "corner(T, c, k)",
        "T at corner k of each cell",
        "T's",
    ),
    info(
        "outflow",
        "outflow(F, e)",
        "Per point, the flows F leaving along its edges less those arriving; per cell, F summed over its sides, outward",
        "F's",
    ),
    // Meshes' cells and sides.
    info(
        "centre",
        "centre(T, c)",
        "The mean of T over each row's corners",
        "T's",
    ),
    info(
        "measure",
        "measure(c, x, y)",
        "Each row's length, area or volume from its corners' coordinates",
        "The coordinates', which need one unit, to the power of the shape's dimension",
    ),
    info(
        "normal",
        "normal(f, k, x, y)",
        "Component k of each side's unit normal, outward from its inner cell",
        "None; the coordinates need one unit",
    ),
    info(
        "inner",
        "inner(u, f)",
        "u (over the cells) in each side's inner cell",
        "u's",
    ),
    info(
        "outer",
        "outer(u, f)",
        "u (over the cells) in each side's outer cell",
        "u's",
    ),
    info(
        "across",
        "across(u, f)",
        "u in the outer cell, or at the boundary the inner",
        "u's",
    ),
    info(
        "boundary",
        "boundary(f)",
        "1 for a side on the boundary, else 0",
        "None",
    ),
];

/// A mistake with its column.
pub(super) type Fail = (usize, String);

/// The axes of a call's value, or its mistake.
pub(super) type Axes = std::result::Result<Vec<usize>, Fail>;

/// A call being checked by [`axes_of`]: its name as written, arguments
/// and column, and what checking its arguments needs.
pub(super) struct Check<'a> {
    name: &'a str,
    args: &'a [Ast],
    col: usize,
    grid: &'a [Axis],
    shapes: &'a Shapes,
    /// The axes' names, in grid order.
    names: Vec<&'a str>,
}

#[allow(dead_code)] // TEMP: until every family is moved
impl<'a> Check<'a> {
    pub(super) fn new(
        name: &'a str,
        args: &'a [Ast],
        col: usize,
        grid: &'a [Axis],
        shapes: &'a Shapes,
    ) -> Self {
        Check {
            name,
            args,
            col,
            grid,
            shapes,
            names: grid.iter().map(|a| a.name.as_str()).collect(),
        }
    }

    /// The axes of an argument.
    fn of(&self, a: &Ast) -> Axes {
        axes_of(a, self.grid, self.shapes)
    }

    fn fail<T>(&self, message: String) -> std::result::Result<T, Fail> {
        Err((self.col, message))
    }

    /// The axis an argument names, if it is a name of one.
    fn axis_named(&self, a: &Ast) -> Option<usize> {
        match a {
            Ast::Name(n, _) => self.names.iter().position(|m| m == n),
            _ => None,
        }
    }

    /// The table an argument names, if it is a name of one.
    fn table_named(&self, a: &Ast) -> Option<usize> {
        self.axis_named(a).filter(|&i| self.grid[i].table.is_some())
    }

    /// Axes by name, as a message lists them.
    fn list(&self, axes: &[usize]) -> String {
        axes.iter()
            .map(|&a| self.names[a])
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The axes listed, or "no axis".
    fn list_or_none(&self, axes: &[usize]) -> String {
        if axes.is_empty() {
            "no axis".to_string()
        } else {
            self.list(axes)
        }
    }

    /// The call has as many arguments as it takes (`ok`), `want` said if not.
    fn count(&self, ok: bool, want: &str) -> std::result::Result<(), Fail> {
        if ok {
            Ok(())
        } else {
            self.fail(format!(
                "'{}' takes {want}, got {} argument(s)",
                self.name,
                self.args.len()
            ))
        }
    }

    /// A stencil's array: a declared or defined quantity by name.
    fn named_array(&self) -> Axes {
        let name = self.name;
        match &self.args[0] {
            Ast::Name(n, _)
                if !self.names.contains(&n.as_str())
                    && self.shapes.get(n).is_some_and(|s| !s.is_empty()) =>
            {
                Ok(self.shapes[n].clone())
            }
            _ => self.fail(format!(
                "'{name}' takes the name of an array, e.g. {name}(T); define one above (D = k*T) for an expression"
            )),
        }
    }

    /// `what`, over `axes`, is not over the axis `a`.
    fn not_over(&self, what: &str, axes: &[usize], a: &str) -> Axes {
        self.fail(format!(
            "{what} is over {}, not {a}",
            self.list_or_none(axes)
        ))
    }

    /// Rows have no neighbours by place (ADR 0079: edges come with stage 2).
    fn no_rows(&self, axes: Vec<usize>) -> Axes {
        match axes.iter().find(|&&a| self.grid[a].table.is_some()) {
            Some(&a) => self.fail(format!(
                "'{}' works along a grid; {} is a table, whose rows have no neighbours by place",
                self.name, self.names[a]
            )),
            None => Ok(axes),
        }
    }
}
