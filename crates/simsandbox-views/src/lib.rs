//! # simsandbox-views
//!
//! How a running model becomes pictures (ADR 0015,
//! `docs/architecture/visualization.md`):
//!
//! - [`history`]: signals recorded over time, rewound and thinned together.
//! - [`view`]: the view contract: a [`view::ViewType`] declares slots and
//!   style properties; a [`view::ViewState`] binds slots to signals and
//!   draws.
//! - [`builtin`]: `plot.xy` (any signals on either axis), `plot.3d`,
//!   `plot.heatmap` (grid fields) and `plot.scatter` (point sets).
//! - [`colour`]: colour encodings shared by views (trail fading, colour by
//!   value) and trail length.
//! - [`view3d`]: the projection and drawing behind `plot.3d`.

pub mod builtin;
pub mod colour;
pub mod history;
pub mod view;
pub mod view3d;

pub use history::{Field, FieldShape, History, SignalRef};
pub use view::{Available, Bindings, Preset, SlotKind, ViewRegistry, ViewState, ViewType};
