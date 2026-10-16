//! The model graph: Blocks, their parameter values, and unit-checked connections.

use crate::block::{Block, BlockSpec, Causality, PortSpec};
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

/// A graph of Blocks and the connections between them.
#[derive(Default)]
pub struct Model {
    pub(crate) blocks: Vec<Box<dyn Block>>,
    pub(crate) params: Vec<Vec<f64>>,
    pub(crate) connections: Vec<Connection>,
}

impl Model {
    pub fn new() -> Self {
        Self::default()
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
