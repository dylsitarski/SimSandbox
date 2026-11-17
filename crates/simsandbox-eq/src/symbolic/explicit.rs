//! [`ExplicitBlock`]: maps and formulas as a kernel Block (ADR 0019).

use super::{CellTransfer, EquationSet, Kind, RuleKind, RuleRun, EVALUATE, ITERATE};
use crate::ode::OdeSystem;
use simsandbox_core::block::{
    AccuracyClass, Block, BlockSpec, Equations, MethodLabels, MethodSpec, SpeedClass, StepContext,
    StepIo,
};
use simsandbox_core::registry::MethodSettings;
use simsandbox_core::table::RowsFrame;
use simsandbox_core::{Result, SimError};
use std::cell::RefCell;
/// A map or a set of formulas as a kernel Block (ADR 0019): maps are
/// iterated once per communication step, formulas are evaluated. Neither
/// approximates anything, so there is no method to choose (ADR 0020).
pub struct ExplicitBlock {
    set: EquationSet,
    method: MethodSpec,
    next: Vec<f64>,
    /// With tables (ADR 0079): its equations compiled for each number of
    /// rows met, one per table (rows change only between steps).
    by_rows: RefCell<std::collections::HashMap<Vec<usize>, EquationSet>>,
    /// Continuous states over rows (ADR 0085 step 3): the method that
    /// integrates them, its workspace, and the tables' versions it was
    /// set up for (a new structure starts it afresh).
    integrator: Option<crate::integrators::Integrator>,
    workspace: crate::implicit::Workspace,
    versions: Vec<u64>,
}

/// Compiled sets kept per numbers of rows, at most.
const KEPT_ROW_COUNTS: usize = 64;

/// Two different tables, both to change.
pub(super) fn two_mut(
    tables: &mut [simsandbox_core::table::Table],
    a: usize,
    b: usize,
) -> (
    &mut simsandbox_core::table::Table,
    &mut simsandbox_core::table::Table,
) {
    assert_ne!(a, b, "two different tables");
    if a < b {
        let (lo, hi) = tables.split_at_mut(b);
        (&mut lo[a], &mut hi[0])
    } else {
        let (lo, hi) = tables.split_at_mut(a);
        (&mut hi[0], &mut lo[b])
    }
}

/// Remove the edges of edge table `et` that end at a row no longer in
/// table `tb` (ADR 0079: edges go with the rows they join).
fn drop_dangling(
    tables: &mut [simsandbox_core::table::Table],
    tb: usize,
    et: usize,
    corners: usize,
) {
    let (members, edges) = two_mut(tables, tb, et);
    let ids: std::collections::HashSet<u64> = members.ids().iter().copied().collect();
    let cols: Vec<Vec<f64>> = (0..corners).map(|c| edges.column(c).to_vec()).collect();
    edges.retain(|k| cols.iter().all(|c| ids.contains(&(c[k] as u64))));
}

