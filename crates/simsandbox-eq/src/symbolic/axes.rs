//! The axes of a definition (ADR 0060): grid axes and the rows of tables
//! (ADR 0079), edge tables, cells and sides (ADR 0085), each with the
//! rows it starts with.

use super::starts::{starting_sides, MeshSpec, Starts};
use super::statements::{as_read, definition_error, read_unit, JoinLine, Statement};
use super::Axis;
use simsandbox_core::units::Unit;
use simsandbox_core::Result;
use simsandbox_expr::{Ast, MAX_AXES};

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
        .all(|m| matches!(m.1 .0, MeshSpec::File { .. }) && m.1 .0 == made[0].1 .0);
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
        // Its own `start` line, or that of the cells whose mesh makes its
        // rows.
        let (line, why) = match (starts.rows.get(&t), made.first()) {
            (Some(s), _) => (s.1, String::new()),
            (None, Some((c, (_, line)))) => {
                (*line, format!(" (the points of {c}'s starting mesh)"))
            }
            (None, None) => (number, String::new()),
        };
        return Err(definition_error(
            line,
            1,
            format!("{t} starts with {start} rows{why} but has at most {max}"),
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
pub(super) fn read_axes(
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

/// Every start is of a table that can start so. Of several that are not,
/// the first written is the one told.
pub(super) fn check_starts(grid: &[Axis], starts: &Starts) -> Result<()> {
    let mut wrong: Vec<(usize, String)> = Vec::new();
    // Declared with `cells` (an edge table has the segment's shape too,
    // but starts as a chain or from a file).
    for (t, (_, line)) in starts
        .specs
        .iter()
        .filter(|(t, _)| !starts.cells.contains_key(*t))
    {
        let edges = grid
            .iter()
            .any(|a| &a.name == t && a.shape.is_some() && a.sides_of.is_none());
        wrong.push((
            *line,
            if edges {
                format!("'{t}' is an edge table, which starts as a chain or from a file ('start {t} = chain', 'start {t} = file \"wires.csv\"'): a grid starts cells ('cells {t} of pts: segment')")
            } else {
                format!("'{t}' is not a table of cells: a grid starts cells ('cells {t} of pts: triangle')")
            },
        ));
    }
    for (t, line) in starts.chains.iter().filter(|(t, _)| {
        !grid.iter().any(|a| {
            &a.name == *t
                && a.shape == Some(crate::mesh::CellShape::Segment)
                && a.sides_of.is_none()
        })
    }) {
        wrong.push((
            *line,
            format!(
                "'{t}' is not an edge table: a chain joins the rows of one ('edges {t} of pts')"
            ),
        ));
    }
    for (t, (_, line)) in starts
        .rows
        .iter()
        .filter(|(t, _)| !grid.iter().any(|a| &a.name == *t && a.table.is_some()))
    {
        wrong.push((
            *line,
            format!("'{t}' is not a table: declare it with 'table {t}'"),
        ));
    }
    match wrong.into_iter().min() {
        Some((line, message)) => Err(definition_error(line, 1, message)),
        None => Ok(()),
    }
}
