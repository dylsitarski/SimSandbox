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
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A saved point in a run, used for rewind and branching: time, every
/// Block's state, tables, outputs and parameter values. Outputs that can
/// be had again exactly from the state are left out (ADR 0063).
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub t: f64,
    step: u64,
    states: Vec<Vec<f64>>,
    tables: Vec<Vec<Table>>,
    /// Per Block: the outputs kept, as `kept` says.
    outputs: Vec<Vec<f64>>,
    kept: Vec<Kept>,
    population_outputs: Vec<Vec<Vec<f64>>>,
    params: Vec<Vec<f64>>,
    /// With several clocks: each clock's time and steps, and every input,
    /// since inputs are taken only at their block's instants.
    clocks: Vec<(f64, u128)>,
    inputs: Vec<Vec<f64>>,
}

/// Whether two runs of numbers are the same, bit for bit.
fn bits_equal(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits())
}

/// Which of a Block's outputs a snapshot keeps (ADR 0063).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kept {
    /// All of them.
    All,
    /// All but those copying its state, which are restored from it.
    ButCopies,
    /// None: they depend on its state and parameters only and are
    /// computed again from them, as checked when the snapshot was taken.
    None,
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
    /// Per member: its inputs' length and its driven parameters (ADR 0075),
    /// held in its inputs after them.
    driven: Vec<(usize, Vec<usize>)>,
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
        let driven: Vec<(usize, Vec<usize>)> = ids
            .iter()
            .map(|&b| (model.blocks[b].spec().input_len(), model.driven_params(b)))
            .collect();
        let units: Vec<(&crate::block::BlockSpec, &str, &[usize])> = ids
            .iter()
            .zip(&driven)
            .map(|(&b, (_, d))| {
                let spec = model.blocks[b].spec();
                (spec, spec.name.as_str(), d.as_slice())
            })
            .collect();
        let links: Vec<Link> = internal.iter().map(Route::link).collect();
        let outputs = OutputPlan::new(&units, &links)?;
        drop(units);
        Ok(Self {
            members,
            driven,
            len,
            outputs,
            internal,
            method: model.loops,
        })
    }

    /// The combined `dx/dt`. Member outputs first, from state, parameters
    /// and the inputs held since the last exchange; then feedthrough
    /// outputs made consistent with the inputs from inside the group, loops
    /// solved (ADR 0040); then every member's inputs from inside the group
    /// (or held, from outside), and the right-hand sides.
    /// The members' blocks, parameters and held inputs are given in member
    /// order, so a group can be advanced with its own members only.
    fn rhs(
        &self,
        members: &Members<'_>,
        t: f64,
        x: &[f64],
        dx: &mut [f64],
        scratch: &mut GroupScratch,
    ) -> Result<()> {
        let Members {
            blocks,
            params,
            held,
        } = members;
        let m = self.members.len();
        let GroupScratch { us, ys, ps } = scratch;
        for v in [&mut *us, &mut *ys, &mut *ps] {
            if v.len() < m {
                v.resize_with(m, Vec::new);
            }
        }
        for k in 0..m {
            us[k].clear();
            us[k].extend_from_slice(held[k]);
            ys[k].clear();
            ys[k].resize(blocks[k].spec().output_len(), 0.0);
        }
        let state = |k: usize| {
            let off = self.members[k].1;
            &x[off..off + blocks[k].spec().states.len()]
        };
        // A member's parameters with the driven ones as its inputs give
        // them (ADR 0075), in `buf`; its own when none is driven.
        let driven = &self.driven;
        let with = |k: usize, u: &[f64], buf: &mut Vec<f64>| -> bool {
            let (n, d) = &driven[k];
            if d.is_empty() {
                return false;
            }
            buf.clear();
            buf.extend_from_slice(params[k]);
            for (j, &i) in d.iter().enumerate() {
                buf[i] = u[n + j];
            }
            true
        };
        for k in 0..m {
            if let Some(eq) = blocks[k].as_equations() {
                let n = driven[k].0;
                let p: &[f64] = if with(k, &us[k], &mut ps[k]) {
                    &ps[k]
                } else {
                    params[k]
                };
                eq.outputs(t, state(k), &us[k][..n], p, &mut ys[k]);
            }
        }
        if !self.outputs.is_empty() {
            self.outputs
                .solve(self.method, &mut ys[..m], &mut us[..m], &mut |k, u, y| {
                    if let Some(eq) = blocks[k].as_equations() {
                        let n = driven[k].0;
                        let p: &[f64] = if with(k, u, &mut ps[k]) {
                            &ps[k]
                        } else {
                            params[k]
                        };
                        eq.outputs(t, state(k), &u[..n], p, y);
                    }
                    Ok(())
                })?;
        }
        for r in &self.internal {
            for i in 0..r.len {
                us[r.to_block][r.to_offset + i] = r.factor * ys[r.from_block][r.from_offset + i];
            }
        }
        for (k, &(_, off)) in self.members.iter().enumerate() {
            let n = blocks[k].spec().states.len();
            if let Some(eq) = blocks[k].as_equations() {
                let ni = driven[k].0;
                let p: &[f64] = if with(k, &us[k], &mut ps[k]) {
                    &ps[k]
                } else {
                    params[k]
                };
                eq.rhs(t, state(k), &us[k][..ni], p, &mut dx[off..off + n]);
            }
        }
        Ok(())
    }
}

/// A fused group's members, in member order: their blocks, parameters
/// and held inputs.
struct Members<'a> {
    blocks: Vec<&'a dyn Block>,
    params: Vec<&'a [f64]>,
    held: Vec<&'a [f64]>,
}