/// Split or merge cells `et` over points `tb` where `cond` holds (ADR
/// 0085 step 2), with the rule's transfers; a merge removes the points
/// it frees that no cells or edges use.
fn refine_cells(
    set: &EquationSet,
    rule: &RuleRun,
    et: usize,
    tb: usize,
    v: &[f64],
    cond: &[f64],
    tables: &mut [simsandbox_core::table::Table],
) -> Result<()> {
    use crate::refine::{Mesh, Transfer, Transfers};
    let spec = &set.tables()[et];
    let n = tables[et].len();
    let marked: Vec<bool> = cond.iter().map(|&c| c != 0.0 && !c.is_nan()).collect();
    let given = |k: usize| -> Vec<f64> {
        let s = rule.values[k];
        if s.len == n {
            v[s.at..s.at + s.len].to_vec()
        } else {
            vec![v[s.at]; n]
        }
    };
    let to = |t: CellTransfer| match t {
        CellTransfer::Copy => Transfer::Copy,
        CellTransfer::Share => Transfer::Share,
        CellTransfer::Mean => Transfer::Mean,
        CellTransfer::Sum => Transfer::Sum,
        CellTransfer::Given(k) => Transfer::Given(given(k)),
    };
    let mut transfers = Transfers {
        cells: vec![Transfer::Copy; spec.columns.len()],
        points: vec![Transfer::Mean; set.tables()[tb].columns.len()],
    };
    for &(on_cells, k, t) in &rule.transfers {
        match on_cells {
            true => transfers.cells[k] = to(t),
            false => transfers.points[k] = to(t),
        }
    }
    let pi = spec.parents.expect("refined cells");
    let mut cells = std::mem::take(&mut tables[et]);
    let mut points = std::mem::take(&mut tables[tb]);
    let mut parents = std::mem::take(&mut tables[pi]);
    let mut mesh = Mesh {
        shape: spec.shape.expect("cells"),
        cells: &mut cells,
        parents: &mut parents,
        points: &mut points,
        point_at: set.tables()[tb].offset(),
        max_cells: spec.max,
        max_points: set.tables()[tb].max,
    };
    let result = match rule.kind {
        RuleKind::Split => crate::refine::split(&mut mesh, &marked, &transfers).map(|_| Vec::new()),
        _ => Ok(crate::refine::merge(&mut mesh, &marked, &transfers)),
    };
    tables[et] = cells;
    tables[tb] = points;
    tables[pi] = parents;
    let freed = result.map_err(|e| SimError::Block(format!("{}: {e}", spec.name)))?;
    if freed.is_empty() {
        return Ok(());
    }
    // Points no cells or edges use any more go.
    let mut used = std::collections::HashSet::new();
    for (other, o) in set.tables().iter().enumerate() {
        if o.ends_of == Some(tb) && o.sides_of.is_none() {
            for c in 0..o.corners() {
                used.extend(tables[other].column(c).iter().map(|&p| p as u64));
            }
        }
    }
    let ids = tables[tb].ids().to_vec();
    tables[tb].retain(|r| !freed.contains(&ids[r]) || used.contains(&ids[r]));
    Ok(())
}

/// Merge (ADR 0079 stage 2): for each edge where `holds(k)`, in edge
/// order, its two rows become one, unless either was merged already by
/// this rule. The `from` row stays, with the values `values(k)`; the `to`
/// row goes, its edges moved to the `from` row; the merged edge goes,
/// and so do edges that would join a row to itself or repeat an earlier
/// edge between the same two rows.
fn merge(
    members: &mut simsandbox_core::table::Table,
    edges: &mut simsandbox_core::table::Table,
    holds: impl Fn(usize) -> bool,
    values: impl Fn(usize) -> Vec<f64>,
) {
    let (from, to) = (edges.column(0).to_vec(), edges.column(1).to_vec());
    let mut used = std::collections::HashSet::new();
    let mut into: std::collections::HashMap<u64, u64> = Default::default();
    let mut merged = vec![false; from.len()];
    for k in 0..from.len() {
        let (a, b) = (from[k] as u64, to[k] as u64);
        if !holds(k) || a == b || used.contains(&a) || used.contains(&b) {
            continue;
        }
        used.insert(a);
        used.insert(b);
        merged[k] = true;
        into.insert(b, a);
        if let Some(r) = members.ids().iter().position(|&i| i == a) {
            for (c, v) in values(k).into_iter().enumerate() {
                members.column_mut(c)[r] = v;
            }
        }
    }
    if into.is_empty() {
        return;
    }
    let ids: Vec<u64> = members.ids().to_vec();
    members.retain(|r| !into.contains_key(&ids[r]));
    let moved = |id: f64| into.get(&(id as u64)).map_or(id, |&a| a as f64);
    let (new_from, new_to): (Vec<f64>, Vec<f64>) = from
        .iter()
        .zip(&to)
        .map(|(&f, &t)| (moved(f), moved(t)))
        .unzip();
    let mut seen = std::collections::HashSet::new();
    let keep: Vec<bool> = (0..from.len())
        .map(|k| {
            let (f, t) = (new_from[k] as u64, new_to[k] as u64);
            !merged[k] && f != t && seen.insert((f.min(t), f.max(t)))
        })
        .collect();
    edges.column_mut(0).copy_from_slice(&new_from);
    edges.column_mut(1).copy_from_slice(&new_to);
    edges.retain(|k| keep[k]);
}

