//! Equations typed as text become an [`OdeSystem`] (ADR 0017).
//!
//! ```text
//! # Lorenz system
//! dx/dt = sigma*(y - x)
//! dy/dt = x*(rho - z) - y
//! dz/dt = x*y - beta*z
//! r = sqrt(x^2 + y^2 + z^2)
//! ```
//!
//! - `dx/dt = …` (or `x' = …`) makes `x` a state that changes continuously.
//! - `x[n+1] = …` makes `x` a state of a discrete map, updated once per step
//!   (ADR 0019); on the right, `x[n]` and `x` both mean the current value.
//!   A block is either continuous or a map. `n` is reserved in maps.
//! - A block with only `name = …` lines is a set of formulas with no state
//!   (ADR 0019), evaluated from its parameters (and `t`).
//! - `name = …` defines a quantity computed from the states, e.g. an
//!   energy; it may use quantities defined above it.
//! - `input u, F` makes `u` and `F` input ports (ADR 0040), set by whatever
//!   is connected to them, instead of parameters. A defined quantity that
//!   uses an input (directly or through quantities above it) depends on it
//!   at the same instant (direct feedthrough); states do not.
//! - Every other name is a parameter (default 1). Each state `x` also gets
//!   an initial-value parameter `x0` (default 1), marked as an initial
//!   condition. `t` is time; `pi` and the function names are reserved.
//! - Every state and defined quantity is an output port of the same name.
//! - Blank lines and lines starting with `#` are ignored.
//! - `state x [m] = 0` declares a state with its unit and initial value
//!   (the default of `x0`); its dynamics are still a `dx/dt` or `x[n+1]`
//!   line (ADR 0049). Declaring is optional.
//! - Units (ADR 0045) go in brackets where a name is introduced:
//!   `state x [m]`, `input F [N], v [m/s]`, `param k [N/m] = 4`, and at
//!   the end of an equation for its left side as written: `E = m*v^2/2
//!   [J]`, `dv/dt = F/m [m/s^2]` (so `v` is in m/s: the rate's unit times
//!   s, or SI when that has no name; ADR 0049). Names without a unit are
//!   dimensionless; `t` is in s. Every equation must balance: terms added share a dimension,
//!   `dx/dt` has `x`'s unit per second, and functions such as `sin` and
//!   `exp` take dimensionless arguments. Values are computed in SI, so
//!   `km` and `m` mix correctly inside a block; ports carry the declared
//!   unit, and connections need equal units (convert with `units.convert`).

use crate::ode::OdeSystem;
use simsandbox_core::block::{
    AccuracyClass, Block, BlockSpec, Domain, Equations, MethodLabels, MethodSpec, ParamSpec,
    PortSpec, Shape, SpeedClass, StepContext, StepIo, TimeSemantics,
};
use simsandbox_core::registry::MethodSettings;
use simsandbox_core::units::Unit;
use simsandbox_core::{Result, SimError};
use simsandbox_expr::program::Span;
use simsandbox_expr::units::{dim_text, dimension_with, from_si, in_si, Dim, NO_DIM};
use simsandbox_expr::{
    is_reserved, parse, ArrayOp, Ast, Edge, Expr, Line as GridLine, Op, Program,
};
use std::cell::RefCell;

/// What a set of typed equations describes (ADR 0019).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `dx/dt = …`: integrated by an ODE method.
    Continuous,
    /// `x[n+1] = …`: iterated once per step (method `iterate`).
    Map,
    /// Only `name = …`: evaluated, no state (method `evaluate`).
    Formulas,
}

impl Kind {
    /// The method a block of this kind uses by default.
    pub fn default_method(self) -> MethodSettings {
        match self {
            Kind::Continuous => MethodSettings::new("rk4").with("substeps", 1.0),
            Kind::Map => MethodSettings::new(ITERATE),
            Kind::Formulas => MethodSettings::new(EVALUATE),
        }
    }
}

/// Method name of map blocks.
pub const ITERATE: &str = "iterate";
/// Method name of formula blocks.
pub const EVALUATE: &str = "evaluate";

/// A system of typed equations.
pub struct EquationSet {
    kind: Kind,
    spec: BlockSpec,
    /// Where the values of `t`, the states, inputs and parameters go in
    /// the registers (each run of them is contiguous), and where the
    /// outputs after the states (defined quantities and the grid's axis)
    /// and the rates are read from.
    layout: Layout,
    /// Computes the defined quantities and the grid's axis (ADR 0060's
    /// instruction list).
    outputs: Program,
    /// Computes the defined quantities, then each state's right-hand side
    /// into the registers of its rate.
    rates: Program,
    /// Computes the initial values of array states from the parameters.
    init: Option<Program>,
    /// For each state: its number of values, and the parameter holding
    /// its initial value (scalar states) or `None` (arrays, set by `init`).
    initial: Vec<(usize, Option<usize>)>,
    /// Register buffers of the programs: outputs, rates, initial values.
    scratch: RefCell<[Vec<f64>; 3]>,
}

/// Register positions of an [`EquationSet`]: `t` at 0, then the state
/// values, input values and parameters, each contiguous.
#[derive(Debug, Clone, Copy)]
struct Layout {
    states: usize,
    inputs: usize,
    params: usize,
    /// The outputs after the states: from here, this many registers.
    outputs_at: usize,
    outputs_len: usize,
    /// The rates of the state values.
    rates_at: usize,
}

/// A grid axis (ADR 0060): `grid x [m] = a .. b, n`.
struct Axis {
    name: String,
    line: usize,
    lo: Ast,
    hi: Ast,
    cells: usize,
}

/// An expression as written: its text, line and column.
type Written = (String, usize, usize);

/// One end condition of an `edge` line, as written.
enum EdgeKind {
    Value(Ast),
    Gradient(Ast),
    Periodic,
}

/// An `edge` line's condition for one end (`high` false: low end) of
/// array `array`.
struct EdgeLine {
    array: String,
    line: usize,
    column: usize,
    /// Which axis of the grid, by place.
    axis: usize,
    high: bool,
    kind: EdgeKind,
}

struct Line {
    number: usize,
    /// Character offset of the right-hand side within the line.
    offset: usize,
    lhs: Lhs,
    ast: Ast,
}

/// Split a trailing unit annotation, ` [unit]`, off the right-hand side
/// of an equation. `[n]` (a map's current value) is not a unit. Returns
/// the text without it and the unit's text and column.
fn split_unit(rhs: &str) -> (String, Option<(String, usize)>) {
    let trimmed = rhs.trim_end();
    if let (Some(open), true) = (trimmed.rfind('['), trimmed.ends_with(']')) {
        let inside = &trimmed[open + 1..trimmed.len() - 1];
        let spaced = trimmed[..open].ends_with(char::is_whitespace);
        let index = matches!(inside.trim(), "n" | "n+1");
        if spaced && !index {
            let col = trimmed[..open].chars().count() + 2;
            return (
                trimmed[..open].to_string(),
                Some((inside.trim().to_string(), col)),
            );
        }
    }
    (rhs.to_string(), None)
}

/// A name with an optional unit: `F`, `F [N]`, `F[N]`.
fn name_and_unit(item: &str) -> (String, Option<String>) {
    match item.split_once('[') {
        Some((n, rest)) => (
            n.trim().to_string(),
            Some(rest.trim_end().trim_end_matches(']').trim().to_string()),
        ),
        None => (item.trim().to_string(), None),
    }
}

fn read_unit(text: &str, line: usize, column: usize) -> Result<Unit> {
    Unit::parse(text).map_err(|e| definition_error(line, column, e))
}

enum Lhs {
    State(String),
    Map(String),
    Defined(String),
}

fn definition_error(line: usize, column: usize, message: impl std::fmt::Display) -> SimError {
    SimError::InvalidDefinition(format!("line {line}, column {column}: {message}"))
}

fn is_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// Read the left-hand side: `dx/dt` or `x'` for a continuous state,
/// `x[n+1]` for a map state, a name otherwise.
fn read_lhs(lhs: &str) -> Option<Lhs> {
    let compact: String = lhs.chars().filter(|c| !c.is_whitespace()).collect();
    if let Some(name) = compact.strip_suffix("[n+1]") {
        return is_name(name).then(|| Lhs::Map(name.into()));
    }
    if let Some(name) = compact.strip_suffix('\'') {
        return is_name(name).then(|| Lhs::State(name.into()));
    }
    if let Some(name) = compact
        .strip_prefix('d')
        .and_then(|r| r.strip_suffix("/dt"))
    {
        if is_name(name) {
            return Some(Lhs::State(name.into()));
        }
    }
    is_name(&compact).then_some(Lhs::Defined(compact))
}

/// The 1-based column where `piece` starts in `text` (1 if absent).
fn column_of(text: &str, piece: &str) -> usize {
    text.find(piece)
        .map_or(1, |i| text[..i].chars().count() + 1)
}

