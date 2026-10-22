//! The co-simulation master and the kernel-owned state store.
//!
//! v0 master: fixed communication step, Jacobi-style exchange (every Block
//! sees the outputs from the end of the previous step). Fused groups
//! (ADR 0023) are advanced as one system, with connections inside the group
//! resolved at every evaluation. Adaptive steps, iteration and rollback
//! build on this; see the roadmap.

use crate::block::{Block, Causality, MethodLabels, StepContext, StepIo};
use crate::graph::{population_index, port_offset, BlockId, Model};
use crate::table::Table;
use crate::{Result, SimError};

/// A saved point in a run, used for rewind and branching: time, every
/// Block's state, tables, outputs and parameter values.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub t: f64,
    step: u64,
    states: Vec<Vec<f64>>,
    tables: Vec<Vec<Table>>,
    outputs: Vec<Vec<f64>>,
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
}

impl GroupPlan {
    fn new(model: &Model, group: usize, routes: &[Route]) -> Self {
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
            .collect();
        Self {
            members,
            len,
            internal,
        }
    }

    /// The combined `dx/dt`: member outputs first (they depend on state and
    /// parameters only), then member inputs from inside the group or, for
    /// inputs from outside, the values held since the last exchange.
    fn rhs(
        &self,
        blocks: &[Box<dyn Block>],
        params: &[Vec<f64>],
        held: &[Vec<f64>],
        t: f64,
        x: &[f64],
        dx: &mut [f64],
    ) {
        let state = |k: usize| {
            let (b, off) = self.members[k];
            &x[off..off + blocks[b].spec().states.len()]
        };
        let ys: Vec<Vec<f64>> = (0..self.members.len())
            .map(|k| {
                let b = self.members[k].0;
                let mut y = vec![0.0; blocks[b].spec().output_len()];
                if let Some(eq) = blocks[b].as_equations() {
                    eq.outputs(t, state(k), &params[b], &mut y);
                }
                y
            })
            .collect();
        let mut us: Vec<Vec<f64>> = self.members.iter().map(|&(b, _)| held[b].clone()).collect();
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
    }
}

/// Runs a [`Model`].
pub struct Simulator {
    model: Model,
    routes: Vec<Route>,
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
            .collect();
        let mut sim = Self {
            model,
            routes,
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
        self.exchange();
        Ok(())
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
            let rhs = |t: f64, x: &[f64], dx: &mut [f64]| plan.rhs(blocks, params, held, t, x, dx);
            group.solver.advance(&rhs, base.t, dt, &mut x)?;
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
        self.exchange();
        Ok(())
    }

    pub fn time(&self) -> f64 {
        self.t
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
        self.model.set_param(id, name, value)
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
        Snapshot {
            t: self.t,
            step: self.step,
            states: self.states.clone(),
            tables: self.tables.clone(),
            outputs: self.outputs.clone(),
            population_outputs: self.population_outputs.clone(),
            params: self.model.params.clone(),
        }
    }

    /// Rewind to a snapshot taken from this simulator, including the
    /// parameter values in effect when it was taken.
    pub fn restore(&mut self, snap: &Snapshot) {
        self.t = snap.t;
        self.step = snap.step;
        self.states.clone_from(&snap.states);
        self.tables.clone_from(&snap.tables);
        self.outputs.clone_from(&snap.outputs);
        self.population_outputs.clone_from(&snap.population_outputs);
        self.model.params.clone_from(&snap.params);
        self.exchange();
    }
}