impl ExplicitBlock {
    /// Wrap a map or formula set; `method` must be `iterate` for maps and
    /// `evaluate` for formulas.
    pub fn new(set: EquationSet, method: &MethodSettings) -> Result<Self> {
        let (expected, label) = match set.kind {
            Kind::Map => (ITERATE, "Iterate the map once per step"),
            Kind::Formulas => (EVALUATE, "Evaluate the formulas"),
            Kind::Continuous => {
                return Err(SimError::InvalidMethod(
                    "continuous equations need an ODE method such as rk4".into(),
                ))
            }
        };
        if method.name != expected || !method.options.is_empty() {
            let what = if set.kind == Kind::Map {
                "maps are iterated"
            } else {
                "formulas are evaluated"
            };
            return Err(SimError::InvalidMethod(format!(
                "{what}: use method '{expected}' with no options"
            )));
        }
        let states = set.spec.states.len();
        Ok(Self {
            set,
            method: MethodSpec {
                name: label.into(),
                // No discretisation error, but not benchmarked case by case.
                labels: MethodLabels {
                    speed: SpeedClass::RealTime,
                    accuracy: AccuracyClass::Quantitative,
                },
            },
            next: vec![0.0; states],
            by_rows: RefCell::new(Default::default()),
            integrator: None,
            workspace: Default::default(),
            versions: Vec::new(),
        })
    }

    /// Continuous equations with tables (ADR 0085 step 3): states over
    /// rows integrated by `integrator` with the tables fixed within each
    /// step, and the rules applied between steps.
    pub fn continuous(set: EquationSet, integrator: crate::integrators::Integrator) -> Self {
        let states = set.spec.states.len();
        Self {
            method: integrator.method_spec(),
            set,
            next: vec![0.0; states],
            by_rows: RefCell::new(Default::default()),
            integrator: Some(integrator),
            workspace: Default::default(),
            versions: Vec::new(),
        }
    }