/// Split at commas outside brackets: `T[x, y] [K], q` gives two items.
fn split_top(s: &str) -> Vec<&str> {
    let (mut depth, mut start, mut out) = (0i32, 0, Vec::new());
    for (i, c) in s.char_indices() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// A declared name with its grid axes, `T[x]` (ADR 0060): the
/// declaration without them, and whether it is an array. Brackets right
/// after the name hold axes when every item names an axis of the grid;
/// otherwise they are a unit (`F[N]`), left in place.
fn split_dims(head: &str, axes: &[&str], line: usize) -> Result<(String, bool)> {
    let h = head.trim_start();
    let Some(open) = h.find('[') else {
        return Ok((head.to_string(), false));
    };
    let (name, rest) = h.split_at(open);
    let Some(close) = rest.find(']') else {
        return Ok((head.to_string(), false));
    };
    let items: Vec<&str> = rest[1..close].split(',').map(str::trim).collect();
    if !is_name(name) || axes.is_empty() || !items.iter().all(|i| axes.contains(i)) {
        return Ok((head.to_string(), false));
    }
    if items != axes {
        return Err(definition_error(
            line,
            1,
            format!(
                "an array lies on the whole grid: write {name}[{}]",
                axes.join(", ")
            ),
        ));
    }
    Ok((format!("{name}{}", &rest[close + 1..]), true))
}

/// A `grid x [m] = a .. b, n` line (ADR 0060), `rest` after `grid`.
fn read_grid(text: &str, rest: &str, line: usize) -> Result<(Axis, Option<String>)> {
    let usage = "write a grid axis as 'grid x [m] = 0 .. length, 50': its name, unit, ends and number of cells";
    let Some((head, body)) = rest.split_once('=') else {
        return Err(definition_error(line, 1, usage));
    };
    let (name, unit) = name_and_unit(head);
    if !is_name(&name) || name == "t" || is_reserved(&name) {
        return Err(definition_error(
            line,
            1,
            format!("'{name}' cannot be an axis name"),
        ));
    }
    let Some((range, cells)) = body.rsplit_once(',') else {
        return Err(definition_error(line, column_of(text, body), usage));
    };
    let cells = match cells.trim().parse::<usize>() {
        Ok(n) if n >= 1 => n,
        _ => {
            return Err(definition_error(
                line,
                column_of(text, cells.trim()),
                format!(
                    "the number of cells is a whole number of at least 1, not '{}'",
                    cells.trim()
                ),
            ))
        }
    };
    let Some((lo, hi)) = range.split_once("..") else {
        return Err(definition_error(line, column_of(text, range), usage));
    };
    let end = |src: &str| -> Result<Ast> {
        let col = column_of(text, src.trim());
        parse(src).map_err(|e| definition_error(line, col + e.column - 1, e.message))
    };
    Ok((
        Axis {
            name,
            line,
            lo: end(lo)?,
            hi: end(hi)?,
            cells,
        },
        unit,
    ))
}

/// An `edge T: x- = left, x+ gradient = 0, x periodic` line (ADR 0060),
/// `rest` after `edge`.
fn read_edges(text: &str, rest: &str, line: usize, axes: &[&str]) -> Result<Vec<EdgeLine>> {
    let Some(&first) = axes.first() else {
        return Err(definition_error(
            line,
            1,
            "edges belong to arrays on a grid: declare one with 'grid x [m] = 0 .. length, 50'",
        ));
    };
    let Some((array, items)) = rest.split_once(':') else {
        return Err(definition_error(
            line,
            1,
            format!("write edges as 'edge T: {first}- = value, {first}+ gradient = 0' or 'edge T: {first} periodic'"),
        ));
    };
    let array = array.trim().to_string();
    let all = axes.join(", ");
    let mut out = Vec::new();
    for item in split_top(items) {
        let item = item.trim();
        let column = column_of(text, item);
        let bad = |what: String| definition_error(line, column, what);
        if let Some(ax) = item.strip_suffix("periodic") {
            let Some(axis) = axes.iter().position(|a| *a == ax.trim()) else {
                return Err(bad(format!(
                    "'{}' is not an axis of the grid ({all}): write e.g. '{first} periodic'",
                    ax.trim()
                )));
            };
            for high in [false, true] {
                out.push(EdgeLine {
                    array: array.clone(),
                    line,
                    column,
                    axis,
                    high,
                    kind: EdgeKind::Periodic,
                });
            }
            continue;
        }
        let Some((left, right)) = item.split_once('=') else {
            return Err(bad(format!(
                "expected '{first}- = value', '{first}+ gradient = value' or '{first} periodic'"
            )));
        };
        let words: Vec<&str> = left.split_whitespace().collect();
        let end = words.first().and_then(|w| {
            let (name, high) = match (w.strip_suffix('-'), w.strip_suffix('+')) {
                (Some(n), _) => (n, false),
                (_, Some(n)) => (n, true),
                _ => return None,
            };
            axes.iter()
                .position(|a| *a == name)
                .map(|axis| (axis, high))
        });
        let Some((axis, high)) = end else {
            return Err(bad(format!(
                "an end is an axis of the grid ({all}) with '-' (low) or '+' (high), e.g. '{first}-', not '{}'",
                left.trim()
            )));
        };
        let gradient = match &words[1..] {
            [] => false,
            ["gradient"] => true,
            _ => {
                return Err(bad(format!(
                    "expected '{} = value' or '{} gradient = value'",
                    words[0], words[0]
                )))
            }
        };
        let col = column_of(text, right.trim());
        let ast =
            parse(right).map_err(|e| definition_error(line, col + e.column - 1, e.message))?;
        out.push(EdgeLine {
            array: array.clone(),
            line,
            column,
            axis,
            high,
            kind: if gradient {
                EdgeKind::Gradient(ast)
            } else {
                EdgeKind::Value(ast)
            },
        });
    }
    Ok(out)
}

/// The variables `e` reads, edge values of its stencils included.
fn reads(e: &Expr, out: &mut Vec<usize>) {
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
            if let ArrayOp::Second { edges, .. } | ArrayOp::First { edges, .. } = op {
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
fn needed<'a>(steps: Vec<(usize, &'a Expr)>, wanted: &[usize]) -> Vec<(usize, &'a Expr)> {
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
fn number(ast: &Ast) -> Option<f64> {
    match ast {
        Ast::Num(v) => Some(*v),
        Ast::Neg(a) => number(a).map(|v| -v),
        _ => None,
    }
}

/// Whether `ast` has one value per cell: it uses an array (a name in
/// `arrays`, a stencil) other than through a reduction.
fn is_array(ast: &Ast, arrays: &[String]) -> bool {
    match ast {
        Ast::Num(_) => false,
        Ast::Name(n, _) => arrays.contains(n),
        Ast::Neg(a) => is_array(a, arrays),
        Ast::Bin(_, a, b) => is_array(a, arrays) || is_array(b, arrays),
        Ast::Call(_, args) => args.iter().any(|a| is_array(a, arrays)),
        Ast::Apply(name, ..) => name == "lap" || name.starts_with("dd"),
    }
}

/// Check every array operation in `ast` (ADR 0060): known, with the
/// right arguments. `arrays` are the names with a value per cell.
fn check_array_ops(
    ast: &Ast,
    grid: &[Axis],
    arrays: &[String],
) -> std::result::Result<(), (usize, String)> {
    let args = match ast {
        Ast::Num(_) | Ast::Name(..) => return Ok(()),
        Ast::Neg(a) => return check_array_ops(a, grid, arrays),
        Ast::Bin(_, a, b) => {
            check_array_ops(a, grid, arrays)?;
            return check_array_ops(b, grid, arrays);
        }
        Ast::Call(_, args) => args,
        Ast::Apply(name, args, col) => {
            let col = *col;
            if grid.is_empty() {
                return Err((
                    col,
                    format!("'{name}' works on arrays: declare a grid with 'grid x [m] = 0 .. length, 50'"),
                ));
            }
            let names: Vec<&str> = grid.iter().map(|a| a.name.as_str()).collect();
            let all = names.join(", ");
            let want = if name == "at" { 1 + grid.len() } else { 1 };
            if args.len() != want {
                return Err((
                    col,
                    format!("'{name}' takes {want} argument(s), got {}", args.len()),
                ));
            }
            let named_array = |what: &str| -> std::result::Result<(), (usize, String)> {
                match &args[0] {
                    Ast::Name(n, _) if arrays.contains(n) && !names.contains(&n.as_str()) => Ok(()),
                    _ => Err((
                        col,
                        format!("{what} takes the name of an array, e.g. {name}(T); define one above (D = k*T) for an expression"),
                    )),
                }
            };
            match name.as_str() {
                "lap" => named_array("'lap'")?,
                d if d.starts_with("dd") => {
                    if !names.contains(&&d[2..]) {
                        return Err((col, format!("'{d}' is a derivative along '{}', which is not an axis of the grid ({all}): write dd{}", &d[2..], names[0])));
                    }
                    named_array(&format!("'{d}'"))?
                }
                "spacing" => {
                    if !matches!(&args[0], Ast::Name(n, _) if names.contains(&n.as_str())) {
                        return Err((
                            col,
                            format!("'spacing' takes an axis: spacing({})", names[0]),
                        ));
                    }
                }
                "at" => {
                    if !is_array(&args[0], arrays) {
                        let cells = vec!["0"; grid.len()].join(", ");
                        return Err((
                            col,
                            format!("'at' takes an array and a cell number along each axis, e.g. at(T, {cells})"),
                        ));
                    }
                    for (a, i) in grid.iter().zip(&args[1..]) {
                        match i {
                            Ast::Num(i)
                                if i.fract() == 0.0 && *i >= 0.0 && (*i as usize) < a.cells => {}
                            _ => {
                                return Err((
                                    col,
                                    format!(
                                        "a cell number along {} is a whole number from 0 to {}",
                                        a.name,
                                        a.cells - 1
                                    ),
                                ))
                            }
                        }
                    }
                }
                _ => {
                    if !is_array(&args[0], arrays) {
                        return Err((col, format!("'{name}' takes an array, e.g. {name}(T)")));
                    }
                }
            }
            args
        }
    };
    args.iter()
        .try_for_each(|a| check_array_ops(a, grid, arrays))
}

impl EquationSet {
    /// Build from the lines of a definition, as a Block named `name`.
    pub fn parse(name: &str, lines: &[String]) -> Result<Self> {
        // `param = 2` and the like are equations defining such a name.
        let keyword = |line: &str, word: &str| -> Option<String> {
            let rest = line.strip_prefix(word)?.strip_prefix(' ')?;
            (!rest.trim_start().starts_with('=')).then(|| rest.to_string())
        };
        // Units by name, as declared (ADR 0045).
        let mut units: std::collections::HashMap<String, Unit> = Default::default();
        // The grid first (ADR 0060): declarations further down refer to
        // its axes wherever they are written. The first axis is x (the
        // fastest in the grid's cells), the second y.
        let mut grid: Vec<Axis> = Vec::new();
        for (k, text) in lines.iter().enumerate() {
            let Some(rest) = keyword(text.trim(), "grid") else {
                continue;
            };
            let (a, unit) = read_grid(text, &rest, k + 1)?;
            if grid.len() == 2 {
                return Err(definition_error(
                    k + 1,
                    1,
                    "a grid has one or two axes (3D grids are not built yet)",
                ));
            }
            if grid.iter().any(|g| g.name == a.name) {
                return Err(definition_error(
                    k + 1,
                    1,
                    format!("the axis {} is declared twice", a.name),
                ));
            }
            if let Some(u) = unit {
                units.insert(a.name.clone(), read_unit(&u, k + 1, 1)?);
            }
            grid.push(a);
        }
        let axes: Vec<&str> = grid.iter().map(|a| a.name.as_str()).collect();
        // How each axis runs through the cells, stored row by row.
        let nx = grid.first().map_or(1, |a| a.cells);
        let lines_of: Vec<GridLine> = grid
            .iter()
            .enumerate()
            .map(|(i, a)| GridLine {
                stride: if i == 0 { 1 } else { nx },
                count: a.cells,
            })
            .collect();

        // Then the left-hand sides, which decide the kind of block, and
        // the declarations.
        let mut sides = Vec::new();
        let mut inputs: Vec<String> = Vec::new();
        // Declared parameters with their defaults.
        let mut declared: Vec<(String, f64, usize)> = Vec::new();
        // `state` declarations (ADR 0049): name and line.
        let mut state_decls: Vec<(String, usize)> = Vec::new();
        // Names with a value per cell, and array states' initial values
        // as written (expression, line, column).
        let mut arrays: Vec<String> = axes.iter().map(|a| a.to_string()).collect();
        let mut array_init: Vec<(String, Option<Written>)> = Vec::new();
        let mut edges: Vec<EdgeLine> = Vec::new();
        // Units given for a rate, `dx/dt = … [m/s]`: unit, line, column.
        let mut rates: Vec<(String, Unit, usize, usize)> = Vec::new();
        for (k, text) in lines.iter().enumerate() {
            let number = k + 1;
            let trimmed = text.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || keyword(trimmed, "grid").is_some()
            {
                continue;
            }
            if let Some(rest) = keyword(trimmed, "edge") {
                edges.extend(read_edges(text, &rest, number, &axes)?);
                continue;
            }
            if let Some(rest) = keyword(trimmed, "state") {
                // `state x [m] = 0`, `state x [m]`, `state x = 0`, and
                // arrays `state T[x] [K] = expression`.
                let (head, value) = match rest.split_once('=') {
                    Some((h, v)) => (h, Some(v)),
                    None => (rest.as_str(), None),
                };
                let (head, array) = split_dims(head, &axes, number)?;
                let (n, unit) = name_and_unit(&head);
                if !is_name(&n) || n == "t" || is_reserved(&n) || axes.contains(&n.as_str()) {
                    return Err(definition_error(
                        number,
                        1,
                        format!("'{n}' cannot be a state name"),
                    ));
                }
                if state_decls.iter().any(|d| d.0 == n) {
                    return Err(definition_error(
                        number,
                        1,
                        format!("'{n}' is declared twice"),
                    ));
                }
                if let Some(u) = unit {
                    units.insert(n.clone(), read_unit(&u, number, 1)?);
                }
                if array {
                    let init = value.map(|v| (v.to_string(), number, column_of(text, v.trim())));
                    array_init.push((n.clone(), init));
                    arrays.push(n.clone());
                } else if let Some(v) = value {
                    let v = v.trim().parse::<f64>().map_err(|_| {
                        definition_error(number, 1, format!("'{}' is not a number", v.trim()))
                    })?;
                    let p = format!("{n}0");
                    if declared.iter().any(|d| d.0 == p) {
                        return Err(definition_error(
                            number,
                            1,
                            format!("{p} is declared twice (here as the initial value of {n})"),
                        ));
                    }
                    declared.push((p, v, number));
                }
                state_decls.push((n, number));
                continue;
            }
            if let Some(rest) = keyword(trimmed, "param") {
                let rest = rest.as_str();
                // `param k [N/m] = 4`, `param k [N/m]`, `param k = 4`.
                let (head, value) = match rest.split_once('=') {
                    Some((h, v)) => {
                        let v = v.trim().parse::<f64>().map_err(|_| {
                            definition_error(number, 1, format!("'{}' is not a number", v.trim()))
                        })?;
                        (h, v)
                    }
                    None => (rest, 1.0),
                };
                let (n, unit) = name_and_unit(head);
                if !is_name(&n) || n == "t" || is_reserved(&n) || axes.contains(&n.as_str()) {
                    return Err(definition_error(
                        number,
                        1,
                        format!("'{n}' cannot be a parameter name"),
                    ));
                }
                if let Some(d) = declared.iter().find(|d| d.0 == n) {
                    let state = n
                        .strip_suffix('0')
                        .filter(|x| state_decls.iter().any(|s| s.0 == *x));
                    let why = match state {
                        Some(x) => format!(
                            "{n} is declared twice (line {} gives it as the initial value of {x})",
                            d.2
                        ),
                        None => format!("'{n}' is declared twice"),
                    };
                    return Err(definition_error(number, 1, why));
                }
                if let Some(u) = unit {
                    units.insert(n.clone(), read_unit(&u, number, 1)?);
                }
                declared.push((n, value, number));
                continue;
            }
            if let Some(names) = keyword(trimmed, "input") {
                for item in split_top(&names)
                    .into_iter()
                    .filter(|n| !n.trim().is_empty())
                {
                    let (item, array) = split_dims(item, &axes, number)?;
                    let (item, unit) = name_and_unit(&item);
                    // Names may also be separated by spaces (no units then).
                    let parts: Vec<&str> = item.split(' ').filter(|n| !n.is_empty()).collect();
                    if (unit.is_some() || array) && parts.len() != 1 {
                        return Err(definition_error(
                            number,
                            1,
                            "separate inputs with commas when giving units or grids: 'input F [N], q[x] [W/m^3]'",
                        ));
                    }
                    if let Some(u) = unit {
                        units.insert(item.clone(), read_unit(&u, number, 1)?);
                    }
                    for n in parts {
                        if !is_name(n) || n == "t" || is_reserved(n) || axes.contains(&n) {
                            return Err(definition_error(
                                number,
                                1,
                                format!("'{n}' cannot be an input name"),
                            ));
                        }
                        if inputs.iter().any(|i| i == n) {
                            return Err(definition_error(
                                number,
                                1,
                                format!("'{n}' is an input twice"),
                            ));
                        }
                        inputs.push(n.to_string());
                        if array {
                            arrays.push(n.to_string());
                        }
                    }
                }
                continue;
            }
            let Some((lhs, rhs)) = text.split_once('=') else {
                return Err(definition_error(
                    number,
                    1,
                    "expected an equation such as 'dx/dt = -k*x' or 'E = x^2'",
                ));
            };
            let lhs_kind = read_lhs(lhs).ok_or_else(|| {
                definition_error(
                    number,
                    1,
                    format!("'{}' is not 'dx/dt', 'x'', 'x[n+1]' or a name", lhs.trim()),
                )
            })?;
            let offset = lhs.chars().count() + 1;
            let (rhs, unit) = split_unit(rhs);
            if let Some((u, col)) = unit {
                // The unit of the left side as written (ADR 0049).
                let unit = read_unit(&u, number, offset + col)?;
                match &lhs_kind {
                    Lhs::State(n) => rates.push((n.clone(), unit, number, offset + col)),
                    Lhs::Map(n) | Lhs::Defined(n) => {
                        if let Some(d) = units.get(n).filter(|d| d.dim != unit.dim) {
                            return Err(definition_error(
                                number,
                                offset + col,
                                format!("{n} is declared in {d}, not {unit}"),
                            ));
                        }
                        units.entry(n.clone()).or_insert(unit);
                    }
                }
            }
            sides.push((number, offset, lhs_kind, rhs));
        }
        if sides.is_empty() {
            return Err(SimError::InvalidDefinition(
                "no equations: write lines like 'dx/dt = -k*x', 'x[n+1] = r*x*(1 - x)' or 'y = a*x^2'"
                    .into(),
            ));
        }
        let continuous = sides.iter().find(|s| matches!(s.2, Lhs::State(_)));
        let map = sides.iter().find(|s| matches!(s.2, Lhs::Map(_)));
        let kind = match (continuous, map) {
            (Some(_), Some(m)) => {
                return Err(definition_error(
                    m.0,
                    1,
                    "a block is either continuous (dx/dt) or a map (x[n+1]), not both",
                ))
            }
            (Some(_), None) => Kind::Continuous,
            (None, Some(_)) => Kind::Map,
            (None, None) => Kind::Formulas,
        };
        if let (Kind::Map, Some(a)) = (kind, grid.first()) {
            return Err(definition_error(
                a.line,
                1,
                "maps cannot use a grid yet: arrays are for continuous equations and formulas",
            ));
        }
        let mut parsed = Vec::new();
        for (number, offset, lhs, rhs) in sides {
            // In maps, x[n] is the current value: blank out "[n]" so columns stay put.
            let rhs = if kind == Kind::Map {
                rhs.replace("[n]", "   ")
            } else {
                rhs
            };
            let ast =
                parse(&rhs).map_err(|e| definition_error(number, offset + e.column, e.message))?;
            parsed.push(Line {
                number,
                offset,
                lhs,
                ast,
            });
        }

        // Names of states and defined quantities, checked for clashes.
        let mut states: Vec<String> = Vec::new();
        let mut defined: Vec<String> = Vec::new();
        for line in &parsed {
            let n = match &line.lhs {
                Lhs::State(n) | Lhs::Map(n) | Lhs::Defined(n) => n,
            };
            if n == "t" || is_reserved(n) {
                return Err(definition_error(
                    line.number,
                    1,
                    format!("'{n}' is a reserved name"),
                ));
            }
            if states.contains(n)
                || defined.contains(n)
                || inputs.contains(n)
                || axes.contains(&n.as_str())
            {
                return Err(definition_error(
                    line.number,
                    1,
                    format!("'{n}' is defined twice"),
                ));
            }
            match &line.lhs {
                Lhs::State(_) | Lhs::Map(_) => states.push(n.clone()),
                Lhs::Defined(_) => defined.push(n.clone()),
            }
        }
        // Every declared state has dynamics, and a rate's unit gives its
        // state's (ADR 0049).
        for (n, number) in &state_decls {
            if !states.contains(n) {
                let how = if defined.contains(n) {
                    format!("'{n}' is defined by '{n} = …', so it is not a state")
                } else {
                    format!("{n} is declared as a state but no line gives d{n}/dt or {n}[n+1]")
                };
                return Err(definition_error(*number, 1, how));
            }
        }
        for (n, rate, number, column) in &rates {
            let from_rate = *rate * Unit::SECOND;
            match units.get(n) {
                Some(x) if x.dim != from_rate.dim => {
                    return Err(definition_error(
                        *number,
                        *column,
                        format!(
                            "d{n}/dt is in {rate}, so {n} would be in {}, but {n} is in {x}",
                            Unit::new(from_rate.dim.0, 1.0)
                        ),
                    ));
                }
                Some(_) => {}
                None => {
                    // A unit without a name (km/h times s) becomes SI.
                    let named = !from_rate
                        .to_string()
                        .starts_with(|c: char| c.is_ascii_digit());
                    let x = if named {
                        from_rate
                    } else {
                        Unit::new(from_rate.dim.0, 1.0)
                    };
                    units.insert(n.clone(), x);
                }
            }
        }
        // Which quantities are arrays (ADR 0060), in order: a defined
        // quantity is one when its right side is. A scalar state cannot
        // change by an array.
        for line in &parsed {
            let array = is_array(&line.ast, &arrays);
            match &line.lhs {
                Lhs::Defined(n) if array => arrays.push(n.clone()),
                Lhs::State(n) if array && !arrays.contains(n) => {
                    return Err(definition_error(
                        line.number,
                        line.offset + 1,
                        format!("d{n}/dt has a value per cell, so {n} must be an array: declare 'state {n}[{}]'", axes.join(", ")),
                    ))
                }
                _ => {}
            }
            check_array_ops(&line.ast, &grid, &arrays)
                .map_err(|(c, m)| definition_error(line.number, line.offset + c, m))?;
        }
        // Array states' initial values, edge values and the grid's ends.
        let mut inits: Vec<(String, Ast, usize, usize)> = Vec::new();
        for (n, init) in &array_init {
            if let Some((src, number, col)) = init {
                let ast = parse(src)
                    .map_err(|e| definition_error(*number, col + e.column - 1, e.message))?;
                inits.push((n.clone(), ast, *number, *col));
            }
        }
        for e in &edges {
            if !arrays.contains(&e.array) || axes.contains(&e.array.as_str()) {
                return Err(definition_error(
                    e.line,
                    1,
                    format!(
                        "'{}' is not an array on the grid, so it has no edges",
                        e.array
                    ),
                ));
            }
            if edges
                .iter()
                .filter(|f| f.array == e.array && f.axis == e.axis && f.high == e.high)
                .count()
                > 1
            {
                return Err(definition_error(
                    e.line,
                    e.column,
                    format!(
                        "{}'s {}{} end has two conditions",
                        e.array,
                        axes[e.axis],
                        if e.high { "+" } else { "-" }
                    ),
                ));
            }
        }

        // Declared parameters, then every other name in order of first use.
        let known = |n: &str| {
            n == "t"
                || states.iter().any(|s| s == n)
                || defined.iter().any(|s| s == n)
                || inputs.iter().any(|s| s == n)
                || axes.contains(&n)
        };
        let mut params: Vec<String> = Vec::new();
        for (n, _, number) in &declared {
            if known(n) {
                return Err(definition_error(
                    *number,
                    1,
                    format!("'{n}' is defined twice"),
                ));
            }
            params.push(n.clone());
        }
        let mut used: Vec<&Ast> = parsed.iter().map(|l| &l.ast).collect();
        used.extend(grid.iter().flat_map(|a| [&a.lo, &a.hi]));
        used.extend(inits.iter().map(|i| &i.1));
        used.extend(edges.iter().filter_map(|e| match &e.kind {
            EdgeKind::Value(a) | EdgeKind::Gradient(a) => Some(a),
            EdgeKind::Periodic => None,
        }));
        for ast in used {
            for (n, _) in ast.names() {
                if !known(&n) && !params.contains(&n) {
                    params.push(n);
                }
            }
        }
        // What each extra expression may use.
        let only = |ast: &Ast,
                    line: usize,
                    col: usize,
                    allowed: &dyn Fn(&str) -> bool,
                    what: &str|
         -> Result<()> {
            match ast.names().into_iter().find(|(n, _)| !allowed(n)) {
                Some((n, c)) => Err(definition_error(
                    line,
                    col + c - 1,
                    format!("{what}, not '{n}'"),
                )),
                None => Ok(()),
            }
        };
        let is_param = |n: &str| params.iter().any(|p| p == n);
        for a in &grid {
            for end in [&a.lo, &a.hi] {
                only(
                    end,
                    a.line,
                    1,
                    &is_param,
                    "a grid's ends are written with parameters",
                )?;
            }
        }
        for (_, ast, line, col) in &inits {
            check_array_ops(ast, &grid, &arrays)
                .map_err(|(c, m)| definition_error(*line, col + c - 1, m))?;
            only(
                ast,
                *line,
                *col,
                &|n| is_param(n) || axes.contains(&n),
                "an initial value is written with parameters and the axes",
            )?;
        }
        for e in &edges {
            if let EdgeKind::Value(ast) | EdgeKind::Gradient(ast) = &e.kind {
                only(
                    ast,
                    e.line,
                    e.column,
                    &|n| {
                        n == "t"
                            || is_param(n)
                            || inputs.iter().any(|i| i == n) && !arrays.contains(&n.to_string())
                    },
                    "an edge value is written with parameters, scalar inputs and t",
                )?;
            }
        }
        if kind == Kind::Map {
            if let Some(line) = parsed
                .iter()
                .find(|l| l.ast.names().iter().any(|(n, _)| n == "n"))
            {
                return Err(definition_error(
                    line.number,
                    1,
                    "'n' is the step number in maps and cannot be used in equations yet",
                ));
            }
        }
        // Scalar states start from a parameter `x0`; arrays from their
        // initial expression (0 if none).
        let mut initial: Vec<Option<usize>> = Vec::new();
        for s in &states {
            if arrays.contains(s) {
                initial.push(None);
                continue;
            }
            let p = format!("{s}0");
            let i = match params.iter().position(|q| *q == p) {
                Some(i) => i,
                None => {
                    params.push(p);
                    params.len() - 1
                }
            };
            initial.push(Some(i));
        }

        // Units (ADR 0045). A block that declares none is dimensionless
        // throughout, `t` included, as typed equations were before units.
        let checked = !units.is_empty();
        for (s, i) in states.iter().zip(&initial) {
            let Some(i) = *i else { continue };
            let x = units.get(s).copied().unwrap_or(Unit::DIMENSIONLESS);
            let p = &params[i];
            match units.get(p) {
                Some(u) if u.dim != x.dim => {
                    let number = declared.iter().find(|d| &d.0 == p).map_or(1, |d| d.2);
                    return Err(definition_error(
                        number,
                        1,
                        format!("{p} is the initial value of {s}, so it is in {x}, not {u}"),
                    ));
                }
                Some(_) => {}
                None => {
                    units.insert(p.clone(), x);
                }
            }
        }
        let time = if checked {
            Unit::SECOND
        } else {
            Unit::DIMENSIONLESS
        };
        let axis_dims: Vec<Dim> = grid
            .iter()
            .map(|a| units.get(&a.name).map_or(NO_DIM, |u| u.dim.0))
            .collect();
        let axis_dim_of = |name: &str| -> Dim {
            axes.iter()
                .position(|a| *a == name)
                .map_or(NO_DIM, |i| axis_dims[i])
        };
        // Array operations' units: a stencil divides by the axis unit.
        let apply_dim = |name: &str,
                         args: &[(&Ast, Dim)],
                         col: usize|
         -> std::result::Result<Dim, (usize, String)> {
            let d = args.first().map_or(NO_DIM, |a| a.1);
            Ok(match name {
                "lap" => {
                    if axis_dims.iter().any(|a| *a != axis_dims[0]) {
                        return Err((
                            col,
                            format!(
                                "lap adds second derivatives along {}, so the axes need the same unit",
                                axes.join(" and ")
                            ),
                        ));
                    }
                    std::array::from_fn(|i| d[i] - 2 * axis_dims[0][i])
                }
                "spacing" => match args[0].0 {
                    Ast::Name(n, _) => axis_dim_of(n),
                    _ => NO_DIM,
                },
                n if n.starts_with("dd") => {
                    let a = axis_dim_of(&n[2..]);
                    std::array::from_fn(|i| d[i] - a[i])
                }
                "at" => {
                    if args[1..].iter().any(|a| a.1 != NO_DIM) {
                        return Err((col, "a cell number has no unit".into()));
                    }
                    d
                }
                _ => d,
            })
        };
        // Defined quantities without a unit take their right side's.
        for line in &parsed {
            let Lhs::Defined(n) = &line.lhs else {
                continue;
            };
            if !checked || units.contains_key(n) {
                continue;
            }
            let of = |m: &str| {
                if m == "t" {
                    time.dim.0
                } else {
                    units.get(m).map_or(NO_DIM, |u| u.dim.0)
                }
            };
            if let Ok(d) = dimension_with(&line.ast, &of, &apply_dim) {
                units.insert(n.clone(), Unit::new(d, 1.0));
            }
        }
        let unit_of = |n: &str| -> Unit {
            if n == "t" {
                time
            } else {
                units.get(n).copied().unwrap_or(Unit::DIMENSIONLESS)
            }
        };
        // Every equation balances.
        if checked {
            let of = |m: &str| unit_of(m).dim.0;
            let balance =
                |ast: &Ast, want: Dim, what: &str, line: usize, col: usize| -> Result<()> {
                    let got = dimension_with(ast, &of, &apply_dim)
                        .map_err(|(c, m)| definition_error(line, col + c - 1, m))?;
                    if got != want {
                        return Err(definition_error(
                            line,
                            col,
                            format!(
                                "{what} is in {} but the right side is in {}",
                                dim_text(want),
                                dim_text(got)
                            ),
                        ));
                    }
                    Ok(())
                };
            for line in &parsed {
                let (what, want) = match &line.lhs {
                    Lhs::State(n) => {
                        let x = unit_of(n).dim.0;
                        (
                            format!("d{n}/dt"),
                            std::array::from_fn(|i| x[i] - Unit::SECOND.dim.0[i]),
                        )
                    }
                    Lhs::Map(n) => (format!("{n}[n+1]"), unit_of(n).dim.0),
                    Lhs::Defined(n) => (n.clone(), unit_of(n).dim.0),
                };
                balance(&line.ast, want, &what, line.number, line.offset + 1)?;
            }
            for (a, dim) in grid.iter().zip(&axis_dims) {
                for end in [&a.lo, &a.hi].into_iter().filter(|e| number(e).is_none()) {
                    balance(end, *dim, &format!("the axis {}", a.name), a.line, 1)?;
                }
            }
            for (n, ast, line, col) in inits.iter().filter(|i| number(&i.1).is_none()) {
                balance(
                    ast,
                    unit_of(n).dim.0,
                    &format!("{n}'s initial value"),
                    *line,
                    *col,
                )?;
            }
            for e in &edges {
                // A plain number is in the array's unit (per axis unit for
                // a gradient).
                if let EdgeKind::Value(a) | EdgeKind::Gradient(a) = &e.kind {
                    if number(a).is_some() {
                        continue;
                    }
                }
                let d = unit_of(&e.array).dim.0;
                match &e.kind {
                    EdgeKind::Value(ast) => balance(
                        ast,
                        d,
                        &format!("{}'s edge value", e.array),
                        e.line,
                        e.column,
                    )?,
                    EdgeKind::Gradient(ast) => balance(
                        ast,
                        std::array::from_fn(|i| d[i] - axis_dims[e.axis][i]),
                        &format!("{}'s edge gradient", e.array),
                        e.line,
                        e.column,
                    )?,
                    EdgeKind::Periodic => {}
                }
            }
        }

        // Variables, laid out in the registers in this order: t, states,
        // inputs, parameters, defined quantities, the axis (these two read
        // as outputs), the grid's ends and spacing, edge values, rates.
        let cells: usize = grid.iter().map(|a| a.cells).product();
        let len_of = |n: &str| {
            if arrays.iter().any(|a| a == n) {
                cells
            } else {
                1
            }
        };
        let mut spans: Vec<Span> = Vec::new();
        let mut scales: Vec<f64> = Vec::new();
        let mut size = 0;
        let mut add = |len: usize, scale: f64| -> usize {
            spans.push(Span { at: size, len });
            scales.push(scale);
            size += len;
            spans.len() - 1
        };
        add(1, time.scale);
        let state_vars: Vec<usize> = states
            .iter()
            .map(|n| add(len_of(n), unit_of(n).scale))
            .collect();
        let input_vars: Vec<usize> = inputs
            .iter()
            .map(|n| add(len_of(n), unit_of(n).scale))
            .collect();
        let param_vars: Vec<usize> = params.iter().map(|n| add(1, unit_of(n).scale)).collect();
        let defined_vars: Vec<usize> = defined
            .iter()
            .map(|n| add(len_of(n), unit_of(n).scale))
            .collect();
        let axis_scales: Vec<f64> = grid.iter().map(|a| unit_of(&a.name).scale).collect();
        // Each axis's position of every cell (read as outputs, after the
        // defined quantities), then its ends and spacing, all in its unit.
        let axis_vars: Vec<usize> = axis_scales.iter().map(|&s| add(cells, s)).collect();
        let grid_vars: Vec<(usize, usize, usize, usize)> = axis_vars
            .iter()
            .zip(&axis_scales)
            .map(|(&x, &s)| (x, add(1, s), add(1, s), add(1, s)))
            .collect();
        let edge_vars: Vec<Option<usize>> = edges
            .iter()
            .map(|e| match e.kind {
                EdgeKind::Periodic => None,
                EdgeKind::Value(_) => Some(add(1, unit_of(&e.array).scale)),
                EdgeKind::Gradient(_) => {
                    Some(add(1, unit_of(&e.array).scale / axis_scales[e.axis]))
                }
            })
            .collect();
        let rate_vars: Vec<usize> = states
            .iter()
            .map(|n| add(len_of(n), unit_of(n).scale))
            .collect();
        let layout = Layout {
            states: state_vars.iter().map(|&v| spans[v].len).sum(),
            inputs: input_vars.iter().map(|&v| spans[v].len).sum(),
            params: params.len(),
            outputs_at: defined_vars
                .first()
                .or(axis_vars.first())
                .map_or(0, |&v| spans[v].at),
            outputs_len: defined_vars
                .iter()
                .chain(&axis_vars)
                .map(|&v| spans[v].len)
                .sum::<usize>(),
            rates_at: rate_vars.first().map_or(size, |&v| spans[v].at),
        };
        let slot_of = |n: &str, defined_so_far: usize| -> Option<usize> {
            if n == "t" {
                return Some(0);
            }
            if let Some(i) = states.iter().position(|s| s == n) {
                return Some(state_vars[i]);
            }
            if let Some(i) = inputs.iter().position(|s| s == n) {
                return Some(input_vars[i]);
            }
            if let Some(i) = params.iter().position(|p| p == n) {
                return Some(param_vars[i]);
            }
            if let Some(i) = axes.iter().position(|a| *a == n) {
                return Some(axis_vars[i]);
            }
            defined[..defined_so_far]
                .iter()
                .position(|d| d == n)
                .map(|i| defined_vars[i])
        };
        // An array's edges along an axis, insulated where none is given.
        let edges_of = |array: &str, axis: usize| -> [Edge; 2] {
            let mut out = [Edge::Gradient(None); 2];
            for (e, v) in edges.iter().zip(&edge_vars) {
                if e.array == array && e.axis == axis {
                    out[e.high as usize] = match (&e.kind, v) {
                        (EdgeKind::Periodic, _) => Edge::Periodic,
                        (EdgeKind::Value(_), Some(v)) => Edge::Value(*v),
                        (_, v) => Edge::Gradient(*v),
                    };
                }
            }
            out
        };
        let spacing_of = |axis: usize| grid_vars[axis].3;
        let apply = |name: &str,
                     args: &[Ast],
                     resolved: Vec<Expr>,
                     _col: usize|
         -> std::result::Result<Expr, simsandbox_expr::Error> {
            let Some(Ast::Name(first_name, _)) = args.first() else {
                unreachable!("checked: one argument at least")
            };
            // A derivative of an array along one axis.
            let along = |axis: usize, second: bool| {
                let a = slot_of(first_name, defined.len()).expect("checked: an array");
                let (line, edges) = (lines_of[axis], edges_of(first_name, axis));
                let op = if second {
                    ArrayOp::Second { line, edges }
                } else {
                    ArrayOp::First { line, edges }
                };
                Expr::Array(op, vec![Expr::Var(a), Expr::Var(spacing_of(axis))])
            };
            let axis_named = |n: &str| axes.iter().position(|a| *a == n).expect("checked: an axis");
            if name == "lap" {
                // The sum of the second derivatives along each axis.
                return Ok((1..grid.len()).fold(along(0, true), |sum, i| {
                    Expr::Bin(Op::Add, Box::new(sum), Box::new(along(i, true)))
                }));
            }
            if name == "spacing" {
                return Ok(Expr::Var(spacing_of(axis_named(first_name))));
            }
            if let Some(ax) = name.strip_prefix("dd") {
                return Ok(along(axis_named(ax), false));
            }
            let first = resolved
                .into_iter()
                .next()
                .expect("checked: one argument at least");
            Ok(match name {
                "sum" => Expr::Array(ArrayOp::Sum, vec![first]),
                "mean" => Expr::Array(ArrayOp::Mean, vec![first]),
                "minof" => Expr::Array(ArrayOp::MinOf, vec![first]),
                "maxof" => Expr::Array(ArrayOp::MaxOf, vec![first]),
                _ => {
                    // at(T, i) or at(T, i, j): cell i + j·nx.
                    let place = args[1..]
                        .iter()
                        .zip(&lines_of)
                        .map(|(a, line)| match a {
                            Ast::Num(i) => *i as usize * line.stride,
                            _ => unreachable!("checked: a whole number"),
                        })
                        .sum();
                    Expr::Array(ArrayOp::At(place), vec![first])
                }
            })
        };
        let resolve = |ast: &Ast, visible: usize, line: usize, offset: usize| -> Result<Expr> {
            ast.resolve_with(&|n| slot_of(n, visible), &apply)
                .map_err(|e| {
                    let hint = if defined
                        .iter()
                        .any(|d| e.message == format!("unknown name '{d}'"))
                    {
                        " (it is defined further down; define it above this line)"
                    } else {
                        ""
                    };
                    definition_error(line, offset + e.column, format!("{}{hint}", e.message))
                })
        };
        // In the unit of the variable it is stored in.
        let stored = |e: Expr, var: usize| from_si(in_si(e, &scales), scales[var]);
        let mut rhs: Vec<Option<Expr>> = vec![None; states.len()];
        let mut defined_exprs = Vec::new();
        for line in &parsed {
            match &line.lhs {
                Lhs::State(n) | Lhs::Map(n) => {
                    let i = states.iter().position(|s| s == n).expect("state listed");
                    let e = resolve(&line.ast, defined.len(), line.number, line.offset)?;
                    rhs[i] = Some(stored(e, state_vars[i]));
                }
                Lhs::Defined(_) => {
                    let k = defined_exprs.len();
                    let e = resolve(&line.ast, k, line.number, line.offset)?;
                    defined_exprs.push(stored(e, defined_vars[k]));
                }
            }
        }
        // The grid: its ends, spacing and cell centres.
        let mut grid_steps: Vec<(usize, Expr)> = Vec::new();
        for ((a, &(x, lo, hi, h)), &line) in grid.iter().zip(&grid_vars).zip(&lines_of) {
            // A plain number is in the axis unit: `0 .. 4` in cm.
            for (end, var) in [(&a.lo, lo), (&a.hi, hi)] {
                let e = match number(end) {
                    Some(v) => Expr::Num(v),
                    None => stored(resolve(end, 0, a.line, 0)?, var),
                };
                grid_steps.push((var, e));
            }
            let width = Expr::Bin(Op::Sub, Box::new(Expr::Var(hi)), Box::new(Expr::Var(lo)));
            grid_steps.push((
                h,
                Expr::Bin(
                    Op::Div,
                    Box::new(width),
                    Box::new(Expr::Num(a.cells as f64)),
                ),
            ));
            grid_steps.push((
                x,
                Expr::Array(
                    ArrayOp::Axis { line, cells },
                    vec![Expr::Var(lo), Expr::Var(h)],
                ),
            ));
        }
        let mut edge_steps: Vec<(usize, Expr)> = Vec::new();
        for (e, v) in edges.iter().zip(&edge_vars) {
            if let (EdgeKind::Value(ast) | EdgeKind::Gradient(ast), Some(v)) = (&e.kind, v) {
                let x = match number(ast) {
                    Some(n) => Expr::Num(n),
                    None => stored(resolve(ast, 0, e.line, e.column - 1)?, *v),
                };
                edge_steps.push((*v, x));
            }
        }
        let mut init_steps = grid_steps.clone();
        for (n, ast, line, col) in &inits {
            let i = states.iter().position(|s| s == n).expect("an array state");
            // A plain number is in the state's unit, as for `state x [m] = 0`.
            let x = match number(ast) {
                Some(v) => Expr::Num(v),
                None => stored(resolve(ast, 0, *line, col - 1)?, state_vars[i]),
            };
            init_steps.push((state_vars[i], x));
        }

        let mut param_specs: Vec<ParamSpec> = params
            .iter()
            .map(|p| ParamSpec {
                name: p.clone(),
                unit: unit_of(p),
                default: declared.iter().find(|d| &d.0 == p).map_or(1.0, |d| d.1),
                suggested_min: 0.0,
                suggested_max: 10.0,
                description: "Parameter of the equations".into(),
                initial_condition: false,
            })
            .collect();
        for (s, i) in states.iter().zip(&initial) {
            if let Some(i) = *i {
                let p = &mut param_specs[i];
                p.initial_condition = true;
                p.suggested_min = -10.0;
                p.description = format!("Initial value of {s}");
            }
        }
        // Parameters in array states' initial values mark them too:
        // changing them changes where the run starts.
        for (n, ast, ..) in &inits {
            for (p, _) in ast.names() {
                if let Some(spec) = param_specs.iter_mut().find(|q| q.name == p) {
                    if !spec.initial_condition && !states.iter().any(|s| s == &p) {
                        let used_elsewhere = parsed
                            .iter()
                            .any(|l| l.ast.names().iter().any(|(m, _)| *m == p));
                        if !used_elsewhere {
                            spec.initial_condition = true;
                            spec.description = format!("Sets the initial value of {n}");
                        }
                    }
                }
            }
        }
        // Which inputs each defined quantity uses, directly or through
        // quantities above it.
        let mut uses: Vec<Vec<String>> = Vec::new();
        for line in parsed.iter().filter(|l| matches!(l.lhs, Lhs::Defined(_))) {
            let mut u: Vec<String> = Vec::new();
            for (n, _) in line.ast.names() {
                let via: Vec<String> = match defined[..uses.len()].iter().position(|d| *d == n) {
                    Some(i) => uses[i].clone(),
                    None if inputs.contains(&n) => vec![n],
                    None => Vec::new(),
                };
                for i in via {
                    if !u.contains(&i) {
                        u.push(i);
                    }
                }
            }
            // In input order, so the declaration reads naturally.
            u.sort_by_key(|i| inputs.iter().position(|x| x == i));
            uses.push(u);
        }
        let shape = |n: &str| {
            if !grid.is_empty() && arrays.iter().any(|a| a == n) {
                Shape::Grid { nx, ny: cells / nx }
            } else {
                Shape::Scalar
            }
        };
        let mut ports: Vec<PortSpec> = inputs
            .iter()
            .map(|n| PortSpec::input(n, unit_of(n), shape(n)))
            .chain(
                states
                    .iter()
                    .map(|n| PortSpec::output(n, unit_of(n), shape(n))),
            )
            .chain(defined.iter().zip(&uses).map(|(n, u)| {
                let u: Vec<&str> = u.iter().map(String::as_str).collect();
                PortSpec::output(n, unit_of(n), shape(n)).depends_on(&u)
            }))
            .collect();
        for (i, a) in grid.iter().enumerate() {
            // Each axis is the coordinate of every array output on the grid.
            let mut x = PortSpec::output(&a.name, unit_of(&a.name), shape(&a.name));
            for n in states.iter().chain(&defined).filter(|n| arrays.contains(n)) {
                x = x.coordinate_of(n, i);
            }
            ports.push(x);
        }

        let mut output_steps: Vec<(usize, &Expr)> =
            grid_steps.iter().map(|(v, e)| (*v, e)).collect();
        output_steps.extend(edge_steps.iter().map(|(v, e)| (*v, e)));
        output_steps.extend(
            defined_exprs
                .iter()
                .enumerate()
                .map(|(k, e)| (defined_vars[k], e)),
        );
        let rhs: Vec<Expr> = rhs
            .into_iter()
            .map(|r| r.expect("one equation per state"))
            .collect();
        let mut rate_steps = output_steps.clone();
        rate_steps.extend(rhs.iter().enumerate().map(|(k, e)| (rate_vars[k], e)));
        let rate_steps = needed(rate_steps, &rate_vars);
        let outputs = Program::with_layout(&spans, size, &output_steps);
        let rates = Program::with_layout(&spans, size, &rate_steps);
        let init = (!array_init.is_empty()).then(|| {
            let steps: Vec<(usize, &Expr)> = init_steps.iter().map(|(v, e)| (*v, e)).collect();
            Program::with_layout(&spans, size, &steps)
        });
        let scratch = RefCell::new([
            outputs.registers(),
            rates.registers(),
            init.as_ref().map_or_else(Vec::new, Program::registers),
        ]);
        let state_names = states
            .iter()
            .flat_map(|s| {
                if arrays.contains(s) {
                    (0..cells).map(|k| format!("{s}[{k}]")).collect::<Vec<_>>()
                } else {
                    vec![s.clone()]
                }
            })
            .collect();
        let initial = states
            .iter()
            .zip(initial)
            .map(|(s, i)| (if arrays.contains(s) { cells } else { 1 }, i))
            .collect();
        Ok(Self {
            kind,
            spec: BlockSpec {
                name: name.into(),
                domain: Domain::Equations,
                ports,
                params: param_specs,
                states: state_names,
                tables: Vec::new(),
                time: match kind {
                    Kind::Continuous => TimeSemantics::Continuous,
                    Kind::Map => TimeSemantics::DiscreteEvent,
                    Kind::Formulas => TimeSemantics::Stateless,
                },
            },
            layout,
            outputs,
            rates,
            init,
            initial,
            scratch,
        })
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// Put time `t`, states `x`, inputs `u` and parameters `p` in the
    /// registers `v`. Missing inputs read as 0.
    fn fill(&self, t: f64, x: &[f64], u: &[f64], p: &[f64], v: &mut [f64]) {
        let Layout {
            states: ns,
            inputs: ni,
            params: np,
            ..
        } = self.layout;
        v[0] = t;
        v[1..=ns].copy_from_slice(&x[..ns]);
        for (k, slot) in v[1 + ns..1 + ns + ni].iter_mut().enumerate() {
            *slot = u.get(k).copied().unwrap_or(0.0);
        }
        v[1 + ns + ni..1 + ns + ni + np].copy_from_slice(&p[..np]);
    }
}

impl Equations for EquationSet {
    fn rhs(&self, t: f64, x: &[f64], u: &[f64], p: &[f64], dx: &mut [f64]) {
        let v = &mut self.scratch.borrow_mut()[1];
        self.fill(t, x, u, p, v);
        self.rates.run(v);
        let first = self.layout.rates_at;
        let n = dx.len().min(self.layout.states);
        dx[..n].copy_from_slice(&v[first..first + n]);
    }

    fn outputs(&self, t: f64, x: &[f64], u: &[f64], p: &[f64], y: &mut [f64]) {
        let v = &mut self.scratch.borrow_mut()[0];
        self.fill(t, x, u, p, v);
        self.outputs.run(v);
        let ns = self.layout.states;
        y[..ns].copy_from_slice(&x[..ns]);
        let (at, len) = (self.layout.outputs_at, self.layout.outputs_len);
        y[ns..ns + len].copy_from_slice(&v[at..at + len]);
    }
}

impl OdeSystem for EquationSet {
    fn spec(&self) -> &BlockSpec {
        &self.spec
    }

    /// Exact: each rate's dependencies on the states, read from the
    /// instructions that compute it (ADR 0060).
    fn sparsity(&self) -> Option<Vec<Vec<usize>>> {
        let ns = self.layout.states;
        let at = self.layout.rates_at;
        Some(self.rates.dependencies(1..1 + ns, at..at + ns))
    }

    fn initial_state(&self, p: &[f64], x0: &mut [f64]) {
        let mut scratch = self.scratch.borrow_mut();
        if let Some(init) = &self.init {
            let v = &mut scratch[2];
            let zero = vec![0.0; self.layout.states];
            self.fill(0.0, &zero, &[], p, v);
            init.run(v);
            x0.copy_from_slice(&v[1..=self.layout.states]);
        }
        let mut at = 0;
        for &(len, param) in &self.initial {
            if let Some(i) = param {
                x0[at] = p[i];
            }
            at += len;
        }
    }
}

/// A map or a set of formulas as a kernel Block (ADR 0019): maps are
/// iterated once per communication step, formulas are evaluated. Neither
/// approximates anything, so there is no method to choose (ADR 0020).
pub struct ExplicitBlock {
    set: EquationSet,
    method: MethodSpec,
    next: Vec<f64>,
}

impl ExplicitBlock {
    /// Wrap a map or formula set; `method` must be `iterate` for maps and
    /// `evaluate` for formulas.
    pub fn new(set: EquationSet, method: &MethodSettings) -> Result<Self> {
        let (expected, label) = match set.kind {
            Kind::Map => (ITERATE, "Iterate the map once per step"),
            Kind::Formulas => (EVALUATE, "Evaluate the formulas"),
            Kind::Continuous => {
                return Err(SimError::InvalidMethod(
                    "continuous equations need an ODE method such as rk4".into(),
                ))
            }
        };
        if method.name != expected || !method.options.is_empty() {
            let what = if set.kind == Kind::Map {
                "maps are iterated"
            } else {
                "formulas are evaluated"
            };
            return Err(SimError::InvalidMethod(format!(
                "{what}: use method '{expected}' with no options"
            )));
        }
        let states = set.spec.states.len();
        Ok(Self {
            set,
            method: MethodSpec {
                name: label.into(),
                // No discretisation error, but not benchmarked case by case.
                labels: MethodLabels {
                    speed: SpeedClass::RealTime,
                    accuracy: AccuracyClass::Quantitative,
                },
            },
            next: vec![0.0; states],
        })
    }
}

impl Block for ExplicitBlock {
    fn spec(&self) -> &BlockSpec {
        &self.set.spec
    }

    fn method(&self) -> &MethodSpec {
        &self.method
    }

    fn init(&mut self, params: &[f64], state: &mut [f64]) -> Result<()> {
        self.set.initial_state(params, state);
        Ok(())
    }

    fn initial_params(&self, state: &[f64], params: &[f64]) -> Option<Vec<f64>> {
        crate::ode::initial_params_by_name(&self.set.spec, state, params, |p, x| {
            self.set.initial_state(p, x)
        })
    }

    fn do_step(&mut self, ctx: StepContext, io: StepIo<'_>) -> Result<()> {
        if self.set.kind == Kind::Map {
            // All states update together from the current values.
            self.set
                .rhs(ctx.t, io.state, io.inputs, io.params, &mut self.next);
            io.state.copy_from_slice(&self.next);
        }
        self.set
            .outputs(ctx.t + ctx.dt, io.state, io.inputs, io.params, io.outputs);
        Ok(())
    }

    fn compute_outputs(&self, t: f64, io: StepIo<'_>) -> Result<()> {
        self.set
            .outputs(t, io.state, io.inputs, io.params, io.outputs);
        Ok(())
    }

    /// Formulas can join a fused group (ADR 0051); maps step on their own.
    fn as_equations(&self) -> Option<&dyn Equations> {
        (self.set.kind == Kind::Formulas).then_some(&self.set as &dyn Equations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn set(lines: &[&str]) -> Result<EquationSet> {
        let lines: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
        EquationSet::parse("m", &lines)
    }

    fn error(lines: &[&str]) -> String {
        match set(lines) {
            Err(SimError::InvalidDefinition(m)) => m,
            Err(e) => panic!("unexpected error {e}"),
            Ok(_) => panic!("expected an error"),
        }
    }

    #[test]
    fn states_parameters_initial_values_and_outputs_are_found() {
        let s = set(&[
            "# a damped oscillator",
            "dx/dt = v",
            "",
            "v' = -k*x - c*v",
            "E = 0.5*v^2 + 0.5*k*x^2",
        ])
        .unwrap();
        let spec = s.spec();
        assert_eq!(spec.states, ["x", "v"]);
        let params: Vec<_> = spec.params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(params, ["k", "c", "x0", "v0"]);
        assert!(spec.params[2].initial_condition && !spec.params[0].initial_condition);
        let outs: Vec<_> = spec.outputs().map(|p| p.name.as_str()).collect();
        assert_eq!(outs, ["x", "v", "E"]);

        // k = 2, c = 0, x = 1, v = 3.
        let p = [2.0, 0.0, 1.0, 3.0];
        let mut x0 = [0.0; 2];
        s.initial_state(&p, &mut x0);
        assert_eq!(x0, [1.0, 3.0]);
        let mut dx = [0.0; 2];
        s.rhs(0.0, &x0, &[], &p, &mut dx);
        assert_eq!(dx, [3.0, -2.0]);
        let mut y = [0.0; 3];
        s.outputs(0.0, &x0, &[], &p, &mut y);
        assert_eq!(y, [1.0, 3.0, 0.5 * 9.0 + 0.5 * 2.0]);
    }

    #[test]
    fn maps_iterate_all_states_together() {
        // A rotation by 90 degrees: needs the old x to compute the new y.
        let s = set(&["x[n+1] = -y[n]", "y[n+1] = x"]).unwrap();
        assert_eq!(s.kind(), Kind::Map);
        let mut b = ExplicitBlock::new(s, &Kind::Map.default_method()).unwrap();
        let (p, mut x, mut y) = ([1.0, 0.0], [0.0; 2], [0.0; 2]);
        b.init(&p, &mut x).unwrap();
        let ctx = StepContext {
            t: 0.0,
            dt: 1.0,
            step: 0,
            seed: 0,
        };
        let mut step = |x: &mut [f64], y: &mut [f64]| {
            b.do_step(
                ctx,
                StepIo {
                    params: &p,
                    inputs: &[],
                    state: x,
                    outputs: y,
                    tables: &mut [],
                    population_inputs: &[],
                    population_outputs: &mut [],
                },
            )
            .unwrap();
        };
        step(&mut x, &mut y);
        assert_eq!(x, [0.0, 1.0]);
        step(&mut x, &mut y);
        assert_eq!(x, [-1.0, 0.0]);
        assert_eq!(y, x);
    }

    #[test]
    fn formulas_have_no_state_and_need_the_evaluate_method() {
        let s = set(&["y = a*x^2 + b"]).unwrap();
        assert_eq!(s.kind(), Kind::Formulas);
        assert!(s.spec().states.is_empty());
        let names: Vec<_> = s.spec().params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["a", "x", "b"]);
        assert!(ExplicitBlock::new(set(&["y = 1"]).unwrap(), &Kind::Map.default_method()).is_err());
        assert!(
            ExplicitBlock::new(set(&["dx/dt = 1"]).unwrap(), &Kind::Map.default_method()).is_err()
        );
        let b = ExplicitBlock::new(s, &Kind::Formulas.default_method()).unwrap();
        let mut y = [0.0];
        b.compute_outputs(
            0.0,
            StepIo {
                params: &[2.0, 3.0, 1.0],
                inputs: &[],
                state: &mut [],
                outputs: &mut y,
                tables: &mut [],
                population_inputs: &[],
                population_outputs: &mut [],
            },
        )
        .unwrap();
        assert_eq!(y, [19.0]);
    }

    #[test]
    fn time_and_defined_quantities_can_be_used() {
        let s = set(&["a = sin(t)", "b = 2*a", "dx/dt = b"]).unwrap();
        let mut dx = [0.0];
        s.rhs(1.0, &[0.0], &[], &[0.0], &mut dx);
        assert_eq!(dx[0], 2.0 * 1.0f64.sin());
    }

    #[test]
    fn mistakes_are_reported_with_line_and_column() {
        assert_eq!(
            error(&["dx/dt = -k*"]),
            "line 1, column 12: expected a number, name or '(', found the end"
        );
        assert!(error(&["dx/dt = 2k"]).starts_with("line 1, column 10: missing operator"));
        assert!(error(&["dx/dt = 1", "dx/dt = 2"]).contains("defined twice"));
        assert!(error(&["dt/dt = 1"]).contains("reserved"));
        assert!(error(&["dsin/dt = 1"]).contains("reserved"));
        assert!(error(&["a = b", "b = 1", "dx/dt = a"]).contains("defined further down"));
        assert!(error(&["x + 1"]).contains("expected an equation"));
        assert!(error(&["2x = 1"]).contains("is not"));
        assert!(error(&["# only a comment"]).contains("no equations"));
        assert!(error(&["dx/dt = 1", "y[n+1] = y"]).contains("not both"));
        assert!(error(&["x[n+1] = x + n"]).contains("step number"));
    }

    fn unit(text: &str) -> Unit {
        Unit::parse(text).unwrap()
    }

    #[test]
    fn units_are_declared_where_names_appear() {
        let s = set(&[
            "input F [N]",
            "param m [kg] = 1000",
            "param c [N*s/m] = 50",
            "state x [m] = 2",
            "dx/dt = v",
            "dv/dt = (F - c*v)/m [m/s^2]",
            "E = m*v^2/2",
        ])
        .unwrap();
        let port = |n: &str| s.spec.ports.iter().find(|p| p.name == n).unwrap().unit;
        assert!(port("F").same_as(&Unit::NEWTON));
        assert!(port("x").same_as(&Unit::METER));
        assert!(port("v").same_as(&unit("m/s")));
        // A defined quantity without a unit takes its right side's: J.
        assert!(port("E").same_as(&unit("J")));
        let param = |n: &str| s.spec.params.iter().find(|p| p.name == n).unwrap();
        assert_eq!(
            (param("m").default, param("m").unit),
            (1000.0, Unit::KILOGRAM)
        );
        assert!(param("x0").unit.same_as(&Unit::METER) && param("x0").initial_condition);
        // The state's declaration gives x0 its default; v0 stays 1.
        assert_eq!((param("x0").default, param("v0").default), (2.0, 1.0));
        assert!(param("v0").unit.same_as(&unit("m/s")));
    }

    #[test]
    fn a_rate_s_unit_is_its_own_and_states_can_be_declared() {
        // [m/s] on dx/dt labels the rate: x is in m.
        let s = set(&["input v [m/s]", "dx/dt = v [m/s]"]).unwrap();
        let x = s.spec.ports.iter().find(|p| p.name == "x").unwrap().unit;
        assert!(x.same_as(&Unit::METER));
        // A rate in mm/s gives mm; one in km/h, whose product with s has
        // no name, gives SI.
        let port = |lines: &[&str], n: &str| {
            let s = set(lines).unwrap();
            s.spec.ports.iter().find(|p| p.name == n).unwrap().unit
        };
        assert!(port(&["param u [mm/s] = 1", "dx/dt = u [mm/s]"], "x").same_as(&unit("mm")));
        assert!(port(&["param u [km/h] = 1", "dx/dt = u [km/h]"], "x").same_as(&Unit::METER));
        // A declared state's unit wins, and the rate must agree with it.
        assert!(
            port(&["state x [km]", "param u [m/s] = 1", "dx/dt = u"], "x").same_as(&unit("km"))
        );
        let e = error(&["state x [m]", "param u [m/s] = 1", "dx/dt = u [m]"]);
        assert!(
            e.contains("dx/dt is in m, so x would be in m·s, but x is in m"),
            "{e}"
        );
        // The old meaning (the state's unit on its rate) no longer balances.
        let e = error(&["input v [m/s]", "dx/dt = v [m]"]);
        assert!(
            e.contains("dx/dt is in m but the right side is in m/s"),
            "{e}"
        );
        // Declarations need dynamics, and are not formulas.
        let e = error(&["state y [m]", "dx/dt = 1"]);
        assert!(e.contains("no line gives dy/dt"), "{e}");
        let e = error(&["state y [m]", "y = 2 [m]"]);
        assert!(e.contains("not a state"), "{e}");
        let e = error(&["state x = 1", "param x0 = 2", "dx/dt = -x"]);
        assert!(e.contains("x0 is declared twice"), "{e}");
        // Maps declare states too.
        let s = set(&["state x [m] = 0.5", "param r = 2", "x[n+1] = r*x[n]"]).unwrap();
        let x0 = s.spec.params.iter().find(|p| p.name == "x0").unwrap();
        assert_eq!(x0.default, 0.5);
        // 'state', 'param' and 'input' are still names on a left side.
        set(&["state = 2", "param = 3", "input = state*param"]).unwrap();
    }

    #[test]
    fn equations_must_balance() {
        let e = error(&["state x [m]", "dx/dt = v", "dv/dt = -x [m/s^2]"]);
        assert!(
            e.contains("line 3") && e.contains("dv/dt is in m/s^2 but the right side is in m"),
            "{e}"
        );
        let e = error(&["input F [N]", "dx/dt = F - x [m/s]"]);
        assert!(e.contains("cannot subtract"), "{e}");
        let e = error(&["param L [m] = 1", "y = sin(L)"]);
        assert!(e.contains("dimensionless"), "{e}");
        let e = error(&["param L [m] = 1", "y = sqrt(L)"]);
        assert!(e.contains("sqrt"), "{e}");
        assert!(error(&["dx/dt = 1 [furlong]"]).contains("unknown symbol 'furlong'"));
        // Names without a unit are dimensionless, which can be what is meant.
        set(&["param L [m] = 2", "param k = 3", "y = k*L [m]"]).unwrap();
        set(&[
            "param A [m^2] = 4",
            "side = sqrt(A) [m]",
            "theta = atan2(side, side)",
        ])
        .unwrap();
    }

    #[test]
    fn values_are_computed_in_si_across_scales() {
        let s = set(&[
            "param d [km] = 2",
            "param t_go [min] = 1",
            "y = d [m]",
            "speed = d/t_go [m/s]",
        ])
        .unwrap();
        let mut y = vec![0.0; 2];
        s.outputs(0.0, &[], &[], &[2.0, 1.0], &mut y);
        assert_eq!(y[0], 2000.0);
        assert!((y[1] - 2000.0 / 60.0).abs() < 1e-12);
        // x in km from a speed in m/s: 1 m/s is 1e-3 km per second.
        let s = set(&["input v [m/s]", "state x [km]", "dx/dt = v"]).unwrap();
        let mut dx = vec![0.0];
        s.rhs(0.0, &[0.0], &[1.0], &[0.0], &mut dx);
        assert!((dx[0] - 1e-3).abs() < 1e-15, "km per second: {}", dx[0]);
    }

    #[test]
    fn maps_and_models_without_units_are_as_before() {
        // `[n]` is the current value, not a unit.
        set(&["x[n+1] = r*x[n]"]).unwrap();
        set(&["x[n+1] = r*x[n] [m]", "param r = 2"]).unwrap();
        // No units anywhere: dimensionless, time included, unchecked.
        let s = set(&["dx/dt = -k*x + t"]).unwrap();
        assert!(s.spec.ports.iter().all(|p| p.unit == Unit::DIMENSIONLESS));
        assert!(error(&["input u v [m]", "y = u"]).contains("commas"));
    }

    /// The block's rates, outputs and initial state at `p`.
    fn eval(s: &EquationSet, p: &[f64], u: &[f64]) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
        let n = s.spec.states.len();
        let mut x = vec![0.0; n];
        s.initial_state(p, &mut x);
        let mut dx = vec![0.0; n];
        s.rhs(0.0, &x, u, p, &mut dx);
        let outs: usize = s.spec.outputs().map(|o| o.shape.len()).sum();
        let mut y = vec![0.0; outs];
        s.outputs(0.0, &x, u, p, &mut y);
        (x, dx, y)
    }

    const HEAT: &[&str] = &[
        "grid x [m] = 0 .. length, 4",
        "param length [m] = 2",
        "param alpha [m^2/s] = 0.5",
        "param peak [K/m] = 1",
        "param left [K] = 10",
        "state T[x] [K] = peak*x",
        "edge T: x- = left",
        "dT/dt = alpha*lap(T)",
        "total = sum(T) [K]",
        "slope = ddx(T)",
    ];

    #[test]
    fn arrays_on_a_grid_have_a_value_per_cell() {
        let s = set(HEAT).unwrap();
        let spec = s.spec();
        assert_eq!(spec.states, ["T[0]", "T[1]", "T[2]", "T[3]"]);
        let outs: Vec<(&str, Shape)> = spec.outputs().map(|p| (p.name.as_str(), p.shape)).collect();
        let grid = Shape::Grid { nx: 4, ny: 1 };
        assert_eq!(
            outs,
            [
                ("T", grid),
                ("total", Shape::Scalar),
                ("slope", grid),
                ("x", grid)
            ]
        );
        let x = spec.ports.iter().find(|p| p.name == "x").unwrap();
        let of: Vec<&str> = x.coordinate.iter().map(|c| c.port.as_str()).collect();
        assert_eq!(of, ["T", "slope"]);
        assert_eq!(x.unit, Unit::METER);
        let slope = spec.ports.iter().find(|p| p.name == "slope").unwrap();
        assert_eq!(slope.unit.to_string(), "K/m");
        // No T0: the parameters in the initial value apply on restart.
        let params: Vec<&str> = spec.params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(params, ["length", "alpha", "peak", "left"]);
        assert!(spec.params[2].initial_condition && !spec.params[0].initial_condition);

        // Cells of 0.5 m centred at 0.25 .. 1.75; T = x initially.
        let p = [2.0, 0.5, 1.0, 10.0];
        let (t, dt, y) = eval(&s, &p, &[]);
        assert_eq!(t, [0.25, 0.75, 1.25, 1.75]);
        // Held at 10 on the left face (ghost 19.75), insulated on the right.
        let lap = |a: f64, b: f64, c: f64| 0.5 * (a - 2.0 * b + c) / 0.25;
        assert_eq!(
            dt,
            [lap(19.75, 0.25, 0.75), 0.0, 0.0, lap(1.25, 1.75, 1.75)]
        );
        assert_eq!(y[..4], t[..]);
        assert_eq!(y[4], 4.0);
        assert_eq!(
            y[5..9],
            [(0.75 - 19.75) / 1.0, 1.0, 1.0, (1.75 - 1.25) / 1.0]
        );
        assert_eq!(y[9..13], [0.25, 0.75, 1.25, 1.75]);
        // A longer grid moves its cells at once (no restart).
        let p2 = [4.0, 0.5, 1.0, 10.0];
        let mut y2 = vec![0.0; 13];
        s.outputs(0.0, &t, &[], &p2, &mut y2);
        assert_eq!(y2[9..13], [0.5, 1.5, 2.5, 3.5]);
    }

    #[test]
    fn arrays_mix_with_scalars_inputs_and_units() {
        let s = set(&[
            "grid r [cm] = 0 .. 4, 2",
            "input q[r] [K/s], boost",
            "param tau [s] = 1",
            "state u[r] [K] = 1",
            "state v [K] = 2",
            "edge u: r periodic",
            "du/dt = q*boost + mean(u)/tau - u/tau",
            "dv/dt = (maxof(u) - v)/tau",
            "first = at(u, 0) [K]",
            "dr = spacing(r) [cm]",
        ])
        .unwrap();
        let spec = s.spec();
        assert_eq!(spec.states, ["u[0]", "u[1]", "v"]);
        let ins: Vec<(&str, Shape)> = spec.inputs().map(|p| (p.name.as_str(), p.shape)).collect();
        assert_eq!(
            ins,
            [
                ("q", Shape::Grid { nx: 2, ny: 1 }),
                ("boost", Shape::Scalar)
            ]
        );
        // tau, v0: v starts from v0, u from its expression.
        let p = [1.0, 2.0];
        let (x, dx, y) = eval(&s, &p, &[3.0, 4.0, 0.5]);
        assert_eq!(x, [1.0, 1.0, 2.0]);
        assert_eq!(dx, [1.5, 2.0, -1.0]);
        assert_eq!(y[..3], [1.0, 1.0, 2.0]);
        assert_eq!(y[3], 1.0);
        // The axis in cm: 2 cm cells centred at 1 and 3.
        assert_eq!(y[4], 2.0);
        assert_eq!(y[5..7], [1.0, 3.0]);
        let dr = spec.ports.iter().find(|p| p.name == "dr").unwrap();
        assert_eq!(dr.unit.to_string(), "cm");
    }

    #[test]
    fn heat_diffuses_and_is_conserved_with_insulated_edges() {
        use crate::ode::OdeBlock;
        use simsandbox_core::graph::Model;
        use simsandbox_core::sim::Simulator;
        let lines: Vec<String> = [
            "grid x [m] = 0 .. 1, 20",
            "param alpha [m^2/s] = 0.01",
            "state T[x] [K] = exp(-((x - 0.5)/0.1)^2 [m^2/m^2])",
            "dT/dt = alpha*lap(T)",
            "total = sum(T) [K]",
            "peak = maxof(T) [K]",
        ]
        .iter()
        .map(|l| l.to_string())
        .collect();
        // The unit annotation inside an expression is not allowed: a
        // dimensionless exponent is written without one.
        assert!(EquationSet::parse("rod", &lines).is_err());
        let mut lines = lines;
        lines[2] = "state T[x] [K] = exp(-((x - 0.5 [m])/0.1)^2)".into();
        assert!(EquationSet::parse("rod", &lines).is_err());
        lines[1] = "param alpha [m^2/s] = 0.01, w [m] = 0.1, c [m] = 0.5".into();
        assert!(EquationSet::parse("rod", &lines).is_err());
        let lines: Vec<String> = [
            "grid x [m] = 0 .. 1, 20",
            "param alpha [m^2/s] = 0.01",
            "param w [m] = 0.1",
            "param c [m] = 0.5",
            "param top [K] = 1",
            "state T[x] [K] = top*exp(-((x - c)/w)^2)",
            "dT/dt = alpha*lap(T)",
            "total = sum(T) [K]",
            "peak = maxof(T) [K]",
        ]
        .iter()
        .map(|l| l.to_string())
        .collect();
        let s = EquationSet::parse("rod", &lines).unwrap();
        let mut m = Model::new();
        let id = m.add(Box::new(OdeBlock::new(
            s,
            crate::integrators::Integrator::Rk4 { substeps: 1 },
        )));
        let mut sim = Simulator::new(m, 0.0).unwrap();
        let total0 = sim.output(id, "total").unwrap()[0];
        let peak0 = sim.output(id, "peak").unwrap()[0];
        for _ in 0..100 {
            sim.step(0.05).unwrap();
        }
        let total = sim.output(id, "total").unwrap()[0];
        let peak = sim.output(id, "peak").unwrap()[0];
        assert!(
            (total - total0).abs() < 1e-12 * total0,
            "{total0} -> {total}"
        );
        assert!(peak < 0.8 * peak0, "{peak0} -> {peak}");
    }

    #[test]
    fn array_mistakes_are_explained() {
        let with = |extra: &[&str]| {
            let mut l = vec![
                "grid x [m] = 0 .. 1, 4",
                "state T[x] [K] = 0",
                "param k [1/s] = 0",
            ];
            l.extend_from_slice(extra);
            error(&l)
        };
        assert!(with(&["dT/dt = lap(2*T)"]).contains("takes the name of an array"));
        assert!(with(&["dT/dt = ddy(T)"]).contains("write ddx"));
        assert!(with(&["dT/dt = k*T", "a = at(T, 4)"]).contains("from 0 to 3"));
        assert!(with(&["dT/dt = k*T", "edge q: x- = 1"]).contains("not an array"));
        assert!(
            with(&["dT/dt = k*T", "edge T: x- = 1, x- gradient = 0"]).contains("two conditions")
        );
        assert!(
            with(&["dT/dt = k*T", "edge T: y- = 1"]).contains("an end is an axis of the grid (x)")
        );
        assert!(with(&["dT/dt = k*T", "s = sum(t)"]).contains("takes an array"));
        assert!(with(&["dT/dt = k*T", "state v", "dv/dt = T"]).contains("must be an array"));
        // A second axis makes the grid 2D, so T[x] no longer covers it.
        assert!(with(&["dT/dt = k*T", "grid y [m] = 0 .. 1, 4"]).contains("write T[x, y]"));
        assert!(with(&[
            "dT/dt = k*T",
            "grid y [m] = 0 .. 1, 4",
            "grid z [m] = 0 .. 1, 4"
        ])
        .contains("one or two axes"));
        assert!(
            with(&["dT/dt = lap(T)"]).contains("dT/dt is in K/s but the right side is in K/m^2")
        );
        let m = with(&["dT/dt = k*T", "param g [K] = 1", "edge T: x- gradient = g"]);
        assert!(m.contains("edge gradient is in K/m"), "{m}");
        assert!(error(&["grid x [m] = 0 .. 1, 0", "y = 1"]).contains("at least 1"));
        assert!(error(&["grid x [m] = 0 .. L(1), 4", "y = 1"]).contains("unknown function"));
        assert!(error(&["grid z = 0 .. 1, 4", "w[n+1] = w"]).contains("maps cannot"));
        assert!(error(&["y = lap(T)"]).contains("declare a grid"));
        // Without a grid, brackets after a name are a unit as before.
        assert!(set(&["input F[N]", "y = F"]).is_ok());
    }

    #[test]
    fn two_axes_make_a_2d_grid() {
        // 4 by 3 cells of 0.5 m by 1 m; u = cos(πx/2)·cos(πy/3), insulated
        // everywhere, is a mode of the Laplacian on this grid.
        let s = set(&[
            "grid x [m] = 0 .. 2, 4",
            "grid y [m] = 0 .. 3, 3",
            "param a [1/s] = 1",
            "param one [m^2] = 1",
            "param amp [K] = 1",
            "state u[x, y] [K] = amp*cos(pi*x/(2 [m]))",
            "du/dt = a*one*lap(u)",
            "corner = at(u, 3, 2)",
            "slope = ddy(u)",
        ]);
        // Units are not written inside expressions.
        assert!(s.is_err());
        let s = set(&[
            "grid x [m] = 0 .. lx, 4",
            "grid y [m] = 0 .. ly, 3",
            "param lx [m] = 2",
            "param ly [m] = 3",
            "param a [m^2/s] = 1",
            "param amp [K] = 1",
            "state u[x, y] [K] = amp*cos(pi*x/lx)*cos(pi*y/ly)",
            "edge u: y+ = 0",
            "du/dt = a*lap(u)",
            "corner = at(u, 3, 2)",
            "slope = ddy(u)",
        ])
        .unwrap();
        let spec = s.spec();
        let grid = Shape::Grid { nx: 4, ny: 3 };
        let outs: Vec<(&str, Shape)> = spec.outputs().map(|p| (p.name.as_str(), p.shape)).collect();
        assert_eq!(
            outs,
            [
                ("u", grid),
                ("corner", Shape::Scalar),
                ("slope", grid),
                ("x", grid),
                ("y", grid)
            ]
        );
        let y = spec.ports.iter().find(|p| p.name == "y").unwrap();
        assert_eq!(y.coordinate[0].axis, 1);
        let p = [2.0, 3.0, 1.0, 1.0];
        let (u, du, out) = eval(&s, &p, &[]);
        // Cell centres: x at 0.25, 0.75, 1.25, 1.75 (fastest); y at 0.5, 1.5, 2.5.
        // Outputs: u (12), corner, slope (12), x (12), y (12).
        assert_eq!(out[25..29], [0.25, 0.75, 1.25, 1.75]);
        assert_eq!(out[33..37], [0.25, 0.75, 1.25, 1.75]);
        assert_eq!(out[37..41], [0.5, 0.5, 0.5, 0.5]);
        assert_eq!(out[45..49], [2.5, 2.5, 2.5, 2.5]);
        let exact = |i: usize, j: usize| {
            ((i as f64 + 0.5) * 0.5 * PI / 2.0).cos() * ((j as f64 + 0.5) * PI / 3.0).cos()
        };
        assert!((u[4 * 2 + 3] - exact(3, 2)).abs() < 1e-15);
        assert_eq!(out[12], u[11]);
        // Interior rows (j = 0, 1 are away from the held y+ edge) decay
        // at the discrete rate of the mode along each axis.
        let rate = |h: f64, k: f64| -4.0 / (h * h) * (k * h / 2.0).sin().powi(2);
        let r = rate(0.5, PI / 2.0) + rate(1.0, PI / 3.0);
        for k in 0..4 {
            assert!(
                (du[k] - r * u[k]).abs() < 1e-12,
                "cell {k}: {} vs {}",
                du[k],
                r * u[k]
            );
        }
        // The top row feels the edge held at 0 instead.
        assert!((du[8] - r * u[8]).abs() > 1e-3);
        // The structure: each cell depends on itself and its 4 neighbours
        // (fewer at the edges).
        use crate::ode::OdeSystem;
        let rows = s.sparsity().unwrap();
        assert_eq!(rows[5], [1, 4, 5, 6, 9]);
        assert_eq!(rows[0], [0, 1, 4]);
    }

    #[test]
    fn two_axes_need_the_same_unit_for_lap() {
        let m = error(&[
            "grid x [m] = 0 .. 1, 4",
            "grid y [s] = 0 .. 1, 4",
            "state u[x, y] = 0",
            "du/dt = lap(u)",
        ]);
        assert!(m.contains("the axes need the same unit"), "{m}");
        assert!(error(&[
            "grid x [m] = 0 .. 1, 4",
            "grid y [m] = 0 .. 1, 4",
            "state u[x, y] = 0",
            "du/dt = ddz(u)"
        ])
        .contains("not an axis of the grid (x, y)"));
        assert!(error(&[
            "grid x = 0 .. 1, 4",
            "grid y = 0 .. 1, 3",
            "state u[x, y] = 0",
            "du/dt = u",
            "c = at(u, 1, 3)"
        ])
        .contains("along y is a whole number from 0 to 2"));
    }
}
