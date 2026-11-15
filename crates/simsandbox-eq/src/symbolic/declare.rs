//! What a definition declares: its axes and tables, then the names its
//! lines introduce (states, parameters, inputs, defined quantities) with
//! their units and axes as written, its equations, edges and rules.

use super::shapes::{cells_of, Shapes};
use super::starts::{quoted_file, read_starts, starting_sides, StartCsv, Starts};
use super::statements::{
    as_read, column_of, definition_error, is_name, name_and_unit, read_edges, read_unit,
    split_dims, split_top, EdgeLine, EquationLine, JoinLine, Lhs, Line, RuleLine, Statement,
    Written,
};
use super::{element_name, Axis, Kind};
use simsandbox_core::units::Unit;
use simsandbox_core::{Result, SimError};
use simsandbox_expr::{is_reserved, parse, Ast, MAX_AXES};

/// A `state` line (ADR 0049), once its state is found among the
/// equations' left sides.
pub(super) struct StateDecl {
    pub(super) name: String,
    pub(super) line: usize,
    /// Its place among the states.
    pub(super) state: usize,
    /// Whether it is declared over axes, `state T[x]`: it starts from
    /// its initial expression, not from a parameter `T0`.
    pub(super) array: bool,
    /// An array's initial value as written.
    pub(super) init: Option<Written>,
}

/// An array parameter (ADR 0069), `param m[k] = 1000`: a parameter per
/// cell, `m[0]`, `m[1]`, …, one after another.
pub(super) struct ArrayParam {
    pub(super) name: String,
    pub(super) cells: usize,
    /// The place of its first element among the declared parameters.
    pub(super) first: usize,
}

/// What the lines declare, each line read: what [`declare`] establishes
/// for every later phase. Names are checked for clashes and for what
/// they may be; nothing is yet known of the axes of defined quantities,
/// of the units of names without one, or of the parameters only used.
pub(super) struct Declared {
    /// The axes and tables, in the order of their lines: the first is x
    /// (the fastest in the grid's cells), the second y (ADR 0060).
    pub(super) grid: Vec<Axis>,
    /// Each axis's starting rows, if it is a table (0 if not).
    pub(super) start_rows: Vec<usize>,
    pub(super) starts: Starts,
    /// Units by name, as declared (ADR 0045) or given by a rate's.
    pub(super) units: std::collections::HashMap<String, Unit>,
    pub(super) kind: Kind,
    /// The equations, in written order, their right sides parsed.
    pub(super) equations: Vec<Line>,
    /// For each equation, its left side's place: among the states for a
    /// state's, among the defined quantities for one of those.
    pub(super) places: Vec<usize>,
    /// The states, those over a table's rows after the others: the block
    /// keeps them in its tables, the rest in its fixed state (ADR 0079).
    /// Each has one equation.
    pub(super) states: Vec<String>,
    /// The defined quantities, in written order.
    pub(super) defined: Vec<String>,
    pub(super) inputs: Vec<String>,
    /// The declared parameters (an array's elements each), with their
    /// defaults and lines.
    pub(super) params: Vec<(String, f64, usize)>,
    pub(super) state_decls: Vec<StateDecl>,
    /// The axes of each name declared over some (an axis is over itself).
    pub(super) arrays: Shapes,
    pub(super) array_params: Vec<ArrayParam>,
    pub(super) edges: Vec<EdgeLine>,
    pub(super) rules: Vec<RuleLine>,
}

/// The axes' names, by place.
pub(super) fn axis_names(grid: &[Axis]) -> Vec<&str> {
    grid.iter().map(|a| a.name.as_str()).collect()
}

/// The table whose rows `name` is over, by its axis: `shapes` has its
/// axes.
pub(super) fn row_table(shapes: &Shapes, grid: &[Axis], name: &str) -> Option<usize> {
    shapes
        .get(name)
        .and_then(|d| d.first().copied())
        .filter(|&a| grid[a].table.is_some())
}

