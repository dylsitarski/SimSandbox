//! SimSandbox's mechanism domain (ADR 0103, ADR 0105): a tree of rigid
//! bodies in the plane on revolute, prismatic and fixed joints, one block
//! per tree (`mech.planar`), its numbers parameters and its state the joint
//! coordinates, solved by Featherstone's articulated-body algorithm and the
//! equation domain's methods, with which it fuses.
//!
//! - [`tree`]: the dynamics (ABA, and CRBA with RNEA as its oracle).
//! - [`rows`]: the `bodies` rows as a model file writes them, checked.
//! - [`block`]: the ODE system, its ports and parameters, and the block.

pub mod block;
pub mod rows;
pub mod tree;

pub use block::{MechBlock, Mechanism, PLANAR};
use simsandbox_core::registry::{Registry, TypeInfo};
use simsandbox_core::Result;

/// The mechanism block's reference, given whole by `describe mech.planar`
/// (ADR 0039).
pub const REFERENCE: &str = include_str!("../../../docs/reference/mechanisms.md");

/// Register `mech.planar`.
pub fn register(registry: &mut Registry) -> Result<()> {
    registry.register(PLANAR, block::build)?;
    registry.describe(
        PLANAR,
        TypeInfo::new(
            "A tree of rigid bodies in the plane on revolute, prismatic and fixed joints, \
             one row of 'bodies' per body and its joint; every number a parameter; outputs \
             each joint's coordinate and rate, the energy, and each body's frame over rows \
             (x, y, angle, vx, vy, omega) for a scene.",
        )
        .method("rk4", &["substeps"])
        .method("dopri5", &["rtol", "atol"])
        .method("trbdf2", &["substeps"])
        .method("backward_euler", &["substeps"])
        .definition(&[
            r#"{"name": "cart", "parent": "ground", "joint": "slider", "kind": "prismatic", "mass": 1.0}"#,
            r#"{"name": "pole", "parent": "cart", "joint": "hinge", "kind": "revolute", "mass": 0.1, "centre": [0, 0.5], "inertia": 0.0083, "start": [0.1, 0]}"#,
        ])
        .reference(REFERENCE),
    )
}
