//! Which axes each quantity is over, and what follows from them: the
//! equations, initial values, edges and rules checked against the axes
//! and tables, and the parameters found.

use super::declare::{axis_names, row_table, Declared};
use super::rules::{check_rules, CheckedRule, RuleLine};
use super::shapes::{axes_of, cells_of, union, Shapes};
use super::statements::{definition_error, EdgeKind, Lhs};
use super::{Axis, Kind, RuleKind};
use simsandbox_core::Result;
use simsandbox_expr::{parse, Ast};

/// The axes of every quantity, by name, on the block's grid.
pub(super) struct Arrays<'a> {
    pub(super) grid: &'a [Axis],
    pub(super) shapes: Shapes,
}

impl Arrays<'_> {
    /// The axes `name` is over (none: one value).
    pub(super) fn of(&self, name: &str) -> Vec<usize> {
        self.shapes.get(name).cloned().unwrap_or_default()
    }

    /// The axes an expression is over, or its mistake with its column.
    pub(super) fn of_ast(&self, ast: &Ast) -> std::result::Result<Vec<usize>, (usize, String)> {
        axes_of(ast, self.grid, &self.shapes)
    }

    /// The table whose rows `name` is over, by its axis.
    pub(super) fn row_table(&self, name: &str) -> Option<usize> {
        row_table(&self.shapes, self.grid, name)
    }

    /// How many values `name` has.
    pub(super) fn len_of(&self, name: &str) -> usize {
        cells_of(&self.of(name), self.grid)
    }

    /// Axes by name, as a message lists them.
    pub(super) fn list(&self, axes: &[usize]) -> String {
        let names: Vec<&str> = axes.iter().map(|&i| self.grid[i].name.as_str()).collect();
        names.join(", ")
    }
}

/// An array state's initial value, `state T[x] = expression`.
pub(super) struct Init {
    /// Its state's place among the states, and name.
    pub(super) state: usize,
    pub(super) name: String,
    pub(super) ast: Ast,
    pub(super) line: usize,
    pub(super) column: usize,
}

/// What [`shape`] establishes: every quantity's axes, each expression
/// over axes its place allows and written with the names it may use,
/// the rules checked against their tables, and the parameters, all found.
pub(super) struct Shaped<'a> {
    pub(super) arrays: Arrays<'a>,
    /// For each axis, whether it is a table following an input's rows
    /// (ADR 0079 stage 1b).
    pub(super) followed: Vec<bool>,
    /// Array states' initial values, in the order of their `state` lines.
    pub(super) inits: Vec<Init>,
    /// The parameters: the declared ones, every other name in order of
    /// first use, then the scalar states' initial values.
    pub(super) params: Vec<String>,
    /// For each state, the parameter it starts from (`x0`); `None` for
    /// an array, which starts from its initial expression (0 if none).
    pub(super) initial: Vec<Option<usize>>,
    /// The rules, in written order.
    pub(super) rules: Vec<CheckedRule<'a>>,
    /// The tables of cells some rule refines, by axis.
    pub(super) refined: std::collections::HashSet<usize>,
    /// For each starting mesh, its points' table (by axis) and the
    /// column, among that table's states, of each coordinate it sets.
    pub(super) mesh_coords: Vec<(usize, Vec<usize>)>,
}