/// A table's axis: `rows` cells now, numbered from 0.
fn rows_axis(name: String, line: usize, rows: usize, max: usize) -> Axis {
    Axis {
        name,
        line,
        lo: Ast::Num(-0.5),
        hi: Ast::Num(rows as f64 - 0.5),
        cells: rows,
        table: Some(max),
        ends_of: None,
        shape: None,
        sides_of: None,
    }
}

/// The axis of a `table` line and its starting rows.
fn table_axis(
    (t, max): (String, usize),
    number: usize,
    starts: &Starts,
    rows: &std::collections::BTreeMap<String, usize>,
) -> Result<(Axis, usize)> {
    // Its points made by a grid of cells over it (ADR 0085).
    let made: Vec<_> = starts.making(&t).collect();
    // One grid, or cells of several shapes from one file.
    let one_file = made
        .iter()
        .all(|m| matches!(m.1 .0, super::starts::MeshSpec::File { .. }) && m.1 .0 == made[0].1 .0);
    if made.len() > 1 && !one_file {
        return Err(definition_error(
            made[1].1 .1,
            1,
            format!("{t}'s points come from one starting grid or file, not two"),
        ));
    }
    if let (Some(&(c, (_, line))), Some(_)) = (made.first(), starts.rows.get(&t)) {
        return Err(definition_error(
            *line,
            1,
            format!("{t}'s rows come from {c}'s starting mesh: give it no 'start {t}'"),
        ));
    }
    let start = match made.first() {
        Some((c, _)) => starts
            .meshes
            .get(*c)
            .map_or(0, |m| m.0.first().map_or(0, Vec::len)),
        None => starts.rows.get(&t).map_or(0, |s| s.0),
    };
    if start > max {
        return Err(definition_error(
            starts.rows[&t].1,
            1,
            format!("{t} starts with {start} rows but has at most {max}"),
        ));
    }
    let cells = rows.get(&t).copied().unwrap_or(start);
    Ok((rows_axis(t, number, cells, max), start))
}

/// The axis of an `edges`, `cells` or `sides` line (its word `word`) and
/// its starting rows: it joins rows of a table declared above it.
fn join_axis(
    word: &str,
    (t, of, max, shape): JoinLine,
    number: usize,
    grid: &[Axis],
    starts: &Starts,
    rows: &std::collections::BTreeMap<String, usize>,
) -> Result<(Axis, usize)> {
    let m = &of;
    let found = grid.iter().position(|g| &g.name == m && g.table.is_some());
    // Sides lie between cells; edges and cells join points. A side's
    // corners among its cell's are kept for counting the starting sides.
    let (ends, sides_of, shape, faces) = if word == "sides" {
        let cells = found.and_then(|c| {
            let own = grid[c].sides_of.is_none() && grid[c].name != t;
            Some((c, grid[c].shape.filter(|_| own)?))
        });
        let Some((c, cs)) = cells else {
            return Err(definition_error(number, 1, format!(
                "'{m}' is not a table of cells declared above: sides lie between cells ('cells {m} of pts: triangle' first)"
            )));
        };
        let Some((side, faces)) = cs.sides() else {
            return Err(definition_error(
                number,
                1,
                format!(
                    "the sides of {} come later: their sides are of two shapes",
                    cs.name()
                ),
            ));
        };
        (grid[c].ends_of, Some(c), side, Some(faces))
    } else {
        let Some(p) = found.filter(|&p| grid[p].ends_of.is_none()) else {
            return Err(definition_error(number, 1, format!(
                "'{m}' is not a table declared above: {word} join the rows of one ('table {m}' first)"
            )));
        };
        (Some(p), None, shape, None)
    };
    if let Some(&(_, line)) = starts.rows.get(&t) {
        return Err(definition_error(
            line,
            1,
            format!("{t} starts as a chain or a grid ('start {t} = chain', 'start {t} = grid …'), not a number of rows"),
        ));
    }
    if let (Some((_, line)), true) = (
        starts.csv.get(&t),
        shape != crate::mesh::CellShape::Segment || word == "sides" || word == "cells",
    ) {
        return Err(definition_error(
            *line,
            1,
            format!("{t} starts from a mesh file ('start {t} = file \"part.msh\" at x, y'): a CSV file starts tables and edges"),
        ));
    }
    let start = match (
        starts.csv.get(&t),
        starts.chains.get(&t),
        starts.specs.get(&t),
    ) {
        (Some(csv), _, _) => csv.0.rows,
        (_, Some(_), _) if shape == crate::mesh::CellShape::Segment && word != "sides" => {
            ends.map_or(0, |p| starts.chain_rows(&grid[p].name))
        }
        (_, _, Some(_)) => starts.meshes.get(&t).map_or(0, |m| m.1.len()),
        _ => match (sides_of, &faces) {
            // Sides of starting cells, found as they will be.
            (Some(c), Some(faces)) => starting_sides(grid, c, faces, starts),
            _ => 0,
        },
    };
    let cells = rows.get(&t).copied().unwrap_or(start);
    let axis = Axis {
        ends_of: ends,
        shape: Some(shape),
        sides_of,
        ..rows_axis(t, number, cells, max)
    };
    Ok((axis, start))
}

