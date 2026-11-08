//! The model graph: Blocks, their parameter values, and unit-checked connections.

use crate::block::{Block, BlockSpec, Causality, PortSpec, TimeSemantics};
use crate::fusion::{Group, GroupSolver};
use crate::{Result, SimError};

/// Index of a Block within a [`Model`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(pub usize);

/// A Block's port, addressed by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortRef {
    pub block: BlockId,
    pub port: String,
}

/// A checked signal connection from an output port to an input port.
#[derive(Debug, Clone, PartialEq)]
pub struct Connection {
    pub from: PortRef,
    pub to: PortRef,
    /// Unit conversion applied to every value: `to = factor * from`.
    pub factor: f64,
}

/// The seed of a model that sets none ([`Model::set_seed`]).
pub const DEFAULT_SEED: u64 = 1;

/// A graph of Blocks and the connections between them.
pub struct Model {
    pub(crate) blocks: Vec<Box<dyn Block>>,
    pub(crate) params: Vec<Vec<f64>>,
    pub(crate) connections: Vec<Connection>,
    /// Parameters driven by wires (ADR 0075): `to.port` names a parameter.
    pub(crate) drives: Vec<Connection>,
    pub(crate) groups: Vec<Group>,
    pub(crate) seed: u64,
    pub(crate) loops: crate::loops::LoopMethod,
    /// Per Block, a state to start from instead of the one its parameters
    /// give (ADR 0042).
    pub(crate) starts: Vec<Option<Start>>,
    /// Clocks the blocks step on, when they step on several (ADR 0066
    /// step 4b); empty for one clock, the step given to `Simulator::step`.
    pub(crate) clocks: Vec<Clock>,
    /// Per Block, its clock in `clocks`.
    pub(crate) clock_of: Vec<usize>,
    /// Per Block, its random stream when not the model's seed and its
    /// name: a block of a used model draws as it would there.
    pub(crate) streams: Vec<Option<u64>>,
}

/// A clock some blocks of a model step on (ADR 0066 step 4b): every `dt`,
/// counted as `every` ticks of the model's common tick, or, on new data,
/// evaluated at every instant of the others and never stepped. Listed
/// `times` (ADR 0067 step 4) are instants of its own: a clock with
/// `every` 0 steps from one to the next; one on new data is evaluated at
/// them too (its data's sample times).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Clock {
    pub dt: f64,
    pub every: u128,
    pub on_data: bool,
    /// Ascending instants after the start, in ticks and in seconds.
    pub times: Vec<(u128, f64)>,
}

impl Clock {
    /// Whether it gives instants of its own: every `dt`, or listed.
    pub fn in_time(&self) -> bool {
        (!self.on_data && self.every > 0) || !self.times.is_empty()
    }
}

/// A Block's state and table rows to start from, replacing what `init`
/// and `init_tables` compute from its parameters: how a run continues
/// from a state reached before (e.g. a steady state), reproducibly
/// (ADR 0042).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Start {
    /// One value per state, in `BlockSpec::states` order.
    pub state: Vec<f64>,
    /// Per table in `BlockSpec::tables` order, its rows.
    pub tables: Vec<Vec<Vec<f64>>>,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            blocks: Vec::new(),
            params: Vec::new(),
            connections: Vec::new(),
            drives: Vec::new(),
            groups: Vec::new(),
            seed: DEFAULT_SEED,
            loops: crate::loops::LoopMethod::default(),
            starts: Vec::new(),
            clocks: Vec::new(),
            clock_of: Vec::new(),
            streams: Vec::new(),
        }
    }
}

impl Model {
    pub fn new() -> Self {
        Self::default()
    }