/// Each defined quantity's axes (ADR 0080): those its right side uses,
/// found in any order of the lines.
fn all_shapes(d: &Declared) -> Shapes {
    let mut shapes = d.arrays.clone();
    for _ in 0..=d.equations.len() {
        let mut changed = false;
        for line in &d.equations {
            if let Lhs::Defined(n) = &line.lhs {
                if let Ok(a) = axes_of(&line.ast, &d.grid, &shapes) {
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
    shapes
}

/// Each equation's right side is over axes its left side has.
fn check_equations(d: &Declared, arrays: &Arrays) -> Result<()> {
    for line in &d.equations {
        let got = arrays
            .of_ast(&line.ast)
            .map_err(|(c, m)| definition_error(line.number, line.offset + c, m))?;
        let (n, what) = match &line.lhs {
            Lhs::State(n) => (n, format!("d{n}/dt")),
            Lhs::Map(n) => (n, format!("{n}[n+1]")),
            Lhs::Defined(_) => continue,
        };
        let own = arrays.of(n);
        if got.iter().any(|a| !own.contains(a)) {
            let all = union(&own, &got);
            let how = if own.is_empty() {
                format!(
                    "{what} has a value per cell, so {n} must be an array: declare 'state {n}[{}]'",
                    arrays.list(&all)
                )
            } else {
                format!(
                    "{what} is over {}, but {n} is over {}: declare 'state {n}[{}]'",
                    arrays.list(&got),
                    arrays.list(&own),
                    arrays.list(&all)
                )
            };
            return Err(definition_error(line.number, line.offset + 1, how));
        }
    }
    Ok(())
}

/// The line a name is introduced on, for a mistake about it.
fn line_of_name(d: &Declared, n: &str) -> usize {
    d.state_decls
        .iter()
        .find(|s| s.name == n)
        .map(|s| s.line)
        .or_else(|| {
            d.equations.iter().find_map(|l| match &l.lhs {
                Lhs::Defined(x) | Lhs::State(x) | Lhs::Map(x) if x == n => Some(l.number),
                _ => None,
            })
        })
        .or_else(|| {
            d.params
                .iter()
                .find(|p| p.0 == format!("{n}[0]"))
                .map(|p| p.2)
        })
        .unwrap_or(1)
}

/// Tables (ADR 0079 stage 1a): which follow an input's rows; a table
/// that changes needs states; a quantity over a table's rows has no
/// other axis, and is a state, an input or a defined quantity.
fn check_tables(d: &Declared, arrays: &Arrays) -> Result<Vec<bool>> {
    let grid = &d.grid;
    // A table keeps its own rows when it has a start, rules (its own,
    // or split and merge on its edges) or edges; an input over such a
    // table is matched to its rows by id. A table with none of these
    // that some input is over follows that input's rows (ADR 0079
    // stages 1b and 2): its rows come from upstream, by id.
    let owns = |a: usize| {
        let name = &grid[a].name;
        d.starts.rows.contains_key(name)
            || d.rules.iter().any(|r| &r.table == name)
            || grid.iter().any(|g| g.ends_of == Some(a))
    };
    let followed: Vec<bool> = (0..grid.len())
        .map(|a| {
            grid[a].table.is_some()
                && grid[a].ends_of.is_none()
                && !owns(a)
                && d.inputs
                    .iter()
                    .any(|i| arrays.shapes.get(i).is_some_and(|x| x.contains(&a)))
        })
        .collect();
    // A table that never changes (no rules, and no states, which make
    // a map) may be read by formulas (ADR 0085).
    if d.kind == Kind::Formulas {
        if let Some((_, t)) = grid
            .iter()
            .enumerate()
            .find(|(a, t)| t.table.is_some() && !(followed[*a] || d.rules.is_empty()))
        {
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
    for (n, dims) in &arrays.shapes {
        let is_axis = grid.iter().any(|a| &a.name == n);
        if is_axis || !dims.iter().any(|&a| grid[a].table.is_some()) {
            continue;
        }
        let why = if dims.len() > 1 {
            format!("{n} is over a table's rows and other axes: a quantity over rows has only them (for now)")
        } else if d.array_params.iter().any(|a| &a.name == n) {
            format!("a parameter has a value per cell, not per row: a value per row is a state, 'state {n}[{}]'", grid[dims[0]].name)
        } else {
            continue;
        };
        return Err(definition_error(line_of_name(d, n), 1, why));
    }
    Ok(followed)
}

/// Array states' initial values, parsed.
fn read_inits(d: &Declared) -> Result<Vec<Init>> {
    let mut inits = Vec::new();
    for s in &d.state_decls {
        if let Some((src, number, col)) = &s.init {
            let ast =
                parse(src).map_err(|e| definition_error(*number, col + e.column - 1, e.message))?;
            inits.push(Init {
                state: s.state,
                name: s.name.clone(),
                ast,
                line: *number,
                column: *col,
            });
        }
    }
    Ok(inits)
}

/// Each edge is an end of an array along one of its axes, given once.
fn check_edges(d: &Declared, arrays: &Arrays) -> Result<()> {
    let grid = &d.grid;
    for e in &d.edges {
        let own = arrays.of(&e.array);
        if grid[e.axis].table.is_some() {
            return Err(definition_error(
                e.line,
                e.column,
                format!("{} is a table: its rows have no ends", grid[e.axis].name),
            ));
        }
        if own.is_empty() || grid.iter().any(|a| a.name == e.array) {
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
                    arrays.list(&own),
                    grid[e.axis].name
                ),
            ));
        }
        if d.edges
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
                    grid[e.axis].name,
                    if e.high { "+" } else { "-" }
                ),
            ));
        }
    }
    Ok(())
}

