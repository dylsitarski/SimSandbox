//! Building an [`EquationSet`] from the lines of a definition.

use super::declare::{declare, Declared};
use super::lower::lower;
use super::shapes::{ast_name, axes_of, cells_of, fit, line_of, needed, number, union};
use super::starts::{MeshSpec, Starts};
use super::statements::{
    definition_error, read_unit, row_params, statements, CheckedRule, EdgeKind, Lhs, RuleLine,
};
use super::{
    element_name, CellTransfer, Copies, EquationSet, Kind, Layout, RowsSource, RuleKind, RuleRun,
    TableRows,
};
use simsandbox_core::block::{BlockSpec, Domain, ParamSpec, PortSpec, Shape, TimeSemantics};
use simsandbox_core::units::Unit;
use simsandbox_core::{Result, SimError};
use simsandbox_expr::program::Span;
use simsandbox_expr::units::{
    dim_div, dim_pow, dim_text, dimension_with, from_si, in_si, Dim, DIM_TOO_HIGH, NO_DIM,
};
use simsandbox_expr::{parse, ArrayOp, Ast, Edge, Expr, Line as GridLine, Op, Program};
use std::cell::RefCell;
impl EquationSet {
    /// Build from the lines of a definition, as a Block named `name`.
    pub fn parse(name: &str, lines: &[String]) -> Result<Self> {
        Self::parse_with_rows(name, lines, &std::collections::BTreeMap::new())
    }

    /// As [`EquationSet::parse`], with each table (ADR 0079) at the given
    /// number of rows instead of its starting number: tables change only
    /// between steps, and the block compiles its equations again then.
    pub fn parse_with_rows(
        name: &str,
        lines: &[String],
        rows: &std::collections::BTreeMap<String, usize>,
    ) -> Result<Self> {
        let written = lines.len();
        let (lines, from) = row_params(lines);
        Self::parse_lines(name, &lines, rows).map_err(|e| {
            // A mistake on a line holding a parameter over rows is one of
            // its `param` line: the line written.
            let SimError::InvalidDefinition(m) = &e else {
                return e;
            };
            let at = m
                .strip_prefix("line ")
                .and_then(|r| r.split_once(", column "))
                .and_then(|(l, r)| Some((l.parse::<usize>().ok()?, r.split_once(": ")?.1)));
            match at {
                Some((l, what)) if l > written => definition_error(from[l - written - 1], 1, what),
                _ => e,
            }
        })
    }

