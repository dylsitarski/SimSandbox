# Rigid-body spike: Rapier against MuJoCo (2026-11-19)

The spike [ADR 0057](../decisions/0057-phase-2-plan.md) item 3 asks for before the first rigid-body backend: the same cases on both engines, measured, for the Phase 2 gate's interactive cart-pole ([roadmap](../roadmap.md#phase-2--breadth)). The decision it proposes is [ADR 0103](../decisions/0103-rigid-body-engine.md) (accepted 2026-11-20). A follow-up the same day added a native articulated-body solver, the same mechanisms as typed equations and an estimate for Featherstone as a typed-equations function: [below](#a-native-articulated-body-solver-2026-11-19). Code and how to run it: [spikes/rigid-body](../../spikes/rigid-body/README.md).

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

_This was the first recommendation (Rapier for everything). The follow-up, [a native articulated-body solver](#a-native-articulated-body-solver-2026-11-19), changes it for jointed mechanisms without contact; the current proposal is [ADR 0103](../decisions/0103-rigid-body-engine.md)._

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

## A native articulated-body solver (2026-11-19)

A follow-up asked for by the owner the same day. SimSandbox hosts any engine a user relies on; the engine it ships as the default must have SimSandbox's own properties (exact replay and rewind, units, verification, AI access). The hypothesis tested: **jointed mechanisms without contact** (chains and trees on revolute and prismatic joints: the cart-pole, pendulum chains, arms) are worth a solver of SimSandbox's own, whose dynamics in joint coordinates are an ODE and whose state is an ordinary kernel state; **contacts** stay with Rapier. Two more routes were added during the day at the owner's request: the same mechanisms **written as typed equations**, and **Featherstone as a built-in typed-equations function** (estimated, not built). The proposal that follows from all of it is in [ADR 0103](../decisions/0103-rigid-body-engine.md).

### Method

- **The native solver** ([`spikes/rigid-body/src/native_2d.rs`](../../spikes/rigid-body/src/native_2d.rs), pure Rust, no new crates): a tree of rigid bodies in the plane (mass, centre of mass, inertia about it), each on one revolute or prismatic joint to its parent or the ground; gravity; joint damping (default 0); joint forces and point forces in world axes at a point fixed on a body (how a grab's spring is applied). Dynamics by Featherstone's articulated-body algorithm (ABA) in 2D spatial algebra (3-vectors ω, vx, vy), O(n); and, as an independent check, the composite rigid-body algorithm (CRBA) with recursive Newton–Euler (RNEA) and a Cholesky solve. Outputs: each body's `x`, `y`, `angle` and velocities at its centre, by forward kinematics. **State: `[q, q̇]` and nothing else.**
- **Integrators.** A semi-implicit Euler written in the spike (q̇ first, then q, as MuJoCo's Euler and Rapier do). RK4, Dormand–Prince 5(4) (rtol 1e−8, atol 1e−10) and TR-BDF2 are **SimSandbox's own**: the spike depends on `simsandbox-eq` by path and calls `Integrator::advance_in`, as the equation domain does. The workspace is unchanged.
- **Typed equations** ([`spikes/rigid-body/typed/`](../../spikes/rigid-body/typed/)): model files loaded by `simsandbox-file` and stepped by the kernel's `Simulator` (the spike depends on `simsandbox-core` and `simsandbox-file` by path), as `simsandbox run` does:
  - `pendulum`, `cartpole` (inputs held over the step: the cart force and the tip force are parameters the host sets before each step, as for the other engines), `cartpole-lqr` (plant, LQR and grab in one block: one ODE), `double-pendulum` (Lagrange equations, 2 × 2 inverse by hand);
  - `chain-N`: an n-link chain in absolute angles, M(φ) and the right-hand side over axes `i`, `j`, accelerations by `solve(M, b)`;
  - `aba-chain-N`: the equation-generator route, the native ABA unrolled into scalar formulas per body by [`generate_aba.py`](../../spikes/rigid-body/typed/generate_aba.py) (the 50- and 200-link files are generated, not committed);
  - `overhead-base`, `overhead-solve`: a built-in call's cost.
- **Same cases and parameters** as above (the same `Sim` trait, models and references), rerun in one process on the same machine (Linux 6.18, Xeon 2.30 GHz, one thread, Rust 1.97.0, release, `-j 2`), measured 2026-11-19. Rapier's and MuJoCo's numbers in that rerun agree with the tables above within a few per cent; the tables below quote them for comparison.
- **New references:** the double pendulum's Lagrange equations written out by hand (absolute angles), RK4 at h = 1e−5 s; the damped pendulum's linear solution at 1e−5 rad; conservation of energy and of momentum.

### N1. Pendulum period and energy

| Engine | a = 0.1 rad, dt 10 ms | dt 1 ms | dt 0.5 ms | a = 2 rad, dt 10 ms | dt 1 ms | dt 0.5 ms | Order | Energy over 100 periods at 2 rad, dt 1 ms: max \|ΔE\|/E0 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Rapier, multibody joints | −2.5e−6 | −2.5e−8 | −6.3e−9 | +1.1e−6 | +1.1e−8 | +2.6e−9 | 2 | 9.7e−4 |
| MuJoCo, Euler | −4.0e−5 | −4.0e−7 | −1.0e−7 | +1.7e−5 | +1.7e−7 | +4.2e−8 | 2 | 1.3e−3 |
| MuJoCo, RK4 | −3.9e−9 | +6.9e−12 | −6.5e−13 | +5.8e−9 | −7.9e−13 | +1.3e−12 | 4 | 2.2e−12 |
| **Native, semi-implicit Euler** | −4.0e−5 | −4.0e−7 | −1.0e−7 | +1.7e−5 | +1.7e−7 | +4.2e−8 | 2 | 1.3e−3 |
| **Native, RK4** | −3.9e−9 | +6.9e−12 | −6.5e−13 | +5.8e−9 | −7.9e−13 | +1.3e−12 | 4 | 2.1e−12 |
| **Native, Dopri5** | −1.2e−8 | +6.1e−12 | −7.0e−13 | +7.5e−9 | −1.1e−12 | +1.2e−12 | (adaptive) | 5.1e−14 |
| **Native, TR-BDF2** | +3.9e−5 | +3.9e−7 | +9.8e−8 | +1.7e−5 | +1.9e−7 | +4.7e−8 | 2 | 6.4e−7 (loses energy) |
| **Typed equations, RK4** | as native RK4 to every digit shown | | | | | | 4 | 2.1e−12 |
| **Typed equations, Dopri5** | −1.2e−8 | +6.4e−12 | +7.2e−14 | +7.5e−9 | −9.2e−13 | +2.4e−12 | (adaptive) | 2.9e−14 |
| **Typed equations, TR-BDF2** | as native TR-BDF2 to every digit shown | | | | | | 2 | 6.4e−7 |

The native solver's semi-implicit Euler and RK4 give **MuJoCo's numbers to every digit printed** (same dynamics in generalized coordinates, same schemes); so do the typed equations with RK4.

### N2. Cart-pole

| Engine | Closed loop, 6 s: max \|θ − θ_ref\| | max \|x − x_ref\| | Open loop: θ − θ_ref at 1 s |
| --- | --- | --- | --- |
| Rapier, multibody joints | 3.3e−4 rad | 3.7e−4 m | +8.5e−4 rad |
| MuJoCo, Euler | 8.8e−4 rad | 7.6e−4 m | +3.6e−3 rad |
| MuJoCo, RK4 | 2.2e−5 rad | 4.6e−5 m | −9.5e−12 rad |
| **Native, semi-implicit Euler** | 8.8e−4 rad | 7.6e−4 m | +3.6e−3 rad |
| **Native, RK4** | **3.3e−9 rad** | 9.0e−9 m | −9.5e−12 rad |
| **Native, Dopri5** | 1.2e−13 rad | 1.1e−13 m | +9.3e−15 rad |
| **Native, TR-BDF2** | 2.7e−7 rad | 7.5e−7 m | +2.5e−6 rad |
| **Typed equations, RK4 / Dopri5 / TR-BDF2** | 3.3e−9 / 1.2e−13 / 2.7e−7 rad | 9.0e−9 / 1.9e−13 / 7.5e−7 m | −9.5e−12 / +8.0e−15 / +2.5e−6 rad |

Native RK4 is about 7000 times closer to the reference than MuJoCo's RK4 in closed loop, while open loop they agree to 1e−12. The likely reason (not isolated): the spike gives MuJoCo the tip force as a world force and torque about the pole's centre (`xfrc_applied`), fixed over the step, while the native solver, like the reference, applies it at the moving tip inside every RK4 stage. Every route balanced through the impulse and the grab; the largest tilt while pulled is 0.2126 rad for all but the Euler-type schemes (0.2121).

### N3. Snapshot and restore

| Engine | Full state | Bit-identical after restore (same objects, new objects) | Size, cart-pole | Take, restore |
| --- | --- | --- | --- | --- |
| Rapier 2D, multibody | the sets, serialized | yes | 6.3 KB | 2.0 µs, 4.3 µs |
| MuJoCo | `mjSTATE_INTEGRATION` | yes | 224 B | 0.1 µs, 0.1 µs |
| **Native (every integrator)** | `[q, q̇]` | **yes** | **32 B** | < 0.05 µs each |
| **Typed equations (every method)** | the kernel's own `Snapshot` (states, parameters, outputs it cannot recompute) | yes; into a newly loaded model too | 32 B of states | 0.3 µs, 0.3 µs (new model: 140 µs, most of it loading) |

**Found on the way:** with TR-BDF2 the native solver first restored into a *new* workspace with 8 of 12 values off by up to 5.9e−12. `simsandbox-eq`'s implicit methods, when a system gives no Jacobian structure, probe it once at the first step's state and keep it; the cart-pole starts at rest, where ∂q̈/∂q̇ (quadratic in q̇) is zero, so the probe misses it, and a workspace probed at another state gives other bits. Passing the structure (q̇'s rates read q̇, q̈'s read everything) fixed it. Typed equations give their structure exactly and are not affected; Rust `OdeSystem`s whose `sparsity()` is `None` are. Suggested as a separate task.

### N4. Determinism

Hashes of every body's pose and velocity at every step of the 6 s cart-pole with the disturbances: the same twice in one process and in two more processes, **for every native and typed variant** (seven more runs, all "yes"). Cross-machine: as for MuJoCo and the typed equations, the native solver calls the platform's `sin` and `cos` (Rust's `f64::sin_cos`), which may differ between operating systems; Rapier avoids this with the `libm` crate under `enhanced-determinism`. It is the same exposure SimSandbox's typed equations already have (`Func::Sin` calls `f64::sin`), so the CI hash job ADR 0103 asks for covers both.

### N5. Speed, and the chains' stability

µs per step at 1 kHz, one thread (cart-pole with the controller in the loop):

| Model | Rapier 2D multibody | MuJoCo Euler | MuJoCo RK4 | Native, semi-implicit Euler | **Native, RK4** | Native, Dopri5 | Native, TR-BDF2 | Typed (`solve` route), RK4 | Typed (generated ABA), RK4 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Cart-pole | 3.7 | 0.70 | 2.4 | 0.15 | **0.54** | 1.0 | 1.6 | 1.05 (closed form) | – |
| Chain, 50 links | 1086 | 13 | 54 | 3.3 | **13.8** | 124 | 2106 | 464 | 150 |
| Chain, 200 links (500 steps) | 58 164 | 321 | 1258 | 12.6 | **50.8** | 319 | 23 070 | 10 400 | 591 |

Typed timings are from a fresh process (`typed-cost`, `generated`); see "an anomaly" below for why. Rapier impulse joints: 20 µs (50 links) and 49 µs (200).

The 50-link chain over 10 s (from horizontal, no damping), energy against E_ref = m g L n²/2 (the potential energy the chain loses by hanging straight down):

| Engine | max \|E − E0\|/E_ref | Stable for 10 s |
| --- | --- | --- |
| Rapier 2D impulse joints | 0.41 (lost) | yes, by losing energy |
| Rapier 2D multibody joints | 0.57 (lost) | yes, by losing energy |
| MuJoCo Euler, implicitfast | – | **no**, reset at 3.125 s |
| MuJoCo RK4 | – | **no**, reset at 7.008 s |
| Native, semi-implicit Euler | – | **no**, not a number at 3.131 s |
| Native, RK4 | – | **no**, not a number at 7.016 s |
| **Native, Dopri5** | **2.7e−7** | **yes** (124 µs a step: 8× real time) |
| Native, TR-BDF2 | 5.1e−2 | yes (2.1 ms a step) |
| Typed (`solve` route, absolute angles), RK4 | 0.16 | yes |
| Typed (`solve` route), Dopri5 | 1.3e−7 | yes |
| 200 links: native Dopri5 | 4.1e−9 | yes (2.8 ms a step with the energy; 319 µs without) |
| 200 links: native RK4, semi-implicit Euler, TR-BDF2 | – | no (6.2 s, 5.3 s, 6.2 s) |

- The fixed-step explicit failures happen **at the same instants as MuJoCo's** (3.13 s and 7.01 s): they are the chain's dynamics at 1 kHz (the free end whips faster than the step resolves), not a defect of either. An adaptive method (Dopri5) carries both chains through with energy kept to 1e−7, which no fixed-step engine here offers. The same RK4 in absolute angles (typed) stayed finite but lost 16 % of the energy: the coordinates change how the whip fails.
- MuJoCo's chain timing differs from the native one mostly by constant factors; native semi-implicit Euler is 4 times faster than MuJoCo's Euler, native RK4 4 times faster than MuJoCo's RK4 (one fewer dimension, no collision pipeline, no general constraint solver).

### N6. Verification beyond the shared cases

**ABA against CRBA + RNEA, and against equations written by hand** (relative difference in q̈, max over draws):

| Check | Draws | Largest difference |
| --- | --- | --- |
| Random trees of 2, 10, 50 bodies (parents drawn among earlier bodies or the ground, 70 % revolute and 30 % prismatic on random axes, random masses, centres, inertias, damping, sideways gravity), random states, joint forces and two point forces | 100 each | 2.2e−15, 1.2e−14, 2.6e−14 |
| Cart-pole against the spike's exact equations (cart force and tip force) | 1000 | 9.6e−16 |
| Double pendulum against its Lagrange equations by hand | 1000 | 3.0e−15 |

**Double pendulum** (two uniform rods of 1 kg and 1 m, from 1 rad and 0.5 rad absolute, 5 s), max |φ − φ_ref| against the hand-written Lagrange equations by RK4 at 1e−5 s (which agrees with itself at 2e−5 s to 6e−14):

| Method | dt 4 ms | 2 ms | 1 ms | 0.5 ms | Order |
| --- | --- | --- | --- | --- | --- |
| Native, semi-implicit Euler | 6.7e−2 | 3.6e−2 | 1.9e−2 | 9.5e−3 | 0.97 |
| Native, RK4 | 1.6e−7 | 1.0e−8 | 6.5e−10 | 4.1e−11 | 3.99 |
| Native, Dopri5 | 3.5e−10 | 1.0e−11 | 3.2e−13 | 2.7e−14 | (adaptive) |
| Native, TR-BDF2 | 7.0e−4 | 1.8e−4 | 4.4e−5 | 1.1e−5 | 2.00 |
| Typed, RK4 | 1.6e−7 | 1.0e−8 | 6.5e−10 | 4.1e−11 | 3.99 |
| Typed, TR-BDF2 | 7.0e−4 | 1.8e−4 | 4.4e−5 | 1.1e−5 | 2.00 |

**Energy** without damping or forcing, dt = 1 ms, 100 s, relative to the energy above the hanging rest: double pendulum, semi-implicit Euler **+12 %** (it is not symplectic when the mass matrix depends on q), RK4 4.9e−11, Dopri5 9.0e−13, TR-BDF2 1.2e−6; a 10-link chain, semi-implicit Euler 78 %, RK4 2.4 %, Dopri5 3.8e−7, TR-BDF2 1.7 %.

**Momentum, free-floating chain.** A floating base costs nothing extra: two massless bodies on prismatic joints along x and y, then the first link on a revolute joint (ABA handles massless bodies, since each joint's articulated inertia includes what hangs on it). Five links in zero gravity, every joint moving, 10 s at 1 ms: drift of linear momentum (relative to |P0|) and angular momentum about the origin: RK4 6.7e−10 and 7.8e−9; Dopri5 7.2e−14 and 1.6e−12; TR-BDF2 4.9e−6 and 7.6e−5; semi-implicit Euler 8.4e−3 and 8.8e−2.

**Joint damping** (b = 0.05 N m s/rad, 1e−5 rad, 10 s) against the linear solution: RK4 1.4e−10, Dopri5 1.2e−10, TR-BDF2 9.2e−6, semi-implicit Euler 1.6e−3 (relative to the amplitude).

**A controller fused with the plant.** The cart-pole with LQR evaluated inside the right-hand side, so plant and controller are one ODE, against the same continuous closed loop on the exact equations (RK4 at 1e−5 s), max |θ − θ_ref| over 6 s:

| Controller | dt 10 ms | 5 ms | 2 ms | 1 ms | Order |
| --- | --- | --- | --- | --- | --- |
| Native ABA, fused, RK4 | 4.4e−8 | 2.7e−9 | 6.7e−11 | 4.2e−12 | 4.01 |
| Typed `cartpole-lqr` (one block), RK4 | 4.4e−8 | 2.7e−9 | 6.7e−11 | 4.2e−12 | 4.01 |
| Typed `cartpole-lqr`, Dopri5 | 9.0e−11 | 3.3e−12 | 3.2e−14 | 1.4e−15 | – |
| Native ABA, controller held over each step (as with any hosted engine) | 2.4e−3 | 1.2e−3 | 4.7e−4 | 2.3e−4 | 1.00 |

What fusing buys is the order of the whole loop: held over the step, the controller's sampling is a first-order gap (2.3e−4 rad at 1 kHz) whatever the plant's integrator; fused, the closed loop is fourth order. A hosted engine (Rapier, MuJoCo) can only hold its inputs over its own step. With the recorded impulse and grab as parameters set between steps, `cartpole-lqr` (RK4, 1 ms) peaks at 0.2119 rad and stays within 9.1e−4 rad of the held-controller reference: the gap is the sampling, not an error.

### N7. Mechanisms as typed equations

**What can be written today.** Everything above for fixed mechanisms, with units checked (`simsandbox check` passed every file at the first try):

| Model | Lines (without comments) | How |
| --- | --- | --- |
| Pendulum | 14 | scalar equations |
| Cart-pole (inputs held) | 27 | the Lagrange equations, 2 × 2 inverse by hand |
| Cart-pole with LQR and grab | 42 | the same, controller and spring as formulas |
| Double pendulum | 24 | Lagrange equations, 2 × 2 inverse by hand |
| n-link chain (any n) | 23 | `grid i`, `grid j`; M and b over (i, j) in closed form; `solve(M, b)`; rows by sums over the links before each |
| n-link chain, ABA generated | 131 (n = 2), 747 (10), 3829 (50), 15 386 (200) | `generate_aba.py`: scalar lines per body, no arrays |

**Where it stops.**

- The `solve` chain exists only because a chain of **uniform links in absolute angles** has a closed-form M (M_ij = m l² μ_ij cos(φ_i − φ_j), with μ a count of the links beyond i and j). A general chain needs M_ij = Σ_k m_k a_ki a_kj over a third axis (expressible, n³ work per evaluation); a **tree** needs "is i an ancestor of k", which the language can only take as a parameter matrix read from a file: it has no recursion along edges and no ordered pass from root to leaves (`of(T, i)` reads one row by id, one hop). Prismatic joints break the cos(φ_i − φ_j) form altogether. So hand-derived M and C do not generalise; they are a per-mechanism derivation.
- Its cost is O(n²) trigonometry plus `solve`'s O(n³) with a copy of M per evaluation: 3.6 µs (2 links), 22 µs (10), 464 µs (50, interpreted: phases this small are not compiled), 10.4 ms (200, compiled; native code 8.9 ms, built in about a second with `SIMSANDBOX_NATIVE=build`). Removing the sums or `solve` showed the sums of n² trigonometric terms are most of it (at 50 links: 229 of 445 µs a step), `solve` about a third.
- **Authoring.** No derivation error happened here, because every typed model was checked against an independent reference the same day (the native ABA to 3e−13, the hand Lagrange reference to 4e−11 at RK4's order). Without such a check, a sign in C or a missing ½ in μ would give a plausible, wrong mechanism: hand-derived M and C are what most needs a reference.
- **The generated ABA** needs no language change and agrees with the native solver to 1.6e−13–1.5e−11 over 2 s. It is linear in n but about **11 times slower than the native ABA** (150 µs against 13.4 µs at 50 links, 591 against 52 at 200; Dopri5 684 and 2779 µs): its 3800 scalar lines run on the interpreter, since only phases of arrays are compiled. The files are machine output nobody would edit (175 KB at 50 links, 743 KB at 200), regenerated on every change to the mechanism.
- **Crossover.** Against the `solve` route the generated ABA wins from about 10–20 links (22 against 28 µs at 10; 464 against 150 at 50). Against the native solver there is no crossover: closed-form typed equations come within 2 times of it only for the smallest mechanisms (cart-pole 0.8–1.05 µs against 0.54).

**An anomaly.** In one long process the compiled 200-link chain took 267 ms a step (and the 50-link chain 3.9 ms in the full suite) instead of 10.4 ms and 0.46 ms in a fresh process. It reproduces whenever the 50-link chain ran with TR-BDF2 earlier in the same process (`SPIKE_TYPED_ONLY=chain-50,chain-200 … typed-cost`); glibc's malloc thresholds are not the cause; the cause is not found. Suggested as a separate task.

### N8. Featherstone as a typed-equations function (estimate)

The owner's fifth option: the language gains a built-in such as `articulated(bodies, q, qd, tau, f_ext)`, as `solve` hides its factorisation, with the native ABA as its body. Not built (no workspace changes); **this is an estimate from measured parts**:

- ABA per evaluation, measured: 0.09 µs (2 links), 0.63 (10), 3.5 (50), 14.3 (200).
- A built-in call through the interpreter, measured: `overhead-solve` against `overhead-base` (two states, a 2 × 2 matrix, RK4): 1.13 against 0.22 µs a step, so **about 0.23 µs per call** (the relabelling, the call, the copy and a 2 × 2 elimination). Fixed per-step cost of a small typed model: about 0.2–0.3 µs.
- Estimate per RK4 step = 4 × (ABA + 0.23 µs) + 0.25 µs, plus forward kinematics for the outputs once a step (about a quarter of an ABA):

| Links | Native block, RK4 (measured) | **Featherstone function, RK4 (estimate)** | Generated ABA, RK4 (measured) | `solve` route, RK4 (measured) |
| --- | --- | --- | --- | --- |
| 2 | 0.46 µs | ≈ 1.5 µs | 4.0 µs | 3.6 µs |
| 10 | 2.7 µs | ≈ 4 µs | 28 µs | 22 µs |
| 50 | 13.3 µs | ≈ 16 µs | 150 µs | 464 µs |
| 200 | 52 µs | ≈ 62 µs | 591 µs | 10 400 µs |

So within about 1.2 times of the native block at 50 links and more, 1.5 times at 10 and 3 times at 2 (where both are under 2 µs: hundreds of times real time at 1 kHz). Expected, not measured: the overhead of passing a table's columns could be larger than `solve`'s, and the function would need its structure declared to implicit methods (dense: every q̈ reads every q and q̇).

**Design questions for the owner** (recommendations in bold):

1. **Units.** The language gives a quantity one unit ([ADR 0045](../decisions/0045-units-in-typed-equations.md); [the reference](../reference/typed-equations.md#units)): an array over a table has one unit for all its rows. Joint coordinates mix units (cart x in m, pole θ in rad, so dimensionless), as do their forces (N and N m). Options: one table per joint kind (`hinges` in rad with torques in N m, `sliders` in m with forces in N), each a separate state array; a unit carried by a joint-kind column (against the language's rule, and `check` could not balance an equation over the rows); dimensionless coordinates with a length scale per slider (loses the check exactly where a mistake is likely). **Recommended: one table per joint kind.** Its cost: the function then gives accelerations for two arrays, and the language has no function with two results: either `hinge_acc(…)` and `slider_acc(…)` share one ABA per evaluation (a cache keyed on its arguments) or the language gains multi-result calls. Mechanisms of one joint kind (chains, arms, pendulums) need neither.
2. **The bodies table and its tree.** Expressible today ([ADR 0079](../decisions/0079-tables-of-members.md), [ADR 0085](../decisions/0085-meshes-are-tables-of-cells.md)): `table bodies`, `start bodies = file "bodies.csv"` with an `id` column; per-row parameters from that file (`param mass[bodies] [kg]`, `param parent[bodies]`, offsets, axis); or the joints as `edges joints of bodies` with `from`/`to` ids. What is missing is only what the function would hide: an ordered pass along the parent edges (leaves to root, root to leaves). The function must check that `parent` makes a tree (no cycles, ids that exist) at load, and a table with rules (rows coming and going) would change the mechanism mid-run: **allow only tables without rules**, at least at first.
3. **Point forces** (a grab's spring) as rows over the bodies: `fx[bodies]`, `fy[bodies]` and the point `px[bodies]`, `py[bodies]` in body axes, one force per body; several forces on one body add up before the call. The spring is a formula of the outputs (`pose`) and the pointer, as [ADR 0093](../decisions/0093-scenes-from-primitive-layers.md) wants: **one force per body row**.
4. **Outputs as rows.** `pose(bodies, q, k)` with k = 0, 1, 2 for x, y, angle, and `velocity(bodies, q, qd, k)`, following `normal(f, k, x, y)`'s component argument; each a cheap O(n) forward pass. The scene's frame over rows takes them as any table's columns.
5. **Jacobians for implicit methods:** finite differences over a declared dense structure now (as the native spike does); exact derivatives of ABA (known algorithms) later, if TR-BDF2 on mechanisms matters. Measured here: with a dense finite-difference Jacobian a TR-BDF2 step costs about 150 RK4 steps at 50 links (2.1 ms against 13.8 µs).
6. **Not covered:** closed loops (a four-bar linkage needs loop-closure constraints: a DAE, by Baumgarte stabilisation or by SimSandbox's implicit methods on an index-reduced form); joint limits (contacts in disguise: penalty springs keep it an ODE but stiff; hard limits are impulses); contacts. These stay with Rapier or wait for their own decision.
7. **Conflicts.** [ADR 0084](../decisions/0084-where-typed-equations-stop.md) point 3 says algorithms built from primitives ship as models, "never as built-in designs", and point 2 asks every addition for a "use library X instead" row; `articulated` is an algorithm (like `solve`, which is linear algebra), so it **widens the language** and needs its own ADR that amends 0084, with Rapier's and MuJoCo's multibody solvers as the "library instead" rows. The owner's rule of 2026-11-14 puts the Phase 2 gate before further typed-equation features ([ADR 0091](../decisions/0091-equations-first-hosting-for-existing-codes.md) point 4); this is the gate's own rigid-body work, but it is still a language feature. **Recommended: not for the gate; after it, as its own ADR, built on the same Rust core as the native block.**

### Effort and fit

- **Size.** The solver core (spatial algebra, ABA, CRBA, RNEA, forward kinematics, energy, momentum, Cholesky) is about 455 lines of Rust without comments; the spike's models 130 and its stepper and adapter 230. Written and cross-checked in one session.
- **Beyond the spike, for a real crate** (expectations, not measured):
  - The block: an `OdeSystem` (it implements `Equations`, so it fuses with typed-equation blocks in one group and uses every integrator), its ports and units, rows of bodies, joint coordinates as scalar outputs, the Jacobian's structure declared: a few hundred lines.
  - File form: tables of bodies (mass, centre, inertia, parent, joint kind, offset, axis), checked (a tree, positive masses, units) and listed by `describe`.
  - Springs, motors and damping on joints: generalized-force terms, small. Joint limits as penalty spring-dampers: small, but stiff (TR-BDF2), and labelled as an approximation.
  - Closed loops: loop-closure constraints and their stabilisation (Baumgarte, or projection), with their own verification: the largest single item, not needed for chains, trees, the cart-pole or arms.
  - 3D: 6-vector spatial algebra, spherical and free joints with quaternion states (states not in a flat Euclidean space: normalisation, or integration on the manifold); about two to three times the 2D work. Rapier and MuJoCo exist for 3D in the meantime.
- **Fit as a block.** States `[q, q̇]` in the kernel's states vector: snapshots, rewind and branching unchanged, 16 bytes per joint; no opaque state, no new capability. Inputs: joint forces (scalars) and point forces (rows). Outputs: bodies as rows (`x`, `y`, `angle`, `vx`, `vy`, `omega`) and joint coordinates as scalars. Fused with a typed controller, plant and controller are integrated together at RK4's order (N6), which no hosted engine allows.
- **Where the line falls.** Native: jointed mechanisms without contact (chains, trees, the cart-pole, arms, pendulums), with damping, springs and motors. Rapier: contacts and collisions, piles, anything dropped or stacked, joint limits that must be hard, closed loops until they have their own decision, and **jointed mechanisms that touch things** (Rapier's multibody joints handle joints and contacts in one solver). A model mixing both by coupling, a native arm whose end pushes Rapier boxes, would exchange contact forces once a step, a step late: stable for slow, soft pushes, wrong for impacts and resting contact, where the contact impulse and the arm's motion must be solved together. **One model, one engine for anything that touches**: a mechanism that must touch moves to Rapier as a whole, and the file form should make that a change of engine, not a rewrite.
