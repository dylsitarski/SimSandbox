//! # simsandbox-eq
//!
//! The equation domain. An [`OdeSystem`] defines *what* is simulated
//! (equations, ports, parameters, states); an [`Integrator`] is the *method*
//! that computes it. [`OdeBlock`] pairs the two into a kernel [`Block`].
//!
//! [`EquationGroup`] fuses several systems into one larger system solved by a
//! single integrator, so tightly coupled equations have no coupling error.
//!
//! [`register`] adds the domain's block types to a model-file registry;
//! with the `views` feature, `register_views` adds their views (ADR 0016).
//!
//! [`Block`]: simsandbox_core::block::Block

pub mod convert;
pub mod fields;
pub mod group;
pub mod integrators;
pub mod ode;
pub mod registry;
pub mod symbolic;
pub mod systems;
#[cfg(feature = "views")]
pub mod views;

pub use group::EquationGroup;
pub use integrators::Integrator;
pub use ode::{OdeBlock, OdeSystem};
pub use registry::register;
pub use symbolic::{EquationSet, ExplicitBlock, Kind};
#[cfg(feature = "views")]
pub use views::register_views;
