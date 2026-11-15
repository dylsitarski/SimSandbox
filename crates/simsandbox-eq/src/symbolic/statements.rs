//! Reading the lines of a definition: which statement a line is, and
//! its pieces (tables, cells, meshes, rules, grids, edges, units, left sides).

use super::{Axis, CellTransfer, RuleKind};
use simsandbox_core::units::Unit;
use simsandbox_core::{Result, SimError};
use simsandbox_expr::{is_reserved, parse, Ast};
/// Parameters over a table's rows (ADR 0069, ADR 0079 point 2), `param
/// g[pipes] [S] = 2`: a value per row that does not change, kept with
/// the row through rules and set by the table's starting file. Written as
/// a state over the rows that holds (`dg/dt = 0`, or `g[n+1] = g` in a
/// block of maps), so it is stored, rewound and passed on as states are.
/// Lines are kept in place, so errors keep their line numbers; the
/// holding lines come after them, each with the line of its `param`.
pub(super) fn row_params(lines: &[String]) -> (Vec<String>, Vec<usize>) {
    let tables: Vec<&str> = lines
        .iter()
        .filter_map(|l| {
            let l = l.trim();
            ["table ", "edges ", "cells ", "sides "]
                .iter()
                .find_map(|w| l.strip_prefix(w))
                .and_then(|r| r.split_whitespace().next())
        })
        .collect();
    let (continuous, maps) = dynamics(lines);
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut held = Vec::new();
    for (k, l) in lines.iter().enumerate() {
        let t = l.trim();
        let over_rows = t.strip_prefix("param ").and_then(|rest| {
            let head = rest.split_once('=').map_or(rest, |(h, _)| h);
            let (n, dims) = head.split_once('[')?;
            let (dims, _) = dims.split_once(']')?;
            dims.split(',')
                .any(|d| tables.contains(&d.trim()))
                .then(|| n.trim().to_string())
        });
        match over_rows {
            Some(n) if continuous || maps => {
                let rest = t.strip_prefix("param ").expect("checked");
                let line = if rest.contains('=') {
                    format!("state {rest}")
                } else {
                    format!("state {rest} = 1")
                };
                out.push(line);
                held.push((n, k + 1));
            }
            _ => out.push(l.clone()),
        }
    }
    let mut from = Vec::with_capacity(held.len());
    for (n, line) in held {
        out.push(if continuous {
            format!("d{n}/dt = 0")
        } else {
            format!("{n}[n+1] = {n}")
        });
        from.push(line);
    }
    (out, from)
}

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

/// A table's starting rows read from a CSV file (ADR 0069 step 3):
/// `start pts = file "nodes.csv"`. Each line a row; an `id` column gives
/// the rows' ids, an edge table's `from` and `to` columns the ids of the
/// rows each edge joins, and every other column the starting values of
/// the quantity over the table it names (a header may give a unit,
/// `g [mS]`).
#[derive(Debug, Clone, PartialEq)]
pub struct StartCsv {
    pub path: String,
    pub rows: usize,
    pub ids: Option<Vec<u64>>,
    /// Name, unit as written, values.
    pub columns: Vec<(String, Option<String>, Vec<f64>)>,
}

impl StartCsv {
    /// A column's values by name.
    pub fn column(&self, name: &str) -> Option<&[f64]> {
        self.columns
            .iter()
            .find(|c| c.0 == name)
            .map(|c| c.2.as_slice())
    }

    /// Read `path` (relative to the model's folder), cached by its bytes.
    pub(super) fn load(path: &str) -> std::result::Result<std::sync::Arc<Self>, String> {
        type Cache =
            std::collections::HashMap<std::path::PathBuf, (Vec<u8>, std::sync::Arc<StartCsv>)>;
        static CACHE: std::sync::Mutex<Option<Cache>> = std::sync::Mutex::new(None);
        let file = std::path::PathBuf::from(path);
        let file = match (file.is_absolute(), simsandbox_core::registry::current_folder()) {
            (false, Some(folder)) => folder.join(file),
            (false, None) => {
                return Err(format!(
                    "{path} is relative to the model's folder: save the model first, or give a full path"
                ))
            }
            _ => file,
        };
        let bytes = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        let cache = cache.get_or_insert_with(Default::default);
        if let Some((b, c)) = cache.get(&file) {
            if *b == bytes {
                return Ok(c.clone());
            }
        }
        let text =
            String::from_utf8(bytes.clone()).map_err(|_| format!("{path}: not a text file"))?;
        let csv = std::sync::Arc::new(Self::parse(path, &text)?);
        cache.insert(file, (bytes, csv.clone()));
        Ok(csv)
    }

