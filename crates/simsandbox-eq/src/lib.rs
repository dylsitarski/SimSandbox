//! # simsandbox-eq
//!
//! The equation domain. An [`OdeSystem`] defines *what* is simulated
//! (equations, ports, parameters, states); an [`Integrator`] is the *method*
//! that computes it. [`OdeBlock`] pairs the two into a kernel [`Block`].
//!
//! [`EquationGroup`] fuses several systems into one larger system solved by a
//! single integrator, so tightly coupled equations have no coupling error.
//!
//! [`register`] adds the domain's block types to a model-file registry.
//!
//! [`Block`]: simsandbox_core::block::Block

pub mod group;
pub mod integrators;
pub mod ode;
pub mod registry;
pub mod systems;

pub use group::EquationGroup;
pub use integrators::Integrator;
pub use ode::{OdeBlock, OdeSystem};
pub use registry::register;
