//! Reading the lines of a definition: which statement a line is, and
//! its pieces (tables, cells, meshes, rules, grids, edges, units, left sides).

use super::rules::{read_rule, RuleLine};
use super::starts::{quoted_file, MeshSpec};
use super::Axis;
use simsandbox_core::units::Unit;
use simsandbox_core::{Result, SimError};
use simsandbox_expr::{is_reserved, parse, Ast};
/// The words that begin a statement other than an equation.
const STATEMENTS: [&str; 14] = [
    "grid", "table", "edges", "cells", "sides", "start", "remove", "add", "split", "merge", "edge",
    "state", "param", "input",
];

/// What follows `word` on a line that is its statement. `param = 2` and
/// the like are equations defining such a name.
pub(super) fn statement(line: &str, word: &str) -> Option<String> {
    let rest = line.strip_prefix(word)?.strip_prefix(' ')?;
    (!rest.trim_start().starts_with('=')).then(|| rest.to_string())
}

/// Whether the lines give a continuous state (`dx/dt = …`, `x' = …`) and
/// whether a map's (`x[n+1] = …`), by the equations' left sides as
/// [`read_lhs`] reads them.
pub(super) fn dynamics(lines: &[String]) -> (bool, bool) {
    let (mut continuous, mut maps) = (false, false);
    for l in lines {
        let t = l.trim();
        if t.starts_with('#') || STATEMENTS.iter().any(|w| statement(t, w).is_some()) {
            continue;
        }
        match l.split_once('=').and_then(|(lhs, _)| read_lhs(lhs)) {
            Some(Lhs::State(_)) => continuous = true,
            Some(Lhs::Map(_)) => maps = true,
            _ => {}
        }
    }
    (continuous, maps)
}

/// The most rows a table has unless it says (`table pts max 200`).
pub const DEFAULT_MAX_ROWS: usize = 100_000;

/// A `table pts` or `table pts max 200` line (ADR 0079), `rest` after
/// `table`: its name and its most rows.
pub(super) fn read_table(rest: &str, line: usize) -> Result<(String, usize)> {
    let usage = "write a table as 'table pts' or 'table pts max 200' (its most rows)";
    let words: Vec<&str> = rest.split_whitespace().collect();
    let (name, max) = match words[..] {
        [name] => (name, DEFAULT_MAX_ROWS),
        [name, "max", n] => match n.parse::<usize>() {
            Ok(m) if m >= 1 => (name, m),
            _ => return Err(definition_error(line, 1, usage)),
        },
        _ => return Err(definition_error(line, 1, usage)),
    };
    if !is_name(name) || name == "t" || is_reserved(name) {
        return Err(definition_error(
            line,
            1,
            format!("'{name}' cannot be a table name"),
        ));
    }
    Ok((name.to_string(), max))
}

/// A `cells tri of pts [max 200]: triangle` line (ADR 0085), `rest`
/// after `cells`: its name, its points' table, its most rows and shape.
pub(super) fn read_cells(
    rest: &str,
    line: usize,
) -> Result<(String, String, usize, crate::mesh::CellShape)> {
    let usage = "write cells as 'cells tri of pts: triangle' or 'cells tri of pts max 500: tetra' (segment, triangle, quad, tetra, hexa, prism, pyramid, simplex d, box d)";
    let Some((head, shape)) = rest.split_once(':') else {
        return Err(definition_error(line, 1, usage));
    };
    let Some(shape) = crate::mesh::CellShape::parse(shape) else {
        return Err(definition_error(
            line,
            1,
            format!("'{}' is not a shape: {usage}", shape.trim()),
        ));
    };
    let words: Vec<&str> = head.split_whitespace().collect();
    let (name, of, max) = match words[..] {
        [c, "of", p] => (c, p, DEFAULT_MAX_ROWS),
        [c, "of", p, "max", n] => match n.parse::<usize>() {
            Ok(m) if m >= 1 => (c, p, m),
            _ => return Err(definition_error(line, 1, usage)),
        },
        _ => return Err(definition_error(line, 1, usage)),
    };
    let (name, _) = read_table(name, line)?;
    Ok((name, of.to_string(), max, shape))
}

