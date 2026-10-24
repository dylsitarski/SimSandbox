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
}

impl Table {
    /// An empty table with `columns` columns.
    pub fn new(columns: usize) -> Self {
        Self {
            ids: Vec::new(),
            columns: vec![Vec::new(); columns],
            next_id: 0,
        }
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
        id
    }

    /// Keep the rows for which `keep(row index)` is true, in order.
    pub fn retain(&mut self, mut keep: impl FnMut(usize) -> bool) {
        let keep: Vec<bool> = (0..self.len()).map(&mut keep).collect();
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
