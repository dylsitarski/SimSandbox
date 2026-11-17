//! Using a built [`EquationSet`]: its programs, tables and rules, filling
//! registers, and the [`Equations`] and [`OdeSystem`] implementations.

use super::explicit::two_mut;
use super::{EquationSet, Kind, Layout, RowsSource, RuleRun, TableRows};
use crate::ode::OdeSystem;
use simsandbox_core::block::{BlockSpec, Equations, GeneratedCode};
use simsandbox_core::table::RowsFrame;
use simsandbox_core::Result;
use simsandbox_expr::{Outside, Program};

/// A frame's values for the rows `ids`, found by id: not a number for a
/// row the frame lacks, or with no frame.
fn by_id(frame: Option<&RowsFrame>, ids: &[u64]) -> Vec<f64> {
    match frame {
        Some(f) if f.ids == ids => f.values.clone(),
        Some(f) => {
            let at: std::collections::HashMap<u64, usize> =
                f.ids.iter().enumerate().map(|(k, &i)| (i, k)).collect();
            ids.iter()
                .map(|i| at.get(i).map_or(f64::NAN, |&k| f.values[k]))
                .collect()
        }
        None => vec![f64::NAN; ids.len()],
    }
}

impl EquationSet {
    /// How many phases of cells of its programs run compiled code.
    pub fn compiled_phases(&self) -> usize {
        [
            Some(&self.outputs),
            Some(&self.rates),
            self.init.as_ref(),
            self.rules_program.as_ref(),
        ]
        .into_iter()
        .flatten()
        .map(Program::compiled)
        .sum()
    }

    /// How its programs run, as `check` reports it: `interpreted` (small
    /// arrays, or `SIMSANDBOX_INTERPRET`), `compiled` (Cranelift), `native`
    /// (a library built from them), or `compiled (why not native)`; when
    /// its programs differ, each named (`rates: native, outputs: compiled`).
    pub fn code(&self) -> String {
        let programs = [
            ("rates", Some(&self.rates)),
            ("outputs", Some(&self.outputs)),
            ("starting values", self.init.as_ref()),
            ("rules", self.rules_program.as_ref()),
        ];
        // Each program's code, its phases said once.
        let each: Vec<(&str, String)> = programs
            .iter()
            .filter_map(|&(name, p)| {
                let mut codes: Vec<String> = Vec::new();
                for c in p?.codes() {
                    if !codes.contains(&c) {
                        codes.push(c);
                    }
                }
                (!codes.is_empty()).then(|| (name, codes.join(" and ")))
            })
            .collect();
        match each.first() {
            None => "interpreted".into(),
            Some((_, first)) if each.iter().all(|(_, c)| c == first) => first.clone(),
            Some(_) => each
                .iter()
                .map(|(name, c)| format!("{name}: {c}"))
                .collect::<Vec<_>>()
                .join(", "),
        }
    }

    /// Run every phase on the interpreter, for comparing with the
    /// compiled code.
    pub fn interpret(&mut self) {
        for p in [
            Some(&mut self.outputs),
            Some(&mut self.rates),
            self.init.as_mut(),
            self.rules_program.as_mut(),
        ]
        .into_iter()
        .flatten()
        {
            p.interpret();
        }
    }

    /// Compile the phases of cells of its programs (ADR 0084 point 6):
    /// `compiler` is given each phase's view and may return code to run
    /// in its place, which must give the interpreter's bits. Returns how
    /// many phases it compiled.
    pub fn compile_phases(
        &mut self,
        compiler: &mut dyn FnMut(
            &simsandbox_expr::PhaseView,
        )
            -> Option<std::sync::Arc<dyn simsandbox_expr::PhaseKernel>>,
    ) -> usize {
        let mut n = self.outputs.compile_phases(compiler) + self.rates.compile_phases(compiler);
        for p in [&mut self.init, &mut self.rules_program]
            .into_iter()
            .flatten()
        {
            n += p.compile_phases(compiler);
        }
        n
    }

