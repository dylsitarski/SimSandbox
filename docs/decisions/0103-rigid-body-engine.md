# 0103 — The first rigid-body engine: Rapier, its state held as an opaque snapshot

- **Status:** Proposed (draft for the owner, 2026-11-19)
- **Date:** 2026-11-19
- **Builds on:** [ADR 0057](0057-phase-2-plan.md) item 3 (the spike and the gate), [ADR 0093](0093-scenes-from-primitive-layers.md) (bodies as rows drawn by a frame over rows; grabbing), [ADR 0062](0062-probes-recording-and-replay.md) (exact runs on any computer, snapshots, memory budget), [ADR 0058](0058-inputs-live-scheduled-replayed.md) (recorded input), [ADR 0091](0091-equations-first-hosting-for-existing-codes.md) (mature engines are wrapped)
- **Evidence:** [the rigid-body spike](../research/2026-11-19-rigid-body-spike.md) ([code](../../spikes/rigid-body/README.md))

## Context

The Phase 2 gate is an interactive cart-pole: a typed-equation controller balances a rigid-body pole live, the user drags the bodies in the scene and pushes with keys, and a rewound run replays bit for bit, also headless. ADR 0057 chose to pick the engine by a spike between MuJoCo and Rapier. The spike ran the same cases on Rapier 0.36.1 (2D and 3D, f64, impulse and multibody joints) and MuJoCo 4.0.1 (C library, Euler, RK4, implicitfast) on Linux:

- Both balance the cart-pole through a recorded impulse and a grab-like pull at 1 kHz, within 1e−3 rad of the exact equations, at 150 to 1300 times real time.
- Both restore a snapshot bit for bit and repeat runs bit for bit in one process and across processes.
- Accuracy:
  - MuJoCo's RK4 is exact to 1e−12.
  - Rapier's multibody joints and MuJoCo's Euler are second order with bounded energy.
  - Rapier's impulse joints are first order and lose 37 % of a 2 rad swing's energy in 100 periods.
- State:
  - MuJoCo's full state is one flat f64 vector (224 bytes for the cart-pole).
  - Rapier's is a graph of objects, 4–9 KB serialized for the cart-pole and 250–650 KB for 100 boxes.
- Determinism across machines:
  - Rapier claims it with `enhanced-determinism`, at no measured cost.
  - MuJoCo disclaims it across operating systems and versions.
- Build:
  - Rapier is pure Rust: about 45 s of clean build, licences all allowed.
  - MuJoCo needs a shared library on each OS and `unsafe` bindings SimSandbox writes itself. The maintained crate, `mujoco-rs`, adds a licence clause forbidding AI use.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| **A. Rapier (`rapier2d-f64` first), `enhanced-determinism`, multibody joints for mechanisms; state as an opaque snapshot** | Same bits on every computer, as ADR 0062 wants (claimed; to be pinned in CI); pure Rust, no `unsafe` in SimSandbox, nothing to ship; native 2D for the gate and ADR 0093; fast enough (cart-pole 170–410× real time) and bit-exact restore | Impulse joints dissipative, no high-order integrator; multibody joints slow on long chains (1 ms a step for 50 links); state not a flat vector: a new capability for opaque block state, and large snapshots with many contacts; pre-1.0 API |
| B. MuJoCo through its C API | Generalized coordinates, RK4, implicit damping: the accurate choice; flat state vector fits the kernel's states as they are, snapshots of bytes in a microsecond; MJCF and the robotics ecosystem; fastest on small mechanisms | No bit-identity across OSes or versions (against ADR 0062's default); a shared library to ship and pin per OS (no Intel-Mac wheel, no import library in the Windows wheel); `unsafe` FFI in a second crate, bindings of our own; its error handler ends the process; 3D only; box piles 2.5× slower than Rapier 3D at defaults |
| C. Both now, behind one block type | Each where it is best | Twice the verification, two state models and two file mappings before the gate needs one |
| D. Write a 2D engine in typed equations | Everything in one language (ADR 0091) | Contacts and joints are what mature engines do well; against "wrap, don't build" |

## Decision (proposed)

A, for the gate. MuJoCo stays the candidate for a later optional backend when a model needs 3D robotics accuracy or MJCF.