/// What a unit advances of one fused group: its plan and solver, its
/// members' slots and its working arrays.
type GroupWork<'a> = (
    &'a GroupPlan,
    &'a mut Box<dyn crate::fusion::GroupSolver>,
    Vec<Slot<'a>>,
    &'a mut GroupScratch,
);

/// A fused group's working arrays for its right-hand side, kept between
/// evaluations: each member's inputs, outputs and (when some are driven)
/// parameters.
#[derive(Default)]
struct GroupScratch {
    us: Vec<Vec<f64>>,
    ys: Vec<Vec<f64>>,
    ps: Vec<Vec<f64>>,
}

/// One Block's part of the run, lent to a unit of work (ADR 0066 step 6).
struct Slot<'a> {
    block: &'a mut Box<dyn Block>,
    params: &'a [f64],
    /// Its inputs, its driven parameters after them (ADR 0075).
    inputs: &'a [f64],
    n_in: usize,
    state: &'a mut Vec<f64>,
    outputs: &'a mut Vec<f64>,
    tables: &'a mut Vec<Table>,
    pin: &'a [Vec<f64>],
    pout: &'a mut Vec<Vec<f64>>,
}

/// What one clock steps between instants: its blocks and its fused
/// groups, which touch nothing another clock's do.
struct Unit<'a> {
    clock: usize,
    t0: f64,
    dt: f64,
    step: u64,
    blocks: Vec<(Slot<'a>, u64)>,
    groups: Vec<GroupWork<'a>>,
}