    /// Whether it has tables (ADR 0079).
    pub fn has_tables(&self) -> bool {
        !self.tables.is_empty()
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// Its tables (ADR 0079).
    pub fn tables(&self) -> &[TableRows] {
        &self.tables
    }

    /// Its rules, in written order (ADR 0079).
    pub fn rules(&self) -> &[RuleRun] {
        &self.rules
    }

    /// The inputs the block's ports give (before the tables' row ids and
    /// the edges' ends).
    pub fn inputs_len(&self) -> usize {
        let ends: usize = self.tables.iter().map(|t| t.places() * t.rows).sum();
        self.layout.inputs - self.tables.iter().map(|t| t.rows).sum::<usize>() - ends
    }

    /// Each edge table's ends as places among its table's rows (ADR 0079
    /// stage 2), `from` then `to`: given after the ids. NaN for an end
    /// not among the rows.
    pub(super) fn ends_places(&self, tables: &[simsandbox_core::table::Table]) -> Vec<f64> {
        let mut u = Vec::new();
        let place_in = |m: usize| Places::new(tables[m].ids());
        for (t, spec) in tables.iter().zip(&self.tables) {
            let Some(m) = spec.ends_of else { continue };
            let place = place_in(m);
            let at = |place: &Places<'_>, id: f64| {
                if id < 0.0 {
                    return f64::NAN;
                }
                place.get(id as u64).map_or(f64::NAN, |k| k as f64)
            };
            for c in 0..spec.corners() {
                u.extend(t.column(c).iter().map(|&id| at(&place, id)));
            }
            // Sides: their cells' places, and the boundary.
            if let Some(cells) = spec.sides_of {
                let n = spec.corners();
                let place = place_in(cells);
                u.extend(t.column(n).iter().map(|&id| at(&place, id)));
                u.extend(t.column(n + 1).iter().map(|&id| at(&place, id)));
                u.extend(
                    t.column(n + 1)
                        .iter()
                        .map(|&id| f64::from(u8::from(id < 0.0))),
                );
                // Across: the outer cell, or the inner one on the boundary.
                u.extend(
                    t.column(n)
                        .iter()
                        .zip(t.column(n + 1))
                        .map(|(&i, &o)| at(&place, if o < 0.0 { i } else { o })),
                );
            }
        }
        u
    }

    /// The same equations compiled for these numbers of rows, one per
    /// table (ADR 0079).
    pub fn with_rows(&self, rows: &[usize]) -> Result<Self> {
        let rows = self
            .tables
            .iter()
            .zip(rows)
            .map(|(t, &n)| (t.name.clone(), n))
            .collect();
        Self::parse_with_rows(&self.source.0, &self.source.1, &rows)
    }

    /// All states, from the fixed ones (`fixed`) and the tables' columns.
    pub(super) fn gather(
        &self,
        fixed: &[f64],
        tables: &[simsandbox_core::table::Table],
    ) -> Vec<f64> {
        let mut x = Vec::with_capacity(self.layout.states);
        x.extend_from_slice(&fixed[..self.layout.fixed]);
        for &(t, c) in &self.row_states {
            x.extend_from_slice(tables[t].column(self.tables[t].offset() + c));
        }
        x
    }