/// The axes, in the order of their lines (ADR 0060): declarations
/// further down refer to them wherever they are written. With each its
/// starting rows, and the axes' units in `units`.
fn read_axes(
    statements: &[Statement],
    starts: &Starts,
    rows: &std::collections::BTreeMap<String, usize>,
    units: &mut std::collections::HashMap<String, Unit>,
) -> Result<(Vec<Axis>, Vec<usize>)> {
    let mut grid: Vec<Axis> = Vec::new();
    let mut start_rows = Vec::new();
    for s in statements {
        let (a, start, unit, number) = if let Statement::Table { line, read } = s {
            let (a, start) = table_axis(as_read(read)?.clone(), *line, starts, rows)?;
            (a, start, None, *line)
        } else if let Some((word, number, read)) = s.join() {
            let line = as_read(read)?.clone();
            let (a, start) = join_axis(word, line, number, &grid, starts, rows)?;
            (a, start, None, number)
        } else if let Statement::Grid { line, read } = s {
            let (a, unit) = as_read(read)?.clone();
            (a, 0, unit, *line)
        } else {
            continue;
        };
        if grid.len() == MAX_AXES {
            return Err(definition_error(
                number,
                1,
                format!("a block has at most {MAX_AXES} axes"),
            ));
        }
        if grid.iter().any(|g| g.name == a.name) {
            return Err(definition_error(
                number,
                1,
                format!("the axis {} is declared twice", a.name),
            ));
        }
        if let Some(u) = unit {
            units.insert(a.name.clone(), read_unit(&u, number, 1)?);
        }
        grid.push(a);
        start_rows.push(start);
    }
    Ok((grid, start_rows))
}

/// Every start is of a table that can start so.
fn check_starts(grid: &[Axis], starts: &Starts) -> Result<()> {
    // Declared with `cells` (an edge table has the segment's shape
    // too, but starts as a chain or from a file).
    if let Some((t, (_, line))) = starts
        .specs
        .iter()
        .find(|(t, _)| !starts.cells.contains_key(*t))
    {
        let edges = grid
            .iter()
            .any(|a| &a.name == t && a.shape.is_some() && a.sides_of.is_none());
        return Err(definition_error(
            *line,
            1,
            if edges {
                format!("'{t}' is an edge table, which starts as a chain or from a file ('start {t} = chain', 'start {t} = file \"wires.csv\"'): a grid starts cells ('cells {t} of pts: segment')")
            } else {
                format!("'{t}' is not a table of cells: a grid starts cells ('cells {t} of pts: triangle')")
            },
        ));
    }
    if let Some((t, line)) = starts.chains.iter().find(|(t, _)| {
        !grid.iter().any(|a| {
            &a.name == *t
                && a.shape == Some(crate::mesh::CellShape::Segment)
                && a.sides_of.is_none()
        })
    }) {
        return Err(definition_error(
            *line,
            1,
            format!(
                "'{t}' is not an edge table: a chain joins the rows of one ('edges {t} of pts')"
            ),
        ));
    }
    if let Some((t, (_, line))) = starts
        .rows
        .iter()
        .find(|(t, _)| !grid.iter().any(|a| &a.name == *t && a.table.is_some()))
    {
        return Err(definition_error(
            *line,
            1,
            format!("'{t}' is not a table: declare it with 'table {t}'"),
        ));
    }
    Ok(())
}