impl Unit<'_> {
    /// Step every block from `t0` over `dt`, then advance each group and
    /// compute its members' outputs.
    fn step(self) -> Result<()> {
        let (t0, dt) = (self.t0, self.dt);
        for (slot, seed) in self.blocks {
            slot.block.do_step(
                StepContext {
                    t: t0,
                    dt,
                    step: self.step,
                    seed,
                },
                StepIo {
                    params: slot.params,
                    inputs: &slot.inputs[..slot.n_in],
                    state: slot.state,
                    outputs: slot.outputs,
                    tables: slot.tables,
                    population_inputs: slot.pin,
                    population_outputs: slot.pout,
                },
            )?;
        }
        for (plan, solver, mut members, scratch) in self.groups {
            let mut x = Vec::with_capacity(plan.len);
            for m in &members {
                x.extend_from_slice(m.state);
            }
            {
                let group = Members {
                    blocks: members.iter().map(|m| m.block.as_ref()).collect(),
                    params: members.iter().map(|m| m.params).collect(),
                    held: members.iter().map(|m| m.inputs).collect(),
                };
                let failed = RefCell::new(None);
                let scratch = RefCell::new(scratch);
                let rhs = |t: f64, x: &[f64], dx: &mut [f64]| {
                    let s = &mut **scratch.borrow_mut();
                    if let Err(e) = plan.rhs(&group, t, x, dx, s) {
                        dx.fill(f64::NAN);
                        failed.borrow_mut().get_or_insert(e);
                    }
                };
                solver.advance(&rhs, t0, dt, &mut x)?;
                if let Some(e) = failed.into_inner() {
                    return Err(e);
                }
            }
            for (m, &(_, off)) in members.iter_mut().zip(&plan.members) {
                let n = m.state.len();
                m.state.copy_from_slice(&x[off..off + n]);
                m.block.compute_outputs(
                    t0 + dt,
                    StepIo {
                        params: m.params,
                        inputs: &m.inputs[..m.n_in],
                        state: m.state,
                        outputs: m.outputs,
                        tables: m.tables,
                        population_inputs: m.pin,
                        population_outputs: m.pout,
                    },
                )?;
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
    plans: Vec<Arc<GroupPlan>>,
    states: Vec<Vec<f64>>,
    tables: Vec<Vec<Table>>,
    inputs: Vec<Vec<f64>>,
    outputs: Vec<Vec<f64>>,
    population_inputs: Vec<Vec<Vec<f64>>>,
    population_outputs: Vec<Vec<Vec<f64>>>,
    /// Each Block's random stream (ADR 0034).
    seeds: Vec<u64>,
    /// The seeds the model gives, before [`Simulator::reseed`].
    base_seeds: Vec<u64>,
    t: f64,
    /// Steps taken since the run started.
    step: u64,
    /// Blocks whose parameters changed since the last step: their outputs
    /// are recomputed before the next one, so the change acts in it.
    changed: Vec<usize>,
    /// Per Block: outputs declared copies of its state, as (output offset,
    /// state offset, length) (ADR 0063).
    copies: Vec<Vec<(usize, usize, usize)>>,
    /// Per Block: whether its outputs depend on its state and parameters
    /// only (no feedthrough, no populations), so a snapshot may compute
    /// them again instead of keeping them (ADR 0063).
    recomputable: Vec<bool>,
    /// With several clocks (ADR 0066 step 4b): each clock's time and steps
    /// taken, and per Block whether its clock has an instant now.
    clock_t: Vec<f64>,
    clock_taken: Vec<u128>,
    active: Vec<bool>,
    /// Per Block: the length of its inputs; its driven parameters (ADR
    /// 0075) follow them in `inputs`, one value each, in `driven` order.
    n_in: Vec<usize>,
    driven: Vec<Vec<usize>>,
    /// Blocks with driven starting values, in the order they are started
    /// again once their drivers' outputs are known.
    starting: Vec<usize>,
    /// Whether clocks stepping at one instant run on several cores, and
    /// what each took (ADR 0066 step 6).
    parallel: crate::parallel::Parallel,
    costs: crate::parallel::Costs,
    /// Each fused group's working arrays, kept between evaluations.
    scratch: Vec<GroupScratch>,
    /// Threads kept for the run, started the first time clocks are worth
    /// stepping on several cores.
    pool: Option<crate::parallel::Pool>,
    /// Per Block: whether its outputs need not be computed when it starts,
    /// before its inputs arrive: it has no state or tables and the output
    /// plan computes every output of it from its inputs, outside loops.
    /// A study, for one, would run every point on inputs of zero.
    late: Vec<bool>,
}

/// Stands in for a Block lent to a thread of the pool for one instant;
/// it is back before anything asks.
struct Lent;

impl Block for Lent {
    fn spec(&self) -> &crate::block::BlockSpec {
        unreachable!("a lent block is back before use")
    }
    fn method(&self) -> &crate::block::MethodSpec {
        unreachable!("a lent block is back before use")
    }
    fn init(&mut self, _: &[f64], _: &mut [f64]) -> Result<()> {
        unreachable!("a lent block is back before use")
    }
    fn do_step(&mut self, _: StepContext, _: StepIo<'_>) -> Result<()> {
        unreachable!("a lent block is back before use")
    }
    fn compute_outputs(&self, _: f64, _: StepIo<'_>) -> Result<()> {
        unreachable!("a lent block is back before use")
    }
}

/// Stands in for a fused group's solver lent to the pool.
struct LentSolver;

impl crate::fusion::GroupSolver for LentSolver {
    fn method(&self) -> &crate::block::MethodSpec {
        unreachable!("a lent solver is back before use")
    }
    fn advance(
        &mut self,
        _: &crate::fusion::GroupRhs<'_>,
        _: f64,
        _: f64,
        _: &mut [f64],
    ) -> Result<()> {
        unreachable!("a lent solver is back before use")
    }
}

/// One Block's part of the run, owned while lent to the pool: what a
/// [`Slot`] borrows.
struct OwnedSlot {
    b: usize,
    block: Box<dyn Block>,
    params: Vec<f64>,
    inputs: Vec<f64>,
    n_in: usize,
    state: Vec<f64>,
    outputs: Vec<f64>,
    tables: Vec<Table>,
    pin: Vec<Vec<f64>>,
    pout: Vec<Vec<f64>>,
    seed: u64,
}

impl OwnedSlot {
    fn lend(&mut self) -> Slot<'_> {
        Slot {
            block: &mut self.block,
            params: &self.params,
            inputs: &self.inputs,
            n_in: self.n_in,
            state: &mut self.state,
            outputs: &mut self.outputs,
            tables: &mut self.tables,
            pin: &self.pin,
            pout: &mut self.pout,
        }
    }
}

/// A fused group, owned while lent to the pool.
struct OwnedGroup {
    g: usize,
    plan: Arc<GroupPlan>,
    solver: Box<dyn crate::fusion::GroupSolver>,
    members: Vec<OwnedSlot>,
    scratch: GroupScratch,
}

/// One clock's work for an instant, owned, to step on a thread of the
/// pool and come back.
struct OwnedUnit {
    clock: usize,
    t0: f64,
    dt: f64,
    step: u64,
    blocks: Vec<OwnedSlot>,
    groups: Vec<OwnedGroup>,
}

impl OwnedUnit {
    fn step(mut self) -> (Self, Result<()>, Duration) {
        let start = Instant::now();
        let unit = Unit {
            clock: self.clock,
            t0: self.t0,
            dt: self.dt,
            step: self.step,
            blocks: self
                .blocks
                .iter_mut()
                .map(|s| {
                    let seed = s.seed;
                    (s.lend(), seed)
                })
                .collect(),
            groups: self
                .groups
                .iter_mut()
                .map(|g| {
                    let members = g.members.iter_mut().map(OwnedSlot::lend).collect();
                    (&*g.plan, &mut g.solver, members, &mut g.scratch)
                })
                .collect(),
        };
        let result = unit.step();
        let took = start.elapsed();
        (self, result, took)
    }
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
        // Driven parameters (ADR 0075): inputs of their own after the
        // block's ports.
        let n_in: Vec<usize> = model.blocks.iter().map(|b| b.spec().input_len()).collect();
        let driven: Vec<Vec<usize>> = (0..model.blocks.len())
            .map(|b| model.driven_params(b))
            .collect();
        for c in &model.drives {
            let (bi, from_spec) = (c.to.block.0, model.blocks[c.from.block.0].spec());
            let spec = model.blocks[bi].spec();
            let (from_offset, len) = port_offset(&from_spec.ports, Causality::Output, &c.from.port)
                .ok_or_else(|| SimError::BadConnection(c.from.port.clone()))?;
            let j = driven[bi]
                .iter()
                .position(|&i| spec.params[i].name == c.to.port)
                .expect("a driven parameter");
            routes.push(Route {
                from_block: c.from.block.0,
                from_offset,
                to_block: bi,
                to_offset: n_in[bi] + j,
                len,
                factor: c.factor,
            });
        }
        let starting = starting_order(&model, &routes, &n_in, &driven)?;

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
            inputs.push(vec![
                0.0;
                spec.input_len()
                    + model.driven_params(inputs.len()).len()
            ]);
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
        for (s, stream) in seeds.iter_mut().zip(&model.streams) {
            if let Some(x) = stream {
                *s = *x;
            }
        }

        let plans = (0..model.groups.len())
            .map(|g| GroupPlan::new(&model, g, &routes).map(Arc::new))
            .collect::<Result<Vec<_>>>()?;
        let units: Vec<(&crate::block::BlockSpec, &str, &[usize])> = model
            .blocks
            .iter()
            .zip(&driven)
            .map(|(b, d)| (b.spec(), b.spec().name.as_str(), d.as_slice()))
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
        // A block with driven parameters may have computed its outputs with
        // values since replaced: kept.
        let recomputable = model
            .blocks
            .iter()
            .zip(&driven)
            .map(|(b, d)| {
                d.is_empty()
                    && b.spec()
                        .outputs()
                        .all(|p| p.feedthrough.is_empty() && !p.shape.is_variable())
            })
            .collect();
        let clocks = model.clocks.len();
        let blocks = model.blocks.len();
        let groups = model.groups.len();
        let mut sim = Self {
            clock_t: vec![t0; clocks],
            clock_taken: vec![0; clocks],
            active: vec![true; blocks],
            copies,
            recomputable,
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
            base_seeds: seeds.clone(),
            seeds,
            t: t0,
            step: 0,
            changed: Vec::new(),
            n_in,
            driven,
            starting,
            parallel: crate::parallel::Parallel::default(),
            costs: crate::parallel::Costs::pooled(clocks),
            scratch: (0..groups).map(|_| GroupScratch::default()).collect(),
            pool: None,
            late: Vec::new(),
        };
        sim.late = (0..sim.model.blocks.len())
            .map(|b| {
                let spec = sim.model.blocks[b].spec();
                spec.states.is_empty()
                    && spec.tables.is_empty()
                    && sim.population_outputs[b].is_empty()
                    && sim.outputs_plan.computes_all_of(b, sim.outputs[b].len())
            })
            .collect();
        sim.restart(t0)?;
        Ok(sim)
    }

    /// Draw from the model's seeds mixed with `salt` (an experiment's
    /// point or replicate, ADR 0067), or from them as they are (`None`),
    /// from the next [`Simulator::restart`] on.
    pub fn reseed(&mut self, salt: Option<u64>) {
        self.seeds = self
            .base_seeds
            .iter()
            .map(|&s| match salt {
                Some(k) => crate::random::stream(s, &format!("run {k}")),
                None => s,
            })
            .collect();
    }

    /// Start the run again at `t0` from the current parameter values: every
    /// Block re-initialises its state, so changed initial conditions take
    /// effect. Snapshots taken earlier stay valid.
    pub fn restart(&mut self, t0: f64) -> Result<()> {
        for bi in 0..self.model.blocks.len() {
            self.start_block(bi, t0, true)?;
        }
        self.t = t0;
        self.step = 0;
        self.changed.clear();
        self.clock_t.fill(t0);
        self.clock_taken.fill(0);
        // Listed instants already past are not reached again.
        for (c, clock) in self.model.clocks.iter().enumerate() {
            self.clock_taken[c] = clock.times.partition_point(|&(_, t)| t <= t0) as u128;
        }
        self.active.fill(true);
        self.resolve(t0)?;
        self.exchange();
        // Driven starting values (ADR 0075): blocks start again from their
        // drivers' outputs, in order, until nothing they start from moves
        // (one pass per link of the longest chain).
        let starting = self.starting.clone();
        let read = |sim: &Self| -> Vec<u64> {
            starting
                .iter()
                .flat_map(|&b| {
                    sim.driven[b]
                        .iter()
                        .map(move |&i| sim.model.params[b][i].to_bits())
                })
                .collect()
        };
        for _ in 0..=starting.len() {
            let before = read(self);
            for &bi in &starting {
                self.start_block(bi, t0, false)?;
            }
            self.resolve(t0)?;
            self.exchange();
            if read(self) == before {
                break;
            }
        }
        Ok(())
    }

    /// Start Block `bi` from its parameters (or saved start) and compute
    /// its outputs at `t0`; `fresh` clears its inputs first.
    fn start_block(&mut self, bi: usize, t0: f64, fresh: bool) -> Result<()> {
        let block = &mut self.model.blocks[bi];
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
        if fresh {
            self.inputs[bi][..self.n_in[bi]].fill(0.0);
            for v in &mut self.population_inputs[bi] {
                v.clear();
            }
            // Its outputs come once its inputs do, in `resolve`.
            if self.late.get(bi).copied().unwrap_or(false) {
                return Ok(());
            }
        }
        block.compute_outputs(
            t0,
            StepIo {
                params,
                inputs: &self.inputs[bi][..self.n_in[bi]],
                state: &mut self.states[bi],
                outputs: &mut self.outputs[bi],
                tables: &mut self.tables[bi],
                population_inputs: &self.population_inputs[bi],
                population_outputs: &mut self.population_outputs[bi],
            },
        )
    }

    /// Apply changed parameters now, at the current time, as the next step
    /// would first: an input block set at a communication point by a link
    /// from another model (ADR 0065) acts on the outputs at that point.
    pub fn refresh(&mut self) -> Result<()> {
        if self.changed.is_empty() {
            return Ok(());
        }
        self.refresh_changed()
    }

    /// Recompute the outputs of Blocks whose parameters changed, then the
    /// outputs that depend on their inputs, and pass them on: a parameter
    /// changed between steps (a slider, an input block, ADR 0058) acts in
    /// the next step rather than one step later.
    fn refresh_changed(&mut self) -> Result<()> {
        let t = self.t;
        // A changed parameter acts at once, whatever the clocks.
        self.active.fill(true);
        for bi in std::mem::take(&mut self.changed) {
            let params = &self.model.params[bi];
            self.model.blocks[bi].compute_outputs(
                t,
                StepIo {
                    params,
                    inputs: &self.inputs[bi][..self.n_in[bi]],
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
        let params = &mut self.model.params;
        let (n_in, driven) = (&self.n_in, &self.driven);
        let (states, tables) = (&mut self.states, &mut self.tables);
        let (pin, pout) = (&self.population_inputs, &mut self.population_outputs);
        let active = (!self.model.clocks.is_empty()).then_some(self.active.as_slice());
        // With several clocks a block computes at its own clock's time.
        let (clock_of, clock_t, clocks) = (&self.model.clock_of, &self.clock_t, &self.model.clocks);
        let time = |b: usize| match clocks.get(clock_of[b]) {
            Some(c) if !c.on_data => clock_t[clock_of[b]],
            _ => t,
        };
        self.outputs_plan.solve_active(
            self.model.loops,
            &mut self.outputs,
            &mut self.inputs,
            &mut |b, u, y| {
                // Driven parameters as the routes left them (ADR 0075).
                for (j, &i) in driven[b].iter().enumerate() {
                    params[b][i] = u[n_in[b] + j];
                }
                blocks[b].compute_outputs(
                    time(b),
                    StepIo {
                        params: &params[b],
                        inputs: &u[..n_in[b]],
                        state: &mut states[b],
                        outputs: y,
                        tables: &mut tables[b],
                        population_inputs: &pin[b],
                        population_outputs: &mut pout[b],
                    },
                )
            },
            active,
        )
    }

    /// The algebraic loops of the model, each as the outputs in it
    /// (`block.port`), found when it was built (ADR 0040).
    pub fn loops(&self) -> Vec<Vec<String>> {
        self.outputs_plan.loops()
    }

    /// Copy outputs to connected inputs, applying unit conversion.
    fn exchange(&mut self) {
        // With several clocks a block takes its inputs only at its own
        // instants, holding them over its step (ADR 0066 step 4b).
        let several = !self.model.clocks.is_empty();
        for r in &self.routes {
            if several && !self.active[r.to_block] {
                continue;
            }
            let from = &self.outputs[r.from_block][r.from_offset..r.from_offset + r.len];
            let to = &mut self.inputs[r.to_block][r.to_offset..r.to_offset + r.len];
            // Times one is the same number, bit for bit: copied as a block.
            if r.factor == 1.0 {
                to.copy_from_slice(from);
            } else {
                to.iter_mut().zip(from).for_each(|(t, f)| *t = r.factor * f);
            }
        }
        for r in &self.population_routes {
            if several && !self.active[r.to_block] {
                continue;
            }
            let from = &self.population_outputs[r.from_block][r.from_index];
            let to = &mut self.population_inputs[r.to_block][r.to_index];
            to.clear();
            to.extend(from.iter().map(|v| r.factor * v));
        }
        // Driven parameters take the values routed to them (ADR 0075).
        for (b, driven) in self.driven.iter().enumerate() {
            for (j, &i) in driven.iter().enumerate() {
                self.model.params[b][i] = self.inputs[b][self.n_in[b] + j];
            }
        }
    }

    /// Advance every Block by one communication step `dt`. With several
    /// clocks ([`Model::set_clocks`]), advance to the next instant of any
    /// of them instead; `dt` is not used.
    pub fn step(&mut self, dt: f64) -> Result<()> {
        if !self.model.clocks.is_empty() {
            return self.advance();
        }
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
                    inputs: &self.inputs[bi][..self.n_in[bi]],
                    state: &mut self.states[bi],
                    outputs: &mut self.outputs[bi],
                    tables: &mut self.tables[bi],
                    population_inputs: &self.population_inputs[bi],
                    population_outputs: &mut self.population_outputs[bi],
                },
            )?;
        }
        for ((plan, group), scratch) in self
            .plans
            .iter()
            .zip(&mut self.model.groups)
            .zip(&mut self.scratch)
        {
            let mut x = Vec::with_capacity(plan.len);
            for &(b, _) in &plan.members {
                x.extend_from_slice(&self.states[b]);
            }
            let ids = || plan.members.iter().map(|&(b, _)| b);
            let inside = Members {
                blocks: ids().map(|b| self.model.blocks[b].as_ref()).collect(),
                params: ids().map(|b| self.model.params[b].as_slice()).collect(),
                held: ids().map(|b| self.inputs[b].as_slice()).collect(),
            };
            // The solver's right-hand side cannot fail; a loop that does
            // not converge is reported after the step.
            let failed = RefCell::new(None);
            let scratch = RefCell::new(scratch);
            let rhs = |t: f64, x: &[f64], dx: &mut [f64]| {
                let s = &mut **scratch.borrow_mut();
                if let Err(e) = plan.rhs(&inside, t, x, dx, s) {
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
                        inputs: &self.inputs[b][..self.n_in[b]],
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
    /// With several clocks, the steps each has taken.
    pub fn clock_steps(&self) -> &[u128] {
        &self.clock_taken
    }

    /// The next instant of any clock, as a time: when the next step ends.
    /// Without clocks, `None` (each step is the `dt` given); `None` too
    /// when every clock has passed its last listed instant.
    pub fn next_instant(&self) -> Option<f64> {
        self.next_tick().map(|(_, t)| t)
    }

    /// Clock `c`'s next instant, in ticks and seconds: its next step, or
    /// its next listed time (ADR 0067 step 4).
    fn instant_of(&self, c: usize) -> Option<(u128, f64)> {
        let clock = &self.model.clocks[c];
        let n = self.clock_taken[c];
        if !clock.times.is_empty() {
            return clock.times.get(n as usize).copied();
        }
        (!clock.on_data && clock.every > 0)
            .then(|| ((n + 1) * clock.every, self.clock_t[c] + clock.dt))
    }

    /// The next instant of any clock, in ticks and seconds.
    fn next_tick(&self) -> Option<(u128, f64)> {
        (0..self.model.clocks.len())
            .filter_map(|c| self.instant_of(c))
            .min_by_key(|&(tick, _)| tick)
    }

    /// One instant with several clocks (ADR 0066 step 4b): the clocks
    /// reaching it step their blocks over their own `dt`, with their
    /// inputs as they took them at their last instant; then those blocks,
    /// and those on new data, compute their outputs in dependency order
    /// and take their inputs. The others hold theirs.
    fn advance(&mut self) -> Result<()> {
        if !self.changed.is_empty() {
            self.refresh_changed()?;
        }
        let clocks = self.model.clocks.clone();
        let Some((tick, now)) = self.next_tick() else {
            return Ok(());
        };
        // The clocks with an instant now (`listed` those at a listed time),
        // and each stepping one's step: its `dt`, or to its listed time.
        let listed: Vec<bool> = (0..clocks.len())
            .map(|c| !clocks[c].times.is_empty() && self.instant_of(c).map(|i| i.0) == Some(tick))
            .collect();
        let reaching: Vec<bool> = (0..clocks.len())
            .map(|c| clocks[c].on_data || self.instant_of(c).map(|i| i.0) == Some(tick))
            .collect();
        let dts: Vec<f64> = (0..clocks.len())
            .map(|c| {
                if listed[c] {
                    now - self.clock_t[c]
                } else {
                    clocks[c].dt
                }
            })
            .collect();
        for (b, a) in self.active.iter_mut().enumerate() {
            *a = reaching[self.model.clock_of[b]];
        }
        // On new data, blocks step only to their own listed instants: their
        // data's samples, which only a step brings (ADR 0067 step 4).
        let stepping = |c: usize| reaching[c] && (!clocks[c].on_data || listed[c]);
        // Each clock stepping now is a unit: its blocks and fused groups,
        // which touch nothing of another clock's, their inputs held. Units
        // run on several cores when worth it, with the same bits (ADR 0066
        // step 6).
        let in_group: Vec<bool> = (0..self.model.blocks.len())
            .map(|b| self.model.group_of(BlockId(b)).is_some())
            .collect();
        let clocks_now: Vec<usize> = (0..clocks.len()).filter(|&c| stepping(c)).collect();
        let parallel = self.costs.worth(self.parallel, &clocks_now);
        let result = if parallel {
            self.step_lent(&clocks_now, &dts, &in_group)
        } else {
            self.step_here(&clocks_now, &dts, &in_group)
        };
        result?;
        for (c, clock) in clocks.iter().enumerate() {
            if listed[c] {
                // At its listed time exactly; one on data counts its
                // instants too.
                self.clock_t[c] = now;
                self.clock_taken[c] += 1;
            } else if stepping(c) {
                self.clock_t[c] += clock.dt;
                self.clock_taken[c] += 1;
            }
        }
        self.t = now;
        self.step += 1;
        self.resolve(self.t)?;
        self.exchange();
        Ok(())
    }

    /// Step the clocks `now` (each by its `dts`) one after another on this
    /// thread, their blocks borrowed where they are.
    fn step_here(&mut self, now: &[usize], dts: &[f64], in_group: &[bool]) -> Result<()> {
        let clock_of = &self.model.clock_of;
        let (params, inputs, n_in) = (&self.model.params, &self.inputs, &self.n_in);
        let pin = &self.population_inputs;
        let mut slots: Vec<Option<Slot>> = self
            .model
            .blocks
            .iter_mut()
            .zip(self.states.iter_mut())
            .zip(self.outputs.iter_mut())
            .zip(self.tables.iter_mut())
            .zip(self.population_outputs.iter_mut())
            .enumerate()
            .map(|(b, ((((block, state), outputs), tables), pout))| {
                Some(Slot {
                    block,
                    params: &params[b],
                    inputs: &inputs[b],
                    n_in: n_in[b],
                    state,
                    outputs,
                    tables,
                    pin: &pin[b],
                    pout,
                })
            })
            .collect();
        let mut units: Vec<Unit> = now
            .iter()
            .map(|&c| Unit {
                clock: c,
                t0: self.clock_t[c],
                dt: dts[c],
                step: self.clock_taken[c] as u64,
                blocks: Vec::new(),
                groups: Vec::new(),
            })
            .collect();
        let unit_of = |c: usize| units.iter().position(|u| u.clock == c);
        let mut placed: Vec<(usize, usize)> = Vec::new();
        for b in 0..slots.len() {
            if !in_group[b] {
                if let Some(u) = unit_of(clock_of[b]) {
                    placed.push((u, b));
                }
            }
        }
        for (u, b) in placed {
            let slot = slots[b].take().expect("each block once");
            units[u].blocks.push((slot, self.seeds[b]));
        }
        for ((plan, group), scratch) in self
            .plans
            .iter()
            .zip(self.model.groups.iter_mut())
            .zip(self.scratch.iter_mut())
        {
            let Some(&(first, _)) = plan.members.first() else {
                continue;
            };
            let Some(u) = units.iter().position(|u| u.clock == clock_of[first]) else {
                continue;
            };
            let members = plan
                .members
                .iter()
                .map(|&(b, _)| slots[b].take().expect("each member once"))
                .collect();
            units[u]
                .groups
                .push((plan, &mut group.solver, members, scratch));
        }
        let done: Vec<_> = units
            .into_iter()
            .map(|unit| {
                let start = Instant::now();
                let clock = unit.clock;
                (clock, unit.step(), start.elapsed())
            })
            .collect();
        drop(slots);
        let mut failed = None;
        for (clock, result, took) in done {
            self.costs.record(clock, took);
            if let (Err(e), None) = (result, &failed) {
                failed = Some(e);
            }
        }
        failed.map_or(Ok(()), Err)
    }

    /// Step the clocks `now` on the pool's threads: each clock's blocks
    /// and groups are lent to it owned (moved, not copied) and come back
    /// stepped; the same bits as on one thread (ADR 0066 step 6).
    fn step_lent(&mut self, now: &[usize], dts: &[f64], in_group: &[bool]) -> Result<()> {
        let mut units: Vec<OwnedUnit> = now
            .iter()
            .map(|&c| OwnedUnit {
                clock: c,
                t0: self.clock_t[c],
                dt: dts[c],
                step: self.clock_taken[c] as u64,
                blocks: Vec::new(),
                groups: Vec::new(),
            })
            .collect();
        let unit_of = |units: &[OwnedUnit], c: usize| units.iter().position(|u| u.clock == c);
        for (b, &grouped) in in_group.iter().enumerate() {
            if grouped {
                continue;
            }
            if let Some(u) = unit_of(&units, self.model.clock_of[b]) {
                let slot = self.take_slot(b);
                units[u].blocks.push(slot);
            }
        }
        for g in 0..self.plans.len() {
            let plan = Arc::clone(&self.plans[g]);
            let Some(&(first, _)) = plan.members.first() else {
                continue;
            };
            let Some(u) = unit_of(&units, self.model.clock_of[first]) else {
                continue;
            };
            let members = plan
                .members
                .iter()
                .map(|&(b, _)| self.take_slot(b))
                .collect();
            let solver = std::mem::replace(&mut self.model.groups[g].solver, Box::new(LentSolver));
            let scratch = std::mem::take(&mut self.scratch[g]);
            units[u].groups.push(OwnedGroup {
                g,
                plan,
                solver,
                members,
                scratch,
            });
        }
        let threads = crate::parallel::threads();
        let pool = self
            .pool
            .get_or_insert_with(|| crate::parallel::Pool::new(threads.saturating_sub(1)));
        let done = pool.each(units, OwnedUnit::step);
        let mut failed = None;
        for (unit, result, took) in done {
            self.costs.record(unit.clock, took);
            for slot in unit.blocks {
                self.put_slot(slot);
            }
            for group in unit.groups {
                self.model.groups[group.g].solver = group.solver;
                self.scratch[group.g] = group.scratch;
                for slot in group.members {
                    self.put_slot(slot);
                }
            }
            if let (Err(e), None) = (result, &failed) {
                failed = Some(e);
            }
        }
        failed.map_or(Ok(()), Err)
    }

    /// Block `b`'s part of the run, taken out to lend.
    fn take_slot(&mut self, b: usize) -> OwnedSlot {
        OwnedSlot {
            b,
            block: std::mem::replace(&mut self.model.blocks[b], Box::new(Lent)),
            params: std::mem::take(&mut self.model.params[b]),
            inputs: std::mem::take(&mut self.inputs[b]),
            n_in: self.n_in[b],
            state: std::mem::take(&mut self.states[b]),
            outputs: std::mem::take(&mut self.outputs[b]),
            tables: std::mem::take(&mut self.tables[b]),
            pin: std::mem::take(&mut self.population_inputs[b]),
            pout: std::mem::take(&mut self.population_outputs[b]),
            seed: self.seeds[b],
        }
    }

    /// A lent part put back.
    fn put_slot(&mut self, s: OwnedSlot) {
        let b = s.b;
        self.model.blocks[b] = s.block;
        self.model.params[b] = s.params;
        self.inputs[b] = s.inputs;
        self.states[b] = s.state;
        self.outputs[b] = s.outputs;
        self.tables[b] = s.tables;
        self.population_inputs[b] = s.pin;
        self.population_outputs[b] = s.pout;
    }

    pub fn model(&self) -> &Model {
        &self.model
    }

    /// Whether clocks stepping at one instant run on several cores (ADR
    /// 0066 step 6): by their cost (the default), never, or always. The
    /// results are the same bits in every case.
    pub fn set_parallel(&mut self, parallel: crate::parallel::Parallel) {
        self.parallel = parallel;
    }

    /// Change a parameter while running (live sliders). Checked like
    /// [`Model::set_param`].
    pub fn set_param(&mut self, id: BlockId, name: &str, value: f64) -> Result<()> {
        if self.model.is_driven(id, name) {
            return Err(SimError::InvalidSetting {
                setting: name.into(),
                reason: "it is driven by a wire (ADR 0075)".into(),
            });
        }
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
        let mut outputs = Vec::with_capacity(self.outputs.len());
        let mut kept = Vec::with_capacity(self.outputs.len());
        for (bi, y) in self.outputs.iter().enumerate() {
            // Computed again from the state, bit for bit: none kept.
            if self.recomputable[bi]
                && !y.is_empty()
                && self
                    .recompute(bi, &self.states[bi], &self.tables[bi])
                    .is_some_and(|z| bits_equal(&z, y))
            {
                outputs.push(Vec::new());
                kept.push(Kept::None);
                continue;
            }
            // Copies of the state, checked bit for bit, are left out.
            let (x, copies) = (&self.states[bi], &self.copies[bi]);
            let copied = !copies.is_empty()
                && copies
                    .iter()
                    .all(|&(o, s, n)| bits_equal(&y[o..o + n], &x[s..s + n]));
            if copied {
                let mut rest = Vec::with_capacity(y.len());
                let mut at = 0;
                let mut ranges = copies.clone();
                ranges.sort_unstable();
                for (o, _, n) in ranges {
                    rest.extend_from_slice(&y[at..o]);
                    at = o + n;
                }
                rest.extend_from_slice(&y[at..]);
                outputs.push(rest);
                kept.push(Kept::ButCopies);
            } else {
                outputs.push(y.clone());
                kept.push(Kept::All);
            }
        }
        Snapshot {
            t: self.t,
            step: self.step,
            states: self.states.clone(),
            tables: self.tables.clone(),
            outputs,
            kept,
            population_outputs: self.population_outputs.clone(),
            params: self.model.params.clone(),
            clocks: self
                .clock_t
                .iter()
                .copied()
                .zip(self.clock_taken.iter().copied())
                .collect(),
            inputs: if self.model.clocks.is_empty() {
                Vec::new()
            } else {
                self.inputs.clone()
            },
        }
    }

    /// Block `bi`'s outputs computed from state `x` and tables at the
    /// current time and parameters, on copies, leaving the run as it is.
    fn recompute(&self, bi: usize, x: &[f64], tables: &[Table]) -> Option<Vec<f64>> {
        let mut y = vec![0.0; self.outputs[bi].len()];
        let mut x = x.to_vec();
        let mut tables = tables.to_vec();
        let mut populations: Vec<Vec<f64>> = vec![Vec::new(); self.population_outputs[bi].len()];
        self.model.blocks[bi]
            .compute_outputs(
                self.t,
                StepIo {
                    params: &self.model.params[bi],
                    inputs: &self.inputs[bi][..self.n_in[bi]],
                    state: &mut x,
                    outputs: &mut y,
                    tables: &mut tables,
                    population_inputs: &self.population_inputs[bi],
                    population_outputs: &mut populations,
                },
            )
            .ok()?;
        Some(y)
    }

    /// Rewind to a snapshot taken from this simulator, including the
    /// parameter values in effect when it was taken.
    pub fn restore(&mut self, snap: &Snapshot) {
        self.changed.clear();
        self.t = snap.t;
        for (c, &(t, n)) in snap.clocks.iter().enumerate() {
            self.clock_t[c] = t;
            self.clock_taken[c] = n;
        }
        self.step = snap.step;
        self.states.clone_from(&snap.states);
        self.tables.clone_from(&snap.tables);
        self.model.params.clone_from(&snap.params);
        for b in 0..self.outputs.len() {
            match snap.kept[b] {
                Kept::All => self.outputs[b].clone_from(&snap.outputs[b]),
                Kept::None => {
                    // As checked when the snapshot was taken.
                    let y = self.recompute(b, &snap.states[b], &snap.tables[b]);
                    let n = self.outputs[b].len();
                    self.outputs[b] = y.unwrap_or_else(|| vec![f64::NAN; n]);
                }
                Kept::ButCopies => {
                    // The kept outputs between the copies, then the copies.
                    let mut ranges = self.copies[b].clone();
                    ranges.sort_unstable();
                    let (rest, x) = (&snap.outputs[b], &snap.states[b]);
                    let y = &mut self.outputs[b];
                    let (mut at, mut from) = (0, 0);
                    for (o, s, n) in ranges {
                        y[at..o].copy_from_slice(&rest[from..from + o - at]);
                        from += o - at;
                        y[o..o + n].copy_from_slice(&x[s..s + n]);
                        at = o + n;
                    }
                    y[at..].copy_from_slice(&rest[from..]);
                }
            }
        }
        self.population_outputs.clone_from(&snap.population_outputs);
        if snap.clocks.is_empty() {
            self.exchange();
        } else {
            // Inputs as each block last took them.
            self.inputs.clone_from(&snap.inputs);
        }
    }
}

/// The blocks whose starting values are driven (ADR 0075), drivers first;
/// refused when a driven starting value depends on itself at the start,
/// through its block's outputs, feedthrough or other driven values.
fn starting_order(
    model: &Model,
    routes: &[Route],
    n_in: &[usize],
    driven: &[Vec<usize>],
) -> Result<Vec<usize>> {
    let spec = |b: usize| model.blocks[b].spec();
    let starts = |b: usize| {
        driven[b]
            .iter()
            .any(|&i| spec(b).params[i].initial_condition)
    };
    if !(0..model.blocks.len()).any(starts) {
        return Ok(Vec::new());
    }
    // Edges between blocks along which a value reaches outputs at the
    // start: a driven starting value; a driven parameter or an input an
    // output reads at the same instant.
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); model.blocks.len()];
    let mut start_edges = Vec::new();
    for r in routes {
        let (src, dst) = (r.from_block, r.to_block);
        let s = spec(dst);
        let reaches = if r.to_offset >= n_in[dst] {
            let q = &s.params[driven[dst][r.to_offset - n_in[dst]]];
            if q.initial_condition {
                start_edges.push((src, dst, q.name.clone()));
                true
            } else {
                s.outputs().any(|p| p.reads_param(&q.name))
            }
        } else {
            let mut at = 0;
            let name = s
                .inputs()
                .find(|p| {
                    let hit = r.to_offset >= at && r.to_offset < at + p.shape.len();
                    at += p.shape.len();
                    hit
                })
                .map(|p| p.name.clone())
                .unwrap_or_default();
            s.outputs().any(|p| p.feedthrough.contains(&name))
        };
        if reaches && !edges[src].contains(&dst) {
            edges[src].push(dst);
        }
    }
    let order = crate::loops::components(&edges);
    let component = |b: usize| order.iter().position(|c| c.contains(&b));
    if let Some((src, dst, name)) = start_edges
        .iter()
        .find(|(src, dst, _)| component(*src) == component(*dst))
    {
        return Err(SimError::InvalidDefinition(format!(
            "the starting value {}.{name} is driven by {}, whose outputs at the start depend \
             on it: driven starting values cannot form a cycle",
            spec(*dst).name,
            spec(*src).name
        )));
    }
    Ok(order.into_iter().flatten().filter(|&b| starts(b)).collect())
}