    pub(super) fn parse(path: &str, text: &str) -> std::result::Result<Self, String> {
        let mut lines = text
            .lines()
            .enumerate()
            .filter(|(_, l)| !l.trim().is_empty() && !l.trim_start().starts_with('#'));
        let Some((_, header)) = lines.next() else {
            return Err(format!("{path} is empty: its first line names the columns"));
        };
        let mut columns: Vec<(String, Option<String>, Vec<f64>)> = Vec::new();
        for h in header.split(',') {
            let h = h.trim();
            let (name, unit) = match h.strip_suffix(']').and_then(|r| r.rsplit_once('[')) {
                Some((n, u)) => (n.trim().to_string(), Some(u.trim().to_string())),
                None => (h.to_string(), None),
            };
            if !is_name(&name) || columns.iter().any(|c| c.0 == name) {
                return Err(format!(
                    "{path}: the column '{name}' is not a name, or comes twice"
                ));
            }
            columns.push((name, unit, Vec::new()));
        }
        let mut rows = 0;
        for (k, line) in lines {
            let cells: Vec<&str> = line.split(',').collect();
            if cells.len() != columns.len() {
                return Err(format!(
                    "{path}, line {}: {} values for {} columns",
                    k + 1,
                    cells.len(),
                    columns.len()
                ));
            }
            for (c, v) in columns.iter_mut().zip(cells) {
                let v: f64 = v.trim().parse().map_err(|_| {
                    format!("{path}, line {}: '{}' is not a number", k + 1, v.trim())
                })?;
                c.2.push(v);
            }
            rows += 1;
        }
        let whole = |name: &str| -> std::result::Result<Option<Vec<u64>>, String> {
            let Some(c) = columns.iter().find(|c| c.0 == name) else {
                return Ok(None);
            };
            c.2.iter()
                .map(|&v| {
                    (v >= 0.0 && v.fract() == 0.0 && v < 2f64.powi(53))
                        .then_some(v as u64)
                        .ok_or_else(|| format!("{path}: {name} {v} is not a whole number from 0"))
                })
                .collect::<std::result::Result<Vec<u64>, String>>()
                .map(Some)
        };
        let ids = whole("id")?;
        if let Some(ids) = &ids {
            let mut seen = std::collections::HashSet::new();
            if let Some(d) = ids.iter().find(|i| !seen.insert(**i)) {
                return Err(format!("{path}: the id {d} comes twice"));
            }
        }
        whole("from")?;
        whole("to")?;
        Ok(Self {
            path: path.to_string(),
            rows,
            ids,
            columns: columns.into_iter().filter(|c| c.0 != "id").collect(),
        })
    }

    /// The rows' ids: from the file, else 0, 1, ….
    pub fn row_ids(&self) -> Vec<u64> {
        self.ids
            .clone()
            .unwrap_or_else(|| (0..self.rows as u64).collect())
    }
}

/// A start of the form `file "nodes.csv"`: the path.
pub(super) fn quoted_file(spec: &str) -> Option<std::result::Result<String, ()>> {
    let rest = spec.trim().strip_prefix("file ")?.trim();
    let Some(inner) = rest.strip_prefix('"') else {
        return Some(Err(()));
    };
    Some(match inner.split_once('"') {
        Some((p, after)) if after.trim().is_empty() => Ok(p.to_string()),
        _ => Err(()),
    })
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

/// How cells start (ADR 0085): a grid, or a mesh file whose points'
/// coordinates go to the states named `at`.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum MeshSpec {
    Grid(crate::mesh::GridStart),
    File { path: String, at: Vec<String> },
}

impl MeshSpec {
    /// The coordinates it sets, by state name.
    pub(super) fn names(&self) -> Vec<String> {
        match self {
            MeshSpec::Grid(g) => g.axes.iter().map(|a| a.0.clone()).collect(),
            MeshSpec::File { at, .. } => at.clone(),
        }
    }

