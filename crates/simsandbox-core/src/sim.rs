//! The co-simulation master and the kernel-owned state store.
//!
//! v0 master: fixed communication step, Jacobi-style exchange (every Block
//! sees the outputs from the end of the previous step). Fused groups
//! (ADR 0023) are advanced as one system, with connections inside the group
//! resolved at every evaluation. Adaptive steps, iteration and rollback
//! build on this; see the roadmap.

use crate::block::{Block, Causality, MethodLabels, StepContext, StepIo};
use crate::graph::{population_index, port_offset, BlockId, Model};
use crate::loops::{Link, LoopMethod, OutputPlan};
use crate::table::Table;
use crate::{Result, SimError};
use std::cell::RefCell;

/// A saved point in a run, used for rewind and branching: time, every
/// Block's state, tables, outputs and parameter values. Outputs that are
/// copies of a Block's state are left out and restored from it (ADR 0063).
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub t: f64,
    step: u64,
    states: Vec<Vec<f64>>,
    tables: Vec<Vec<Table>>,
    /// Per Block: every output, or (`compact`) those not copying the state.
    outputs: Vec<Vec<f64>>,
    compact: Vec<bool>,
    population_outputs: Vec<Vec<Vec<f64>>>,
    params: Vec<Vec<f64>>,
}

/// A route from a variable-length output to a variable-length input
/// (ADR 0034).
struct PopulationRoute {
    from_block: usize,
    from_index: usize,
    to_block: usize,
    to_index: usize,
    factor: f64,
}

/// One resolved route from an output slice to an input slice.
struct Route {
    from_block: usize,
    from_offset: usize,
    to_block: usize,
    to_offset: usize,
    len: usize,
    factor: f64,
}

/// How one fused group is wired: where each member's state sits in the
/// combined state, and the connections between members.
struct GroupPlan {
    /// Block index and state offset of each member.
    members: Vec<(usize, usize)>,
    len: usize,
    /// Routes between members, with `from_block`/`to_block` as member
    /// positions in `members`.
    internal: Vec<Route>,
    /// Feedthrough outputs among the members, ordered, with their loops.
    outputs: OutputPlan,
    method: LoopMethod,
}

impl Route {
    fn link(&self) -> Link {
        Link {
            from: self.from_block,
            from_offset: self.from_offset,
            to: self.to_block,
            to_offset: self.to_offset,
            len: self.len,
            factor: self.factor,
        }
    }
}

impl GroupPlan {
    fn new(model: &Model, group: usize, routes: &[Route]) -> Result<Self> {
        let ids: Vec<usize> = model.groups[group].members.iter().map(|b| b.0).collect();
        let mut members = Vec::new();
        let mut len = 0;
        for &b in &ids {
            members.push((b, len));
            len += model.blocks[b].spec().states.len();
        }
        let position = |b: usize| ids.iter().position(|&m| m == b);
        let internal = routes
            .iter()
            .filter_map(|r| {
                Some(Route {
                    from_block: position(r.from_block)?,
                    to_block: position(r.to_block)?,
                    ..*r
                })
            })
            .collect::<Vec<Route>>();
        let units: Vec<(&crate::block::BlockSpec, &str)> = ids
            .iter()
            .map(|&b| {
                let spec = model.blocks[b].spec();
                (spec, spec.name.as_str())
            })
            .collect();
        let links: Vec<Link> = internal.iter().map(Route::link).collect();
        Ok(Self {
            members,
            len,
            outputs: OutputPlan::new(&units, &links)?,
            internal,
            method: model.loops,
        })
    }

