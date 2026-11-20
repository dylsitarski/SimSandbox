# Rigid-body spike: Rapier, MuJoCo, a native solver and typed equations (ADR 0057 item 3)

The spike [ADR 0057](../../docs/decisions/0057-phase-2-plan.md) item 3 asks for before the first rigid-body backend is chosen: the same cases on both engines, printed as Markdown tables. Results, findings and the recommendation: [the spike's report](../../docs/research/2026-11-19-rigid-body-spike.md); the decision it proposes: [ADR 0103](../../docs/decisions/0103-rigid-body-engine.md).

This crate is not part of the workspace, so neither engine enters SimSandbox's build, CI or `cargo deny` check. It depends on `simsandbox-eq`, `simsandbox-core` and `simsandbox-file` by path (SimSandbox's integrators and the kernel that runs typed equations), without changing them. The follow-up of 2026-11-19 (a native solver, typed equations, Featherstone as a typed-equations function): [the report's section](../../docs/research/2026-11-19-rigid-body-spike.md#a-native-articulated-body-solver-2026-11-19).

| Engine | How | Where |
| --- | --- | --- |
| Rapier 0.36.1 | `rapier2d-f64` and `rapier3d-f64` from crates.io, feature `enhanced-determinism` (default here; `--no-default-features` turns it off) | `src/rapier_2d.rs`, `src/rapier_3d.rs` |
| MuJoCo 4.0.1 | its C library, bindings written by hand for the dozen functions used; `mjModel` and `mjData` opaque, every value through `mj_getState`/`mj_setState` | `src/mujoco.rs` (feature `mujoco`) |
| Native 2D | Featherstone's articulated-body algorithm (and CRBA + RNEA as a check) in 2D spatial algebra, pure Rust; a semi-implicit Euler written there, and RK4, Dormand–Prince and TR-BDF2 from `simsandbox-eq`; state `[q, q̇]` | `src/native_2d.rs` |
| Typed equations | the models as `eq.equations` files, loaded by `simsandbox-file` and stepped by the kernel's `Simulator` with RK4, Dormand–Prince or TR-BDF2 | `typed/*.ssbx.json`, `src/typed.rs` |

The cases (`src/main.rs`), on Rapier 2D and 3D with impulse and with multibody joints and on MuJoCo with its Euler, RK4 and implicitfast integrators; the models, the exact answers and the controller are in `src/common.rs`:

1. `pendulum`: the period of a compact bob against the exact (elliptic-integral) period at 0.1 and 2 rad over five step sizes, and the energy over 100 periods.
2. `cartpole`: a cart-pole at 1 kHz balanced by LQR through a recorded impulse and a pull toward a held pointer, against the exact equations by RK4; and falling open loop.
3. `rewind`: snapshot at step k, run to n, restore, run again with the recorded input, compare bit for bit; four kinds of snapshot, with their size and time.
4. `determinism`: hashes of every body's pose and velocity at every step, twice in one process and in two more processes.
5. `speed`: the cart-pole, a chain of 50 links and 100 boxes falling into a pile.
6. `contacts`: a box on the ground, a box on a box, a box dropped onto a box.

Cases 1 to 5 also run the native solver and the typed equations (the 200-link chain is in `speed` too). Their own cases (`src/native_cases.rs`):

7. `chain`: the 50- and 200-link chains for 10 s: energy against the energy the chain loses by hanging, and when an engine fails.
8. `native`: ABA against CRBA + RNEA on random trees and against the cart-pole's and the double pendulum's equations written by hand; ABA and CRBA timed at 2, 10, 50, 200 links; the double pendulum against its Lagrange equations (order); energy, momentum of a free-floating chain, joint damping; a controller fused with the plant against one held over the step.
9. `typed`: the typed double pendulum, the `solve(M, b)` chains against the native solver, the cart-pole with its LQR in the typed block, the generated ABA (`generated`) and the cost of each typed model (`typed-cost`; `SPIKE_TYPED_ONLY=chain-50,chain-200` limits it).

The typed models in `typed/`: `pendulum`, `cartpole` (inputs as parameters set before each step), `cartpole-lqr` (controller and grab in the block), `double-pendulum`, `chain-N` (n links through `solve`), `aba-chain-N` (the ABA unrolled into scalar lines by `python3 typed/generate_aba.py`, which writes the 50- and 200-link files, left out of git for their size) and `overhead-*` (a built-in call's cost). `target/release/rigid-body-spike typed-time FILE METHOD STEPS` times any model file (with `SIMSANDBOX_NATIVE=build` it first waits for the native library).

## Build and run

```sh
cd spikes/rigid-body
./fetch-mujoco.sh                      # MuJoCo 4.0.1 from PyPI into ./mujoco (ignored by git)
cargo build --release -j 2 --features mujoco
python3 typed/generate_aba.py          # the generated typed models (aba-chain-*)
target/release/rigid-body-spike        # every case, as Markdown (about 35 minutes); or name cases: pendulum cartpole …
SPIKE_QUICK=1 target/release/rigid-body-spike   # shorter runs
cargo build --release -j 2             # Rapier only, without MuJoCo
```

- **Getting MuJoCo.** `fetch-mujoco.sh` downloads the `mujoco-core` wheel from PyPI (it ships `libmujoco.so.4.0.1` and `include/mujoco/*.h`; GitHub release archives were not needed) and unpacks the library, headers and licence into `./mujoco/`. It is written for Linux x86-64. Elsewhere, take the matching wheel (`--platform`) or a release archive from GitHub and set `MUJOCO_DIR` to a folder with `lib/` and `include/`. `build.rs` links `lib/libmujoco.so` with an rpath to it.
- The bindings check at start that the library is the version their constants were copied from (`mj_version() == 4000001`).
- `SPIKE_RAPIER_ITERS=n` changes Rapier's solver iterations (substeps) per step for experiments; the tables use the default (4). `trace` prints the dropped box's path.

- Run typed-equation timings in a fresh process (`typed-cost`, `generated`): in one long process the compiled 200-link chain was once 26 times slower (see the report).

Measured 2026-11-19 on Linux (Intel Xeon at 2.30 GHz, 4 cores, Rust 1.97.0).