    /// `grid x 0 .. 1 by 8, …` or `file "mesh.msh" at x, y`.
    pub(super) fn parse(text: &str) -> Option<std::result::Result<Self, String>> {
        if let Some(g) = text.strip_prefix("grid ") {
            return Some(crate::mesh::GridStart::parse(g).map(MeshSpec::Grid));
        }
        let rest = text.strip_prefix("file ")?;
        let usage = "write a mesh file's start as 'start tri = file \"part.msh\" at x, y' (the states its points' coordinates go to)";
        let Some((path, at)) = rest
            .trim()
            .strip_prefix('"')
            .and_then(|r| r.split_once('"'))
        else {
            return Some(Err(usage.into()));
        };
        let Some(at) = at.trim().strip_prefix("at ") else {
            return Some(Err(usage.into()));
        };
        let at: Vec<String> = at.split(',').map(|n| n.trim().to_string()).collect();
        if at.is_empty() || at.len() > 3 || at.iter().any(|n| !is_name(n)) {
            return Some(Err(usage.into()));
        }
        Some(Ok(MeshSpec::File {
            path: path.to_string(),
            at,
        }))
    }
}

/// The starting points and cells of each table starting from a grid or a
/// file (ADR 0085), by cell table: points (one list per coordinate the
/// start sets) and corners as places among them. Cells over the same
/// points from the same file share them.
#[allow(clippy::type_complexity)]
pub(super) fn start_meshes(
    specs: &std::collections::HashMap<String, (MeshSpec, usize)>,
    cell_decls: &std::collections::HashMap<String, (String, crate::mesh::CellShape)>,
) -> Result<std::collections::HashMap<String, crate::mesh::Start>> {
    let mut out = std::collections::HashMap::new();
    for (c, (spec, line)) in specs {
        let Some((pts, shape)) = cell_decls.get(c) else {
            continue;
        };
        let err = |m: String| definition_error(*line, 1, m);
        match spec {
            MeshSpec::Grid(g) => {
                out.insert(c.clone(), (g.points(), g.cells(*shape).map_err(err)?));
            }
            MeshSpec::File { path, at } => {
                let file = std::path::PathBuf::from(path);
                let file = match (
                    file.is_absolute(),
                    simsandbox_core::registry::current_folder(),
                ) {
                    (false, Some(folder)) => folder.join(file),
                    _ => file,
                };
                let mesh = crate::mesh::FileMesh::load(&file).map_err(err)?;
                // Every table over these points starting from this file.
                let group: Vec<(&String, crate::mesh::CellShape)> = specs
                    .iter()
                    .filter(|(_, (s, _))| s == spec)
                    .filter_map(|(d, _)| {
                        cell_decls.get(d).filter(|o| o.0 == *pts).map(|o| (d, o.1))
                    })
                    .collect();
                let shapes: Vec<crate::mesh::CellShape> = group.iter().map(|g| g.1).collect();
                let (coords, cells) = mesh.select(&shapes).map_err(err)?;
                let k = group.iter().position(|g| g.0 == c).expect("in its group");
                if cells[k].is_empty() {
                    return Err(err(format!("{path} has no {} cells", shape.name())));
                }
                out.insert(c.clone(), (coords[..at.len()].to_vec(), cells[k].clone()));
            }
        }
    }
    Ok(out)
}

/// How many sides the starting cells of axis `c` have (ADR 0085).
pub(super) fn starting_sides(
    grid: &[Axis],
    c: usize,
    chains: &std::collections::HashMap<String, usize>,
    meshes: &std::collections::HashMap<String, crate::mesh::Start>,
    starts: &std::collections::HashMap<String, (usize, usize)>,
) -> Result<usize> {
    let shape = grid[c].shape.expect("cells");
    let corners: Vec<Vec<usize>> = match (meshes.get(&grid[c].name), chains.get(&grid[c].name)) {
        (Some((_, cells)), _) => cells.clone(),
        (_, Some(_)) => {
            let n = grid[c]
                .ends_of
                .and_then(|p| starts.get(&grid[p].name))
                .map_or(0, |s| s.0);
            (1..n).map(|k| vec![k - 1, k]).collect()
        }
        _ => Vec::new(),
    };
    let (_, faces) = shape.sides().expect("checked: one shape of side");
    let mut seen = std::collections::HashSet::new();
    for cell in &corners {
        for f in &faces {
            let mut key: Vec<usize> = f.iter().map(|&k| cell[k]).collect();
            key.sort_unstable();
            seen.insert(key);
        }
    }
    Ok(seen.len())
}