    /// The combined `dx/dt`. Member outputs first, from state, parameters
    /// and the inputs held since the last exchange; then feedthrough
    /// outputs made consistent with the inputs from inside the group, loops
    /// solved (ADR 0040); then every member's inputs from inside the group
    /// (or held, from outside), and the right-hand sides.
    fn rhs(
        &self,
        blocks: &[Box<dyn Block>],
        params: &[Vec<f64>],
        held: &[Vec<f64>],
        t: f64,
        x: &[f64],
        dx: &mut [f64],
    ) -> Result<()> {
        let state = |k: usize| {
            let (b, off) = self.members[k];
            &x[off..off + blocks[b].spec().states.len()]
        };
        let mut us: Vec<Vec<f64>> = self.members.iter().map(|&(b, _)| held[b].clone()).collect();
        let mut ys: Vec<Vec<f64>> = (0..self.members.len())
            .map(|k| {
                let b = self.members[k].0;
                let mut y = vec![0.0; blocks[b].spec().output_len()];
                if let Some(eq) = blocks[b].as_equations() {
                    eq.outputs(t, state(k), &us[k], &params[b], &mut y);
                }
                y
            })
            .collect();
        if !self.outputs.is_empty() {
            self.outputs
                .solve(self.method, &mut ys, &mut us, &mut |k, u, y| {
                    let b = self.members[k].0;
                    if let Some(eq) = blocks[b].as_equations() {
                        eq.outputs(t, state(k), u, &params[b], y);
                    }
                    Ok(())
                })?;
        }
        for r in &self.internal {
            for i in 0..r.len {
                us[r.to_block][r.to_offset + i] = r.factor * ys[r.from_block][r.from_offset + i];
            }
        }
        for (k, &(b, off)) in self.members.iter().enumerate() {
            let n = blocks[b].spec().states.len();
            if let Some(eq) = blocks[b].as_equations() {
                eq.rhs(t, state(k), &us[k], &params[b], &mut dx[off..off + n]);
            }
        }
        Ok(())
    }
}

/// Runs a [`Model`].
pub struct Simulator {
    model: Model,
    routes: Vec<Route>,
    /// Feedthrough outputs of all Blocks, ordered, with their loops
    /// (ADR 0040).
    outputs_plan: OutputPlan,
    population_routes: Vec<PopulationRoute>,
    plans: Vec<GroupPlan>,
    states: Vec<Vec<f64>>,
    tables: Vec<Vec<Table>>,
    inputs: Vec<Vec<f64>>,
    outputs: Vec<Vec<f64>>,
    population_inputs: Vec<Vec<Vec<f64>>>,
    population_outputs: Vec<Vec<Vec<f64>>>,
    /// Each Block's random stream (ADR 0034).
    seeds: Vec<u64>,
    t: f64,
    /// Steps taken since the run started.
    step: u64,
    /// Blocks whose parameters changed since the last step: their outputs
    /// are recomputed before the next one, so the change acts in it.
    changed: Vec<usize>,
    /// Per Block: outputs declared copies of its state, as (output offset,
    /// state offset, length) (ADR 0063).
    copies: Vec<Vec<(usize, usize, usize)>>,
}

impl Snapshot {
    /// How many numbers it holds: what it costs in memory, at 8 bytes each.
    pub fn values(&self) -> usize {
        let flat = |v: &Vec<Vec<f64>>| v.iter().map(Vec::len).sum::<usize>();
        let tables: usize = self
            .tables
            .iter()
            .flatten()
            .map(|t| t.len() * (t.columns() + 1))
            .sum();
        let populations: usize = self.population_outputs.iter().map(flat).sum();
        flat(&self.states) + flat(&self.outputs) + flat(&self.params) + tables + populations
    }
}