/// An expression as written: its text, line and column.
pub(super) type Written = (String, usize, usize);

/// One end condition of an `edge` line, as written.
pub(super) enum EdgeKind {
    Value(Ast),
    Gradient(Ast),
    Periodic,
}

/// An `edge` line's condition for one end (`high` false: low end) of
/// array `array`.
pub(super) struct EdgeLine {
    pub(super) array: String,
    pub(super) line: usize,
    pub(super) column: usize,
    /// Which axis of the grid, by place.
    pub(super) axis: usize,
    pub(super) high: bool,
    pub(super) kind: EdgeKind,
}

pub(super) struct Line {
    pub(super) number: usize,
    /// Character offset of the right-hand side within the line.
    pub(super) offset: usize,
    pub(super) lhs: Lhs,
    pub(super) ast: Ast,
}

/// Split a trailing unit annotation, ` [unit]`, off the right-hand side
/// of an equation. `[n]` (a map's current value) is not a unit. Returns
/// the text without it and the unit's text and column.
pub(super) fn split_unit(rhs: &str) -> (String, Option<(String, usize)>) {
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
pub(super) fn name_and_unit(item: &str) -> (String, Option<String>) {
    match item.split_once('[') {
        Some((n, rest)) => (
            n.trim().to_string(),
            Some(rest.trim_end().trim_end_matches(']').trim().to_string()),
        ),
        None => (item.trim().to_string(), None),
    }
}

pub(super) fn read_unit(text: &str, line: usize, column: usize) -> Result<Unit> {
    Unit::parse(text).map_err(|e| definition_error(line, column, e))
}

#[derive(Clone)]
pub(super) enum Lhs {
    State(String),
    Map(String),
    Defined(String),
}

pub(super) fn definition_error(
    line: usize,
    column: usize,
    message: impl std::fmt::Display,
) -> SimError {
    SimError::InvalidDefinition(format!("line {line}, column {column}: {message}"))
}

pub(super) fn is_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// Read the left-hand side: `dx/dt` or `x'` for a continuous state,
/// `x[n+1]` for a map state, a name otherwise.
pub(super) fn read_lhs(lhs: &str) -> Option<Lhs> {
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
pub(super) fn column_of(text: &str, piece: &str) -> usize {
    text.find(piece)
        .map_or(1, |i| text[..i].chars().count() + 1)
}

/// Split at commas outside brackets: `T[x, y] [K], q` gives two items.
pub(super) fn split_top(s: &str) -> Vec<&str> {
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

/// A declared name with its axes, `T[x, y]` (ADR 0060, ADR 0080): the
/// declaration without them, and its axes (places in the grid lines) if
/// it is an array. Brackets right after the name hold axes when every
/// item names an axis of the grid; otherwise they are a unit (`F[N]`),
/// left in place.
pub(super) fn split_dims(
    head: &str,
    axes: &[&str],
    line: usize,
) -> Result<(String, Option<Vec<usize>>)> {
    let h = head.trim_start();
    let Some(open) = h.find('[') else {
        return Ok((head.to_string(), None));
    };
    let (name, rest) = h.split_at(open);
    let Some(close) = rest.find(']') else {
        return Ok((head.to_string(), None));
    };
    let items: Vec<&str> = rest[1..close].split(',').map(str::trim).collect();
    if !is_name(name) || axes.is_empty() || !items.iter().all(|i| axes.contains(i)) {
        return Ok((head.to_string(), None));
    }
    let places: Vec<usize> = items
        .iter()
        .map(|i| axes.iter().position(|a| a == i).expect("an axis"))
        .collect();
    if places.windows(2).any(|w| w[0] >= w[1]) {
        let mut sorted = places.clone();
        sorted.sort_unstable();
        sorted.dedup();
        let names: Vec<&str> = sorted.iter().map(|&a| axes[a]).collect();
        return Err(definition_error(
            line,
            1,
            format!(
                "name each axis once, in the order of the grid lines: write {name}[{}]",
                names.join(", ")
            ),
        ));
    }
    Ok((format!("{name}{}", &rest[close + 1..]), Some(places)))
}

/// A `grid x [m] = a .. b, n` line (ADR 0060), `rest` after `grid`.
pub(super) fn read_grid(text: &str, rest: &str, line: usize) -> Result<(Axis, Option<String>)> {
    let usage = "write a grid axis as 'grid x [m] = 0 .. length, 50' (its name, unit, ends and number of cells), or 'grid k = 3' for cells numbered 0, 1, 2";
    let Some((head, body)) = rest.split_once('=') else {
        return Err(definition_error(line, 1, usage));
    };
    let (name, unit) = name_and_unit(head);
    // An index axis (ADR 0070): cells numbered 0 .. n-1, with no unit.
    if !body.contains("..") && !body.contains(',') {
        let n = body.trim();
        return match n.parse::<usize>() {
            Ok(cells)
                if cells >= 1
                    && unit.is_none()
                    && is_name(&name)
                    && name != "t"
                    && !is_reserved(&name) =>
            {
                Ok((
                    Axis {
                        name,
                        line,
                        lo: Ast::Num(-0.5),
                        hi: Ast::Num(cells as f64 - 0.5),
                        cells,
                        table: None,
                        ends_of: None,
                        shape: None,
                        sides_of: None,
                    },
                    None,
                ))
            }
            Ok(_) if unit.is_some() => Err(definition_error(
                line,
                1,
                "an index axis ('grid k = 3') numbers its cells and has no unit",
            )),
            _ => Err(definition_error(line, column_of(text, n), usage)),
        };
    }
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
            table: None,
            ends_of: None,
            shape: None,
            sides_of: None,
        },
        unit,
    ))
}