/// A column's value in an `add` rule: its name, expression and column.
pub(super) type Given = (String, Ast, usize);

/// A rule as checked: its table and source axes (by place), the rule, and
/// each column's value (name, expression, line, column).
pub(super) type CheckedRule<'a> = (
    usize,
    Option<usize>,
    &'a RuleLine,
    Vec<(String, Ast, usize, usize)>,
    Vec<(bool, usize, CellTransfer)>,
);

/// A rule changing a table between steps (ADR 0079), as written.
pub(super) struct RuleLine {
    pub(super) line: usize,
    pub(super) kind: RuleKind,
    pub(super) table: String,
    /// `remove pts where cond`: `None`; `add pts [from s] [where c]: …`:
    /// the axis rows come from, if any, and the values given.
    pub(super) add: Option<(Option<String>, Vec<Given>)>,
    /// The condition (`where`), 1 if none.
    pub(super) cond: Ast,
    pub(super) cond_column: usize,
}

/// `remove pts where cond` or `add pts [from s] [where cond][: c = e, …]`.
pub(super) fn read_rule(text: &str, word: &str, rest: &str, line: usize) -> Result<RuleLine> {
    let usage = match word {
        "remove" => "write 'remove pts where condition'",
        "split" | "merge" => "write 'split segs where condition' or 'merge segs where condition: c = mean' (each column mean, copy, share, sum or a value)",
        _ => "write 'add pts: c = value', 'add pts where condition: …' or 'add pts from k where condition: c = value'",
    };
    let kind = match word {
        "remove" => RuleKind::Remove,
        "split" => RuleKind::Split,
        "merge" => RuleKind::Merge,
        _ => RuleKind::Add,
    };
    let bad = |m: String| definition_error(line, 1, m);
    let parse_at = |src: &str| -> Result<(Ast, usize)> {
        let col = column_of(text, src.trim());
        parse(src)
            .map(|a| (a, col))
            .map_err(|e| definition_error(line, col + e.column - 1, e.message))
    };
    let (head, assigns) = match (word, rest.split_once(':')) {
        ("add" | "split" | "merge", Some((h, a))) => (h, Some(a)),
        _ => (rest, None),
    };
    let (head, cond) = match head.split_once(" where ") {
        Some((h, c)) => (h, Some(c)),
        None => (head, None),
    };
    let words: Vec<&str> = head.split_whitespace().collect();
    let (table, from) = match (word, &words[..]) {
        (_, [t]) => (t.to_string(), None),
        ("add", [t, "from", s]) => (t.to_string(), Some(s.to_string())),
        _ => return Err(bad(usage.into())),
    };
    if word != "add" && cond.is_none() {
        return Err(bad(usage.into()));
    }
    let (cond, cond_column) = match cond {
        Some(c) => parse_at(c)?,
        None => (Ast::Num(1.0), 1),
    };
    let add = if word != "remove" {
        let mut values = Vec::new();
        for item in assigns.map(split_top).unwrap_or_default() {
            if item.trim().is_empty() {
                continue;
            }
            let Some((n, e)) = item.split_once('=') else {
                return Err(bad(usage.into()));
            };
            // A transfer by name (ADR 0079 point 4), for split and merge.
            if word != "add" && matches!(e.trim(), "mean" | "copy" | "share" | "sum") {
                let col = column_of(text, e.trim());
                values.push((n.trim().to_string(), Ast::Name(e.trim().into(), col), col));
                continue;
            }
            let (ast, col) = parse_at(e)?;
            values.push((n.trim().to_string(), ast, col));
        }
        Some((from, values))
    } else {
        None
    };
    Ok(RuleLine {
        line,
        kind,
        table,
        add,
        cond,
        cond_column,
    })
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