/// Whether a rule's value is a transfer (`copy`, `mean`, …), not a
/// quantity.
fn is_transfer(r: &RuleLine, ast: &Ast) -> bool {
    matches!(r.kind, RuleKind::Split | RuleKind::Merge) && super::rules::transfer(ast).is_some()
}

/// The parameters: the declared ones, then every other name in order of
/// first use.
fn parameters(d: &Declared, inits: &[Init]) -> Result<Vec<String>> {
    let known = |n: &str| {
        n == "t"
            || d.array_params.iter().any(|a| a.name == n)
            || d.states.iter().any(|s| s == n)
            || d.defined.iter().any(|s| s == n)
            || d.inputs.iter().any(|s| s == n)
            || d.grid.iter().any(|a| a.name == n)
    };
    let mut params: Vec<String> = Vec::new();
    for (n, _, number) in &d.params {
        if known(n) {
            return Err(definition_error(
                *number,
                1,
                format!("'{n}' is defined twice"),
            ));
        }
        params.push(n.clone());
    }
    let mut used: Vec<&Ast> = d.equations.iter().map(|l| &l.ast).collect();
    used.extend(d.grid.iter().flat_map(|a| [&a.lo, &a.hi]));
    used.extend(inits.iter().map(|i| &i.ast));
    used.extend(d.edges.iter().filter_map(|e| match &e.kind {
        EdgeKind::Value(a) | EdgeKind::Gradient(a) => Some(a),
        EdgeKind::Periodic => None,
    }));
    // Then the rules' conditions and values.
    for r in &d.rules {
        used.push(&r.cond);
        for (_, ast, _) in r.add.iter().flat_map(|a| &a.1) {
            if !is_transfer(r, ast) {
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
    Ok(params)
}

/// An expression is written with the names `allowed` only.
fn only(
    ast: &Ast,
    line: usize,
    col: usize,
    allowed: &dyn Fn(&str) -> bool,
    what: &str,
) -> Result<()> {
    match ast.names().into_iter().find(|(n, _)| !allowed(n)) {
        Some((n, c)) => Err(definition_error(
            line,
            col + c - 1,
            format!("{what}, not '{n}'"),
        )),
        None => Ok(()),
    }
}

/// What each expression outside the equations may use: a grid's ends,
/// the initial values (over their state's axes) and the edge values;
/// and a map's equations and rules do not use `n`.
fn check_uses(d: &Declared, arrays: &Arrays, inits: &[Init], params: &[String]) -> Result<()> {
    let axes = axis_names(&d.grid);
    let is_param = |n: &str| params.iter().any(|p| p == n);
    for a in &d.grid {
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
    for i in inits {
        let (n, line, col) = (&i.name, i.line, i.column);
        let got = arrays
            .of_ast(&i.ast)
            .map_err(|(c, m)| definition_error(line, col + c - 1, m))?;
        let own = arrays.of(n);
        if got.iter().any(|a| !own.contains(a)) {
            return Err(definition_error(
                line,
                col,
                format!(
                    "{n}'s initial value is over {}, but {n} is over {}",
                    arrays.list(&got),
                    arrays.list(&own)
                ),
            ));
        }
        only(
            &i.ast,
            line,
            col,
            // An array parameter too (`state x[p] = x_start`, ADR 0069).
            // And a starting mesh's coordinates (ADR 0085 step 3).
            &|n| {
                is_param(n)
                    || axes.contains(&n)
                    || d.array_params.iter().any(|a| a.name == n)
                    || d.starts.specs.values().any(|(m, _)| m.names().iter().any(|c| c == n))
            },
            "an initial value is written with parameters, the axes and a starting mesh's coordinates",
        )?;
    }
    for e in &d.edges {
        if let EdgeKind::Value(ast) | EdgeKind::Gradient(ast) = &e.kind {
            only(
                ast,
                e.line,
                e.column,
                &|n| {
                    n == "t"
                        || is_param(n)
                        || d.inputs.iter().any(|i| i == n) && arrays.of(n).is_empty()
                },
                "an edge value is written with parameters, scalar inputs and t",
            )?;
        }
    }
    if d.kind == Kind::Map {
        let uses_n = |a: &Ast| a.names().iter().any(|(n, _)| n == "n");
        let in_rule = |r: &&RuleLine| {
            uses_n(&r.cond) || r.add.iter().flat_map(|a| &a.1).any(|g| uses_n(&g.1))
        };
        let line = d
            .equations
            .iter()
            .find(|l| uses_n(&l.ast))
            .map(|l| l.number)
            .or_else(|| d.rules.iter().find(in_rule).map(|r| r.line));
        if let Some(line) = line {
            return Err(definition_error(
                line,
                1,
                "'n' is the step number in maps and cannot be used in equations yet",
            ));
        }
    }
    Ok(())
}

/// A starting grid or mesh file gives its points' coordinates: states
/// over them (ADR 0085). For each, its points' table and the columns.
fn mesh_coords(d: &Declared, arrays: &Arrays) -> Result<Vec<(usize, Vec<usize>)>> {
    let mut out = Vec::new();
    for (c, (g, line)) in &d.starts.specs {
        let pts = d.starts.cells.get(c).map(|o| o.0.as_str()).unwrap_or("");
        let p_axis = d.grid.iter().position(|a| a.name == pts);
        let mut columns = Vec::new();
        for name in &g.names() {
            let column = p_axis.and_then(|p| {
                d.states
                    .iter()
                    .filter(|s| arrays.row_table(s) == Some(p))
                    .position(|s| s == name)
            });
            let Some(column) = column else {
                return Err(definition_error(
                    *line,
                    1,
                    format!(
                        "the starting mesh sets {name} at each point: declare 'state {name}[{pts}]'"
                    ),
                ));
            };
            columns.push(column);
        }
        if let Some(p) = p_axis {
            out.push((p, columns));
        }
    }
    Ok(out)
}

/// Scalar states start from a parameter `x0`, added if not declared;
/// arrays from their initial expression.
fn initial_params(d: &Declared, arrays: &Arrays, params: &mut Vec<String>) -> Vec<Option<usize>> {
    let mut initial: Vec<Option<usize>> = Vec::new();
    for s in &d.states {
        if !arrays.of(s).is_empty() {
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
    initial
}

/// Find every quantity's axes and check the definition against them.
pub(super) fn shape(d: &Declared) -> Result<Shaped<'_>> {
    let arrays = Arrays {
        grid: &d.grid,
        shapes: all_shapes(d),
    };
    check_equations(d, &arrays)?;
    let followed = check_tables(d, &arrays)?;
    let inits = read_inits(d)?;
    check_edges(d, &arrays)?;
    let mut params = parameters(d, &inits)?;
    check_uses(d, &arrays, &inits, &params)?;
    let mesh_coords = mesh_coords(d, &arrays)?;
    let (rules, refined) = check_rules(d, &arrays, &inits)?;
    let initial = initial_params(d, &arrays, &mut params);
    Ok(Shaped {
        arrays,
        followed,
        inits,
        params,
        initial,
        rules,
        refined,
        mesh_coords,
    })
}
