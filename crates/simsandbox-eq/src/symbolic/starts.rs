//! How tables start (ADR 0079, ADR 0069 step 3, ADR 0085): a number of
//! rows, a chain, a CSV file, a grid of cells or a mesh file.

use super::statements::{as_read, definition_error, is_name, StartHow, StartLine, Statement};
use super::Axis;
use simsandbox_core::Result;

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

/// How many sides the starting cells of axis `c` have (ADR 0085):
/// `faces` are a side's corners among its cell's.
pub(super) fn starting_sides(
    grid: &[Axis],
    c: usize,
    faces: &[Vec<usize>],
    starts: &Starts,
) -> usize {
    let name = &grid[c].name;
    let corners: Vec<Vec<usize>> = match (starts.meshes.get(name), starts.chains.get(name)) {
        (Some((_, cells)), _) => cells.clone(),
        (_, Some(_)) => {
            let n = grid[c]
                .ends_of
                .and_then(|p| starts.rows.get(&grid[p].name))
                .map_or(0, |s| s.0);
            (1..n).map(|k| vec![k - 1, k]).collect()
        }
        _ => Vec::new(),
    };
    let mut seen = std::collections::HashSet::new();
    for cell in &corners {
        for f in faces {
            let mut key: Vec<usize> = f.iter().map(|&k| cell[k]).collect();
            key.sort_unstable();
            seen.insert(key);
        }
    }
    seen.len()
}

/// How every table starts, as its `start` line and the `cells` lines say:
/// what [`read_starts`] establishes for the axes and everything after.
/// Each start has the line it is written on; a table has one at most.
pub(super) struct Starts {
    /// `start pts = 5`: a number of rows. A table or an edge table read
    /// from a CSV file has its file's rows here too, or the rows a run
    /// has when compiled again for them.
    pub(super) rows: std::collections::HashMap<String, (usize, usize)>,
    /// `start segs = chain` (ADR 0079 stage 2): an edge from each
    /// starting row to the next.
    pub(super) chains: std::collections::HashMap<String, usize>,
    /// `start pts = file "nodes.csv"` (ADR 0069 step 3), when first
    /// compiled.
    pub(super) csv: std::collections::HashMap<String, (std::sync::Arc<StartCsv>, usize)>,
    /// `start tri = grid …` or `= file "part.msh" at x, y` (ADR 0085):
    /// cells, which also make their points.
    pub(super) specs: std::collections::HashMap<String, (MeshSpec, usize)>,
    /// Each cell table's points and shape, as declared (`cells tri of
    /// pts: triangle`).
    pub(super) cells: std::collections::HashMap<String, (String, crate::mesh::CellShape)>,
    /// The starting meshes, when first compiled (later the rows are known).
    pub(super) meshes: std::collections::HashMap<String, crate::mesh::Start>,
}

impl Starts {
    /// The cells whose starting mesh makes the rows of the table `points`.
    pub(super) fn making<'a>(
        &'a self,
        points: &'a str,
    ) -> impl Iterator<Item = (&'a String, &'a (MeshSpec, usize))> {
        self.specs
            .iter()
            .filter(move |(c, _)| self.cells.get(*c).is_some_and(|o| o.0 == points))
    }

    /// The rows a chain over the table `points` starts with: an edge
    /// from each starting row to the next.
    pub(super) fn chain_rows(&self, points: &str) -> usize {
        self.rows.get(points).map_or(0, |s| s.0).saturating_sub(1)
    }
}

/// The `cells` lines, then the `start` lines, in written order: the
/// first passes over a definition, since a grid of cells makes its
/// points' rows wherever the lines are. `rows` are the rows a run has,
/// when compiled again for them.
pub(super) fn read_starts(
    statements: &[Statement],
    rows: &std::collections::BTreeMap<String, usize>,
) -> Result<Starts> {
    let mut starts = Starts {
        rows: Default::default(),
        chains: Default::default(),
        csv: Default::default(),
        specs: Default::default(),
        cells: Default::default(),
        meshes: Default::default(),
    };
    // Read first: a grid start makes the points too.
    for s in statements {
        if let Statement::Cells { read, .. } = s {
            let (name, of, _, shape) = as_read(read)?.clone();
            starts.cells.insert(name, (of, shape));
        }
    }
    // Each table's `start` line: one at most.
    let mut started: std::collections::HashMap<String, usize> = Default::default();
    for s in statements {
        let Statement::Start { line, read } = s else {
            continue;
        };
        let line = *line;
        let start = as_read(read)?;
        let t = &start.table;
        if let Some(first) = started.insert(t.clone(), line) {
            return Err(definition_error(
                line,
                1,
                format!("{t} has two starts, on lines {first} and {line}: give it one"),
            ));
        }
        match &start.how {
            StartHow::Chain => {
                starts.chains.insert(t.clone(), line);
            }
            // Rows from a CSV file (ADR 0069 step 3); cells from a mesh
            // file are below.
            StartHow::File { csv, .. } if !starts.cells.contains_key(t) => {
                let path = csv.clone().map_err(|()| {
                    definition_error(
                        line,
                        1,
                        "write a table's file as 'start pts = file \"nodes.csv\"'",
                    )
                })?;
                let t = t.clone();
                let is_edges = statements
                    .iter()
                    .any(|s| matches!(s, Statement::Edges { first: Some(f), .. } if *f == t));
                // Compiled again for the rows a run has (the file is read
                // when first compiled, in the model's folder, as a mesh is).
                if let Some(&n) = rows.get(&t) {
                    if !is_edges {
                        starts.rows.insert(t, (n, line));
                    }
                    continue;
                }
                let csv = StartCsv::load(&path).map_err(|e| definition_error(line, 1, e))?;
                if !is_edges {
                    starts.rows.insert(t.clone(), (csv.rows, line));
                }
                starts.csv.insert(t, (csv, line));
            }
            StartHow::File { mesh: spec, .. } | StartHow::Grid(spec) => {
                let spec = spec.clone().map_err(|e| definition_error(line, 1, e))?;
                starts.specs.insert(t.clone(), (spec, line));
            }
            StartHow::Rows(n) => {
                starts.rows.insert(t.clone(), (*n, line));
            }
            StartHow::Unknown => return Err(StartLine::unknown(line)),
        }
    }
    let fresh: std::collections::HashMap<String, (MeshSpec, usize)> = starts
        .specs
        .iter()
        .filter(|(c, _)| !rows.contains_key(*c))
        .map(|(c, v)| (c.clone(), v.clone()))
        .collect();
    starts.meshes = start_meshes(&fresh, &starts.cells)?;
    Ok(starts)
}
