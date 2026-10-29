//! # simsandbox-views
//!
//! How a running model becomes pictures (ADR 0015,
//! `docs/architecture/visualization.md`):
//!
//! - [`history`]: signals recorded over time, rewound and thinned together.
//! - [`view`]: the view contract: a [`view::ViewType`] declares slots and
//!   style properties; a [`view::ViewState`] binds slots to signals and
//!   draws.
//! - [`builtin`]: `plot.xy` (any signals on either axis) and `plot.3d`.
//! - [`colour`]: colour encodings shared by views (trail fading, colour by
//!   value) and trail length.
//! - [`fields`]: views of grids and point sets: `plot.heatmap`,
//!   `plot.profile`, `plot.scatter`.
//! - [`surface`]: `plot.surface` and `plot.contour` (grids as shapes).
//! - [`view3d`]: the projection and drawing behind `plot.3d` and
//!   `plot.surface`.

pub mod builtin;
pub mod colour;
pub mod cut;
pub mod fields;
pub mod history;
pub mod surface;
pub mod view;
pub mod view3d;

pub use history::{Coordinate, Field, FieldShape, History, Recorded, SignalRef};
pub use view::{
    Available, Bindings, Context, Preset, SlotKind, Swap, ViewRegistry, ViewState, ViewType,
};