    /// The equations compiled for these tables' numbers of rows.
    fn compiled(
        &self,
        tables: &[simsandbox_core::table::Table],
    ) -> Result<std::cell::Ref<'_, EquationSet>> {
        let n = self.set.tables.len();
        let rows: Vec<usize> = tables[..n].iter().map(|t| t.len()).collect();
        self.compiled_for(rows)
    }

    /// The equations compiled for these numbers of rows, one per table.
    fn compiled_for(&self, rows: Vec<usize>) -> Result<std::cell::Ref<'_, EquationSet>> {
        if !self.by_rows.borrow().contains_key(&rows) {
            let set = self.set.with_rows(&rows)?;
            let mut kept = self.by_rows.borrow_mut();
            if kept.len() >= KEPT_ROW_COUNTS {
                kept.clear();
            }
            kept.insert(rows.clone(), set);
        }
        Ok(std::cell::Ref::map(self.by_rows.borrow(), |kept| {
            &kept[&rows]
        }))
    }

    /// Make each table that follows an input's rows hold those rows, by id
    /// (ADR 0079 stage 1b): rows kept keep their values; new rows take
    /// their states' starting values.
    fn follow(
        &self,
        params: &[f64],
        rows_in: &[Vec<f64>],
        tables: &mut [simsandbox_core::table::Table],
    ) -> Result<()> {
        for (tb, t) in self.set.tables.iter().enumerate() {
            let Some(k) = t.follows else {
                continue;
            };
            let ids = rows_in
                .get(k)
                .and_then(|f| RowsFrame::decode(f, 1))
                .map(|f| f.ids)
                .unwrap_or_default();
            if ids == tables[tb].ids() {
                continue;
            }
            let fresh: Vec<u64> = {
                let old: std::collections::HashSet<u64> =
                    tables[tb].ids().iter().copied().collect();
                ids.iter().copied().filter(|i| !old.contains(i)).collect()
            };
            let starts = self.starting_rows(params, tb, &fresh, tables)?;
            let mut next = starts.into_iter();
            let table = &mut tables[tb];
            let kept: Vec<Vec<f64>> = (0..table.len()).map(|r| table.row(r)).collect();
            table.assign(&ids, |_, old| match old {
                Some(r) => kept[r].clone(),
                None => next.next().expect("a starting row per new id"),
            });
        }
        Ok(())
    }

    /// The starting values of new rows `ids` of table `tb`, one row of
    /// its columns each, as `state … = …` gives them (which may read
    /// `id(…)` and parameters).
    fn starting_rows(
        &self,
        params: &[f64],
        tb: usize,
        ids: &[u64],
        tables: &[simsandbox_core::table::Table],
    ) -> Result<Vec<Vec<f64>>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let n = self.set.tables.len();
        let mut rows: Vec<usize> = tables[..n].iter().map(|t| t.len()).collect();
        rows[tb] = ids.len();
        let set = self.compiled_for(rows.clone())?;
        let mut u = vec![0.0; set.inputs_len()];
        for (k, t) in tables[..n].iter().enumerate() {
            match k == tb {
                true => u.extend(ids.iter().map(|&i| i as f64)),
                false => u.extend(t.ids().iter().map(|&i| i as f64)),
            }
        }
        u.extend(set.ends_places(tables));
        let mut x0 = vec![0.0; set.states_len()];
        set.initial_state_with(params, &u, &mut x0);
        let mut columns = vec![Vec::new(); set.tables[tb].columns.len()];
        let mut at = set.fixed_len();
        for &(t, c) in &set.row_states {
            if t == tb {
                columns[c] = x0[at..at + rows[t]].to_vec();
            }
            at += rows[t];
        }
        Ok((0..ids.len())
            .map(|r| columns.iter().map(|c| c[r]).collect())
            .collect())
    }

    /// One step of a map with tables (ADR 0079): every state, fixed or
    /// over rows, updates together; then the rules change the tables in
    /// their written order; then the outputs.
    fn step_with_tables(&mut self, ctx: StepContext, io: StepIo<'_>) -> Result<()> {
        let (t, p) = (ctx.t, io.params);
        self.follow(p, io.population_inputs, io.tables)?;
        // A new structure: an integrator learns it afresh.
        let versions: Vec<u64> = io.tables.iter().map(|t| t.version()).collect();
        if versions != self.versions {
            self.workspace = Default::default();
            self.versions = versions;
        }
        let mut ws = std::mem::take(&mut self.workspace);
        let set = self.compiled(io.tables)?;
        let x = set.gather(io.state, io.tables);
        let u = set.with_ids(io.inputs, io.population_inputs, io.tables);
        let mut next = vec![0.0; x.len()];
        match &self.integrator {
            None => set.rhs(t, &x, &u, p, &mut next),
            Some(integrator) => {
                next.copy_from_slice(&x);
                let f = |t: f64, x: &[f64], dx: &mut [f64]| set.rhs(t, x, &u, p, dx);
                let structure = || Some(set.sparsity_at(t, &x, &u, p));
                integrator.advance_in(&f, t, ctx.dt, &mut next, &mut ws, Some(&structure))?;
            }
        }
        let nf = set.fixed_len();
        io.state.copy_from_slice(&next[..nf]);
        let mut at = nf;
        for &(tb, c) in &set.row_states {
            let n = io.tables[tb].len();
            let off = set.tables()[tb].offset();
            io.tables[tb]
                .column_mut(off + c)
                .copy_from_slice(&next[at..at + n]);
            at += n;
        }
        let rules = set.rules().len();
        drop(set);
        self.workspace = ws;
        for r in 0..rules {
            let set = self.compiled(io.tables)?;
            let x = set.gather(io.state, io.tables);
            let u = set.with_ids(io.inputs, io.population_inputs, io.tables);
            let v = set.rule_registers(t + ctx.dt, &x, &u, p);
            let rule = &set.rules()[r];
            let tb = set
                .tables()
                .iter()
                .position(|tr| tr.axis == rule.table)
                .expect("a table of the block");
            let holds = |c: f64| c != 0.0 && !c.is_nan();
            let cond = &v[rule.cond.at..rule.cond.at + rule.cond.len];
            let row_at = |k: usize| -> Vec<f64> {
                rule.values
                    .iter()
                    .map(|s| v[s.at + if s.len > 1 { k } else { 0 }])
                    .collect()
            };
            let edges = rule.edges.map(|e| {
                set.tables()
                    .iter()
                    .position(|tr| tr.axis == e)
                    .expect("an edge table of the block")
            });
            let max = set.tables()[tb].max;
            match rule.kind {
                RuleKind::Remove => {
                    io.tables[tb].retain(|k| !holds(cond[k]));
                    // Edges and cells go with the points they join (sides
                    // are refreshed below).
                    for (et, spec) in set.tables().iter().enumerate() {
                        if spec.ends_of == Some(tb) && spec.sides_of.is_none() {
                            drop_dangling(io.tables, tb, et, spec.corners());
                        }
                    }
                }
                RuleKind::Add => {
                    let table = &mut io.tables[tb];
                    for (k, _) in cond.iter().enumerate().filter(|(_, &c)| holds(c)) {
                        if table.len() >= max {
                            break;
                        }
                        table.push(&row_at(k));
                    }
                }
                RuleKind::Split | RuleKind::Merge
                    if edges.is_some_and(|e| set.tables()[e].parents.is_some()) =>
                {
                    let et = edges.expect("cells");
                    refine_cells(&set, rule, et, tb, &v, cond, io.tables)?;
                }
                RuleKind::Split => {
                    let et = edges.expect("split acts on edges");
                    let emax = set.tables()[et].max;
                    let (members, edges) = two_mut(io.tables, tb, et);
                    for (k, _) in cond.iter().enumerate().filter(|(_, &c)| holds(c)) {
                        if members.len() >= max || edges.len() >= emax {
                            break;
                        }
                        // A new row in the middle: the edge a→b becomes
                        // a→m, and b's side a new edge m→b.
                        let m = members.push(&row_at(k)) as f64;
                        let b = edges.column(1)[k];
                        edges.column_mut(1)[k] = m;
                        // The new edge copies the split one's own values.
                        let mut row = edges.row(k);
                        row[0] = m;
                        row[1] = b;
                        edges.push(&row);
                    }
                }
                RuleKind::Merge => {
                    let et = edges.expect("merge acts on edges");
                    let (members, edges) = two_mut(io.tables, tb, et);
                    merge(members, edges, |k| holds(cond[k]), row_at);
                    // Other edges of those rows go with the rows merged away.
                    for (other, spec) in set.tables().iter().enumerate() {
                        if other != et && spec.ends_of == Some(tb) && spec.sides_of.is_none() {
                            drop_dangling(io.tables, tb, other, spec.corners());
                        }
                    }
                }
            }
        }
        // Sides follow their cells (ADR 0085).
        self.set.refresh_sides(io.tables);
        self.write_outputs(t + ctx.dt, io)
    }

    /// Every state's starting value, the tables at their starting rows
    /// with ids from 0.
    fn starting(&self, params: &[f64]) -> Vec<f64> {
        let set = &self.set;
        let mut u = vec![0.0; set.inputs_len()];
        for t in &set.tables {
            match &t.start_file {
                Some(f) => u.extend(f.row_ids().iter().map(|&i| i as f64)),
                None => u.extend((0..t.start).map(|i| i as f64)),
            }
        }
        // Corners' places, from a starting mesh or chain: starting values
        // may read a mesh's coordinates through its corners (ADR 0085).
        for t in &set.tables {
            let n = t.start;
            let mut places = vec![0.0; t.places() * n];
            match (&t.start_cells, t.chain) {
                (Some(cells), _) => {
                    for (r, c) in cells.iter().enumerate().take(n) {
                        for (k, &p) in c.iter().enumerate() {
                            places[k * n + r] = p as f64;
                        }
                    }
                }
                (None, true) => {
                    for r in 0..n {
                        places[r] = r as f64;
                        places[n + r] = (r + 1) as f64;
                    }
                }
                _ => {}
            }
            // Edges from a file: their ends' places among their table's
            // starting rows, by id.
            if let (Some(f), Some(m)) = (&t.start_file, t.ends_of) {
                let place: std::collections::HashMap<u64, usize> = match &set.tables[m].start_file {
                    Some(nodes) => nodes.row_ids().into_iter().zip(0..).collect(),
                    None => (0..set.tables[m].start as u64).zip(0..).collect(),
                };
                for (k, end) in ["from", "to"].into_iter().enumerate() {
                    for (r, id) in f.column(end).expect("checked").iter().enumerate() {
                        places[k * n + r] = place[&(*id as u64)] as f64;
                    }
                }
            }
            u.extend(places);
        }
        // The coordinates a starting mesh sets.
        let mut known = vec![0.0; set.states_len()];
        let mut at = set.fixed_len();
        for &(tb, c) in &set.row_states {
            let t = &set.tables[tb];
            if let Some(points) = &t.start_points {
                if let Some(axis) = t.coords.iter().position(|&k| k == c) {
                    let n = t.start.min(points[axis].len());
                    known[at..at + n].copy_from_slice(&points[axis][..n]);
                }
            }
            at += t.start;
        }
        let mut x0 = vec![0.0; set.states_len()];
        set.initial_state_from(params, &known, &u, &mut x0);
        // Values from a table's starting file (ADR 0069 step 3).
        let mut at = set.fixed_len();
        for &(tb, c) in &set.row_states {
            let t = &set.tables[tb];
            if let Some(v) = t.start_file.as_ref().and_then(|f| f.column(&t.columns[c])) {
                let n = t.start.min(v.len());
                x0[at..at + n].copy_from_slice(&v[..n]);
            }
            at += t.start;
        }
        x0
    }

    fn write_outputs(&self, t: f64, io: StepIo<'_>) -> Result<()> {
        self.follow(io.params, io.population_inputs, io.tables)?;
        let set = self.compiled(io.tables)?;
        let x = set.gather(io.state, io.tables);
        let u = set.with_ids(io.inputs, io.population_inputs, io.tables);
        set.outputs(t, &x, &u, io.params, io.outputs);
        set.rows_outputs(io.tables, io.population_outputs);
        Ok(())
    }
}

