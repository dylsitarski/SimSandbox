//! # simsandbox-session
//!
//! An open model as every interface uses it (the desktop app, the CLI and
//! the Python API, ADR 0039): the simulator, rewind snapshots, the recorded
//! history the views draw from, filters (ADR 0030–0032) and sweeps
//! (ADR 0027–0028).
//!
//! - [`session`]: [`Session`], loading a model file and running it.
//! - [`filters`]: expression and cut filters, compiled.
//! - [`views`]: the model's views, opened from the file, as tiles.
//! - [`render`]: the views drawn to an image without a window.
//! - [`describe`]: the catalogue of types and model checks, as JSON
//!   (ADR 0039).
//! - [`examples`]: the built-in example models and the registries of every
//!   block type and view this build knows.

pub mod describe;
pub mod examples;
pub mod filters;
pub mod raster;
pub mod render;
pub mod session;
pub mod views;

pub use session::Session;
