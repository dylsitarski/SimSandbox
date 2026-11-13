//! Tables (ADR 0034): populations of rows that grow and shrink, owned by
//! the kernel beside a Block's fixed state, so snapshots, rewind and saving
//! treat them like any state. Each row has a stable id given at birth and
//! never reused within a run.

/// A table a Block declares: its name and its columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableSpec {
    pub name: String,
    pub columns: Vec<String>,
}

impl TableSpec {
    pub fn new(name: &str, columns: &[&str]) -> Self {
        Self {
            name: name.into(),
            columns: columns.iter().map(|c| c.to_string()).collect(),
        }
    }
}

/// A table's rows, stored column by column, oldest first.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Table {
    ids: Vec<u64>,
    columns: Vec<Vec<f64>>,
    next_id: u64,
    /// Counts changes to its rows (ADR 0079): rows added, removed or
    /// replaced. Equal versions mean the same rows in the same order.
    version: u64,
}

impl Table {
    /// An empty table with `columns` columns.
    pub fn new(columns: usize) -> Self {
        Self {
            ids: Vec::new(),
            columns: vec![Vec::new(); columns],
            next_id: 0,
            version: 0,
        }
    }

    /// Its structure version (ADR 0079): it grows with each change to the
    /// rows, never otherwise.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Make its rows those with `ids` (another table's, followed by id),
    /// each row's values given by `row(id, place among the old rows)`;
    /// ids then continue past the largest. A change of rows is a new
    /// version.
    pub fn assign(&mut self, ids: &[u64], mut row: impl FnMut(u64, Option<usize>) -> Vec<f64>) {
        if ids == self.ids.as_slice() {
            return;
        }
        let old: std::collections::HashMap<u64, usize> =
            self.ids.iter().enumerate().map(|(k, &i)| (i, k)).collect();
        let rows: Vec<Vec<f64>> = ids.iter().map(|&i| row(i, old.get(&i).copied())).collect();
        for c in &mut self.columns {
            c.clear();
        }
        for r in &rows {
            assert_eq!(r.len(), self.columns.len(), "one value per column");
            for (c, &v) in self.columns.iter_mut().zip(r) {
                c.push(v);
            }
        }
        self.ids = ids.to_vec();
        self.next_id = self.next_id.max(ids.iter().max().map_or(0, |m| m + 1));
        self.version += 1;
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Each row's id, oldest first.
    pub fn ids(&self) -> &[u64] {
        &self.ids
    }

    pub fn column(&self, c: usize) -> &[f64] {
        &self.columns[c]
    }

    pub fn column_mut(&mut self, c: usize) -> &mut [f64] {
        &mut self.columns[c]
    }

    pub fn columns(&self) -> usize {
        self.columns.len()
    }

    /// Row `r`'s values, one per column.
    pub fn row(&self, r: usize) -> Vec<f64> {
        self.columns.iter().map(|c| c[r]).collect()
    }

    /// Add a row (one value per column) and return its new id.
    ///
    /// # Panics
    /// If `values` does not have one value per column.
    pub fn push(&mut self, values: &[f64]) -> u64 {
        assert_eq!(values.len(), self.columns.len(), "one value per column");
        for (c, &v) in self.columns.iter_mut().zip(values) {
            c.push(v);
        }
        let id = self.next_id;
        self.next_id += 1;
        self.ids.push(id);
        self.version += 1;
        id
    }

    /// Keep the rows for which `keep(row index)` is true, in order.
    pub fn retain(&mut self, mut keep: impl FnMut(usize) -> bool) {
        let keep: Vec<bool> = (0..self.len()).map(&mut keep).collect();
        if keep.iter().any(|k| !k) {
            self.version += 1;
        }
        let mut k = keep.iter();
        self.ids.retain(|_| *k.next().expect("one flag per row"));
        for c in &mut self.columns {
            let mut k = keep.iter();
            c.retain(|_| *k.next().expect("one flag per row"));
        }
    }

    /// Remove every row and start ids from 0 again (a new run).
    pub fn reset(&mut self) {
        self.ids.clear();
        for c in &mut self.columns {
            c.clear();
        }
        self.next_id = 0;
        self.version = 0;
    }
}

/// One frame of a rows port (ADR 0079): the table's structure version,
/// its rows' ids, and `dim` values per row, row after row. Carried on the
/// kernel's variable-length channel as `[version, n, ids…, values…]`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RowsFrame {
    pub version: u64,
    pub ids: Vec<u64>,
    pub values: Vec<f64>,
}

