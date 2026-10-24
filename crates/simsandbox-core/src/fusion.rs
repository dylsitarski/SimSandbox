//! Fused groups (ADR 0023): equation Blocks advanced together as one system
//! by one method, so the connections between them carry no coupling error.
//!
//! The kernel wires the members' [`Equations`](crate::block::Equations)
//! into one right-hand side, resolving connections inside the group at every
//! evaluation; the domain supplies the method as a [`GroupSolver`]. Members
//! stay ordinary Blocks of the model: their ids, parameters, states and
//! outputs do not change, only how they are stepped.

use crate::block::MethodSpec;
use crate::graph::BlockId;
use crate::Result;

/// The combined right-hand side of a group: `dx/dt` for the members' states
/// concatenated in member order.
pub type GroupRhs<'a> = dyn Fn(f64, &[f64], &mut [f64]) + 'a;

/// A method that advances a fused group, supplied by its domain (e.g. an
/// integrator for equation Blocks).
pub trait GroupSolver: Send {
    fn method(&self) -> &MethodSpec;

    /// Advance the combined state `x` from `t` to `t + dt`.
    fn advance(&mut self, rhs: &GroupRhs<'_>, t: f64, dt: f64, x: &mut [f64]) -> Result<()>;
}

/// Blocks solved together, in the order they were given.
pub struct Group {
    pub(crate) members: Vec<BlockId>,
    pub(crate) solver: Box<dyn GroupSolver>,
}

impl Group {
    pub fn members(&self) -> &[BlockId] {
        &self.members
    }

    /// The method the whole group runs.
    pub fn method(&self) -> &MethodSpec {
        self.solver.method()
    }
}