/// The names the lines after the axes introduce, gathered line by line.
struct Names<'a> {
    grid: &'a [Axis],
    axes: Vec<&'a str>,
    units: std::collections::HashMap<String, Unit>,
    inputs: Vec<String>,
    /// Declared parameters with their defaults and lines.
    params: Vec<(String, f64, usize)>,
    /// `state` lines: name, line, and for an array its initial value as
    /// written, if any.
    states: Vec<(String, usize, Option<Option<Written>>)>,
    arrays: Shapes,
    array_params: Vec<ArrayParam>,
    edges: Vec<EdgeLine>,
    rules: Vec<RuleLine>,
    /// Units given for a rate, `dx/dt = … [m/s]`: unit, line, column.
    rates: Vec<(String, Unit, usize, usize)>,
    /// The equations: line and what was read.
    equations: Vec<(usize, &'a EquationLine)>,
}

impl Names<'_> {
    /// Whether `n` may name a state, a parameter or an input.
    fn free(&self, n: &str) -> bool {
        is_name(n) && n != "t" && !is_reserved(n) && !self.axes.contains(&n)
    }

    /// `state x [m] = 0`, `state x [m]`, `state x = 0`, and arrays
    /// `state T[x] [K] = expression`; `rest` after `state`.
    fn state(&mut self, number: usize, text: &str, rest: &str) -> Result<()> {
        let (head, value) = match rest.split_once('=') {
            Some((h, v)) => (h, Some(v)),
            None => (rest, None),
        };
        let (head, array) = split_dims(head, &self.axes, number)?;
        let (n, unit) = name_and_unit(&head);
        if !self.free(&n) {
            return Err(definition_error(
                number,
                1,
                format!("'{n}' cannot be a state name"),
            ));
        }
        if self.states.iter().any(|d| d.0 == n) {
            return Err(definition_error(
                number,
                1,
                format!("'{n}' is declared twice"),
            ));
        }
        if let Some(u) = unit {
            self.units.insert(n.clone(), read_unit(&u, number, 1)?);
        }
        let mut init = None;
        if let Some(dims) = array {
            init = Some(value.map(|v| (v.to_string(), number, column_of(text, v.trim()))));
            self.arrays.insert(n.clone(), dims);
        } else if let Some(v) = value {
            let v = v.trim().parse::<f64>().map_err(|_| {
                definition_error(number, 1, format!("'{}' is not a number", v.trim()))
            })?;
            let p = format!("{n}0");
            if self.params.iter().any(|d| d.0 == p) {
                return Err(definition_error(
                    number,
                    1,
                    format!("{p} is declared twice (here as the initial value of {n})"),
                ));
            }
            self.params.push((p, v, number));
        }
        self.states.push((n, number, init));
        Ok(())
    }

    /// `param k [N/m] = 4`, `param k [N/m]`, `param k = 4`; an array's
    /// values from a file, `param m[k] [kg] = file "masses.csv"` (ADR
    /// 0069 step 3); `rest` after `param`.
    fn param(&mut self, number: usize, rest: &str) -> Result<()> {
        let mut from_file = None;
        let (head, value) = match rest.split_once('=') {
            Some((h, v)) => match quoted_file(v) {
                Some(file) => {
                    let path = file.map_err(|()| {
                        definition_error(
                            number,
                            1,
                            "write an array's file as 'param m[k] = file \"m.csv\"'",
                        )
                    })?;
                    let csv = StartCsv::load(&path).map_err(|e| definition_error(number, 1, e))?;
                    from_file = Some(csv);
                    (h, 1.0)
                }
                None => {
                    let v = v.trim().parse::<f64>().map_err(|_| {
                        definition_error(number, 1, format!("'{}' is not a number", v.trim()))
                    })?;
                    (h, v)
                }
            },
            None => (rest, 1.0),
        };
        // `param m[k] [kg] = 1000` (ADR 0069): a value per cell,
        // each its own parameter `m[0]`, `m[1]`, ….
        let (head, array) = split_dims(head, &self.axes, number)?;
        let (n, unit) = name_and_unit(&head);
        if !self.free(&n) {
            return Err(definition_error(
                number,
                1,
                format!("'{n}' cannot be a parameter name"),
            ));
        }
        if let Some(d) = self.params.iter().find(|d| d.0 == n) {
            let state = n
                .strip_suffix('0')
                .filter(|x| self.states.iter().any(|s| s.0 == *x));
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
            self.units.insert(n.clone(), read_unit(&u, number, 1)?);
        }
        let Some(dims) = array else {
            if from_file.is_some() {
                return Err(definition_error(
                    number,
                    1,
                    format!("'{n}' is one value: a file gives an array's values ('param {n}[k] = file …')"),
                ));
            }
            self.params.push((n, value, number));
            return Ok(());
        };
        let cells = cells_of(&dims, self.grid);
        let values = match &from_file {
            None => None,
            Some(csv) => Some(self.file_values(csv, &n, cells, number)?),
        };
        let first = self
            .params
            .iter()
            .position(|d| d.0 == element_name(&n, 0))
            .unwrap_or(self.params.len());
        for k in 0..cells {
            let e = element_name(&n, k);
            if let Some(u) = self.units.get(&n).copied() {
                self.units.insert(e.clone(), u);
            }
            let v = values.as_ref().map_or(value, |v| v[k]);
            self.params.push((e, v, number));
        }
        self.array_params.push(ArrayParam {
            name: n.clone(),
            cells,
            first,
        });
        self.arrays.insert(n, dims);
        Ok(())
    }

    /// An array parameter's values from a file: the column named after
    /// it, or its only column; a value per cell, in the cells' order, in
    /// the parameter's unit.
    fn file_values(
        &self,
        csv: &StartCsv,
        n: &str,
        cells: usize,
        number: usize,
    ) -> Result<Vec<f64>> {
        let col = csv
            .columns
            .iter()
            .find(|c| c.0 == n)
            .or(if csv.columns.len() == 1 {
                csv.columns.first()
            } else {
                None
            })
            .ok_or_else(|| {
                definition_error(number, 1, format!("{} has no column '{n}'", csv.path))
            })?;
        if col.2.len() != cells {
            return Err(definition_error(
                number,
                1,
                format!(
                    "{} has {} values of {n}, for {cells} cells",
                    csv.path,
                    col.2.len()
                ),
            ));
        }
        let f = match (&col.1, self.units.get(n)) {
            (Some(u), Some(want)) => {
                let given = read_unit(u, number, 1)?;
                if given.dim != want.dim {
                    return Err(definition_error(
                        number,
                        1,
                        format!("{}: '{n}' is in {u}, which is not {n}'s unit", csv.path),
                    ));
                }
                given.scale / want.scale
            }
            _ => 1.0,
        };
        Ok(col.2.iter().map(|v| v * f).collect())
    }

    /// `input F [N], q[x] [W/m^3]`, `names` after `input`.
    fn input(&mut self, number: usize, names: &str) -> Result<()> {
        for item in split_top(names)
            .into_iter()
            .filter(|n| !n.trim().is_empty())
        {
            let (item, array) = split_dims(item, &self.axes, number)?;
            let (item, unit) = name_and_unit(&item);
            // Names may also be separated by spaces (no units then).
            let parts: Vec<&str> = item.split(' ').filter(|n| !n.is_empty()).collect();
            if (unit.is_some() || array.is_some()) && parts.len() != 1 {
                return Err(definition_error(
                    number,
                    1,
                    "separate inputs with commas when giving units or grids: 'input F [N], q[x] [W/m^3]'",
                ));
            }
            if let Some(u) = unit {
                self.units.insert(item.clone(), read_unit(&u, number, 1)?);
            }
            for n in parts {
                if !self.free(n) {
                    return Err(definition_error(
                        number,
                        1,
                        format!("'{n}' cannot be an input name"),
                    ));
                }
                if self.inputs.iter().any(|i| i == n) {
                    return Err(definition_error(
                        number,
                        1,
                        format!("'{n}' is an input twice"),
                    ));
                }
                self.inputs.push(n.to_string());
                if let Some(dims) = &array {
                    self.arrays.insert(n.to_string(), dims.clone());
                }
            }
        }
        Ok(())
    }

    /// An equation's unit, written after it: its left side's (ADR 0049).
    fn equation_unit(&mut self, number: usize, eq: &EquationLine) -> Result<()> {
        let Some((unit, column)) = eq.unit else {
            return Ok(());
        };
        match &eq.lhs {
            Lhs::State(n) => self.rates.push((n.clone(), unit, number, column)),
            Lhs::Map(n) | Lhs::Defined(n) => {
                if let Some(d) = self.units.get(n).filter(|d| d.dim != unit.dim) {
                    return Err(definition_error(
                        number,
                        column,
                        format!("{n} is declared in {d}, not {unit}"),
                    ));
                }
                self.units.entry(n.clone()).or_insert(unit);
            }
        }
        Ok(())
    }
}