impl Simulator {
    /// Validate the model (every input connected), initialise all Blocks and
    /// compute outputs at `t0`.
    pub fn new(model: Model, t0: f64) -> Result<Self> {
        let mut routes = Vec::new();
        let mut population_routes = Vec::new();
        for (bi, block) in model.blocks.iter().enumerate() {
            let spec = block.spec();
            for p in spec.inputs() {
                let conn = model
                    .connections
                    .iter()
                    .find(|c| c.to.block.0 == bi && c.to.port == p.name)
                    .ok_or_else(|| {
                        SimError::UnconnectedInput(format!("{}.{}", spec.name, p.name))
                    })?;
                let from_spec = model.blocks[conn.from.block.0].spec();
                if p.shape.is_variable() {
                    let index = |ports, causality, name: &str| {
                        population_index(ports, causality, name)
                            .ok_or_else(|| SimError::BadConnection(name.to_string()))
                    };
                    population_routes.push(PopulationRoute {
                        from_block: conn.from.block.0,
                        from_index: index(&from_spec.ports, Causality::Output, &conn.from.port)?,
                        to_block: bi,
                        to_index: index(&spec.ports, Causality::Input, &p.name)?,
                        factor: conn.factor,
                    });
                    continue;
                }
                let (from_offset, len) =
                    port_offset(&from_spec.ports, Causality::Output, &conn.from.port)
                        .ok_or_else(|| SimError::BadConnection(conn.from.port.clone()))?;
                let (to_offset, _) = port_offset(&spec.ports, Causality::Input, &p.name)
                    .ok_or_else(|| SimError::BadConnection(p.name.clone()))?;
                routes.push(Route {
                    from_block: conn.from.block.0,
                    from_offset,
                    to_block: bi,
                    to_offset,
                    len,
                    factor: conn.factor,
                });
            }
        }

        let mut states = Vec::new();
        let mut tables = Vec::new();
        let mut inputs = Vec::new();
        let mut outputs = Vec::new();
        let mut population_inputs = Vec::new();
        let mut population_outputs = Vec::new();
        let mut seeds = Vec::new();
        for block in &model.blocks {
            let spec = block.spec();
            states.push(vec![0.0; spec.states.len()]);
            tables.push(
                spec.tables
                    .iter()
                    .map(|t| Table::new(t.columns.len()))
                    .collect(),
            );
            inputs.push(vec![0.0; spec.input_len()]);
            outputs.push(vec![0.0; spec.output_len()]);
            let variable = |ports: Vec<_>| -> Vec<Vec<f64>> { vec![Vec::new(); ports.len()] };
            population_inputs.push(variable(
                spec.inputs().filter(|p| p.shape.is_variable()).collect(),
            ));
            population_outputs.push(variable(
                spec.outputs().filter(|p| p.shape.is_variable()).collect(),
            ));
            seeds.push(crate::random::stream(model.seed, &spec.name));
        }

        let plans = (0..model.groups.len())
            .map(|g| GroupPlan::new(&model, g, &routes))
            .collect::<Result<Vec<_>>>()?;
        let units: Vec<(&crate::block::BlockSpec, &str)> = model
            .blocks
            .iter()
            .map(|b| (b.spec(), b.spec().name.as_str()))
            .collect();
        let links: Vec<Link> = routes.iter().map(Route::link).collect();
        let outputs_plan = OutputPlan::new(&units, &links)?;
        if model.loops == LoopMethod::Refuse {
            if let Some(l) = outputs_plan.loops().first() {
                return Err(SimError::AlgebraicLoop(format!(
                    "{} form a loop, and run.loops is 'refuse'; break it, or let it be \
                     solved with 'newton'",
                    l.join(" → ")
                )));
            }
        }
        // Outputs copying a state (ADR 0063), where they fit.
        let copies = model
            .blocks
            .iter()
            .zip(&states)
            .map(|(b, x)| {
                let spec = b.spec();
                spec.outputs()
                    .filter(|p| !p.shape.is_variable())
                    .filter_map(|p| {
                        let s = p.state?;
                        let (off, len) = port_offset(&spec.ports, Causality::Output, &p.name)?;
                        (s + len <= x.len()).then_some((off, s, len))
                    })
                    .collect()
            })
            .collect();
        let mut sim = Self {
            copies,
            model,
            routes,
            outputs_plan,
            population_routes,
            plans,
            states,
            tables,
            inputs,
            outputs,
            population_inputs,
            population_outputs,
            seeds,
            t: t0,
            step: 0,
            changed: Vec::new(),
        };
        sim.restart(t0)?;
        Ok(sim)
    }

