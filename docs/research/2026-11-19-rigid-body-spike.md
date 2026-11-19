# Rigid-body spike: Rapier against MuJoCo (2026-11-19)

The spike [ADR 0057](../decisions/0057-phase-2-plan.md) item 3 asks for before the first rigid-body backend: the same cases on both engines, measured, for the Phase 2 gate's interactive cart-pole ([roadmap](../roadmap.md#phase-2--breadth)). The decision it proposes is [ADR 0103](../decisions/0103-rigid-body-engine.md) (Proposed). Code and how to run it: [spikes/rigid-body](../../spikes/rigid-body/README.md).

## Method

- **Engines.**
  - Rapier 0.36.1: `rapier2d-f64` and `rapier3d-f64` from crates.io, feature `enhanced-determinism` on, f64 throughout (SimSandbox's states are f64). Each jointed case runs with impulse joints (maximal coordinates, joints solved as constraints) and with multibody joints (reduced coordinates).
  - MuJoCo 4.0.1 (released 2026-11-11): the C library and headers from the `mujoco-core` wheel on PyPI, bindings written by hand. Integrators Euler (MuJoCo's default, semi-implicit), RK4 and implicitfast. MuJoCo is 3D; the planar models use a slide joint along x and hinges about −y.
- **Same models on all.** Masses and inertias are given explicitly, not derived from shapes. Contact cases use 0.2 m boxes of 1 kg, friction 1, no restitution, each engine's default contact settings. Everything at 1 kHz unless a table says otherwise.
- **References** (verification policy): the pendulum's exact period, 4 K(sin(a/2)) / ω0, by the arithmetic–geometric mean; the cart-pole's exact Lagrange equations integrated by RK4 with 20 substeps, with the same controller and the same disturbance rule.
- **Machine:** Linux 6.18, Intel Xeon at 2.30 GHz (4 cores, one used), 15 GB; Rust 1.97.0, release build, `cargo -j 2`. Measured 2026-11-19. Times vary by a few per cent between runs.
- **Not tested here:** Windows and macOS (no machine); cross-machine determinism (one machine); MuJoCo built from source by CMake. What is said about them below comes from the engines' documentation and packages, and is marked so.

## Results

### 1. Pendulum period

A compact bob, 1 kg with 0.01 kg m² about its centre, 1 m from the pivot (a physical pendulum, ω0² = m g L / (I + m L²)). Period error = (measured − exact) / exact, the period being the mean over ten periods between upward zero crossings. Rapier 2D and 3D gave the same numbers to the digits shown, so they share a row.

| Engine | a = 0.1 rad, dt 10 ms | dt 1 ms | dt 0.5 ms | a = 2 rad, dt 10 ms | dt 1 ms | dt 0.5 ms | Observed order |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Rapier, impulse joints | −3.2e−6 | −9.0e−8 | −3.9e−8 | −8.9e−2 | **−1.5e−2** | −7.8e−3 | 1 |
| Rapier, multibody joints | −2.5e−6 | −2.5e−8 | −6.3e−9 | +1.1e−6 | +1.1e−8 | +2.6e−9 | 2 |
| MuJoCo, Euler (= implicitfast here) | −4.0e−5 | −4.0e−7 | −1.0e−7 | +1.7e−5 | +1.7e−7 | +4.2e−8 | 2 |
| MuJoCo, RK4 | −3.9e−9 | +6.9e−12 | −6.5e−13 | +5.8e−9 | −7.9e−13 | +1.3e−12 | 4 until the period measurement's own error (≈1e−12) |

At 0.1 rad the small-angle formula is itself 6.3e−4 short of the exact period, so the elliptic integral is the reference throughout.

Energy over 100 periods at dt = 1 ms, from the bodies' velocities and heights (so a drifting joint would show):

| Engine | a = 0.1 rad: largest \|ΔE\|/E0 | at the end | a = 2 rad: largest \|ΔE\|/E0 | at the end | Largest joint drift |
| --- | --- | --- | --- | --- | --- |
| Rapier, impulse joints | 2.2e−3 | −1.8e−3 | **3.7e−1** | **−3.7e−1** | 8.6e−7 m |
| Rapier, multibody joints | 1.2e−3 | +2.6e−6 | 9.7e−4 | −4.3e−7 | 0 |
| MuJoCo, Euler | 1.6e−3 | +2.0e−6 | 1.3e−3 | −6.8e−7 | 0 |
| MuJoCo, RK4 | 2.5e−12 | −2.5e−12 | 2.2e−12 | −1.8e−12 | 0 |

- Rapier's impulse joints lose energy steadily at large swings: 37 % in 100 periods at 2 rad, the period 1.5 % short at 1 ms, first order in the step. More solver iterations (which are substeps in Rapier 0.36) shrink it in proportion: 1 iteration −72 %, 4 (default) −37 %, 16 −12 % over 100 periods.
- Rapier's multibody joints and MuJoCo's Euler are symplectic-like: second order, energy bounded, no drift.
- Rapier's multibody joints come with an angular damping of 0.1 N m s/rad per degree of freedom by default, and start from their joint coordinates, not from where the bodies were placed; the spike zeroes the damping and sets the coordinates (`apply_displacements` after one `forward_kinematics`, since until the first step a fixed root counts as free).

### 2. Cart-pole

Cart 1 kg on a slider; uniform pole 0.1 kg, 1 m, hinged at the cart's centre; starting 0.1 rad from upright; 1 kHz. Controller: continuous-time LQR on the linearized model (Q = diag(10, 1, 100, 1), R = 0.1, K = [−10.0, −12.1, 78.1, 18.7]), ±30 N, the stand-in for a typed-equation controller. Disturbances as the scene would send them: an impulse of 0.05 N s on the pole's tip at 1 s; from 3 s to 4 s a pull on the tip toward a held pointer at (0.4, 0.9) m by a spring of 5 N/m and damper of 1 N s/m computed from the state each step (the grab, done in the model). Every engine balanced it through both.

| Engine | Closed loop, 6 s: max \|θ − θ_ref\| | max \|x − x_ref\| | Open loop (falling): θ − θ_ref at 1 s | Joint drift |
| --- | --- | --- | --- | --- |
| Rapier, impulse joints | 2.2e−4 rad | 1.7e−4 m | +4.4e−4 rad | 7.1e−7 m |
| Rapier, multibody joints | 3.3e−4 rad | 3.7e−4 m | +8.5e−4 rad | 0 |
| MuJoCo, Euler | 8.8e−4 rad | 7.6e−4 m | +3.6e−3 rad | 0 |
| MuJoCo, RK4 | 2.2e−5 rad | 4.6e−5 m | −9.5e−12 rad | 0 |

Rapier 2D and 3D agree to the digits shown. The reference's largest tilt while pulled is 0.2126 rad; all engines are within 0.0005 rad of it.

### 3. Snapshot and restore for rewind

Run to step k with the recorded input, snapshot, run to n, restore, run k..n again with the same recorded input, compare every body's pose and velocity bit for bit (cart-pole k = 2000, n = 5000; box on box 300, 1500; 100 boxes 500, 1500).

| Engine | What is the full state | Bit-identical after restore | Size: cart-pole / box on box / 100 boxes | Take, restore: cart-pole | 100 boxes |
| --- | --- | --- | --- | --- | --- |
| Rapier 2D | Every set: bodies, colliders, impulse and multibody joints, island manager, broad and narrow phase (contact manifolds with their warm-start impulses), CCD solver. The `PhysicsPipeline` holds only workspace | yes: as clones, as bytes into the same objects, and into a new pipeline | 4.2 KB (multibody 6.3 KB) / 6.1 KB / 247 KB (bincode) | 1.5 µs, 3.0 µs (bytes); 3.4, 1.3 (clones) | 65 µs, 146 µs (bytes) |
| Rapier 3D | as 2D | yes, all three ways | 5.7 KB (multibody 9.1 KB) / 9.8 KB / 645 KB | 2.0 µs, 3.5 µs | 192 µs, 375 µs |
| MuJoCo | `mj_getState(mjSTATE_INTEGRATION)`: time, qpos, qvel, act, history, plastic, plugin, the user inputs (ctrl, applied forces, mocap, …) and the warm-start accelerations: one flat f64 vector | yes: into the same `mjData` and into a new one; also by `mj_copyData` | 224 B / 552 B / 24.9 KB | 0.2 µs, 0.1 µs | 1.3 µs, 0.4 µs |

Poses and velocities alone (a flat vector of body states):

- Rapier with impulse joints and no contacts: enough, bit for bit (joint warm starting is off by default).
- Rapier with multibody joints: not enough (the joint coordinates live in the multibody).
- Rapier with contacts: not enough (the contact warm start lives in the narrow phase); the runs diverge by centimetres.
- MuJoCo without the warm start (`mjSTATE_FULLPHYSICS`): enough without contacts; with contacts the bits differ, by at most 6e−12, as MuJoCo's documentation says ("warmstart accelerations have only a very small effect … but should be saved if bit-wise equality is required").

### 4. Determinism

Every body's pose and velocity after every step, hashed bit for bit: cart-pole 6 s with the disturbances (every engine and joint kind), 100 boxes 2 s. Same hash twice in one process and in two more processes, for every engine: **yes**, all eleven runs. Rapier built without `enhanced-determinism` gives different bits (4 of 6 hashes change) and the same speed (within 4 %).

What the engines claim (documentation, not tested here):

- Rapier: "a perfectly deterministic simulation on different machines, as long as they are compliant with IEEE 754-2008", with `enhanced-determinism` (software `libm` instead of the platform's maths, no 8-lane SIMD) and without the `parallel` feature.
- MuJoCo: the pipeline is deterministic, but "exact reproducibility is only guaranteed within a single version, on the same architecture … will likely be different between MuJoCo versions or different operating systems". Its engine thread pool is off unless a host creates one.

### 5. Speed (one thread, 1 kHz physics)

| Model | Rapier 2D impulse | Rapier 2D multibody | Rapier 3D impulse | Rapier 3D multibody | MuJoCo Euler | MuJoCo implicitfast | MuJoCo RK4 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Cart-pole (200 000 steps) | 2.4 µs (412× real time) | 3.9 µs (257×) | 6.6 µs (151×) | 5.9 µs (168×) | **0.75 µs (1341×)** | 0.81 µs | 2.5 µs |
| Chain of 50 links from horizontal (10 000 steps) | 21 µs (48×) | **1142 µs (0.9×)** | 85 µs (12×) | **2230 µs (0.4×)** | 14 µs (73×) | 21 µs | 54 µs |
| 100 boxes falling into a pile (3000 steps) | 58 µs (17×) | – | 253 µs (4×) | – | 624 µs (1.6×) | 684 µs | – |

- MuJoCo's 50-link chain becomes unstable at 1 kHz (Euler and implicitfast at 3.1 s, RK4 at 7.0 s; the whip at the free end). MuJoCo then prints a warning and resets the model to its start; the spike counts the warning. Rapier's impulse joints stay bounded on the same chain, by losing energy.
- Rapier's multibody solve grows steeply with the number of links (dense; 50 links cost 300 times the cart-pole), so long articulated chains are not real-time with it.
- MuJoCo's box pile at its defaults (Newton solver, pyramidal cones) is 2.5 times slower than Rapier 3D and still moving at 0.06 m/s after 3 s; Rapier's pile was at rest.

### 6. Contacts

| Case | Rapier 2D | Rapier 3D | MuJoCo (Euler and implicitfast alike) |
| --- | --- | --- | --- |
| Box resting on the ground, 10 s | 0.09991 m, still | 0.09993 m, still | 0.09989 m, still (≤ 4e−12 m/s) |
| Box on a box, 0.05 m off centre, 10 s | 0.29921 m, still | 0.29932 m, still | 0.29955 m, creeping at ≤ 1.3e−5 m/s |
| Box dropped 0.3 m onto a box, off centre and tilted 0.1 rad, 3 s | tumbled off (ends at x = 0.21 m on the ground) | tumbled off (x = 0.21 m) | stayed on top (0.29955 m) |

All runs were deterministic (same hash twice). Both engines' stacks are stable. The dropped box differs between the engines: on Rapier the corner impact gives it a spin (−10 rad/s) that the flat landing does not stop, at any solver iteration count tried (1 to 16); MuJoCo's soft contacts absorb it. Neither is checked against a reference: a corner impact has no exact answer to compare with.

## Build, platforms, licences

- **Rapier: pure Rust** (319 uses of `unsafe` inside it, none in a host's code). Builds wherever Rust does (documentation; tried on Linux only). Clean release build of a program using it, `-j 2`: `rapier2d-f64` 44 s, `rapier3d-f64` 50 s (debug 24 s and 26 s); 67 crates in its tree, all under licences `deny.toml` allows; `cargo deny` on the spike passes licences, bans and sources (two duplicate-version warnings, `hashbrown` and `syn`). Its maths moved from nalgebra to glam in recent releases; it is pre-1.0 and its API changes between minor versions.
- **MuJoCo: a C/C++ library shipped as a shared library** (`libmujoco.so` 7.5 MB here; `mujoco.dll` 7.0 MB, `libmujoco.dylib` 12 MB in the wheels), or built from source by CMake (not tried). Compiling the hand-written bindings adds nothing measurable (no crates). Getting it:
  - The PyPI wheels of 4.0.1 (checked 2026-11-19) cover Linux x86-64 and arm64, Windows x86-64 and macOS **arm64 only** (no Intel Mac wheel). The Windows wheel has the DLL without an import library (`mujoco.lib`); Rust can link it with `raw-dylib`, or the GitHub release archive provides one (documentation).
  - The app would ship the library beside its executable on every platform and keep its version pinned (the bits change between versions).
  - Its default error handler prints and calls `exit`; a host replaces it (`mju_user_error`), but the handler must not return, so a model error raised inside a step can only end the process or be avoided by checking first.
- **Bindings for MuJoCo.**
  - `mujoco-rs` 7.1.0 targets MuJoCo 3.17, not 4.0. Its licence is MIT or Apache-2.0 **with an added reservation of rights** forbidding text and data mining and use in training, evaluating or running AI models ("non-standard" on crates.io). `cargo deny` would refuse it, and the reservation conflicts with a project developed by AI agents. The spike read only its licence file and neither built nor read its code.
  - `mujoco-sys`, `mujoco-rust` and `mujoco-rs-sys` are from 2020–2023 (MuJoCo 2.x).
  - So the binding would be SimSandbox's own: the spike's dozen functions by hand, or bindgen's output from the headers checked in, so that fields such as `xpos` and `xquat` can be read.
  - Either way it is `unsafe` FFI, which the workspace forbids (`unsafe_code = "forbid"`) everywhere but `simsandbox-compiled` (`deny`, allowed where marked). A MuJoCo crate would be a second such exception.
- **Licences:** Rapier Apache-2.0; MuJoCo Apache-2.0 (its wheel's `LICENSE`); both in `deny.toml`'s list. The spike's own `bincode` 1.3 fails `cargo deny`'s advisories (RUSTSEC-2025-0141, unmaintained): a real crate needs another serde format.
- **2D:** Rapier has a native 2D engine (`rapier2d`), which matches ADR 0093's 2D-first scenes. MuJoCo is 3D only; a planar model is built from slide and hinge joints, which worked exactly here, but free planar bodies with contacts need three joints each (two slides and a hinge) in place of a free joint.

## Fit with SimSandbox

- **Block contract.** Both fit a discrete-time block stepping at a fixed `dt`: forces and grab forces in as inputs held over the step, bodies out as rows (`x`, `y`, `angle`, velocities, by stable id) and joint coordinates as scalars. Neither needs feedthrough.
- **State.**
  - MuJoCo's integration state is one flat f64 vector, so it can be the block's kernel state as every other block's is (fixed size per model). Snapshots, rewind and branching then work unchanged, at 0.2–1.3 µs a step to copy in and out.
  - Rapier's full state is a graph of objects. The kernel would hold it as an opaque blob per snapshot (4–9 KB for the cart-pole, 250–650 KB for a 100-box pile), which needs a capability the block contract does not have yet ("external Blocks must save and restore their own"). With snapshots every 10 steps (ADR 0062) at 1 kHz that is 0.6–0.9 MB/s for the cart-pole and 25–65 MB/s for the pile: the pile fills the default 1 GB budget in 15–40 s. Contact-rich scenes would need snapshots further apart, with replay between them (exact, since runs are deterministic).
- **Model files.** MuJoCo has MJCF; SimSandbox would still write its own form for 2D bodies and keep units and checks. Rapier has no format; SimSandbox defines one (bodies, shapes, joints as tables in the file), and MJCF for 3D could later be read by `mjcf-rs`/`rapier3d-mjcf` (Dimforge's pure-Rust MJCF parser and loader, 0.36.1; their README says they are largely written with AI assistance).
- **Accuracy.** MuJoCo offers generalized coordinates by default, RK4 (exact to 1e−12 here) and implicit damping. Rapier offers maximal coordinates with impulse joints, which are first order and dissipative at large swings, and reduced coordinates with multibody joints, which are second order and conserve energy like MuJoCo's Euler, but slow on long chains.
- **API pitfalls met:**
  - Rapier keeps user forces and torques until reset, and `add_force_at_point` adds a torque that `reset_forces` does not clear (`reset_torques` does). The first cart-pole runs fell over from this.
  - Rapier's multibody defaults: the damping and the starting coordinates above.
  - MuJoCo resets an unstable simulation with only a warning. By default it prints warnings and appends them to `MUJOCO_LOG.TXT` in the working folder; a host installs its own handler (`mju_user_warning`, as the spike does).

## Recommendation

**Rapier (`rapier2d-f64` first, `rapier3d-f64` when 3D scenes arrive), with `enhanced-determinism`, multibody joints for mechanisms like the cart-pole, and its state held as an opaque snapshot.** MuJoCo stays the candidate for a later, optional backend for robotics-grade 3D work (MJCF models, RK4, generalized coordinates), wrapped when a model needs it.

Reasons:

1. **Exact runs on every computer.** ADR 0062 makes "the same bits on any computer" the default. Rapier claims cross-platform determinism with `enhanced-determinism`, at no measured cost in speed. MuJoCo disclaims it across operating systems and versions. Both were deterministic on one machine.
2. **Builds like the rest.** Pure Rust, about 45 s added to a clean build, every licence already allowed, no `unsafe` in SimSandbox, nothing to ship or fetch per platform. MuJoCo needs a shared library on each OS (no Intel-Mac wheel, no Windows import library in the wheel), version pinning, a second `unsafe` crate, and bindings SimSandbox writes itself, since the maintained crate's licence is unusable.
3. **2D first.** The gate's scene and ADR 0093's first primitives are 2D; Rapier has a 2D engine, MuJoCo emulates the plane with joints.
4. **Good enough for the gate, measured.**
   - Multibody joints: the pendulum's period within 1e−8 at 1 ms (second order) and energy bounded over 100 periods.
   - Cart-pole: within 4e−4 rad of the exact closed-loop run, 170–410 times real time.
   - Bit-exact restore however the state was kept.
   - Stable resting contacts.

Risks:

- **Accuracy of impulse joints.** They lose 37 % of a large swing's energy in 100 periods. Labels must say so, and models must use multibody joints where energy matters. Rapier has no RK4 or higher-order option: accuracy comes only from smaller steps or more substeps.
- **Multibody speed.** About 1 ms a step for 50 links in 2D, dense in the number of degrees of freedom. Long chains and ragdolls are not real-time with them.
- **State size.** Contact-rich scenes make large snapshots (650 KB for 100 boxes in 3D). This needs keyframes further apart with replay between them, or delta encoding, and a new kernel capability for opaque block state.
- **Cross-platform determinism is a claim, not yet a measurement.** The first CI job should pin hashes of the spike's runs on Linux, Windows and macOS.
- **Pre-1.0.** Rapier's API moves between minor versions (the move to glam), and one organisation maintains it. Pin exact versions as with Cranelift.
- **Contact behaviour differs between engines** (the dropped box). Neither is "right" without a reference; contact models need their own verification cases before they are labelled.

What would change the recommendation: if 3D robotics models (MJCF from elsewhere) or reference-grade accuracy (RK4 in generalized coordinates) became the next need rather than playable 2D scenes, MuJoCo's strengths would weigh more than its build and determinism costs.