    /// The inputs in declaration order, then each table's row ids: the
    /// fixed ones from `inputs`, those over a table's rows from the frames
    /// `rows_in` (ADR 0079 stage 1b), each row's value found by its id
    /// (not a number where the frame has no such row).
    pub(super) fn with_ids(
        &self,
        inputs: &[f64],
        rows_in: &[Vec<f64>],
        tables: &[simsandbox_core::table::Table],
    ) -> Vec<f64> {
        if self.inputs_in.iter().all(|i| i.1.is_none()) {
            let mut u = inputs.to_vec();
            for t in &tables[..self.tables.len()] {
                u.extend(t.ids().iter().map(|&i| i as f64));
            }
            u.extend(self.ends_places(tables));
            return u;
        }
        let mut u = Vec::with_capacity(self.layout.inputs);
        let (mut fixed, mut k) = (0, 0);
        for &(len, table) in &self.inputs_in {
            match table {
                None => {
                    let end = (fixed + len).min(inputs.len());
                    u.extend_from_slice(&inputs[fixed.min(end)..end]);
                    u.resize(u.len() + (fixed + len - end), 0.0);
                    fixed += len;
                }
                Some(t) => {
                    let frame = rows_in.get(k).and_then(|f| RowsFrame::decode(f, 1));
                    u.extend(by_id(frame.as_ref(), tables[t].ids()));
                    k += 1;
                }
            }
        }
        for t in &tables[..self.tables.len()] {
            u.extend(t.ids().iter().map(|&i| i as f64));
        }
        u.extend(self.ends_places(tables));
        u
    }

    /// Write each output over a table's rows as a frame: the table's
    /// version and ids, and the values (ADR 0079 stage 1b). After
    /// [`Equations::outputs`], whose registers hold the defined ones.
    pub(super) fn rows_outputs(
        &self,
        tables: &[simsandbox_core::table::Table],
        out: &mut [Vec<f64>],
    ) {
        let v = &self.scratch.borrow()[0];
        for ((t, src), frame) in self.rows_out.iter().zip(out) {
            let table = &tables[*t];
            frame.clear();
            frame.push(table.version() as f64);
            frame.push(table.len() as f64);
            frame.extend(table.ids().iter().map(|&i| i as f64));
            match *src {
                // After a table's corners (and sides' cells, refinement's
                // columns): its states.
                RowsSource::Column(c) => {
                    frame.extend_from_slice(table.column(self.tables[*t].offset() + c))
                }
                RowsSource::Registers(s) => frame.extend_from_slice(&v[s.at..s.at + s.len]),
                // Row by row, each cell's corners.
                RowsSource::Corners(n) => {
                    for r in 0..table.len() {
                        frame.extend((0..n).map(|k| table.column(k)[r]));
                    }
                }
            }
        }
    }

    /// Keep every table of sides up to date with its cells (ADR 0085): a
    /// side still there keeps its row and id.
    pub(super) fn refresh_sides(&self, tables: &mut [simsandbox_core::table::Table]) {
        for (f, t) in self.tables.iter().enumerate() {
            let Some(c) = t.sides_of else { continue };
            let shape = self.tables[c].shape.expect("cells");
            let (_, faces) = shape.sides().expect("checked: one shape of side");
            // Refined boxes have small sides on large ones (ADR 0085).
            let hanging = self.tables[c]
                .parents
                .filter(|_| !shape.simplicial())
                .map(|p| {
                    (
                        std::mem::take(&mut tables[p]),
                        crate::refine::box_side_axes(shape).expect("a box"),
                    )
                });
            let (cells, sides) = two_mut(tables, c, f);
            crate::mesh::update_sides(
                cells,
                sides,
                shape.corners(),
                &faces,
                hanging.as_ref().map(|(p, a)| (p, a.as_slice())),
            );
            if let (Some((p, _)), Some(at)) = (hanging, self.tables[c].parents) {
                tables[at] = p;
            }
        }
    }

    /// Every state value: the fixed ones, then those over rows.
    pub fn states_len(&self) -> usize {
        self.layout.states
    }

    /// The fixed state values (those the block keeps outside its tables).
    pub fn fixed_len(&self) -> usize {
        self.layout.fixed
    }

    /// The starting state, with `u` the inputs (the tables' row ids after
    /// the block's own inputs).
    pub fn initial_state_with(&self, p: &[f64], u: &[f64], x0: &mut [f64]) {
        let zero = vec![0.0; self.layout.states];
        self.initial_state_from(p, &zero, u, x0)
    }