    /// Start the run again at `t0` from the current parameter values: every
    /// Block re-initialises its state, so changed initial conditions take
    /// effect. Snapshots taken earlier stay valid.
    pub fn restart(&mut self, t0: f64) -> Result<()> {
        for (bi, block) in self.model.blocks.iter_mut().enumerate() {
            let params = &self.model.params[bi];
            block.init(params, &mut self.states[bi])?;
            for t in &mut self.tables[bi] {
                t.reset();
            }
            block.init_tables(params, self.seeds[bi], &mut self.tables[bi])?;
            // A saved start replaces what the parameters give (ADR 0042).
            if let Some(start) = &self.model.starts[bi] {
                self.states[bi].copy_from_slice(&start.state);
                for (table, rows) in self.tables[bi].iter_mut().zip(&start.tables) {
                    table.reset();
                    for row in rows {
                        table.push(row);
                    }
                }
            }
            self.inputs[bi].fill(0.0);
            for v in &mut self.population_inputs[bi] {
                v.clear();
            }
            block.compute_outputs(
                t0,
                StepIo {
                    params,
                    inputs: &self.inputs[bi],
                    state: &mut self.states[bi],
                    outputs: &mut self.outputs[bi],
                    tables: &mut self.tables[bi],
                    population_inputs: &self.population_inputs[bi],
                    population_outputs: &mut self.population_outputs[bi],
                },
            )?;
        }
        self.t = t0;
        self.step = 0;
        self.changed.clear();
        self.resolve(t0)?;
        self.exchange();
        Ok(())
    }

    /// Recompute the outputs of Blocks whose parameters changed, then the
    /// outputs that depend on their inputs, and pass them on: a parameter
    /// changed between steps (a slider, an input block, ADR 0058) acts in
    /// the next step rather than one step later.
    fn refresh_changed(&mut self) -> Result<()> {
        let t = self.t;
        for bi in std::mem::take(&mut self.changed) {
            let params = &self.model.params[bi];
            self.model.blocks[bi].compute_outputs(
                t,
                StepIo {
                    params,
                    inputs: &self.inputs[bi],
                    state: &mut self.states[bi],
                    outputs: &mut self.outputs[bi],
                    tables: &mut self.tables[bi],
                    population_inputs: &self.population_inputs[bi],
                    population_outputs: &mut self.population_outputs[bi],
                },
            )?;
        }
        self.resolve(t)?;
        self.exchange();
        Ok(())
    }

    /// Make feedthrough outputs consistent with their inputs at time `t`,
    /// solving loops (ADR 0040).
    fn resolve(&mut self, t: f64) -> Result<()> {
        if self.outputs_plan.is_empty() {
            return Ok(());
        }
        let blocks = &self.model.blocks;
        let params = &self.model.params;
        let (states, tables) = (&mut self.states, &mut self.tables);
        let (pin, pout) = (&self.population_inputs, &mut self.population_outputs);
        self.outputs_plan.solve(
            self.model.loops,
            &mut self.outputs,
            &mut self.inputs,
            &mut |b, u, y| {
                blocks[b].compute_outputs(
                    t,
                    StepIo {
                        params: &params[b],
                        inputs: u,
                        state: &mut states[b],
                        outputs: y,
                        tables: &mut tables[b],
                        population_inputs: &pin[b],
                        population_outputs: &mut pout[b],
                    },
                )
            },
        )
    }

    /// The algebraic loops of the model, each as the outputs in it
    /// (`block.port`), found when it was built (ADR 0040).
    pub fn loops(&self) -> Vec<Vec<String>> {
        self.outputs_plan.loops()
    }

    /// Copy outputs to connected inputs, applying unit conversion.
    fn exchange(&mut self) {
        for r in &self.routes {
            for k in 0..r.len {
                self.inputs[r.to_block][r.to_offset + k] =
                    r.factor * self.outputs[r.from_block][r.from_offset + k];
            }
        }
        for r in &self.population_routes {
            let from = &self.population_outputs[r.from_block][r.from_index];
            let to = &mut self.population_inputs[r.to_block][r.to_index];
            to.clear();
            to.extend(from.iter().map(|v| r.factor * v));
        }
    }

