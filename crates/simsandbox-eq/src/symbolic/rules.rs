//! The rules that change a table between steps (ADR 0079, ADR 0085 step
//! 2), checked against the tables: what each acts on and the value of
//! every column of the rows it makes.

use super::declare::Declared;
use super::shaped::{Arrays, Init};
use super::statements::{column_of, definition_error, split_top};
use super::{CellTransfer, RuleKind};
use simsandbox_core::{Result, SimError};
use simsandbox_expr::{parse, Ast, Op};

/// A column's value in an `add` rule: its name, expression and column.
pub(super) type Given = (String, Ast, usize);

/// What a split or merge may give a column by name (ADR 0079 point 4).
const TRANSFERS: [&str; 4] = ["mean", "copy", "share", "sum"];

/// A transfer written as a column's value: the word alone, so `(share)`
/// is a quantity named `share`. Kept as a call without arguments, which
/// no expression is, so nothing later takes the one for the other.
fn transfer_value(word: &str, column: usize) -> Ast {
    Ast::Apply(word.into(), Vec::new(), column)
}

/// The transfer a column's value names, if it is one.
pub(super) fn transfer(ast: &Ast) -> Option<&str> {
    match ast {
        Ast::Apply(n, args, _) if args.is_empty() && TRANSFERS.contains(&n.as_str()) => {
            Some(n.as_str())
        }
        _ => None,
    }
}

/// A rule changing a table between steps (ADR 0079), as written.
#[derive(Clone)]
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
            if word != "add" && TRANSFERS.contains(&e.trim()) {
                let col = column_of(text, e.trim());
                values.push((n.trim().to_string(), transfer_value(e.trim(), col), col));
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

/// A column's value in a rule: its name, expression, line and column.
pub(super) type Value = (String, Ast, usize, usize);

/// A rule as checked.
pub(super) struct CheckedRule<'a> {
    /// The table whose rows it adds or removes, by axis: for `split` and
    /// `merge`, the points of the edges or cells it is written for.
    pub(super) table: usize,
    /// The axis its condition and values are over, if any: the one new
    /// rows come from (`add pts from k`), or the edges or cells split
    /// or merged.
    pub(super) source: Option<usize>,
    pub(super) rule: &'a RuleLine,
    /// The values it computes, each for a column.
    pub(super) values: Vec<Value>,
    /// Cells split or merged: how each state over the points (false)
    /// and over the cells (true) passes on, by column.
    pub(super) transfers: Vec<(bool, usize, CellTransfer)>,
}

impl CheckedRule<'_> {
    /// The axes its condition and values are spread over.
    pub(super) fn over(&self) -> Vec<usize> {
        match (&self.rule.add, self.source) {
            (None, _) => vec![self.table],
            (Some(_), Some(s)) => vec![s],
            (Some(_), None) => Vec::new(),
        }
    }
}

/// What checking a rule needs.
struct Checker<'a, 'b> {
    d: &'a Declared,
    arrays: &'b Arrays<'a>,
    inits: &'b [Init],
}

fn bad(r: &RuleLine, column: usize, message: String) -> SimError {
    definition_error(r.line, column, message)
}