impl Block for ExplicitBlock {
    fn spec(&self) -> &BlockSpec {
        &self.set.spec
    }

    fn method(&self) -> &MethodSpec {
        &self.method
    }

    fn init(&mut self, params: &[f64], state: &mut [f64]) -> Result<()> {
        if self.set.tables.is_empty() {
            self.set.initial_state(params, state);
        } else {
            let x0 = self.starting(params);
            state.copy_from_slice(&x0[..self.set.fixed_len()]);
        }
        Ok(())
    }

    /// Each table's starting rows, with ids from 0 (ADR 0079).
    fn init_tables(
        &mut self,
        params: &[f64],
        _seed: u64,
        tables: &mut [simsandbox_core::table::Table],
    ) -> Result<()> {
        if self.set.tables.is_empty() {
            return Ok(());
        }
        let x0 = self.starting(params);
        let mut at = self.set.fixed_len();
        let mut columns: Vec<Vec<Vec<f64>>> = self
            .set
            .tables
            .iter()
            .map(|t| vec![Vec::new(); t.columns.len()])
            .collect();
        for &(tb, c) in &self.set.row_states {
            let n = self.set.tables[tb].start;
            columns[tb][c] = x0[at..at + n].to_vec();
            at += n;
        }
        // Points made by a grid or a file take its coordinates (ADR 0085).
        for (tb, t) in self.set.tables.iter().enumerate() {
            let Some(points) = &t.start_points else {
                continue;
            };
            for (axis, &col) in t.coords.iter().enumerate() {
                columns[tb][col].clone_from(&points[axis]);
            }
        }
        for (tb, t) in self.set.tables.iter().enumerate() {
            let table = &mut tables[tb];
            table.reset();
            if t.sides_of.is_some() {
                continue;
            }
            // Each row's corners: a chain's ends, or a grid's cells (point
            // ids are their places at the start).
            let corners: Vec<Vec<f64>> = match (&t.start_cells, t.chain) {
                // Edges from a file: the ids they join.
                _ if t.start_file.is_some() && t.ends_of.is_some() => {
                    let f = t.start_file.as_ref().expect("checked");
                    let (from, to) = (
                        f.column("from").expect("checked"),
                        f.column("to").expect("checked"),
                    );
                    from.iter().zip(to).map(|(&a, &b)| vec![a, b]).collect()
                }
                (Some(cells), _) => cells
                    .iter()
                    .map(|c| c.iter().map(|&p| p as f64).collect())
                    .collect(),
                (None, true) => (0..t.start)
                    .map(|k| vec![k as f64, (k + 1) as f64])
                    .collect(),
                (None, false) => vec![Vec::new(); t.start],
            };
            // Triangles from a file are labelled for bisection: the longest
            // side joins corners 0 and 2 (ADR 0085 step 2).
            let label = |mut c: Vec<f64>| -> Vec<f64> {
                let pts = t
                    .ends_of
                    .and_then(|m| self.set.tables[m].start_points.as_ref());
                let (Some(x), true, Some(crate::mesh::CellShape::Triangle)) =
                    (pts, t.from_file && t.parents.is_some(), t.shape)
                else {
                    return c;
                };
                let len = |a: usize, b: usize| -> f64 {
                    x.iter()
                        .map(|axis| (axis[c[a] as usize] - axis[c[b] as usize]).powi(2))
                        .sum()
                };
                let (l01, l12, l20) = (len(0, 1), len(1, 2), len(2, 0));
                if l01 > l12 && l01 > l20 {
                    c.swap(1, 2);
                } else if l12 > l20 {
                    c.swap(0, 1);
                }
                c
            };
            for (r, row) in corners.into_iter().enumerate().take(t.start) {
                let mut row = label(row);
                if t.parents.is_some() {
                    row.extend([0.0, -1.0, -1.0]);
                }
                row.extend(columns[tb].iter().map(|c| c[r]));
                table.push(&row);
            }
            // Ids from a file: the rows as pushed, under those ids.
            if let Some(ids) = t.start_file.as_ref().and_then(|f| f.ids.clone()) {
                let rows: Vec<Vec<f64>> = (0..table.len()).map(|r| table.row(r)).collect();
                let place: std::collections::HashMap<u64, usize> =
                    ids.iter().copied().zip(0..).collect();
                table.reset();
                table.assign(&ids, |id, _| rows[place[&id]].clone());
            }
            if let Some(p) = t.parents {
                tables[p].reset();
            }
        }
        // Sides of the starting cells, their values as they start.
        self.set.refresh_sides(tables);
        for (tb, t) in self.set.tables.iter().enumerate() {
            if t.sides_of.is_none() {
                continue;
            }
            let off = t.offset();
            for (c, values) in columns[tb].iter().enumerate() {
                let col = tables[tb].column_mut(off + c);
                let n = col.len().min(values.len());
                col[..n].copy_from_slice(&values[..n]);
            }
        }
        Ok(())
    }