    /// Advance every Block by one communication step `dt`.
    pub fn step(&mut self, dt: f64) -> Result<()> {
        if !self.changed.is_empty() {
            self.refresh_changed()?;
        }
        let base = StepContext {
            t: self.t,
            dt,
            step: self.step,
            seed: 0,
        };
        for (bi, block) in self.model.blocks.iter_mut().enumerate() {
            if self
                .model
                .groups
                .iter()
                .any(|g| g.members.contains(&BlockId(bi)))
            {
                continue;
            }
            block.do_step(
                StepContext {
                    seed: self.seeds[bi],
                    ..base
                },
                StepIo {
                    params: &self.model.params[bi],
                    inputs: &self.inputs[bi],
                    state: &mut self.states[bi],
                    outputs: &mut self.outputs[bi],
                    tables: &mut self.tables[bi],
                    population_inputs: &self.population_inputs[bi],
                    population_outputs: &mut self.population_outputs[bi],
                },
            )?;
        }
        for (plan, group) in self.plans.iter().zip(&mut self.model.groups) {
            let mut x = Vec::with_capacity(plan.len);
            for &(b, _) in &plan.members {
                x.extend_from_slice(&self.states[b]);
            }
            let (blocks, params, held) = (&self.model.blocks, &self.model.params, &self.inputs);
            // The solver's right-hand side cannot fail; a loop that does
            // not converge is reported after the step.
            let failed = RefCell::new(None);
            let rhs = |t: f64, x: &[f64], dx: &mut [f64]| {
                if let Err(e) = plan.rhs(blocks, params, held, t, x, dx) {
                    dx.fill(f64::NAN);
                    failed.borrow_mut().get_or_insert(e);
                }
            };
            group.solver.advance(&rhs, base.t, dt, &mut x)?;
            if let Some(e) = failed.into_inner() {
                return Err(e);
            }
            for &(b, off) in &plan.members {
                let n = self.states[b].len();
                self.states[b].copy_from_slice(&x[off..off + n]);
                self.model.blocks[b].compute_outputs(
                    base.t + dt,
                    StepIo {
                        params: &self.model.params[b],
                        inputs: &self.inputs[b],
                        state: &mut self.states[b],
                        outputs: &mut self.outputs[b],
                        tables: &mut self.tables[b],
                        population_inputs: &self.population_inputs[b],
                        population_outputs: &mut self.population_outputs[b],
                    },
                )?;
            }
        }
        self.t += dt;
        self.step += 1;
        self.resolve(self.t)?;
        self.exchange();
        Ok(())
    }

    pub fn time(&self) -> f64 {
        self.t
    }

    /// Start a Block from `start` (or from its parameters with `None`) at
    /// the next restart; see [`Model::set_start`].
    pub fn set_start(&mut self, id: BlockId, start: Option<crate::graph::Start>) -> Result<()> {
        self.model.set_start(id, start)
    }

    /// The current state and table rows of a Block, as a start (ADR 0042).
    pub fn current(&self, id: BlockId) -> crate::graph::Start {
        crate::graph::Start {
            state: self.states[id.0].clone(),
            tables: self.tables[id.0]
                .iter()
                .map(|t| (0..t.len()).map(|r| t.row(r)).collect())
                .collect(),
        }
    }

    /// Kernel-owned state of one Block.
    pub fn state(&self, id: BlockId) -> &[f64] {
        &self.states[id.0]
    }

    /// A Block's tables, in `BlockSpec::tables` order.
    pub fn tables(&self, id: BlockId) -> &[Table] {
        &self.tables[id.0]
    }

    /// A Block's table by name.
    pub fn table(&self, id: BlockId, name: &str) -> Option<&Table> {
        let spec = self.model.blocks.get(id.0)?.spec();
        let i = spec.tables.iter().position(|t| t.name == name)?;
        Some(&self.tables[id.0][i])
    }

    /// Steps taken since the run started.
    pub fn step_count(&self) -> u64 {
        self.step
    }