/// The kind of block the equations' left sides make (ADR 0019).
fn kind_of(equations: &[(usize, &EquationLine)]) -> Result<Kind> {
    if equations.is_empty() {
        return Err(SimError::InvalidDefinition(
            "no equations: write lines like 'dx/dt = -k*x', 'x[n+1] = r*x*(1 - x)' or 'y = a*x^2'"
                .into(),
        ));
    }
    let continuous = equations.iter().find(|s| matches!(s.1.lhs, Lhs::State(_)));
    let map = equations.iter().find(|s| matches!(s.1.lhs, Lhs::Map(_)));
    match (continuous, map) {
        (Some(_), Some(m)) => Err(definition_error(
            m.0,
            1,
            "a block is either continuous (dx/dt) or a map (x[n+1]), not both",
        )),
        (Some(_), None) => Ok(Kind::Continuous),
        (None, Some(_)) => Ok(Kind::Map),
        (None, None) => Ok(Kind::Formulas),
    }
}

/// A rate's unit gives its state's (ADR 0049), where the state has none.
fn units_from_rates(
    rates: &[(String, Unit, usize, usize)],
    units: &mut std::collections::HashMap<String, Unit>,
) -> Result<()> {
    for (n, rate, number, column) in rates {
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
    Ok(())
}

/// Read what the statements declare, in the language's order: the
/// `cells` lines, the `start` lines, the axes in written order, then
/// every other line in written order. `rows` gives each table's rows
/// when the block is compiled again for them.
pub(super) fn declare(
    statements: &[Statement],
    rows: &std::collections::BTreeMap<String, usize>,
) -> Result<Declared> {
    let starts = read_starts(statements, rows)?;
    let mut units = Default::default();
    let (grid, start_rows) = read_axes(statements, &starts, rows, &mut units)?;
    check_starts(&grid, &starts)?;

    // Then the left-hand sides, which decide the kind of block, and
    // the declarations.
    let axes = axis_names(&grid);
    let mut names = Names {
        grid: &grid,
        // An axis is over itself.
        arrays: axes
            .iter()
            .enumerate()
            .map(|(i, a)| (a.to_string(), vec![i]))
            .collect(),
        axes,
        units,
        inputs: Vec::new(),
        params: Vec::new(),
        states: Vec::new(),
        array_params: Vec::new(),
        edges: Vec::new(),
        rules: Vec::new(),
        rates: Vec::new(),
        equations: Vec::new(),
    };
    for s in statements {
        match s {
            Statement::Blank
            | Statement::Grid { .. }
            | Statement::Table { .. }
            | Statement::Edges { .. }
            | Statement::Cells { .. }
            | Statement::Sides { .. }
            | Statement::Start { .. } => {}
            Statement::Rule { read } => names.rules.push(as_read(read)?.clone()),
            Statement::Edge { line, text, rest } => {
                let edges = read_edges(text, rest, *line, &names.axes)?;
                names.edges.extend(edges);
            }
            Statement::State { line, text, rest } => names.state(*line, text, rest)?,
            Statement::Param { line, rest } => names.param(*line, rest)?,
            Statement::Input { line, rest } => names.input(*line, rest)?,
            Statement::Equation { line, read } => {
                let eq = as_read(read)?;
                names.equation_unit(*line, eq)?;
                names.equations.push((*line, eq));
            }
        }
    }
    let Names {
        axes,
        mut units,
        inputs,
        params,
        states: state_lines,
        arrays,
        array_params,
        edges,
        rules,
        rates,
        equations: sides,
        ..
    } = names;
    let kind = kind_of(&sides)?;
    let mut equations = Vec::new();
    for (number, eq) in sides {
        // In maps, x[n] is the current value: blank out "[n]" so columns stay put.
        let rhs = if kind == Kind::Map {
            eq.rhs.replace("[n]", "   ")
        } else {
            eq.rhs.clone()
        };
        let ast =
            parse(&rhs).map_err(|e| definition_error(number, eq.offset + e.column, e.message))?;
        equations.push(Line {
            number,
            offset: eq.offset,
            lhs: eq.lhs.clone(),
            ast,
        });
    }

    // Names of states and defined quantities, checked for clashes; each
    // state with its equation.
    let mut states: Vec<(String, usize)> = Vec::new();
    let mut defined: Vec<String> = Vec::new();
    let mut places = vec![0; equations.len()];
    for (k, line) in equations.iter().enumerate() {
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
        if states.iter().any(|s| &s.0 == n)
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
            Lhs::State(_) | Lhs::Map(_) => states.push((n.clone(), k)),
            Lhs::Defined(_) => {
                places[k] = defined.len();
                defined.push(n.clone());
            }
        }
    }
    // States over tables' rows after the others: the block keeps them
    // in its tables, the rest in its fixed state (ADR 0079).
    states.sort_by_key(|(n, _)| row_table(&arrays, &grid, n).is_some());
    for (i, (_, k)) in states.iter().enumerate() {
        places[*k] = i;
    }
    let states: Vec<String> = states.into_iter().map(|s| s.0).collect();
    // Every declared state has dynamics.
    let mut state_decls = Vec::new();
    for (n, number, init) in state_lines {
        let Some(state) = states.iter().position(|s| *s == n) else {
            let how = if defined.contains(&n) {
                format!("'{n}' is defined by '{n} = …', so it is not a state")
            } else {
                format!("{n} is declared as a state but no line gives d{n}/dt or {n}[n+1]")
            };
            return Err(definition_error(number, 1, how));
        };
        state_decls.push(StateDecl {
            name: n,
            line: number,
            state,
            array: init.is_some(),
            init: init.flatten(),
        });
    }
    units_from_rates(&rates, &mut units)?;
    Ok(Declared {
        grid,
        start_rows,
        starts,
        units,
        kind,
        equations,
        places,
        states,
        defined,
        inputs,
        params,
        state_decls,
        arrays,
        array_params,
        edges,
        rules,
    })
}