impl RowsFrame {
    /// As carried on a port.
    pub fn encode(&self) -> Vec<f64> {
        let mut v = Vec::with_capacity(2 + self.ids.len() + self.values.len());
        v.push(self.version as f64);
        v.push(self.ids.len() as f64);
        v.extend(self.ids.iter().map(|&i| i as f64));
        v.extend_from_slice(&self.values);
        v
    }

    /// From a port's frame, `dim` values per row; `None` if it is not one
    /// (empty before anything was sent).
    pub fn decode(v: &[f64], dim: usize) -> Option<Self> {
        let (&version, rest) = v.split_first()?;
        let (&n, rest) = rest.split_first()?;
        let n = n as usize;
        if rest.len() != n * (1 + dim) {
            return None;
        }
        Some(Self {
            version: version as u64,
            ids: rest[..n].iter().map(|&i| i as u64).collect(),
            values: rest[n..].to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_count_changes_and_rows_follow_by_id() {
        // ADR 0079: a version per change of rows; assign keeps values by id.
        let mut t = Table::new(1);
        assert_eq!(t.version(), 0);
        t.push(&[10.0]);
        t.push(&[11.0]);
        t.push(&[12.0]);
        assert_eq!(t.version(), 3);
        t.retain(|_| true);
        assert_eq!(t.version(), 3, "nothing removed: the same rows");
        t.retain(|r| r != 1);
        assert_eq!((t.version(), t.ids()), (4, &[0u64, 2][..]));
        // Follow other rows: 2 stays, 7 is new, 0 goes.
        t.assign(&[2, 7], |id, old| {
            vec![old.map_or(-1.0, |k| 100.0 + k as f64 + id as f64)]
        });
        assert_eq!(t.ids(), [2, 7]);
        assert_eq!(t.column(0), [103.0, -1.0]);
        assert_eq!(t.version(), 5);
        t.assign(&[2, 7], |_, _| unreachable!("the same rows: nothing to do"));
        assert_eq!(t.push(&[0.0]), 8, "ids continue past the largest");
        // A frame on a port and back.
        let f = RowsFrame {
            version: 6,
            ids: vec![2, 7, 8],
            values: vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        };
        assert_eq!(RowsFrame::decode(&f.encode(), 2), Some(f.clone()));
        assert_eq!(RowsFrame::decode(&f.encode(), 1), None);
        assert_eq!(RowsFrame::decode(&[], 1), None);
    }

    #[test]
    fn rows_keep_their_ids_through_births_and_deaths() {
        let mut t = Table::new(2);
        for k in 0..5 {
            assert_eq!(t.push(&[k as f64, 10.0 * k as f64]), k);
        }
        t.retain(|r| r % 2 == 0);
        assert_eq!(t.ids(), [0, 2, 4]);
        assert_eq!(t.column(1), [0.0, 20.0, 40.0]);
        assert_eq!(t.row(1), [2.0, 20.0]);
        // Ids are never reused within a run.
        assert_eq!(t.push(&[9.0, 9.0]), 5);
        t.column_mut(0)[0] = -1.0;
        assert_eq!(t.row(0), [-1.0, 0.0]);
        t.reset();
        assert!(t.is_empty());
        assert_eq!(t.push(&[0.0, 0.0]), 0);
    }
}
