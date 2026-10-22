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
    pub(crate) groups: Vec<Group>,
    pub(crate) seed: u64,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            blocks: Vec::new(),
            params: Vec::new(),
            connections: Vec::new(),
            groups: Vec::new(),
            seed: DEFAULT_SEED,
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

    /// Add a Block with its default parameter values.
    pub fn add(&mut self, block: Box<dyn Block>) -> BlockId {
        self.params.push(block.spec().default_params());
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
    /// unit dimension. Compatible units with different scales are converted.
    pub fn connect(&mut self, from: (BlockId, &str), to: (BlockId, &str)) -> Result<()> {
        let out = self.port(from.0, from.1)?.clone();
        let inp = self.port(to.0, to.1)?.clone();
        if out.causality != Causality::Output || inp.causality != Causality::Input {
            return Err(SimError::BadConnection(format!(
                "'{}' -> '{}' must go from an output to an input",
                from.1, to.1
            )));
        }
        if out.shape != inp.shape {
            return Err(SimError::ShapeMismatch {
                from: format!("{:?}", out.shape),
                to: format!("{:?}", inp.shape),
            });
        }
        let factor = out
            .unit
            .conversion_to(&inp.unit)
            .ok_or(SimError::UnitMismatch {
                from: out.unit.to_string(),
                to: inp.unit.to_string(),
            })?;
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

    /// Solve these Blocks together as one system with `solver` (ADR 0023).
    /// Each must be a continuous equation Block (it implements `Equations`)
    /// in no other group; connections between members are then resolved
    /// inside every right-hand-side evaluation instead of once per step.
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
                    "'{name}' is not a continuous equation block"
                )));
            }
        }
        self.groups.push(Group {
            members: members.to_vec(),
            solver,
        });
        Ok(())
    }

    /// Whether a Block can join a fused group: a continuous equation Block.
    pub fn is_fusable(&self, id: BlockId) -> bool {
        self.block(id)
            .is_ok_and(|b| b.as_equations().is_some() && b.spec().time == TimeSemantics::Continuous)
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
