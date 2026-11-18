//! # simsandbox-eq
//!
//! The equation domain. An [`OdeSystem`] defines *what* is simulated
//! (equations, ports, parameters, states); an [`Integrator`] is the *method*
//! that computes it. [`OdeBlock`] pairs the two into a kernel [`Block`].
//!
//! The master fuses connected equation Blocks into one system solved by a
//! single [`integrators::GroupIntegrator`], so tightly coupled equations
//! have no coupling error (ADR 0023).
//!
//! [`register`] adds the domain's block types to a model-file registry;
//! with the `views` feature, `register_views` adds their views (ADR 0016).
//!
//! [`Block`]: simsandbox_core::block::Block

pub mod convert;
pub mod fields;
pub mod file_input;
pub mod implicit;
pub mod input;
pub mod integrators;
pub mod mesh;
pub mod ode;
pub mod refine;
pub mod registry;
pub mod sparse_lu;
pub mod symbolic;
pub mod systems;
#[cfg(feature = "views")]
pub mod views;

pub use integrators::Integrator;
pub use ode::{OdeBlock, OdeSystem};
pub use registry::register;
pub use symbolic::{EquationSet, ExplicitBlock, Kind};
#[cfg(feature = "views")]
pub use views::register_views;