    fn initial_params(&self, state: &[f64], params: &[f64]) -> Option<Vec<f64>> {
        // A table's rows are not parameters: not for blocks with tables.
        if !self.set.tables.is_empty() {
            return None;
        }
        crate::ode::initial_params_by_name(&self.set.spec, state, params, |p, x| {
            self.set.initial_state(p, x)
        })
    }

    fn do_step(&mut self, ctx: StepContext, io: StepIo<'_>) -> Result<()> {
        if !self.set.tables.is_empty() {
            return self.step_with_tables(ctx, io);
        }
        if self.set.kind == Kind::Map {
            // All states update together from the current values.
            self.set
                .rhs(ctx.t, io.state, io.inputs, io.params, &mut self.next);
            io.state.copy_from_slice(&self.next);
        }
        self.set
            .outputs(ctx.t + ctx.dt, io.state, io.inputs, io.params, io.outputs);
        Ok(())
    }

    fn compute_outputs(&self, t: f64, io: StepIo<'_>) -> Result<()> {
        if !self.set.tables.is_empty() {
            return self.write_outputs(t, io);
        }
        self.set
            .outputs(t, io.state, io.inputs, io.params, io.outputs);
        Ok(())
    }

    /// Formulas can join a fused group (ADR 0051); maps step on their own.
    fn as_equations(&self) -> Option<&dyn Equations> {
        (self.set.kind == Kind::Formulas && self.set.tables.is_empty())
            .then_some(&self.set as &dyn Equations)
    }

    fn code(&self) -> Option<String> {
        Some(self.set.code())
    }
}
