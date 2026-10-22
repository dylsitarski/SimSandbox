//! # simsandbox-core
//!
//! The SimSandbox kernel. It knows how to connect, schedule and observe
//! models; it contains no physics. Every simulation is wrapped as a
//! [`Block`](block::Block) and connected through unit-checked ports.
//!
//! Layout:
//! - [`units`]: SI dimensions and unit conversion, checked at connect time.
//! - [`block`]: the Block contract (required trait + optional capabilities)
//!   and the speed/accuracy labels every method carries.
//! - [`graph`]: the model graph of Blocks and connections.
//! - [`fusion`]: groups of equation Blocks solved together as one system.
//! - [`sim`]: the co-simulation master and the kernel-owned state store.
//! - [`registry`]: block types by name, for loading models from files.
//! - [`sampling`]: sampling designs for sweeps (grid, random, Latin
//!   hypercube, Sobol).
//! - [`table`]: populations of rows owned by the kernel (ADR 0034).
//! - [`random`]: counter-based random numbers (ADR 0034).

pub mod block;
pub mod error;
pub mod fusion;
pub mod graph;
pub mod random;
pub mod registry;
pub mod sampling;
pub mod sim;
pub mod table;
pub mod units;

pub use error::{Result, SimError};