/// An `edge T: x- = left, x+ gradient = 0, x periodic` line (ADR 0060),
/// `rest` after `edge`.
pub(super) fn read_edges(
    text: &str,
    rest: &str,
    line: usize,
    axes: &[&str],
) -> Result<Vec<EdgeLine>> {
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

/// How a `start` line may be written, for both `start` lines.
const START_USAGE: &str =
    "write a table's starting rows as 'start pts = 5' (edges: 'start segs = chain'; cells: 'start tri = grid x 0 .. 1 by 8, y 0 .. 1 by 8')";

/// A `table pts [max 200]` line: its name and its most rows.
pub(super) type TableLine = (String, usize);

/// An `edges`, `cells` or `sides` line: its name, the table it is of,
/// its most rows and its shape (edges and sides: a segment, the sides'
/// own shape being found from their cells').
pub(super) type JoinLine = (String, String, usize, crate::mesh::CellShape);

/// `edges segs of pts [max 200]` (ADR 0079 stage 2), `cells tri of pts
/// [max 200]: triangle` and `sides f of tri [max 200]` (ADR 0085), `rest`
/// after the word.
fn read_join(word: &str, rest: &str, line: usize) -> Result<JoinLine> {
    if word == "cells" {
        return read_cells(rest, line);
    }
    let usage = format!("write {word} as '{word} segs of pts' or '{word} segs of pts max 200'");
    let words: Vec<&str> = rest.split_whitespace().collect();
    let (e, m, max) = match words[..] {
        [e, "of", m] => (e, m, DEFAULT_MAX_ROWS),
        [e, "of", m, "max", n] => match n.parse::<usize>() {
            Ok(x) if x >= 1 => (e, m, x),
            _ => return Err(definition_error(line, 1, usage)),
        },
        _ => return Err(definition_error(line, 1, usage)),
    };
    let (t, _) = read_table(e, line)?;
    Ok((t, m.to_string(), max, crate::mesh::CellShape::Segment))
}

/// A `start pts = …` line: its table, as written, and what it starts as.
pub(super) struct StartLine {
    pub(super) table: String,
    pub(super) how: StartHow,
}

/// What a table starts as (ADR 0079, ADR 0069 step 3, ADR 0085).
pub(super) enum StartHow {
    /// `chain`: an edge from each starting row to the next.
    Chain,
    /// `file "…"`, read both ways: a table's or an edge table's rows come
    /// from a CSV file, cells from a mesh file; which it is depends on
    /// the table's declaration, known once every line is read.
    File {
        csv: std::result::Result<String, ()>,
        mesh: std::result::Result<MeshSpec, String>,
    },
    /// `grid x 0 .. 1 by 8, …`: cells and their points.
    Grid(std::result::Result<MeshSpec, String>),
    /// A number of rows.
    Rows(usize),
    /// None of these.
    Unknown,
}

/// `rest` after `start`.
fn read_start(rest: &str, line: usize) -> Result<StartLine> {
    let Some((t, n)) = rest.split_once('=') else {
        return Err(definition_error(line, 1, START_USAGE));
    };
    let how = match (quoted_file(n), MeshSpec::parse(n.trim())) {
        _ if n.trim() == "chain" => StartHow::Chain,
        (Some(csv), Some(mesh)) => StartHow::File { csv, mesh },
        (None, Some(spec)) => StartHow::Grid(spec),
        // (Whatever begins with `file` is read as a mesh's file too.)
        (_, None) => match n.trim().parse::<usize>() {
            Ok(n) => StartHow::Rows(n),
            Err(_) => StartHow::Unknown,
        },
    };
    Ok(StartLine {
        table: t.trim().to_string(),
        how,
    })
}

impl StartLine {
    /// The mistake of a start that is none of the ways to write one.
    pub(super) fn unknown(line: usize) -> SimError {
        definition_error(line, 1, START_USAGE)
    }
}

/// An equation as far as its line alone says: its left side, where its
/// right side begins, the right side's text without its unit, and the
/// unit written after it with its column. The right side is parsed once
/// the block's kind is known (a map's `x[n]` is its present value).
pub(super) struct EquationLine {
    pub(super) offset: usize,
    pub(super) lhs: Lhs,
    pub(super) rhs: String,
    pub(super) unit: Option<(Unit, usize)>,
}

fn read_equation(text: &str, line: usize) -> Result<EquationLine> {
    let Some((lhs, rhs)) = text.split_once('=') else {
        return Err(definition_error(
            line,
            1,
            "expected an equation such as 'dx/dt = -k*x' or 'E = x^2'",
        ));
    };
    let lhs_kind = read_lhs(lhs).ok_or_else(|| {
        definition_error(
            line,
            1,
            format!("'{}' is not 'dx/dt', 'x'', 'x[n+1]' or a name", lhs.trim()),
        )
    })?;
    let offset = lhs.chars().count() + 1;
    let (rhs, unit) = split_unit(rhs);
    let unit = match unit {
        // The unit of the left side as written (ADR 0049).
        Some((u, col)) => Some((read_unit(&u, line, offset + col)?, offset + col)),
        None => None,
    };
    Ok(EquationLine {
        offset,
        lhs: lhs_kind,
        rhs,
        unit,
    })
}

/// One line of a definition, as its own text says it: which statement it
/// is, and what its reader finds. A line that cannot be read keeps its
/// mistake (`read` is an `Err`), reported when its turn comes among the
/// lines ([`super::declare`] takes them in the language's order, not the
/// written one). Declarations of names (`edge`, `state`, `param`, `input`)
/// stay as written: brackets after a name hold axes or a unit depending
/// on the axes declared, so they are read once the axes are known.
pub(super) enum Statement {
    /// A blank line or a comment.
    Blank,
    /// `grid x [m] = 0 .. 1, 50`: the axis and its unit as written.
    Grid {
        line: usize,
        read: Result<(Axis, Option<String>)>,
    },
    /// `table pts [max 200]`.
    Table {
        line: usize,
        read: Result<TableLine>,
    },
    /// `edges segs of pts [max 200]`; `first` is the word after `edges`.
    Edges {
        line: usize,
        first: Option<String>,
        read: Result<JoinLine>,
    },
    /// `cells tri of pts [max 200]: triangle`.
    Cells { line: usize, read: Result<JoinLine> },
    /// `sides f of tri [max 200]`.
    Sides { line: usize, read: Result<JoinLine> },
    /// `start pts = 5`.
    Start {
        line: usize,
        read: Result<StartLine>,
    },
    /// `remove`, `add`, `split` and `merge`; the rule has its line.
    Rule { read: Result<RuleLine> },
    /// `edge T: x- = 1`, `rest` after `edge` (its ends are axes).
    Edge {
        line: usize,
        text: String,
        rest: String,
    },
    /// `state x[k] [m] = 0`, `rest` after `state`.
    State {
        line: usize,
        text: String,
        rest: String,
    },
    /// `param k [N/m] = 4`, `rest` after `param`.
    Param { line: usize, rest: String },
    /// `input F [N], q[x]`, `rest` after `input`.
    Input { line: usize, rest: String },
    /// Every other line: an equation, whose left side says what it gives.
    Equation {
        line: usize,
        read: Result<EquationLine>,
    },
}

impl Statement {
    /// A line declaring a table over another's rows: its word, line and
    /// what was read.
    pub(super) fn join(&self) -> Option<(&'static str, usize, &Result<JoinLine>)> {
        match self {
            Statement::Edges { line, read, .. } => Some(("edges", *line, read)),
            Statement::Cells { line, read } => Some(("cells", *line, read)),
            Statement::Sides { line, read } => Some(("sides", *line, read)),
            _ => None,
        }
    }
}

/// What was read on a line, or its mistake.
pub(super) fn as_read<T>(read: &Result<T>) -> Result<&T> {
    read.as_ref().map_err(Clone::clone)
}

/// Every line as its statement, each read once. Whether the block is a
/// map is found first: a map's rules may write a state's present value
/// `x[n]`, as its equations may.
pub(super) fn statements(lines: &[String]) -> Vec<Statement> {
    let (_, maps) = dynamics(lines);
    lines
        .iter()
        .enumerate()
        .map(|(k, text)| statement_of(text, k + 1, maps))
        .collect()
}

fn statement_of(text: &str, line: usize, maps: bool) -> Statement {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return Statement::Blank;
    }
    let Some((word, rest)) = STATEMENTS
        .into_iter()
        .find_map(|w| statement(trimmed, w).map(|r| (w, r)))
    else {
        return Statement::Equation {
            line,
            read: read_equation(text, line),
        };
    };
    match word {
        "grid" => Statement::Grid {
            line,
            read: read_grid(text, &rest, line),
        },
        "table" => Statement::Table {
            line,
            read: read_table(&rest, line),
        },
        "edges" => Statement::Edges {
            line,
            first: rest.split_whitespace().next().map(str::to_string),
            read: read_join(word, &rest, line),
        },
        "cells" => Statement::Cells {
            line,
            read: read_join(word, &rest, line),
        },
        "sides" => Statement::Sides {
            line,
            read: read_join(word, &rest, line),
        },
        "start" => Statement::Start {
            line,
            read: read_start(&rest, line),
        },
        "edge" => Statement::Edge {
            line,
            text: text.to_string(),
            rest,
        },
        "state" => Statement::State {
            line,
            text: text.to_string(),
            rest,
        },
        "param" => Statement::Param { line, rest },
        "input" => Statement::Input { line, rest },
        // The rules that change a table between steps (ADR 0079).
        _ => {
            // `[n]` blanked, so columns stay put.
            let text = if maps {
                text.replace("[n]", "   ")
            } else {
                text.to_string()
            };
            // What follows the word and its space, which are still there.
            let read = read_rule(&text, word, &text.trim()[word.len() + 1..], line);
            Statement::Rule { read }
        }
    }
}