    /// The lines as [`row_params`] gives them, compiled.
    fn parse_lines(
        name: &str,
        lines: &[String],
        rows: &std::collections::BTreeMap<String, usize>,
    ) -> Result<Self> {
        let statements = statements(lines);
        let Declared {
            grid,
            start_rows,
            starts:
                Starts {
                    rows: starts,
                    chains,
                    csv: csv_starts,
                    specs: grid_starts,
                    cells: cell_decls,
                    meshes,
                },
            mut units,
            kind,
            equations: parsed,
            places,
            states,
            defined,
            inputs,
            params: declared,
            state_decls,
            arrays: mut shapes,
            array_params,
            edges,
            rules: rule_lines,
        } = declare(&statements, rows)?;
        let _ = (&chains, &starts);
        let axes: Vec<&str> = grid.iter().map(|a| a.name.as_str()).collect();
        let is_table = |i: usize| grid[i].table.is_some();
        // Each defined quantity's axes (ADR 0080): those its right side
        // uses, found in any order of the lines.
        for _ in 0..=parsed.len() {
            let mut changed = false;
            for line in &parsed {
                if let Lhs::Defined(n) = &line.lhs {
                    if let Ok(a) = axes_of(&line.ast, &grid, &shapes) {
                        if shapes.get(n).map_or(!a.is_empty(), |b| *b != a) {
                            shapes.insert(n.clone(), a);
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        let axes_of_name = |n: &str| shapes.get(n).cloned().unwrap_or_default();
        let list = |a: &[usize]| a.iter().map(|&i| axes[i]).collect::<Vec<_>>().join(", ");
        for line in &parsed {
            let got = axes_of(&line.ast, &grid, &shapes)
                .map_err(|(c, m)| definition_error(line.number, line.offset + c, m))?;
            let (n, what) = match &line.lhs {
                Lhs::State(n) => (n, format!("d{n}/dt")),
                Lhs::Map(n) => (n, format!("{n}[n+1]")),
                Lhs::Defined(_) => continue,
            };
            let own = axes_of_name(n);
            if got.iter().any(|a| !own.contains(a)) {
                let all = union(&own, &got);
                let how = if own.is_empty() {
                    format!("{what} has a value per cell, so {n} must be an array: declare 'state {n}[{}]'", list(&all))
                } else {
                    format!(
                        "{what} is over {}, but {n} is over {}: declare 'state {n}[{}]'",
                        list(&got),
                        list(&own),
                        list(&all)
                    )
                };
                return Err(definition_error(line.number, line.offset + 1, how));
            }
        }
        // Tables (ADR 0079 stage 1a): in maps; a quantity over a table's
        // rows has no other axis; values per row are states or defined
        // quantities (no parameters, inputs or edges over rows yet).
        let line_of_name = |n: &str| -> usize {
            state_decls
                .iter()
                .find(|d| d.name == n)
                .map(|d| d.line)
                .or_else(|| {
                    parsed.iter().find_map(|l| match &l.lhs {
                        Lhs::Defined(d) | Lhs::State(d) | Lhs::Map(d) if d == n => Some(l.number),
                        _ => None,
                    })
                })
                .or_else(|| {
                    declared
                        .iter()
                        .find(|d| d.0 == format!("{n}[0]"))
                        .map(|d| d.2)
                })
                .unwrap_or(1)
        };
        // A table some input is over follows that input's rows (ADR 0079
        // stage 1b): its rows come from upstream, by id, not from rules.
        // A table keeps its own rows when it has a start, rules (its own,
        // or split and merge on its edges) or edges; an input over such a
        // table is matched to its rows by id. A table with none of these
        // that some input is over follows that input's rows (ADR 0079
        // stages 1b and 2).
        let owns = |a: usize| {
            let name = &grid[a].name;
            starts.contains_key(name)
                || rule_lines.iter().any(|r| &r.table == name)
                || grid.iter().any(|g| g.ends_of == Some(a))
        };
        let followed: Vec<bool> = (0..grid.len())
            .map(|a| {
                is_table(a)
                    && grid[a].ends_of.is_none()
                    && !owns(a)
                    && inputs
                        .iter()
                        .any(|i| shapes.get(i).is_some_and(|d| d.contains(&a)))
            })
            .collect();
        if let Some((_, t)) = grid.iter().enumerate().find(|(a, t)| {
            // A table that never changes (no rules, and no states,
            // which make a map) may be read by formulas (ADR 0085).
            t.table.is_some()
                && !(kind == Kind::Formulas && (followed[*a] || rule_lines.is_empty()))
        }) {
            if kind == Kind::Formulas {
                return Err(definition_error(
                    t.line,
                    1,
                    format!(
                        "the table {} needs states (x[n+1] = … or dx/dt = …): tables change between steps",
                        t.name
                    ),
                ));
            }
        }
        for (n, dims) in &shapes {
            if axes.contains(&n.as_str()) || !dims.iter().any(|&a| is_table(a)) {
                continue;
            }
            let why = if dims.len() > 1 {
                format!("{n} is over a table's rows and other axes: a quantity over rows has only them (for now)")
            } else if array_params.iter().any(|a| &a.name == n) {
                format!("a parameter has a value per cell, not per row: a value per row is a state, 'state {n}[{}]'", axes[dims[0]])
            } else {
                continue;
            };
            return Err(definition_error(line_of_name(n), 1, why));
        }
        // Array states' initial values, edge values and the grid's ends.
        let mut inits: Vec<(String, Ast, usize, usize)> = Vec::new();
        let mut init_states: Vec<usize> = Vec::new();
        for d in state_decls.iter().filter(|d| d.array) {
            let n = &d.name;
            if let Some((src, number, col)) = &d.init {
                let ast = parse(src)
                    .map_err(|e| definition_error(*number, col + e.column - 1, e.message))?;
                inits.push((n.clone(), ast, *number, *col));
                init_states.push(d.state);
            }
        }
        for e in &edges {
            let own = axes_of_name(&e.array);
            if is_table(e.axis) {
                return Err(definition_error(
                    e.line,
                    e.column,
                    format!("{} is a table: its rows have no ends", axes[e.axis]),
                ));
            }
            if own.is_empty() || axes.contains(&e.array.as_str()) {
                return Err(definition_error(
                    e.line,
                    1,
                    format!(
                        "'{}' is not an array on the grid, so it has no edges",
                        e.array
                    ),
                ));
            }
            if !own.contains(&e.axis) {
                return Err(definition_error(
                    e.line,
                    e.column,
                    format!(
                        "{} is over {}, so it has no edges along {}",
                        e.array,
                        list(&own),
                        axes[e.axis]
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
                || array_params.iter().any(|a| a.name == n)
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
        // Then the rules' conditions and values (a transfer's name, `copy`
        // or `mean`, is not a quantity).
        for r in &rule_lines {
            used.push(&r.cond);
            let transfers = matches!(r.kind, RuleKind::Split | RuleKind::Merge);
            for (_, ast, _) in r.add.iter().flat_map(|a| &a.1) {
                let transfer = transfers
                    && matches!(ast, Ast::Name(n, _) if matches!(n.as_str(), "mean" | "copy" | "share" | "sum"));
                if !transfer {
                    used.push(ast);
                }
            }
        }
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
        for (n, ast, line, col) in &inits {
            let got = axes_of(ast, &grid, &shapes)
                .map_err(|(c, m)| definition_error(*line, col + c - 1, m))?;
            let own = axes_of_name(n);
            if got.iter().any(|a| !own.contains(a)) {
                return Err(definition_error(
                    *line,
                    *col,
                    format!(
                        "{n}'s initial value is over {}, but {n} is over {}",
                        list(&got),
                        list(&own)
                    ),
                ));
            }
            only(
                ast,
                *line,
                *col,
                // An array parameter too (`state x[p] = x_start`, ADR 0069).
                // And a starting mesh's coordinates (ADR 0085 step 3).
                &|n| {
                    is_param(n)
                        || axes.contains(&n)
                        || array_params.iter().any(|a| a.name == n)
                        || grid_starts.values().any(|(m, _)| m.names().iter().any(|c| c == n))
                },
                "an initial value is written with parameters, the axes and a starting mesh's coordinates",
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
                            || inputs.iter().any(|i| i == n) && axes_of_name(n).is_empty()
                    },
                    "an edge value is written with parameters, scalar inputs and t",
                )?;
            }
        }
        if kind == Kind::Map {
            let uses_n = |a: &Ast| a.names().iter().any(|(n, _)| n == "n");
            let in_rule = |r: &&RuleLine| {
                uses_n(&r.cond) || r.add.iter().flat_map(|a| &a.1).any(|g| uses_n(&g.1))
            };
            let line = parsed
                .iter()
                .find(|l| uses_n(&l.ast))
                .map(|l| l.number)
                .or_else(|| rule_lines.iter().find(in_rule).map(|r| r.line));
            if let Some(line) = line {
                return Err(definition_error(
                    line,
                    1,
                    "'n' is the step number in maps and cannot be used in equations yet",
                ));
            }
        }
        // States over tables' rows after the others: the block keeps them
        // in its tables, the rest in its fixed state (ADR 0079).
        let row_table = |n: &str| -> Option<usize> {
            shapes
                .get(n)
                .and_then(|d| d.first().copied())
                .filter(|&a| is_table(a))
        };
        // A starting grid gives its points' coordinates: states over them
        // (ADR 0085).
        for (c, (g, line)) in &grid_starts {
            let pts = cell_decls.get(c).map(|o| o.0.as_str()).unwrap_or("");
            let p_axis = axes.iter().position(|a| *a == pts);
            for name in &g.names() {
                let over =
                    |s: &&String| *s == name && row_table(s).is_some() && row_table(s) == p_axis;
                if !states.iter().any(|s| over(&s)) {
                    return Err(definition_error(
                        *line,
                        1,
                        format!(
                            "the starting mesh sets {name} at each point: declare 'state {name}[{pts}]'"
                        ),
                    ));
                }
            }
        }
        // Rules, checked: the table, the axis new rows come from, and each
        // column's value (its starting value where not given).
        let mut rules: Vec<CheckedRule> = Vec::new();
        let mut refined: std::collections::HashSet<usize> = Default::default();
        for r in &rule_lines {
            let bad = |c: usize, m: String| definition_error(r.line, c, m);
            let Some(t) = axes
                .iter()
                .position(|a| *a == r.table)
                .filter(|&i| is_table(i))
            else {
                return Err(bad(
                    1,
                    format!(
                        "'{}' is not a table: declare it with 'table {}'",
                        r.table, r.table
                    ),
                ));
            };
            let within = |ast: &Ast, col: usize, allowed: &[usize], what: &str| -> Result<()> {
                let got = axes_of(ast, &grid, &shapes).map_err(|(c, m)| bad(col + c - 1, m))?;
                if got.iter().any(|a| !allowed.contains(a)) {
                    let names: Vec<&str> = got.iter().map(|&a| axes[a]).collect();
                    return Err(bad(
                        col,
                        format!(
                            "{what} is over {}, not the rows it adds or removes",
                            names.join(", ")
                        ),
                    ));
                }
                Ok(())
            };
            // Cells are refined by their shape's rule (ADR 0085 step 2).
            if matches!(r.kind, RuleKind::Split | RuleKind::Merge)
                && grid[t].sides_of.is_none()
                && grid[t]
                    .shape
                    .is_some_and(|s| s != crate::mesh::CellShape::Segment)
            {
                let e = t;
                let shape = grid[e].shape.expect("cells");
                if !shape.simplicial() && crate::refine::box_codes(shape).is_none() {
                    return Err(bad(
                        1,
                        format!(
                            "{} cells are not refined yet: simplices and boxes are",
                            shape.name()
                        ),
                    ));
                }
                if shape.simplicial()
                    && shape.dim() >= 3
                    && matches!(grid_starts.get(&r.table), Some((MeshSpec::File { .. }, _)))
                {
                    return Err(bad(
                        1,
                        format!(
                            "{} read from a file are not refined yet (their bisection needs a labelling): start them from a grid",
                            shape.name()
                        ),
                    ));
                }
                let m = grid[e].ends_of.expect("cells have points");
                within(&r.cond, r.cond_column, &[e], "the condition")?;
                let given = r.add.as_ref().map(|a| a.1.clone()).unwrap_or_default();
                let point_cols: Vec<&String> =
                    states.iter().filter(|s| row_table(s) == Some(m)).collect();
                let cell_cols: Vec<&String> =
                    states.iter().filter(|s| row_table(s) == Some(e)).collect();
                if let Some((n, _, c)) = given
                    .iter()
                    .find(|g| !point_cols.contains(&&g.0) && !cell_cols.contains(&&g.0))
                {
                    return Err(bad(
                        *c,
                        format!("'{n}' is not a state over {} or {}", axes[m], axes[e]),
                    ));
                }
                let split = r.kind == RuleKind::Split;
                let mut values = Vec::new();
                let mut transfers = Vec::new();
                for (on_cells, cols) in [(false, &point_cols), (true, &cell_cols)] {
                    for (k, col) in cols.iter().enumerate() {
                        let g = given.iter().find(|g| &g.0 == *col);
                        let kind = match g {
                            None if on_cells && split => CellTransfer::Copy,
                            None => CellTransfer::Mean,
                            Some((_, Ast::Name(n, _), c))
                                if matches!(n.as_str(), "copy" | "share" | "mean" | "sum") =>
                            {
                                let ok: &[&str] = match (on_cells, split) {
                                    (false, true) => &["mean"],
                                    (false, false) => &[],
                                    (true, true) => &["copy", "share"],
                                    (true, false) => &["mean", "sum", "copy"],
                                };
                                if !ok.contains(&n.as_str()) {
                                    let what = if split { "split" } else { "merge" };
                                    let owner = if on_cells { "a cell" } else { "a new point" };
                                    return Err(bad(
                                        *c,
                                        match ok {
                                            [] => format!("merge leaves the points' values as they are: {col} is not given"),
                                            _ => format!("on {what}, {owner}'s {col} is {} or a value, not {n}", ok.join(", ")),
                                        },
                                    ));
                                }
                                match n.as_str() {
                                    "copy" => CellTransfer::Copy,
                                    "share" => CellTransfer::Share,
                                    "sum" => CellTransfer::Sum,
                                    _ => CellTransfer::Mean,
                                }
                            }
                            Some((_, ast, c)) => {
                                if !on_cells && !split {
                                    return Err(bad(*c, format!("merge leaves the points' values as they are: {col} is not given")));
                                }
                                within(ast, *c, &[e], &format!("{col}'s value"))?;
                                values.push(((*col).clone(), ast.clone(), r.line, *c));
                                CellTransfer::Given(values.len() - 1)
                            }
                        };
                        transfers.push((on_cells, k, kind));
                    }
                }
                refined.insert(e);
                rules.push((m, Some(e), r, values, transfers));
                continue;
            }
            // Split and merge act on an edge table's edges, making and
            // filling rows of the table it joins (ADR 0079 stage 2).
            if matches!(r.kind, RuleKind::Split | RuleKind::Merge) {
                let e = t;
                if grid[e].sides_of.is_some() {
                    return Err(bad(
                        1,
                        format!(
                            "{} are kept by SimSandbox from their cells: split or merge the cells instead",
                            r.table
                        ),
                    ));
                }
                let Some(m) = grid[e].ends_of else {
                    return Err(bad(
                        1,
                        format!(
                            "'{}' is not an edge table: split and merge act on edges ('edges {} of pts')",
                            r.table, r.table
                        ),
                    ));
                };
                within(&r.cond, r.cond_column, &[e], "the condition")?;
                let given = r.add.as_ref().map(|a| a.1.clone()).unwrap_or_default();
                let columns: Vec<&String> =
                    states.iter().filter(|s| row_table(s) == Some(m)).collect();
                if let Some((n, _, c)) = given.iter().find(|g| !columns.contains(&&g.0)) {
                    return Err(bad(*c, format!("'{n}' is not a state over {}", axes[m])));
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
                        None if r.kind == RuleKind::Split => (Ast::Name("copy".into(), 1), 1),
                        None => (Ast::Name("mean".into(), 1), 1),
                    };
                    if let Ast::Name(n, _) = &ast {
                        if n == "share" || n == "sum" {
                            return Err(bad(c, format!("{n} is for cells' values; an edge's rows take mean, copy or a value")));
                        }
                    }
                    let ast = match &ast {
                        Ast::Name(n, _) if n == "copy" => end("from_end", col),
                        Ast::Name(n, _) if n == "mean" => Ast::Bin(
                            Op::Div,
                            Box::new(Ast::Bin(
                                Op::Add,
                                Box::new(end("from_end", col)),
                                Box::new(end("to_end", col)),
                            )),
                            Box::new(Ast::Num(2.0)),
                        ),
                        _ => ast,
                    };
                    within(&ast, c, &[e], &format!("{col}'s value"))?;
                    values.push((col.clone(), ast, r.line, c));
                }
                rules.push((m, Some(e), r, values, Vec::new()));
                continue;
            }
            if r.kind == RuleKind::Add && grid[t].ends_of.is_some() {
                return Err(bad(
                    1,
                    format!(
                        "edges and cells are made by their start ('= chain', '= grid …') and by split, not added: {}",
                        r.table
                    ),
                ));
            }
            if grid[t].sides_of.is_some() {
                return Err(bad(
                    1,
                    format!(
                        "{} are kept by SimSandbox from their cells: change the cells instead",
                        r.table
                    ),
                ));
            }
            match &r.add {
                None => {
                    within(&r.cond, r.cond_column, &[t], "the condition")?;
                    rules.push((t, None, r, Vec::new(), Vec::new()));
                }
                Some((from, given)) => {
                    let src = match from {
                        Some(f) => Some(axes.iter().position(|a| a == f).ok_or_else(|| {
                            bad(1, format!("'{f}' is not an axis or table: new rows come from one, 'add {} from k'", r.table))
                        })?),
                        None => None,
                    };
                    let allowed: Vec<usize> = src.into_iter().collect();
                    within(&r.cond, r.cond_column, &allowed, "the condition")?;
                    let columns: Vec<&String> =
                        states.iter().filter(|s| row_table(s) == Some(t)).collect();
                    if let Some((n, _, c)) = given.iter().find(|g| !columns.contains(&&g.0)) {
                        return Err(bad(
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
                            None => match inits.iter().find(|i| &i.0 == col) {
                                // Its starting value, if that is one value.
                                Some(i) if axes_of(&i.1, &grid, &shapes).is_ok_and(|a| a.is_empty()) => {
                                    (i.1.clone(), i.3)
                                }
                                Some(_) => {
                                    return Err(bad(1, format!("'add {}' must give {col}: its starting value depends on the rows", r.table)))
                                }
                                None => (Ast::Num(0.0), 1),
                            },
                        };
                        within(&ast, c, &allowed, &format!("{col}'s value"))?;
                        values.push((col.clone(), ast, r.line, c));
                    }
                    rules.push((t, src, r, values, Vec::new()));
                }
            }
        }
        // Scalar states start from a parameter `x0`; arrays from their
        // initial expression (0 if none).
        let mut initial: Vec<Option<usize>> = Vec::new();
        for s in &states {
            if !axes_of_name(s).is_empty() {
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
        // A mesh's coordinates are in one unit.
        let one_unit = |name: &str,
                        coords: &[(&Ast, Dim)],
                        col: usize|
         -> std::result::Result<Dim, (usize, String)> {
            let first = coords.first().map_or(NO_DIM, |a| a.1);
            match coords.iter().find(|a| a.1 != first) {
                Some((a, d)) => Err((
                    a.names().first().map_or(col, |n| n.1),
                    format!(
                        "'{name}' takes coordinates in one unit, not {} and {}",
                        dim_text(first),
                        dim_text(*d)
                    ),
                )),
                None => Ok(first),
            }
        };
        // Array operations' units: a stencil divides by the axis unit.
        let apply_dim = |name: &str,
                         args: &[(&Ast, Dim)],
                         col: usize|
         -> std::result::Result<Dim, (usize, String)> {
            let d = args.first().map_or(NO_DIM, |a| a.1);
            Ok(match name {
                "lap" => {
                    // Along the array's own axes (ADR 0080).
                    let own = match args[0].0 {
                        Ast::Name(n, _) => axes_of_name(n),
                        _ => Vec::new(),
                    };
                    let Some(&first) = own.first() else {
                        return Ok(d);
                    };
                    if own.iter().any(|&a| axis_dims[a] != axis_dims[first]) {
                        let names: Vec<&str> = own.iter().map(|&a| axes[a]).collect();
                        return Err((
                            col,
                            format!(
                                "lap adds second derivatives along {}, so the axes need the same unit",
                                names.join(" and ")
                            ),
                        ));
                    }
                    dim_pow(axis_dims[first], 2)
                        .and_then(|h2| dim_div(d, h2))
                        .ok_or_else(|| (col, DIM_TOO_HIGH.to_string()))?
                }
                "spacing" => match args[0].0 {
                    Ast::Name(n, _) => axis_dim_of(n),
                    _ => NO_DIM,
                },
                // A pure number.
                "rcond" => NO_DIM,
                "solve" => {
                    // A x = b: x in b's unit over A's.
                    let b = args.get(1).map_or(NO_DIM, |a| a.1);
                    dim_div(b, d).ok_or_else(|| (col, DIM_TOO_HIGH.to_string()))?
                }
                n if n.starts_with("dd") => {
                    let axis = if n == "dd" {
                        args.get(1).map_or("", |a| ast_name(a.0))
                    } else {
                        &n[2..]
                    };
                    let a = axis_dim_of(axis);
                    dim_div(d, a).ok_or_else(|| (col, DIM_TOO_HIGH.to_string()))?
                }
                "at" => {
                    if args[1..].iter().any(|a| a.1 != NO_DIM) {
                        return Err((col, "a cell number has no unit".into()));
                    }
                    d
                }
                // A cell number (ADR 0070).
                "argminof" | "argmaxof" => NO_DIM,
                "of" => {
                    if args.get(1).is_some_and(|a| a.1 != NO_DIM) {
                        return Err((col, "a row's id has no unit".into()));
                    }
                    d
                }
                // A length, area or volume: the coordinates' unit to the
                // power of the cells' dimension (a point counts 1).
                "measure" => {
                    let power = match args[0].0 {
                        Ast::Name(n, _) => grid.iter().find(|g| &g.name == n),
                        _ => None,
                    }
                    .and_then(|g| g.shape)
                    .map_or(0, |s| s.dim());
                    let x = one_unit(name, args.get(1..).unwrap_or_default(), col)?;
                    dim_pow(x, power as i8).ok_or_else(|| (col, DIM_TOO_HIGH.to_string()))?
                }
                // A direction.
                "normal" => {
                    one_unit(name, args.get(2..).unwrap_or_default(), col)?;
                    NO_DIM
                }
                // The first argument's: `centre(T, c)` and a value read
                // elsewhere (`from_end(T, e)`, `inner(u, f)`); those after
                // it name tables and axes or are plain numbers.
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
                    // Zero is zero in every unit: `dx/dt = 0` holds x.
                    if matches!(ast, Ast::Num(v) if *v == 0.0) {
                        return Ok(());
                    }
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
                            dim_div(x, Unit::SECOND.dim.0).ok_or_else(|| {
                                definition_error(line.number, 1, DIM_TOO_HIGH.to_string())
                            })?,
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
                        dim_div(d, axis_dims[e.axis]).ok_or_else(|| {
                            definition_error(e.line, e.column, DIM_TOO_HIGH.to_string())
                        })?,
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
        let len_of = |n: &str| cells_of(&axes_of_name(n), &grid);
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
        // Each table's row ids, given with the inputs (ADR 0079).
        let ids_vars: Vec<Option<usize>> = grid
            .iter()
            .map(|a| a.table.map(|_| add(a.cells, 1.0)))
            .collect();
        // Each edge table's ends, as places among its table's rows (ADR
        // 0079 stage 2), given after the ids.
        // Places per corner, then for sides the inner and outer cells'
        // places, the boundary flags and the cell across (ADR 0085).
        let ends_vars: Vec<Option<Vec<usize>>> = grid
            .iter()
            .map(|a| {
                let n = a.shape?.corners() + if a.sides_of.is_some() { 4 } else { 0 };
                Some((0..n).map(|_| add(a.cells, 1.0)).collect())
            })
            .collect();
        let param_vars: Vec<usize> = params.iter().map(|n| add(1, unit_of(n).scale)).collect();
        let axis_scales: Vec<f64> = grid.iter().map(|a| unit_of(&a.name).scale).collect();
        // The outputs: defined quantities, then each axis's position of
        // every cell; those over a table's rows after them, not outputs.
        let mut defined_vars = vec![0; defined.len()];
        let mut axis_vars = vec![0; grid.len()];
        let mut out_vars = Vec::new();
        for rows in [false, true] {
            for (k, n) in defined.iter().enumerate() {
                if row_table(n).is_some() == rows {
                    defined_vars[k] = add(len_of(n), unit_of(n).scale);
                    if !rows {
                        out_vars.push(defined_vars[k]);
                    }
                }
            }
            for (i, a) in grid.iter().enumerate() {
                if a.table.is_some() == rows {
                    axis_vars[i] = add(a.cells, axis_scales[i]);
                    if !rows {
                        out_vars.push(axis_vars[i]);
                    }
                }
            }
        }
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
        // Each rule's condition and the values of the rows it adds, over
        // the rows it removes or those it adds from (ADR 0079).
        let rule_vars: Vec<(usize, Vec<usize>)> = rules
            .iter()
            .map(|(t, src, r, values, _)| {
                let n = match (&r.add, src) {
                    (None, _) => grid[*t].cells,
                    (Some(_), Some(s)) => grid[*s].cells,
                    (Some(_), None) => 1,
                };
                let cond = add(n, 1.0);
                let vals = values
                    .iter()
                    .map(|(c, ..)| add(n, unit_of(c).scale))
                    .collect();
                (cond, vals)
            })
            .collect();
        let rate_vars: Vec<usize> = states
            .iter()
            .map(|n| add(len_of(n), unit_of(n).scale))
            .collect();
        // An array parameter reads its elements' registers, one after
        // another (ADR 0069).
        let array_param_vars: Vec<usize> = array_params
            .iter()
            .map(|a| {
                spans.push(Span {
                    at: spans[param_vars[a.first]].at,
                    len: a.cells,
                });
                scales.push(unit_of(&a.name).scale);
                spans.len() - 1
            })
            .collect();
        let layout = Layout {
            states: state_vars.iter().map(|&v| spans[v].len).sum(),
            fixed: states
                .iter()
                .zip(&state_vars)
                .filter(|(n, _)| row_table(n).is_none())
                .map(|(_, &v)| spans[v].len)
                .sum(),
            inputs: input_vars
                .iter()
                .chain(ids_vars.iter().flatten())
                .copied()
                .chain(ends_vars.iter().flatten().flatten().copied())
                .map(|v| spans[v].len)
                .sum(),
            params: params.len(),
            outputs_at: out_vars.first().map_or(0, |&v| spans[v].at),
            outputs_len: out_vars.iter().map(|&v| spans[v].len).sum::<usize>(),
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
            if let Some(i) = array_params.iter().position(|a| a.name == n) {
                return Some(array_param_vars[i]);
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
        // The stencils, along an array's own axes (ADR 0080).
        let stencil = |name: &str, args: &[Ast]| -> Expr {
            let first_name = ast_name(&args[0]);
            let axis_named = |n: &str| axes.iter().position(|a| *a == n).expect("checked: an axis");
            if name == "spacing" {
                return Expr::Var(spacing_of(axis_named(first_name)));
            }
            let own = axes_of_name(first_name);
            let a = slot_of(first_name, defined.len()).expect("checked: an array");
            // A derivative of an array along one axis.
            let along = |axis: usize, second: bool| {
                let (line, edges) = (line_of(&own, axis, &grid), edges_of(first_name, axis));
                let op = if second {
                    ArrayOp::Second { line, edges }
                } else {
                    ArrayOp::First { line, edges }
                };
                Expr::Array(op, vec![Expr::Var(a), Expr::Var(spacing_of(axis))])
            };
            if name == "lap" {
                // The sum of the second derivatives along each axis.
                return own[1..].iter().fold(along(own[0], true), |sum, &i| {
                    Expr::Bin(Op::Add, Box::new(sum), Box::new(along(i, true)))
                });
            }
            if name == "prev" || name == "next" {
                let axis = match args.get(1) {
                    Some(Ast::Name(n, _)) => axis_named(n),
                    _ => own[0],
                };
                let op = ArrayOp::Shift {
                    line: line_of(&own, axis, &grid),
                    edges: edges_of(first_name, axis),
                    up: name == "next",
                };
                return Expr::Array(op, vec![Expr::Var(a), Expr::Var(spacing_of(axis))]);
            }
            let axis = if name == "dd" {
                ast_name(&args[1])
            } else {
                &name[2..]
            };
            along(axis_named(axis), false)
        };
        // An expression with its axes; spread over `to` when given.
        let resolve = |ast: &Ast,
                       visible: usize,
                       to: Option<&[usize]>,
                       line: usize,
                       offset: usize|
         -> Result<Expr> {
            let (e, from) = lower(
                ast,
                &grid,
                &shapes,
                &|n| slot_of(n, visible),
                &stencil,
                &ids_vars,
                &ends_vars,
            )
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
            })?;
            Ok(match to {
                Some(to) => fit(e, &from, to, None, &grid),
                None => e,
            })
        };
        // In the unit of the variable it is stored in.
        let stored = |e: Expr, var: usize| from_si(in_si(e, &scales), scales[var]);
        let mut rhs: Vec<(usize, Expr)> = Vec::new();
        let mut defined_exprs = Vec::new();
        for (line, &i) in parsed.iter().zip(&places) {
            match &line.lhs {
                Lhs::State(n) | Lhs::Map(n) => {
                    let own = axes_of_name(n);
                    let e = resolve(
                        &line.ast,
                        defined.len(),
                        Some(&own),
                        line.number,
                        line.offset,
                    )?;
                    rhs.push((i, stored(e, state_vars[i])));
                }
                Lhs::Defined(_) => {
                    let k = defined_exprs.len();
                    let e = resolve(&line.ast, k, None, line.number, line.offset)?;
                    defined_exprs.push(stored(e, defined_vars[k]));
                }
            }
        }
        // The grid: its ends, spacing and cell centres.
        let mut grid_steps: Vec<(usize, Expr)> = Vec::new();
        for (a, &(x, lo, hi, h)) in grid.iter().zip(&grid_vars) {
            // A plain number is in the axis unit: `0 .. 4` in cm.
            for (end, var) in [(&a.lo, lo), (&a.hi, hi)] {
                let e = match number(end) {
                    Some(v) => Expr::Num(v),
                    None => stored(resolve(end, 0, None, a.line, 0)?, var),
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
                // Over the axis alone (ADR 0080).
                Expr::Array(
                    ArrayOp::Axis {
                        line: GridLine {
                            stride: 1,
                            count: a.cells,
                        },
                        cells: a.cells,
                    },
                    vec![Expr::Var(lo), Expr::Var(h)],
                ),
            ));
        }
        let mut edge_steps: Vec<(usize, Expr)> = Vec::new();
        for (e, v) in edges.iter().zip(&edge_vars) {
            if let (EdgeKind::Value(ast) | EdgeKind::Gradient(ast), Some(v)) = (&e.kind, v) {
                let x = match number(ast) {
                    Some(n) => Expr::Num(n),
                    None => stored(resolve(ast, 0, None, e.line, e.column - 1)?, *v),
                };
                edge_steps.push((*v, x));
            }
        }
        let mut init_steps = grid_steps.clone();
        for ((n, ast, line, col), &i) in inits.iter().zip(&init_states) {
            // A plain number is in the state's unit, as for `state x [m] = 0`.
            let x = match number(ast) {
                Some(v) => Expr::Num(v),
                None => stored(
                    resolve(ast, 0, Some(&axes_of_name(n)), *line, col - 1)?,
                    state_vars[i],
                ),
            };
            init_steps.push((state_vars[i], x));
        }

        // Rules' conditions and the values of new rows (ADR 0079), from the
        // values after a step.
        let mut rule_steps: Vec<(usize, Expr)> = Vec::new();
        for ((t, src, r, values, _), (cond_var, value_vars)) in rules.iter().zip(&rule_vars) {
            let to: Vec<usize> = match (&r.add, src) {
                (None, _) => vec![*t],
                (Some(_), Some(s)) => vec![*s],
                (Some(_), None) => Vec::new(),
            };
            let e = resolve(&r.cond, defined.len(), Some(&to), r.line, r.cond_column - 1)?;
            rule_steps.push((*cond_var, e));
            for ((_, ast, line, c), &var) in values.iter().zip(value_vars) {
                let e = resolve(ast, defined.len(), Some(&to), *line, c - 1)?;
                rule_steps.push((var, stored(e, var)));
            }
        }
        let rule_runs: Vec<RuleRun> = rules
            .iter()
            .zip(&rule_vars)
            .map(|((t, src, r, _, transfers), (cond, values))| RuleRun {
                table: *t,
                kind: r.kind,
                edges: src.filter(|_| matches!(r.kind, RuleKind::Split | RuleKind::Merge)),
                cond: spans[*cond],
                values: values.iter().map(|&v| spans[v]).collect(),
                transfers: transfers.clone(),
            })
            .collect();
        let tables: Vec<TableRows> = grid
            .iter()
            .enumerate()
            .filter_map(|(i, a)| {
                let max = a.table?;
                Some(TableRows {
                    axis: i,
                    name: a.name.clone(),
                    max,
                    start: start_rows[i],
                    rows: a.cells,
                    columns: states
                        .iter()
                        .filter(|s| row_table(s) == Some(i))
                        .cloned()
                        .collect(),
                    follows: inputs
                        .iter()
                        .filter(|n| row_table(n).is_some())
                        .position(|n| row_table(n) == Some(i))
                        .filter(|_| followed[i]),
                    ends_of: a
                        .ends_of
                        .map(|m| grid[..m].iter().filter(|g| g.table.is_some()).count()),
                    chain: chains.contains_key(&a.name),
                    shape: a.shape,
                    sides_of: a
                        .sides_of
                        .map(|m| grid[..m].iter().filter(|g| g.table.is_some()).count()),
                    start_cells: meshes.get(&a.name).map(|m| m.1.clone()),
                    start_points: grid_starts
                        .iter()
                        .find(|(c, _)| cell_decls.get(*c).is_some_and(|o| o.0 == a.name))
                        .and_then(|(c, _)| meshes.get(c))
                        .map(|m| m.0.clone()),
                    coords: grid_starts
                        .iter()
                        .find(|(c, _)| cell_decls.get(*c).is_some_and(|o| o.0 == a.name))
                        .map(|(_, (g, _))| {
                            let cols: Vec<&String> =
                                states.iter().filter(|s| row_table(s) == Some(i)).collect();
                            g.names()
                                .iter()
                                .map(|n| {
                                    cols.iter().position(|c| *c == n).expect("checked: a state")
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    parents: refined.contains(&i).then_some(0),
                    from_file: matches!(grid_starts.get(&a.name), Some((MeshSpec::File { .. }, _))),
                    start_file: csv_starts.get(&a.name).map(|c| c.0.clone()),
                })
            })
            .collect();
        // Parents' tables after the declared ones.
        let mut tables = tables;
        // Rows from CSV files (ADR 0069 step 3): each column names one of
        // the table's quantities (converted to its unit), an edge file has
        // `from` and `to`, joining rows of its table by id.
        for k in 0..tables.len() {
            let Some(csv) = tables[k].start_file.clone() else {
                continue;
            };
            let line = csv_starts[&tables[k].name].1;
            let err = |m: String| definition_error(line, 1, m);
            let edges = tables[k].ends_of.is_some();
            let mut csv = (*csv).clone();
            if edges && (csv.column("from").is_none() || csv.column("to").is_none()) {
                return Err(err(format!(
                    "{}: an edge table's file has 'from' and 'to' columns (the ids of the rows each edge joins)",
                    csv.path
                )));
            }
            for (name, unit, values) in &mut csv.columns {
                if edges && (name == "from" || name == "to") {
                    continue;
                }
                if !tables[k].columns.contains(name) {
                    return Err(err(format!(
                        "{}: the column '{name}' is not a quantity over {} (it has {})",
                        csv.path,
                        tables[k].name,
                        if tables[k].columns.is_empty() {
                            "none".to_string()
                        } else {
                            tables[k].columns.join(", ")
                        }
                    )));
                }
                if let Some(u) = unit {
                    let given = read_unit(u, line, 1)?;
                    let want = units
                        .get(name.as_str())
                        .copied()
                        .unwrap_or(Unit::DIMENSIONLESS);
                    if given.dim != want.dim {
                        return Err(err(format!(
                            "{}: the column '{name}' is in {u}, but {name} is not",
                            csv.path
                        )));
                    }
                    let f = given.scale / want.scale;
                    for v in values.iter_mut() {
                        *v *= f;
                    }
                }
            }
            if let Some(m) = tables[k].ends_of {
                let (Some(from), Some(to)) = (csv.column("from"), csv.column("to")) else {
                    return Err(err(format!(
                        "{}: an edge table's file has 'from' and 'to' columns (the ids of the rows each edge joins)",
                        csv.path
                    )));
                };
                let ids: std::collections::HashSet<u64> = match &tables[m].start_file {
                    Some(c) => c.row_ids().into_iter().collect(),
                    None => (0..tables[m].start as u64).collect(),
                };
                if let Some(v) = from.iter().chain(to).find(|v| !ids.contains(&(**v as u64))) {
                    return Err(err(format!(
                        "{}: {v} is not the id of a starting row of {}",
                        csv.path, tables[m].name
                    )));
                }
            }
            tables[k].start_file = Some(std::sync::Arc::new(csv));
        }
        let n = tables.len();
        for (next, t) in (n..).zip(tables.iter_mut().filter(|t| t.parents.is_some())) {
            t.parents = Some(next);
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
        // Which inputs and parameters each defined quantity uses, directly
        // or through quantities above it (ADR 0040, ADR 0075). An array
        // parameter is all its elements.
        let param_names = |n: &str| -> Vec<String> {
            match array_params.iter().find(|a| a.name == n) {
                Some(a) => (0..a.cells).map(|k| element_name(n, k)).collect(),
                None if param_specs.iter().any(|p| p.name == n) => vec![n.to_string()],
                None => Vec::new(),
            }
        };
        // A table that follows an input holds that input's rows at each
        // instant (ADR 0079 stage 1b): whatever reads its rows (a quantity
        // over it, `id`, `rows`) depends on that input at the same instant.
        let follower = |n: &str| -> Option<String> {
            let a = axes
                .iter()
                .position(|a| *a == n)
                .or_else(|| row_table(n))
                .filter(|&a| followed[a])?;
            inputs.iter().find(|i| row_table(i) == Some(a)).cloned()
        };
        let mut uses: Vec<Vec<String>> = Vec::new();
        let mut reads: Vec<Vec<String>> = Vec::new();
        for line in parsed.iter().filter(|l| matches!(l.lhs, Lhs::Defined(_))) {
            let mut u: Vec<String> = Vec::new();
            let mut r: Vec<String> = Vec::new();
            if let Lhs::Defined(d) = &line.lhs {
                u.extend(follower(d));
            }
            for (n, _) in line.ast.names() {
                if let Some(i) = follower(&n) {
                    if !u.contains(&i) {
                        u.push(i);
                    }
                }
                let (via, params): (Vec<String>, Vec<String>) =
                    match defined[..uses.len()].iter().position(|d| *d == n) {
                        Some(i) => (uses[i].clone(), reads[i].clone()),
                        None if inputs.contains(&n) => (vec![n], Vec::new()),
                        None => (Vec::new(), param_names(&n)),
                    };
                for i in via {
                    if !u.contains(&i) {
                        u.push(i);
                    }
                }
                for p in params {
                    if !r.contains(&p) {
                        r.push(p);
                    }
                }
            }
            // In input and parameter order, so the declaration reads
            // naturally.
            u.sort_by_key(|i| inputs.iter().position(|x| x == i));
            r.sort_by_key(|p| param_specs.iter().position(|q| &q.name == p));
            uses.push(u);
            reads.push(r);
        }
        // A port has its quantity's axes (ADR 0080): one is a line, two a
        // 2D field with the first along x.
        // A port over its quantity's axes (ADR 0081): one is a line, two
        // a 2D field with the first along x, more a tensor.
        // A quantity over a table's rows carries them, with their ids and
        // version (ADR 0079 stage 1b).
        let shape = |n: &str| {
            if row_table(n).is_some() {
                return Shape::Rows { dim: 1 };
            }
            let dims: Vec<usize> = axes_of_name(n).iter().map(|&a| grid[a].cells).collect();
            Shape::of_axes(&dims)
        };
        let named = |p: PortSpec, n: &str| -> PortSpec {
            let names: Vec<&str> = axes_of_name(n).iter().map(|&a| axes[a]).collect();
            p.axes_named(&names)
        };
        // A state's output is the state itself (ADR 0063), in state order.
        let mut state_at = 0;
        let mut ports: Vec<PortSpec> = inputs
            .iter()
            .map(|n| named(PortSpec::input(n, unit_of(n), shape(n)), n))
            .chain(states.iter().map(|n| {
                let p = PortSpec::output(n, unit_of(n), shape(n));
                if row_table(n).is_some() {
                    let f: Vec<String> = follower(n).into_iter().collect();
                    let f: Vec<&str> = f.iter().map(String::as_str).collect();
                    return named(p.depends_on(&f), n);
                }
                let p = p.from_state(state_at);
                state_at += shape(n).len();
                named(p, n)
            }))
            .chain(
                defined
                    .iter()
                    .zip(uses.iter().zip(&reads))
                    .map(|(n, (u, r))| {
                        let u: Vec<&str> = u.iter().map(String::as_str).collect();
                        let r: Vec<&str> = r.iter().map(String::as_str).collect();
                        named(PortSpec::output(n, unit_of(n), shape(n)), n)
                            .depends_on(&u)
                            .depends_on_params(&r)
                    }),
            )
            .collect();
        for (i, a) in grid.iter().enumerate().filter(|(_, a)| a.table.is_none()) {
            // Each axis is the coordinate of every output over it.
            let bounds: Vec<String> = [&a.lo, &a.hi]
                .iter()
                .flat_map(|e| e.names())
                .flat_map(|(n, _)| param_names(&n))
                .collect();
            let bounds: Vec<&str> = bounds.iter().map(String::as_str).collect();
            let mut x = named(
                PortSpec::output(&a.name, unit_of(&a.name), shape(&a.name)),
                &a.name,
            )
            .depends_on_params(&bounds);
            for n in states.iter().chain(&defined) {
                if let Some(k) = axes_of_name(n).iter().position(|&a| a == i) {
                    x = x.coordinate_of(n, k);
                }
            }
            ports.push(x);
        }
        // Each table of cells with a starting mesh is an output of its own:
        // each cell's corners, as ids of its points, for views (ADR 0086).
        let mut mesh_tables = Vec::new();
        for (i, a) in grid.iter().enumerate() {
            let (Some(shape), None, Some(_)) = (a.shape, a.sides_of, a.ends_of) else {
                continue;
            };
            let (Some(faces), Some((spec, _))) = (shape.drawn_faces(), grid_starts.get(&a.name))
            else {
                continue;
            };
            let coordinates = spec.names();
            if !(2..=3).contains(&coordinates.len()) || states.iter().any(|s| s == &a.name) {
                continue;
            }
            ports.push(
                PortSpec::output(
                    &a.name,
                    Unit::DIMENSIONLESS,
                    Shape::Rows {
                        dim: shape.corners(),
                    },
                )
                .axes_named(&[&a.name])
                .depends_on_params(&[])
                .mesh_of(simsandbox_core::block::MeshOf {
                    shape: shape.name(),
                    dim: coordinates.len(),
                    faces,
                    coordinates,
                }),
            );
            mesh_tables.push((i, shape.corners()));
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
        // One per state, in state order.
        rhs.sort_by_key(|r| r.0);
        let rhs: Vec<Expr> = rhs.into_iter().map(|r| r.1).collect();
        let mut rate_steps = output_steps.clone();
        rate_steps.extend(rhs.iter().enumerate().map(|(k, e)| (rate_vars[k], e)));
        let rate_steps = needed(rate_steps, &rate_vars);
        let mut outputs = Program::with_layout(&spans, size, &output_steps);
        let mut rates = Program::with_layout(&spans, size, &rate_steps);
        let rules_program = (!rule_steps.is_empty()).then(|| {
            let mut steps = output_steps.clone();
            steps.extend(rule_steps.iter().map(|(v, e)| (*v, e)));
            Program::with_layout(&spans, size, &steps)
        });
        // Tables' states are gathered for each evaluation: no arrays in
        // place.
        let (outputs_copy, rates_copy) = if tables.is_empty() {
            (
                Copies::place(
                    &mut outputs,
                    layout.states,
                    layout.outputs_at,
                    layout.outputs_len,
                ),
                Copies::place(&mut rates, layout.states, layout.rates_at, layout.states),
            )
        } else {
            (Copies::none(), Copies::none())
        };
        let mut init = state_decls.iter().any(|d| d.array).then(|| {
            let steps: Vec<(usize, &Expr)> = init_steps.iter().map(|(v, e)| (*v, e)).collect();
            Program::with_layout(&spans, size, &steps)
        });
        let mut rules_program = rules_program;
        // Large arrays' phases compiled, once their arrays are placed
        // (ADR 0084).
        for p in [
            Some(&mut outputs),
            Some(&mut rates),
            init.as_mut(),
            rules_program.as_mut(),
        ]
        .into_iter()
        .flatten()
        {
            simsandbox_compiled::compile(p);
        }
        let scratch = RefCell::new([
            outputs.registers(),
            rates.registers(),
            init.as_ref().map_or_else(Vec::new, Program::registers),
            rules_program
                .as_ref()
                .map_or_else(Vec::new, Program::registers),
        ]);
        let state_names = states
            .iter()
            .filter(|s| row_table(s).is_none())
            .flat_map(|s| {
                if !axes_of_name(s).is_empty() {
                    (0..len_of(s))
                        .map(|k| element_name(s, k))
                        .collect::<Vec<_>>()
                } else {
                    vec![s.clone()]
                }
            })
            .collect();
        let initial = states
            .iter()
            .zip(initial)
            .map(|(s, i)| (len_of(s), i))
            .collect();
        // Whether anything reads `t` (ADR 0083).
        let reads_time = parsed
            .iter()
            .map(|l| &l.ast)
            .chain(inits.iter().map(|i| &i.1))
            .chain(edges.iter().filter_map(|e| match &e.kind {
                EdgeKind::Value(a) | EdgeKind::Gradient(a) => Some(a),
                EdgeKind::Periodic => None,
            }))
            .chain(rule_lines.iter().flat_map(|r| {
                std::iter::once(&r.cond)
                    .chain(r.add.iter().flat_map(|(_, v)| v.iter().map(|g| &g.1)))
            }))
            .any(|a| a.names().iter().any(|(n, _)| n == "t"));
        let table_of = |a: usize| tables.iter().position(|t| t.axis == a);
        let inputs_in = inputs
            .iter()
            .map(|n| (len_of(n), row_table(n).and_then(table_of)))
            .collect();
        // In port order: states, then defined quantities.
        let rows_out = states
            .iter()
            .filter_map(|n| {
                let t = table_of(row_table(n)?)?;
                let c = tables[t].columns.iter().position(|c| c == n)?;
                Some((t, RowsSource::Column(c)))
            })
            .chain(defined.iter().enumerate().filter_map(|(k, n)| {
                let t = table_of(row_table(n)?)?;
                Some((t, RowsSource::Registers(spans[defined_vars[k]])))
            }))
            .chain(
                mesh_tables
                    .iter()
                    .filter_map(|&(i, n)| Some((table_of(i)?, RowsSource::Corners(n)))),
            )
            .collect();
        Ok(Self {
            kind,
            reads_time,
            spec: BlockSpec {
                name: name.into(),
                domain: Domain::Equations,
                ports,
                params: param_specs,
                states: state_names,
                tables: tables
                    .iter()
                    .map(|t| {
                        // Edges: `from`, `to`; cells: their corners; sides:
                        // corners, `inner`, `outer`; then states.
                        let mut cols: Vec<String> = match (t.shape, t.sides_of) {
                            (Some(crate::mesh::CellShape::Segment), None) => {
                                vec!["from".into(), "to".into()]
                            }
                            (Some(s), _) => (0..s.corners()).map(|k| format!("c{k}")).collect(),
                            (None, _) => Vec::new(),
                        };
                        if t.sides_of.is_some() {
                            cols.extend(["inner".to_string(), "outer".to_string()]);
                        }
                        if t.parents.is_some() {
                            cols.extend(["level", "parent", "child"].map(String::from));
                        }
                        cols.extend(t.columns.iter().cloned());
                        let cols: Vec<&str> = cols.iter().map(String::as_str).collect();
                        simsandbox_core::table::TableSpec::new(&t.name, &cols)
                    })
                    // The cells split and not merged back (ADR 0085 step 2).
                    .chain(tables.iter().filter(|t| t.parents.is_some()).map(|t| {
                        let mut cols: Vec<String> =
                            (0..t.corners()).map(|k| format!("c{k}")).collect();
                        cols.extend(["level", "parent", "child"].map(String::from));
                        let cols: Vec<&str> = cols.iter().map(String::as_str).collect();
                        simsandbox_core::table::TableSpec::new(
                            &format!("{}~parents", t.name),
                            &cols,
                        )
                    }))
                    .collect(),
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
            outputs_copy,
            rates_copy,
            row_states: states
                .iter()
                .filter_map(|s| {
                    let a = row_table(s)?;
                    let t = tables.iter().position(|t| t.axis == a)?;
                    Some((t, tables[t].columns.iter().position(|c| c == s)?))
                })
                .collect(),
            tables,
            rules: rule_runs,
            rules_program,
            inputs_in,
            rows_out,
            source: (name.to_string(), lines.to_vec()),
        })
    }
}
