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
//! - [`sim`]: the co-simulation master and the kernel-owned state store.

pub mod block;
pub mod error;
pub mod graph;
pub mod sim;
pub mod units;

pub use error::{Result, SimError};