impl<'a> Checker<'a, '_> {
    fn axis_name(&self, axis: usize) -> &str {
        &self.d.grid[axis].name
    }

    /// The states over the rows of the table `axis`: its columns.
    fn columns(&self, axis: usize) -> Vec<&'a String> {
        self.d
            .states
            .iter()
            .filter(|s| self.arrays.row_table(s) == Some(axis))
            .collect()
    }

    /// An expression of rule `r` is over the axes `allowed` only.
    fn within(
        &self,
        r: &RuleLine,
        ast: &Ast,
        col: usize,
        allowed: &[usize],
        what: &str,
    ) -> Result<()> {
        let got = self
            .arrays
            .of_ast(ast)
            .map_err(|(c, m)| bad(r, col + c - 1, m))?;
        if got.iter().any(|a| !allowed.contains(a)) {
            return Err(bad(
                r,
                col,
                format!(
                    "{what} is over {}, not the rows it adds or removes",
                    self.arrays.list(&got)
                ),
            ));
        }
        Ok(())
    }

    /// The values a rule gives, as written.
    fn given(r: &RuleLine) -> Vec<Given> {
        r.add.as_ref().map(|a| a.1.clone()).unwrap_or_default()
    }

    /// `split` and `merge` of the cells `e` of shape `shape` over the
    /// points `m`: refined by their shape's rule (ADR 0085 step 2).
    fn refine(
        &self,
        r: &'a RuleLine,
        e: usize,
        m: usize,
        shape: crate::mesh::CellShape,
    ) -> Result<CheckedRule<'a>> {
        if !shape.simplicial() && crate::refine::box_codes(shape).is_none() {
            return Err(bad(
                r,
                1,
                format!(
                    "{} cells are not refined yet: simplices and boxes are",
                    shape.name()
                ),
            ));
        }
        if shape.simplicial()
            && shape.dim() >= 3
            && matches!(
                self.d.starts.specs.get(&r.table),
                Some((super::starts::MeshSpec::File { .. }, _))
            )
        {
            return Err(bad(
                r,
                1,
                format!(
                    "{} read from a file are not refined yet (their bisection needs a labelling): start them from a grid",
                    shape.name()
                ),
            ));
        }
        self.within(r, &r.cond, r.cond_column, &[e], "the condition")?;
        let given = Self::given(r);
        let point_cols = self.columns(m);
        let cell_cols = self.columns(e);
        if let Some((n, _, c)) = given
            .iter()
            .find(|g| !point_cols.contains(&&g.0) && !cell_cols.contains(&&g.0))
        {
            return Err(bad(
                r,
                *c,
                format!(
                    "'{n}' is not a state over {} or {}",
                    self.axis_name(m),
                    self.axis_name(e)
                ),
            ));
        }
        let split = r.kind == RuleKind::Split;
        let mut values = Vec::new();
        let mut transfers = Vec::new();
        for (on_cells, cols) in [(false, &point_cols), (true, &cell_cols)] {
            for (k, col) in cols.iter().enumerate() {
                let g = given.iter().find(|g| &g.0 == *col);
                let kind = match g.map(|g| (transfer(&g.1), g)) {
                    None if on_cells && split => CellTransfer::Copy,
                    None => CellTransfer::Mean,
                    Some((Some(n), (_, _, c))) => {
                        let ok: &[&str] = match (on_cells, split) {
                            (false, true) => &["mean"],
                            (false, false) => &[],
                            (true, true) => &["copy", "share"],
                            (true, false) => &["mean", "sum", "copy"],
                        };
                        if !ok.contains(&n) {
                            let what = if split { "split" } else { "merge" };
                            let owner = if on_cells { "a cell" } else { "a new point" };
                            return Err(bad(
                                r,
                                *c,
                                match ok {
                                    [] => format!("merge leaves the points' values as they are: {col} is not given"),
                                    _ => format!("on {what}, {owner}'s {col} is {} or a value, not {n}", ok.join(", ")),
                                },
                            ));
                        }
                        match n {
                            "copy" => CellTransfer::Copy,
                            "share" => CellTransfer::Share,
                            "sum" => CellTransfer::Sum,
                            _ => CellTransfer::Mean,
                        }
                    }
                    Some((None, (_, ast, c))) => {
                        if !on_cells && !split {
                            return Err(bad(
                                r,
                                *c,
                                format!("merge leaves the points' values as they are: {col} is not given"),
                            ));
                        }
                        self.within(r, ast, *c, &[e], &format!("{col}'s value"))?;
                        values.push(((*col).clone(), ast.clone(), r.line, *c));
                        CellTransfer::Given(values.len() - 1)
                    }
                };
                transfers.push((on_cells, k, kind));
            }
        }
        Ok(CheckedRule {
            table: m,
            source: Some(e),
            rule: r,
            values,
            transfers,
        })
    }

    /// `split` and `merge` on the table `e`, not cells: they act on an
    /// edge table's edges, making and filling rows of the table it
    /// joins (ADR 0079 stage 2).
    fn on_edges(&self, r: &'a RuleLine, e: usize) -> Result<CheckedRule<'a>> {
        let grid = &self.d.grid;
        if grid[e].sides_of.is_some() {
            return Err(bad(
                r,
                1,
                format!(
                    "{} are kept by SimSandbox from their cells: split or merge the cells instead",
                    r.table
                ),
            ));
        }
        let Some(m) = grid[e].ends_of else {
            return Err(bad(
                r,
                1,
                format!(
                    "'{}' is not an edge table: split and merge act on edges ('edges {} of pts')",
                    r.table, r.table
                ),
            ));
        };
        self.within(r, &r.cond, r.cond_column, &[e], "the condition")?;
        let given = Self::given(r);
        let columns = self.columns(m);
        if let Some((n, _, c)) = given.iter().find(|g| !columns.contains(&&g.0)) {
            return Err(bad(
                r,
                *c,
                format!("'{n}' is not a state over {}", self.axis_name(m)),
            ));
        }
        // Transfers: `copy` (the edge's `from` row's value) or
        // `mean` (of its two rows'); copy on split, mean on merge
        // unless given (ADR 0079 point 4).
        let end = |f: &str, col: &str| {
            Ast::Apply(
                f.into(),
                vec![Ast::Name(col.into(), 1), Ast::Name(r.table.clone(), 1)],
                1,
            )
        };
        let mut values = Vec::new();
        for col in columns {
            let (ast, c) = match given.iter().find(|g| &g.0 == col) {
                Some((_, ast, c)) => (ast.clone(), *c),
                None if r.kind == RuleKind::Split => (transfer_value("copy", 1), 1),
                None => (transfer_value("mean", 1), 1),
            };
            if let Some(n) = transfer(&ast) {
                if n == "share" || n == "sum" {
                    return Err(bad(
                        r,
                        c,
                        format!(
                            "{n} is for cells' values; an edge's rows take mean, copy or a value"
                        ),
                    ));
                }
            }
            let ast = match transfer(&ast) {
                Some("copy") => end("from_end", col),
                // `mean`: `share` and `sum` are refused above.
                Some(_) => Ast::Bin(
                    Op::Div,
                    Box::new(Ast::Bin(
                        Op::Add,
                        Box::new(end("from_end", col)),
                        Box::new(end("to_end", col)),
                    )),
                    Box::new(Ast::Num(2.0)),
                ),
                None => ast,
            };
            self.within(r, &ast, c, &[e], &format!("{col}'s value"))?;
            values.push((col.clone(), ast, r.line, c));
        }
        Ok(CheckedRule {
            table: m,
            source: Some(e),
            rule: r,
            values,
            transfers: Vec::new(),
        })
    }

    /// `remove pts where c` and `add pts [from k] [where c]: …` on the
    /// table `t`.
    fn on_rows(&self, r: &'a RuleLine, t: usize) -> Result<CheckedRule<'a>> {
        let grid = &self.d.grid;
        if r.kind == RuleKind::Add && grid[t].ends_of.is_some() {
            return Err(bad(
                r,
                1,
                format!(
                    "edges and cells are made by their start ('= chain', '= grid …') and by split, not added: {}",
                    r.table
                ),
            ));
        }
        if grid[t].sides_of.is_some() {
            return Err(bad(
                r,
                1,
                format!(
                    "{} are kept by SimSandbox from their cells: change the cells instead",
                    r.table
                ),
            ));
        }
        let Some((from, given)) = &r.add else {
            self.within(r, &r.cond, r.cond_column, &[t], "the condition")?;
            return Ok(CheckedRule {
                table: t,
                source: None,
                rule: r,
                values: Vec::new(),
                transfers: Vec::new(),
            });
        };
        let src = match from {
            Some(f) => Some(grid.iter().position(|a| &a.name == f).ok_or_else(|| {
                bad(
                    r,
                    1,
                    format!(
                        "'{f}' is not an axis or table: new rows come from one, 'add {} from k'",
                        r.table
                    ),
                )
            })?),
            None => None,
        };
        let allowed: Vec<usize> = src.into_iter().collect();
        self.within(r, &r.cond, r.cond_column, &allowed, "the condition")?;
        let columns = self.columns(t);
        if let Some((n, _, c)) = given.iter().find(|g| !columns.contains(&&g.0)) {
            return Err(bad(
                r,
                *c,
                format!(
                    "'{n}' is not a state over {}: its rows hold {}",
                    r.table,
                    columns
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }
        let mut values = Vec::new();
        for col in columns {
            let (ast, c) = match given.iter().find(|g| &g.0 == col) {
                Some((_, ast, c)) => (ast.clone(), *c),
                None => match self.inits.iter().find(|i| &i.name == col) {
                    // Its starting value, if that is one value.
                    Some(i) if self.arrays.of_ast(&i.ast).is_ok_and(|a| a.is_empty()) => {
                        (i.ast.clone(), i.column)
                    }
                    Some(_) => {
                        return Err(bad(
                            r,
                            1,
                            format!(
                                "'add {}' must give {col}: its starting value depends on the rows",
                                r.table
                            ),
                        ))
                    }
                    None => (Ast::Num(0.0), 1),
                },
            };
            self.within(r, &ast, c, &allowed, &format!("{col}'s value"))?;
            values.push((col.clone(), ast, r.line, c));
        }
        Ok(CheckedRule {
            table: t,
            source: src,
            rule: r,
            values,
            transfers: Vec::new(),
        })
    }
}

/// Check each rule: its table, the axis new rows come from, and each
/// column's value (its starting value where not given). With them, the
/// tables of cells they refine.
pub(super) fn check_rules<'a>(
    d: &'a Declared,
    arrays: &Arrays<'a>,
    inits: &[Init],
) -> Result<(Vec<CheckedRule<'a>>, std::collections::HashSet<usize>)> {
    let checker = Checker { d, arrays, inits };
    let mut rules = Vec::new();
    let mut refined = std::collections::HashSet::new();
    for r in &d.rules {
        let Some(t) = d
            .grid
            .iter()
            .position(|a| a.name == r.table)
            .filter(|&i| d.grid[i].table.is_some())
        else {
            return Err(bad(
                r,
                1,
                format!(
                    "'{}' is not a table: declare it with 'table {}'",
                    r.table, r.table
                ),
            ));
        };
        let a = &d.grid[t];
        let rule = match (r.kind, a.sides_of, a.shape, a.ends_of) {
            (RuleKind::Split | RuleKind::Merge, None, Some(shape), Some(m))
                if shape != crate::mesh::CellShape::Segment =>
            {
                refined.insert(t);
                checker.refine(r, t, m, shape)?
            }
            (RuleKind::Split | RuleKind::Merge, ..) => checker.on_edges(r, t)?,
            _ => checker.on_rows(r, t)?,
        };
        rules.push(rule);
    }
    Ok((rules, refined))
}
