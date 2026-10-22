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
//! - [`run`]: a model file run headless, every output collected.
//! - [`render`]: the views drawn to an image without a window.
//! - [`canvas`]: the node canvas drawn and edited with egui.
//! - [`diagram`]: the model graph for the node canvas, its layout and edits.
//! - [`describe`]: the catalogue of types and model checks, as JSON
//!   (ADR 0039).
//! - [`examples`]: the built-in example models and the registries of every
//!   block type and view this build knows.

pub mod canvas;
pub mod describe;
pub mod diagram;
pub mod examples;
pub mod filters;
pub mod raster;
pub mod render;
pub mod run;
pub mod session;
pub mod views;

pub use session::Session;
/// A model file, as interfaces read and write it.
pub use simsandbox_file::ModelFile;
/// The model file's JSON Schema.
pub use simsandbox_file::SCHEMA;
