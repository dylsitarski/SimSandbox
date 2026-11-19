# Rigid-body spike: Rapier against MuJoCo (ADR 0057 item 3)

The spike [ADR 0057](../../docs/decisions/0057-phase-2-plan.md) item 3 asks for before the first rigid-body backend is chosen: the same cases on both engines, printed as Markdown tables. Results, findings and the recommendation: [the spike's report](../../docs/research/2026-11-19-rigid-body-spike.md); the decision it proposes: [ADR 0103](../../docs/decisions/0103-rigid-body-engine.md).

This crate is not part of the workspace, so neither engine enters SimSandbox's build, CI or `cargo deny` check.

| Engine | How | Where |
| --- | --- | --- |
| Rapier 0.36.1 | `rapier2d-f64` and `rapier3d-f64` from crates.io, feature `enhanced-determinism` (default here; `--no-default-features` turns it off) | `src/rapier_2d.rs`, `src/rapier_3d.rs` |
| MuJoCo 4.0.1 | its C library, bindings written by hand for the dozen functions used; `mjModel` and `mjData` opaque, every value through `mj_getState`/`mj_setState` | `src/mujoco.rs` (feature `mujoco`) |

The cases (`src/main.rs`), on Rapier 2D and 3D with impulse and with multibody joints and on MuJoCo with its Euler, RK4 and implicitfast integrators; the models, the exact answers and the controller are in `src/common.rs`:

1. `pendulum`: the period of a compact bob against the exact (elliptic-integral) period at 0.1 and 2 rad over five step sizes, and the energy over 100 periods.
2. `cartpole`: a cart-pole at 1 kHz balanced by LQR through a recorded impulse and a pull toward a held pointer, against the exact equations by RK4; and falling open loop.
3. `rewind`: snapshot at step k, run to n, restore, run again with the recorded input, compare bit for bit; four kinds of snapshot, with their size and time.
4. `determinism`: hashes of every body's pose and velocity at every step, twice in one process and in two more processes.
5. `speed`: the cart-pole, a chain of 50 links and 100 boxes falling into a pile.
6. `contacts`: a box on the ground, a box on a box, a box dropped onto a box.

## Build and run

```sh
cd spikes/rigid-body
./fetch-mujoco.sh                      # MuJoCo 4.0.1 from PyPI into ./mujoco (ignored by git)
cargo build --release -j 2 --features mujoco
target/release/rigid-body-spike        # every case, as Markdown; or name cases: pendulum cartpole …
SPIKE_QUICK=1 target/release/rigid-body-spike   # shorter runs
cargo build --release -j 2             # Rapier only, without MuJoCo
```

- **Getting MuJoCo.** `fetch-mujoco.sh` downloads the `mujoco-core` wheel from PyPI (it ships `libmujoco.so.4.0.1` and `include/mujoco/*.h`; GitHub release archives were not needed) and unpacks the library, headers and licence into `./mujoco/`. It is written for Linux x86-64. Elsewhere, take the matching wheel (`--platform`) or a release archive from GitHub and set `MUJOCO_DIR` to a folder with `lib/` and `include/`. `build.rs` links `lib/libmujoco.so` with an rpath to it.
- The bindings check at start that the library is the version their constants were copied from (`mj_version() == 4000001`).
- `SPIKE_RAPIER_ITERS=n` changes Rapier's solver iterations (substeps) per step for experiments; the tables use the default (4). `trace` prints the dropped box's path.

Measured 2026-11-19 on Linux (Intel Xeon at 2.30 GHz, 4 cores, Rust 1.97.0).