1. **Crate.** A domain crate `simsandbox-rigid` (domain `RigidBody`) depending on `rapier2d-f64` (later `rapier3d-f64`) with `enhanced-determinism` and without `parallel`, exact versions pinned. It needs no `unsafe`: the workspace's `forbid` holds. Rapier's types stay inside the crate.
2. **State for rewind.**
   - The block keeps its Rapier world live between steps. It gives the kernel an opaque snapshot (the serialized sets: bodies, colliders, both joint sets, island manager, broad and narrow phase, CCD) and restores from one.
   - A new optional capability trait carries this (`ExternalState`: save to bytes, restore from bytes). The kernel stores the bytes in its `Snapshot` beside the states vector, and counts them against the memory budget.
   - The snapshot's serialization is a maintained serde format checked by `cargo deny` (not bincode 1, which is unmaintained).
   - When the snapshots would exceed the budget, they are taken further apart and the steps between are replayed.
3. **File form.** The block's parameters describe the world as tables, 2D, in SI units, checked by `check` and listed by `describe`:
   - `bodies` (id, kind: dynamic, fixed or kinematic, pose, velocity, mass and inertia or density);
   - `shapes` (body, box, disc or capsule, size, friction, restitution), one to one with ADR 0101's primitives;
   - `joints` (kind: revolute, prismatic or fixed; the two bodies, anchors, axis, limits; `articulated: true` for multibody joints).
   - MJCF import for 3D is left for later; `mjcf-rs` is a candidate.
4. **Outputs as rows.** A table `bodies` over the dynamic bodies' stable ids, with `x`, `y`, `angle`, `vx`, `vy`, `omega` (and the fixed sizes as columns), so one frame over rows and a layer per kind of shape draw any number of bodies (ADR 0093 point 8). Each joint's coordinate and rate are also scalar outputs (`slider.x`, `hinge.theta`), which the controller reads.
5. **Inputs.** Joint forces or torques as scalar inputs (the cart's force), and external forces as rows: body id, force, point of application, held over the step. User forces are cleared every step (forces and torques both: Rapier keeps them otherwise).
6. **Grabbing.** The pull toward the pointer is computed in the model, not in the engine: a spring and damper from the grabbed body's point to `input.pointer`'s position while `picked` names it, written in typed equations from the body rows and fed back as a force row. Replay holds because the pointer is recorded input and the force is a function of it and the state. An engine-side mouse joint (a spring joint to a kinematic body) was rejected: it would be engine state outside the model, invisible to `check`, units and probes.
7. **Labels.** Impulse joints: first order, dissipative at large motion. Multibody joints: second order, energy bounded at 1 kHz. Contacts: unverified until a reference case exists. None `Verified` (ADR 0061).
8. **Verification tests to keep** (the spike's cases as tests, in `simsandbox-rigid/tests/verification.rs`):
   - The pendulum period against 4 K(sin(a/2)) / ω0 at 0.1 and 2 rad: multibody within 1e−7 at 1 ms, error ratio about 4 when the step halves; impulse joints bounded by their measured error.
   - Energy over 100 periods: multibody within 2e−3.
   - The cart-pole falling open loop against the exact equations by RK4: within 1e−3 rad at 1 s.
   - The cart-pole closed loop with a recorded impulse and grab.
   - Snapshot, restore and replay bit for bit, for the cart-pole, a box stack and a 100-box pile.
   - The same run twice gives the same hash, and a pinned hash per platform in CI on Linux, Windows and macOS.
   - A resting box and a resting stack staying within 1e−4 m for 10 s.
   - The verification policy's reference table gains the pendulum period.

## Consequences

- The block contract gains the `ExternalState` capability, and snapshots gain opaque bytes per block. [Block contract](../architecture/block-contract.md) and [solver backends](../architecture/solver-backends.md) are updated when this is accepted, and the kernel still contains no physics.
- A rigid-body run is exact on one machine (measured) and, by Rapier's claim, across machines; the CI hash job turns the claim into a test.
- `deny.toml` needs no new licence. Build time grows by about 45 s clean for 2D (50 s more for 3D).
- Models needing long articulated chains in real time, or reference-grade accuracy, wait for MuJoCo as a second backend. That would be a crate with `unsafe` FFI (as `simsandbox-compiled` is), bindings of SimSandbox's own, the shared library shipped per OS, and its state as a flat f64 vector in the kernel.
- Rejected here: an engine-side mouse joint for grabbing; `mujoco-rs` (licence).
- Follow-up: the ADR for the file form in detail ([changing the file format](../guides/changing-the-file-format.md)); the snapshot spacing rule under the memory budget.

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): a rigid-body world block (a block of a new domain, its bodies as rows); opaque block state in snapshots, which the kernel's flat states vector cannot express for an engine whose state is a graph of objects.
