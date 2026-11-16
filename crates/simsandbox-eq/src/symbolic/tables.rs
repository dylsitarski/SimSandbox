//! The tables of a block (ADR 0079): each table's rows, columns and how
//! it starts, as the block keeps them while it runs.

use super::declare::Declared;
use super::shaped::Shaped;
use super::starts::MeshSpec;
use super::statements::{definition_error, read_unit};
use super::units::Units;
use super::TableRows;
use simsandbox_core::units::Unit;
use simsandbox_core::Result;

/// The table of the axis `i`, as declared and started.
fn table_rows(d: &Declared, s: &Shaped, i: usize, max: usize) -> TableRows {
    let (a, starts) = (&d.grid[i], &d.starts);
    let row_table = |n: &str| s.arrays.row_table(n);
    // Its place among the tables, from its place among the axes.
    let table_of = |m: usize| d.grid[..m].iter().filter(|g| g.table.is_some()).count();
    // The cells whose starting mesh makes its rows, if any does.
    let made_by = starts.making(&a.name).next();
    TableRows {
        axis: i,
        name: a.name.clone(),
        max,
        start: d.start_rows[i],
        rows: a.cells,
        columns: d
            .states
            .iter()
            .filter(|n| row_table(n) == Some(i))
            .cloned()
            .collect(),
        follows: d
            .inputs
            .iter()
            .filter(|n| row_table(n).is_some())
            .position(|n| row_table(n) == Some(i))
            .filter(|_| s.followed[i]),
        ends_of: a.ends_of.map(table_of),
        chain: starts.chains.contains_key(&a.name),
        shape: a.shape,
        sides_of: a.sides_of.map(table_of),
        start_cells: starts.meshes.get(&a.name).map(|m| m.1.clone()),
        start_points: made_by
            .and_then(|(c, _)| starts.meshes.get(c))
            .map(|m| m.0.clone()),
        coords: s
            .mesh_coords
            .iter()
            .find(|m| m.0 == i)
            .map(|m| m.1.clone())
            .unwrap_or_default(),
        parents: s.refined.contains(&i).then_some(0),
        from_file: matches!(starts.specs.get(&a.name), Some((MeshSpec::File { .. }, _))),
        start_file: starts.csv.get(&a.name).map(|c| c.0.clone()),
    }
}

/// Rows from a CSV file (ADR 0069 step 3), for the table `k` starting
/// from the file `csv` named on `line`: each column names one of the
/// table's quantities (converted to its unit), an edge file has `from`
/// and `to`, joining rows of its table by id.
fn start_file(
    tables: &[TableRows],
    k: usize,
    csv: &super::StartCsv,
    line: usize,
    units: &Units,
) -> Result<super::StartCsv> {
    let table = &tables[k];
    let err = |m: String| definition_error(line, 1, m);
    let edges = table.ends_of.is_some();
    let mut csv = csv.clone();
    let no_ends = |path: &str| {
        err(format!(
            "{path}: an edge table's file has 'from' and 'to' columns (the ids of the rows each edge joins)"
        ))
    };
    if edges && (csv.column("from").is_none() || csv.column("to").is_none()) {
        return Err(no_ends(&csv.path));
    }
    for (name, unit, values) in &mut csv.columns {
        if edges && (name == "from" || name == "to") {
            continue;
        }
        if !table.columns.contains(name) {
            return Err(err(format!(
                "{}: the column '{name}' is not a quantity over {} (it has {})",
                csv.path,
                table.name,
                if table.columns.is_empty() {
                    "none".to_string()
                } else {
                    table.columns.join(", ")
                }
            )));
        }
        if let Some(u) = unit {
            let given = read_unit(u, line, 1)?;
            let want = units.get(name.as_str()).unwrap_or(Unit::DIMENSIONLESS);
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
    if let Some(m) = table.ends_of {
        let (Some(from), Some(to)) = (csv.column("from"), csv.column("to")) else {
            return Err(no_ends(&csv.path));
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
    Ok(csv)
}

/// The block's tables, in the order of their declarations; the tables
/// of refined cells' parents come after them (each table's `parents` is
/// its parents' place).
pub(super) fn tables(d: &Declared, s: &Shaped, units: &Units) -> Result<Vec<TableRows>> {
    let mut tables: Vec<TableRows> = d
        .grid
        .iter()
        .enumerate()
        .filter_map(|(i, a)| Some(table_rows(d, s, i, a.table?)))
        .collect();
    for k in 0..tables.len() {
        let Some(csv) = tables[k].start_file.clone() else {
            continue;
        };
        let line = d.starts.csv[&tables[k].name].1;
        let csv = start_file(&tables, k, &csv, line, units)?;
        tables[k].start_file = Some(std::sync::Arc::new(csv));
    }
    let n = tables.len();
    for (next, t) in (n..).zip(tables.iter_mut().filter(|t| t.parents.is_some())) {
        t.parents = Some(next);
    }
    Ok(tables)
}