    /// The seed every random draw derives from (ADR 0034): the same seed
    /// gives the same run.
    pub fn set_seed(&mut self, seed: u64) {
        self.seed = seed;
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Step blocks on several clocks (ADR 0066 step 4b): `clock_of` gives
    /// each Block's clock in `clocks`. `Simulator::step` then advances to
    /// the next instant of any clock; blocks step at their clock's instants
    /// only, and take their inputs only then.
    pub fn set_clocks(&mut self, clocks: Vec<Clock>, clock_of: Vec<usize>) -> Result<()> {
        if clock_of.len() != self.blocks.len() || clock_of.iter().any(|&c| c >= clocks.len()) {
            return Err(SimError::InvalidSetting {
                setting: "clocks".into(),
                reason: "every block needs one of the clocks".into(),
            });
        }
        if !clocks.iter().any(Clock::in_time) {
            return Err(SimError::InvalidSetting {
                setting: "clocks".into(),
                reason: "at least one clock runs in time".into(),
            });
        }
        if clocks.iter().any(|c| c.every > 0 && !c.times.is_empty()) {
            return Err(SimError::InvalidSetting {
                setting: "clocks".into(),
                reason: "a clock steps every dt or at listed times, not both".into(),
            });
        }
        self.clocks = clocks;
        self.clock_of = clock_of;
        Ok(())
    }

    /// Back to one clock, the step given to `Simulator::step`.
    pub fn clear_clocks(&mut self) {
        self.clocks.clear();
        self.clock_of.fill(0);
    }

    /// The last listed instant of any clock, in seconds: where a run that
    /// only data drives ends (ADR 0067 step 4).
    pub fn last_listed(&self) -> Option<f64> {
        self.clocks
            .iter()
            .filter_map(|c| c.times.last().map(|t| t.1))
            .reduce(f64::max)
    }

    /// The clocks set by [`Model::set_clocks`], and each Block's.
    pub fn clocks(&self) -> (&[Clock], &[usize]) {
        (&self.clocks, &self.clock_of)
    }

    /// The random stream of a Block, instead of the one from the model's
    /// seed and its name (ADR 0066: a block of a used model draws as it
    /// would in that model alone).
    pub fn set_stream(&mut self, block: BlockId, stream: u64) -> Result<()> {
        self.block(block)?;
        self.streams[block.0] = Some(stream);
        Ok(())
    }

    /// How algebraic loops are solved (ADR 0040).
    pub fn set_loop_method(&mut self, method: crate::loops::LoopMethod) {
        self.loops = method;
    }

    pub fn loop_method(&self) -> crate::loops::LoopMethod {
        self.loops
    }

    /// Add a Block with its default parameter values.
    pub fn add(&mut self, block: Box<dyn Block>) -> BlockId {
        self.params.push(block.spec().default_params());
        self.starts.push(None);
        self.clock_of.push(0);
        self.streams.push(None);
        self.blocks.push(block);
        BlockId(self.blocks.len() - 1)
    }

    /// Set a parameter by name. Any finite value is accepted, including
    /// values outside the suggested range.
    pub fn set_param(&mut self, block: BlockId, name: &str, value: f64) -> Result<()> {
        let idx = param_index(self.block(block)?.spec(), name)?;
        if !value.is_finite() {
            return Err(SimError::InvalidParam {
                name: name.into(),
                value,
            });
        }
        self.params[block.0][idx] = value;
        Ok(())
    }

    /// Start a Block from `start` instead of its parameters' initial
    /// state, or from its parameters again with `None`. Checked against
    /// the Block's states and tables.
    pub fn set_start(&mut self, block: BlockId, start: Option<Start>) -> Result<()> {
        let spec = self.block(block)?.spec();
        if let Some(s) = &start {
            let bad = |reason: String| SimError::InvalidSetting {
                setting: "start".into(),
                reason,
            };
            if s.state.len() != spec.states.len() {
                return Err(bad(format!(
                    "{} has {} states, not {}",
                    spec.name,
                    spec.states.len(),
                    s.state.len()
                )));
            }
            if s.tables.len() != spec.tables.len() {
                return Err(bad(format!(
                    "{} has {} tables, not {}",
                    spec.name,
                    spec.tables.len(),
                    s.tables.len()
                )));
            }
            for (t, rows) in spec.tables.iter().zip(&s.tables) {
                if let Some(r) = rows.iter().find(|r| r.len() != t.columns.len()) {
                    return Err(bad(format!(
                        "rows of {}.{} have {} values ({}), not {}",
                        spec.name,
                        t.name,
                        t.columns.len(),
                        t.columns.join(", "),
                        r.len()
                    )));
                }
            }
            let values = s.state.iter().chain(s.tables.iter().flatten().flatten());
            if values.into_iter().any(|v| !v.is_finite()) {
                return Err(bad("values must be finite".into()));
            }
        }
        self.starts[block.0] = start;
        Ok(())
    }

    /// The state a Block starts from, if not its parameters'.
    pub fn start(&self, block: BlockId) -> Option<&Start> {
        self.starts.get(block.0)?.as_ref()
    }

    /// Current value of a parameter.
    pub fn param(&self, block: BlockId, name: &str) -> Result<f64> {
        let idx = param_index(self.block(block)?.spec(), name)?;
        Ok(self.params[block.0][idx])
    }

    /// Current values of all parameters of a Block, in `BlockSpec::params` order.
    pub fn params(&self, block: BlockId) -> Result<&[f64]> {
        self.block(block)?;
        Ok(&self.params[block.0])
    }

    /// Ids of all Blocks, in insertion order.
    pub fn block_ids(&self) -> impl Iterator<Item = BlockId> {
        (0..self.blocks.len()).map(BlockId)
    }

    /// Connect an output port to an input port, checking direction, shape and
    /// unit: the units must be the same, scale included (ADR 0045).
    pub fn connect(&mut self, from: (BlockId, &str), to: (BlockId, &str)) -> Result<()> {
        let out = self.port(from.0, from.1)?.clone();
        let inp = self.port(to.0, to.1)?.clone();
        if out.causality != Causality::Output || inp.causality != Causality::Input {
            return Err(SimError::BadConnection(format!(
                "'{}' -> '{}' must go from an output to an input",
                from.1, to.1
            )));
        }
        // The same lengths along the same axes in order (ADR 0081); the
        // axes' names belong to each block.
        if out.shape != inp.shape {
            return Err(SimError::ShapeMismatch {
                from: crate::block::describe_shape(&out.shape),
                to: crate::block::describe_shape(&inp.shape),
            });
        }
        if !out.unit.same_as(&inp.unit) {
            return Err(SimError::UnitMismatch {
                from: out.unit.to_string(),
                to: inp.unit.to_string(),
            });
        }
        let factor = 1.0;
        let to_ref = PortRef {
            block: to.0,
            port: to.1.into(),
        };
        if self.connections.iter().any(|c| c.to == to_ref) {
            return Err(SimError::BadConnection(format!(
                "input '{}' is already connected",
                to.1
            )));
        }
        self.connections.push(Connection {
            from: PortRef {
                block: from.0,
                port: from.1.into(),
            },
            to: to_ref,
            factor,
        });
        Ok(())
    }

    /// Drive a parameter by an output (ADR 0075): the parameter takes the
    /// output's value whenever its Block reads it, in place of its own.
    /// Checked as a connection is: a scalar output in the parameter's unit.
    pub fn drive(&mut self, from: (BlockId, &str), to: (BlockId, &str)) -> Result<()> {
        let out = self.port(from.0, from.1)?.clone();
        let spec = self.block(to.0)?.spec();
        let idx = param_index(spec, to.1)?;
        let param = &spec.params[idx];
        if out.causality != Causality::Output {
            return Err(SimError::BadConnection(format!(
                "'{}' -> '{}' must come from an output",
                from.1, to.1
            )));
        }
        if out.shape != crate::block::Shape::Scalar {
            return Err(SimError::ShapeMismatch {
                from: format!("{:?}", out.shape),
                to: "Scalar".into(),
            });
        }
        if !out.unit.same_as(&param.unit) {
            return Err(SimError::UnitMismatch {
                from: out.unit.to_string(),
                to: param.unit.to_string(),
            });
        }
        let to_ref = PortRef {
            block: to.0,
            port: to.1.into(),
        };
        if self.drives.iter().any(|c| c.to == to_ref) {
            return Err(SimError::BadConnection(format!(
                "parameter '{}' is already driven",
                to.1
            )));
        }
        self.drives.push(Connection {
            from: PortRef {
                block: from.0,
                port: from.1.into(),
            },
            to: to_ref,
            factor: 1.0,
        });
        Ok(())
    }

    /// The parameters driven by wires (ADR 0075), `to.port` naming each.
    pub fn drives(&self) -> &[Connection] {
        &self.drives
    }

    /// Whether a parameter is driven by a wire.
    pub fn is_driven(&self, block: BlockId, name: &str) -> bool {
        self.drives
            .iter()
            .any(|c| c.to.block == block && c.to.port == name)
    }

    /// The indices of a Block's driven parameters, in `BlockSpec::params`
    /// order.
    pub(crate) fn driven_params(&self, block: usize) -> Vec<usize> {
        let spec = self.blocks[block].spec();
        (0..spec.params.len())
            .filter(|&i| self.is_driven(BlockId(block), &spec.params[i].name))
            .collect()
    }

    /// Solve these Blocks together as one system with `solver` (ADR 0023).
    /// Each must be an equation Block (it implements `Equations`), continuous
    /// or stateless (ADR 0051), in no other group, and at least one must be
    /// continuous; connections between members are then resolved inside
    /// every right-hand-side evaluation instead of once per step.
    pub fn fuse(&mut self, members: &[BlockId], solver: Box<dyn GroupSolver>) -> Result<()> {
        if members.is_empty() {
            return Err(SimError::CannotFuse(
                "a group needs at least one block".into(),
            ));
        }
        for (k, &id) in members.iter().enumerate() {
            let block = self.block(id)?;
            let name = &block.spec().name;
            if members[..k].contains(&id) {
                return Err(SimError::CannotFuse(format!("'{name}' is listed twice")));
            }
            if self.group_of(id).is_some() {
                return Err(SimError::CannotFuse(format!(
                    "'{name}' is already in a group"
                )));
            }
            if !self.is_fusable(id) {
                return Err(SimError::CannotFuse(format!(
                    "'{name}' is not a continuous or stateless equation block"
                )));
            }
        }
        if members.iter().all(|&id| self.is_algebraic(id)) {
            return Err(SimError::CannotFuse(
                "a group needs a block with states to solve".into(),
            ));
        }
        self.groups.push(Group {
            members: members.to_vec(),
            solver,
        });
        Ok(())
    }

    /// Whether a Block can join a fused group: an equation Block,
    /// continuous or stateless (ADR 0051).
    pub fn is_fusable(&self, id: BlockId) -> bool {
        self.block(id).is_ok_and(|b| {
            b.as_equations().is_some()
                && matches!(
                    b.spec().time,
                    TimeSemantics::Continuous | TimeSemantics::Stateless
                )
        })
    }

    /// Whether a Block is a stateless equation Block (a formula, a unit
    /// conversion): in a fused group it is evaluated inside every
    /// right-hand-side evaluation, and it does not decide the group's
    /// method (ADR 0051).
    pub fn is_algebraic(&self, id: BlockId) -> bool {
        self.block(id)
            .is_ok_and(|b| b.as_equations().is_some() && b.spec().time == TimeSemantics::Stateless)
    }

    /// The fused groups, in the order they were made.
    pub fn groups(&self) -> &[Group] {
        &self.groups
    }

    /// The index of the group a Block belongs to, if any.
    pub fn group_of(&self, id: BlockId) -> Option<usize> {
        self.groups.iter().position(|g| g.members.contains(&id))
    }

    pub fn block(&self, id: BlockId) -> Result<&dyn Block> {
        self.blocks
            .get(id.0)
            .map(|b| b.as_ref())
            .ok_or_else(|| SimError::BadConnection(format!("no block #{}", id.0)))
    }

    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn connections(&self) -> &[Connection] {
        &self.connections
    }

    fn port(&self, id: BlockId, name: &str) -> Result<&PortSpec> {
        let spec = self.block(id)?.spec();
        spec.ports
            .iter()
            .find(|p| p.name == name)
            .ok_or_else(|| SimError::BadConnection(format!("'{}' has no port '{name}'", spec.name)))
    }
}

fn param_index(spec: &BlockSpec, name: &str) -> Result<usize> {
    spec.params
        .iter()
        .position(|p| p.name == name)
        .ok_or_else(|| SimError::UnknownParam {
            block: spec.name.clone(),
            name: name.into(),
        })
}

/// Position of a named variable-length port (`Shape::Population`) among a
/// Block's variable-length inputs or outputs.
pub(crate) fn population_index(
    ports: &[PortSpec],
    causality: Causality,
    name: &str,
) -> Option<usize> {
    ports
        .iter()
        .filter(|p| p.causality == causality && p.shape.is_variable())
        .position(|p| p.name == name)
}

/// Offset of a named port within a Block's flattened inputs or outputs.
pub(crate) fn port_offset(
    ports: &[PortSpec],
    causality: Causality,
    name: &str,
) -> Option<(usize, usize)> {
    let mut offset = 0;
    for p in ports.iter().filter(|p| p.causality == causality) {
        if p.name == name {
            return Some((offset, p.shape.len()));
        }
        offset += p.shape.len();
    }
    None
}