    /// As [`EquationSet::initial_state_with`], the starting values reading
    /// the states `known` (a starting mesh's coordinates, ADR 0085).
    pub fn initial_state_from(&self, p: &[f64], known: &[f64], u: &[f64], x0: &mut [f64]) {
        let mut scratch = self.scratch.borrow_mut();
        if let Some(init) = &self.init {
            let v = &mut scratch[2];
            self.fill(0.0, known, u, p, v);
            init.run(v);
            x0.copy_from_slice(&v[1..=self.layout.states]);
        }
        let mut at = 0;
        for &(len, param) in &self.initial {
            if let Some(i) = param {
                x0[at] = p[i];
            }
            at += len;
        }
    }

    /// For each rate, the states it reads, with the places of a mesh's
    /// corners and sides read from the inputs `u` (ADR 0085 step 3): as
    /// sparse as the mesh.
    pub fn sparsity_at(&self, t: f64, x: &[f64], u: &[f64], p: &[f64]) -> Vec<Vec<usize>> {
        let ns = self.layout.states;
        let at = self.layout.rates_at;
        let mut v = self.scratch.borrow()[1].clone();
        self.fill(t, x, u, p, &mut v);
        self.rates.run(&mut v);
        self.rates.dependencies_at(Some(&v), 1..1 + ns, at..at + ns)
    }

    /// Run the rules' program on `x` (after a step): the registers, from
    /// which [`RuleRun`]'s spans read each rule's values.
    pub fn rule_registers(&self, t: f64, x: &[f64], u: &[f64], p: &[f64]) -> Vec<f64> {
        let Some(program) = &self.rules_program else {
            return Vec::new();
        };
        let v = &mut self.scratch.borrow_mut()[3];
        self.fill(t, x, u, p, v);
        program.run(v);
        v.clone()
    }

    /// Put time `t`, states `x`, inputs `u` and parameters `p` in the
    /// registers `v`. Missing inputs read as 0.
    fn fill(&self, t: f64, x: &[f64], u: &[f64], p: &[f64], v: &mut [f64]) {
        let ns = self.layout.states;
        v[1..=ns].copy_from_slice(&x[..ns]);
        self.fill_rest(t, u, p, v);
    }

    /// As [`EquationSet::fill`], with only the parts `states` of the
    /// states.
    #[allow(clippy::too_many_arguments)]
    fn fill_parts(
        &self,
        t: f64,
        x: &[f64],
        states: &[std::ops::Range<usize>],
        u: &[f64],
        p: &[f64],
        v: &mut [f64],
    ) {
        for r in states {
            v[1 + r.start..1 + r.end].copy_from_slice(&x[r.clone()]);
        }
        self.fill_rest(t, u, p, v);
    }

    /// Time, inputs and parameters into the registers.
    fn fill_rest(&self, t: f64, u: &[f64], p: &[f64], v: &mut [f64]) {
        let Layout {
            states: ns,
            inputs: ni,
            params: np,
            ..
        } = self.layout;
        v[0] = t;
        for (k, slot) in v[1 + ns..1 + ns + ni].iter_mut().enumerate() {
            *slot = u.get(k).copied().unwrap_or(0.0);
        }
        v[1 + ns + ni..1 + ns + ni + np].copy_from_slice(&p[..np]);
    }
}

/// Where each of a table's rows lies, by its id: ids in ascending order
/// (rows added at their end, as tables grow) are searched for, others
/// looked up in a map. Ids are unique, so both find the same place
/// (2026-11-17: building the map at every evaluation was a sixth of a
/// step of the bridge network).
enum Places<'a> {
    Ascending(&'a [u64]),
    Map(std::collections::HashMap<u64, usize>),
}

impl<'a> Places<'a> {
    fn new(ids: &'a [u64]) -> Self {
        if ids.windows(2).all(|w| w[0] < w[1]) {
            Places::Ascending(ids)
        } else {
            Places::Map(ids.iter().enumerate().map(|(k, &i)| (i, k)).collect())
        }
    }