    /// Current value of a named output port (of any length for a
    /// variable-length port).
    pub fn output(&self, id: BlockId, port: &str) -> Option<&[f64]> {
        let spec = self.model.blocks.get(id.0)?.spec();
        if let Some(i) = population_index(&spec.ports, Causality::Output, port) {
            return Some(&self.population_outputs[id.0][i]);
        }
        let (off, len) = port_offset(&spec.ports, Causality::Output, port)?;
        Some(&self.outputs[id.0][off..off + len])
    }

    /// The model being run: Block specs, parameter values and connections.
    pub fn model(&self) -> &Model {
        &self.model
    }

    /// Change a parameter while running (live sliders). Checked like
    /// [`Model::set_param`].
    pub fn set_param(&mut self, id: BlockId, name: &str, value: f64) -> Result<()> {
        self.model.set_param(id, name, value)?;
        if !self.changed.contains(&id.0) {
            self.changed.push(id.0);
        }
        Ok(())
    }

    /// Current value of a parameter.
    pub fn param(&self, id: BlockId, name: &str) -> Result<f64> {
        self.model.param(id, name)
    }

    /// Speed and accuracy labels of the whole model.
    /// Fused groups count with their group's method instead of their own.
    pub fn labels(&self) -> Option<MethodLabels> {
        let m = &self.model;
        let single = m
            .block_ids()
            .filter(|&id| m.group_of(id).is_none())
            .map(|id| m.blocks[id.0].method().labels);
        let groups = m.groups.iter().map(|g| g.method().labels);
        MethodLabels::combine(single.chain(groups))
    }

    pub fn snapshot(&self) -> Snapshot {
        // Outputs copying the state, checked bit for bit, are left out.
        let mut outputs = Vec::with_capacity(self.outputs.len());
        let mut compact = Vec::with_capacity(self.outputs.len());
        for ((y, x), copies) in self.outputs.iter().zip(&self.states).zip(&self.copies) {
            let copied = !copies.is_empty()
                && copies.iter().all(|&(o, s, n)| {
                    y[o..o + n]
                        .iter()
                        .zip(&x[s..s + n])
                        .all(|(a, b)| a.to_bits() == b.to_bits())
                });
            if copied {
                let mut kept = Vec::with_capacity(y.len());
                let mut at = 0;
                let mut ranges = copies.clone();
                ranges.sort_unstable();
                for (o, _, n) in ranges {
                    kept.extend_from_slice(&y[at..o]);
                    at = o + n;
                }
                kept.extend_from_slice(&y[at..]);
                outputs.push(kept);
            } else {
                outputs.push(y.clone());
            }
            compact.push(copied);
        }
        Snapshot {
            t: self.t,
            step: self.step,
            states: self.states.clone(),
            tables: self.tables.clone(),
            outputs,
            compact,
            population_outputs: self.population_outputs.clone(),
            params: self.model.params.clone(),
        }
    }

    /// Rewind to a snapshot taken from this simulator, including the
    /// parameter values in effect when it was taken.
    pub fn restore(&mut self, snap: &Snapshot) {
        self.changed.clear();
        self.t = snap.t;
        self.step = snap.step;
        self.states.clone_from(&snap.states);
        self.tables.clone_from(&snap.tables);
        for (b, y) in self.outputs.iter_mut().enumerate() {
            if !snap.compact[b] {
                y.clone_from(&snap.outputs[b]);
                continue;
            }
            // The kept outputs between the copies, then the copies.
            let mut ranges = self.copies[b].clone();
            ranges.sort_unstable();
            let (kept, x) = (&snap.outputs[b], &snap.states[b]);
            let (mut at, mut from) = (0, 0);
            for (o, s, n) in ranges {
                y[at..o].copy_from_slice(&kept[from..from + o - at]);
                from += o - at;
                y[o..o + n].copy_from_slice(&x[s..s + n]);
                at = o + n;
            }
            y[at..].copy_from_slice(&kept[from..]);
        }
        self.population_outputs.clone_from(&snap.population_outputs);
        self.model.params.clone_from(&snap.params);
        self.exchange();
    }
}