    fn get(&self, id: u64) -> Option<usize> {
        match self {
            Places::Ascending(ids) => ids.binary_search(&id).ok(),
            Places::Map(map) => map.get(&id).copied(),
        }
    }
}

impl Equations for EquationSet {
    fn rhs(&self, t: f64, x: &[f64], u: &[f64], p: &[f64], dx: &mut [f64]) {
        let v = &mut self.scratch.borrow_mut()[1];
        let (ns, at) = (self.layout.states, self.layout.rates_at);
        let copies = &self.rates_copy;
        if !copies.placed || dx.len() < ns {
            self.fill(t, x, u, p, v);
            self.rates.run(v);
            let n = dx.len().min(ns);
            dx[..n].copy_from_slice(&v[at..at + n]);
            return;
        }
        // Large arrays read from `x` and written to `dx` where they are.
        self.fill_parts(t, x, &copies.states, u, p, v);
        self.rates.run_outside(
            v,
            Outside {
                reads: &x[..ns],
                reads_at: 1,
                writes: &mut dx[..ns],
                writes_at: at,
            },
        );
        for r in &copies.results {
            dx[r.clone()].copy_from_slice(&v[at + r.start..at + r.end]);
        }
    }

    fn outputs(&self, t: f64, x: &[f64], u: &[f64], p: &[f64], y: &mut [f64]) {
        let v = &mut self.scratch.borrow_mut()[0];
        let ns = self.layout.states;
        let (at, len) = (self.layout.outputs_at, self.layout.outputs_len);
        let copies = &self.outputs_copy;
        if !copies.placed {
            // The fixed states are outputs; states over rows are not.
            let nf = self.layout.fixed;
            y[..nf].copy_from_slice(&x[..nf]);
            self.fill(t, x, u, p, v);
            self.outputs.run(v);
            y[nf..nf + len].copy_from_slice(&v[at..at + len]);
            return;
        }
        y[..ns].copy_from_slice(&x[..ns]);
        self.fill_parts(t, x, &copies.states, u, p, v);
        // Large arrays read from `x` and written to `y` where they are.
        let (_, rest) = y.split_at_mut(ns);
        self.outputs.run_outside(
            v,
            Outside {
                reads: &x[..ns],
                reads_at: 1,
                writes: &mut rest[..len],
                writes_at: at,
            },
        );
        for r in &copies.results {
            y[ns + r.start..ns + r.end].copy_from_slice(&v[at + r.start..at + r.end]);
        }
    }
}

impl GeneratedCode for EquationSet {
    fn code(&self) -> String {
        EquationSet::code(self)
    }
}

impl OdeSystem for EquationSet {
    fn spec(&self) -> &BlockSpec {
        &self.spec
    }

    fn as_generated_code(&self) -> Option<&dyn GeneratedCode> {
        Some(self)
    }

    /// Exact: each rate's dependencies on the states, read from the
    /// instructions that compute it (ADR 0060).
    fn sparsity(&self) -> Option<Vec<Vec<usize>>> {
        let ns = self.layout.states;
        let at = self.layout.rates_at;
        Some(self.rates.dependencies(1..1 + ns, at..at + ns))
    }

    fn initial_state(&self, p: &[f64], x0: &mut [f64]) {
        self.initial_state_with(p, &[], x0)
    }
}

#[cfg(test)]
mod places_tests {
    use super::Places;

    #[test]
    fn places_by_id_are_the_same_searched_or_mapped() {
        for ids in [
            vec![10u64, 11, 12, 13],
            vec![13, 10, 12, 11],
            vec![],
            vec![7],
        ] {
            let p = Places::new(&ids);
            for (k, &id) in ids.iter().enumerate() {
                assert_eq!(p.get(id), Some(k), "{ids:?}");
            }
            assert_eq!(p.get(99), None);
        }
        assert!(matches!(Places::new(&[1, 3, 5]), Places::Ascending(_)));
        assert!(matches!(Places::new(&[3, 1]), Places::Map(_)));
    }
}
