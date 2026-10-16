# Status

_Last updated: 2026-10-16_

**Phase:** 0 · Foundations ([roadmap](roadmap.md#phase-0--foundations)). **Gate:** the Lorenz system runs live on the desktop with sliders. **Gate met:** no.

## What works

- Cargo workspace with three crates: `simsandbox-core`, `simsandbox-eq`, `simsandbox-cli`; CI on Windows, macOS and Linux.
- Block contract: required `Block` trait plus optional `Equations`, `Rollback`, `Derivatives`, `Realtime` capability traits (`crates/simsandbox-core/src/block.rs`).
- SI units with dimension checking and automatic conversion at connect time (`units.rs`).
- Model graph with unit-, shape- and direction-checked signal connections (`graph.rs`).
- Co-simulation master: fixed communication step, Jacobi exchange; kernel-owned state; snapshots and rewind that also restore parameter values; live parameter changes (`sim.rs`).
- Parameter access: `Model::param`, `Model::params`, `Model::block_ids`, `Simulator::model`, `Simulator::param`; `set_param` accepts any finite value and rejects unknown names (`SimError::UnknownParam`) and NaN or infinite values (`SimError::InvalidParam`); `ParamSpec` ranges are suggestions ([ADR 0011](decisions/0011-parameter-entry.md)).
- Equation domain: Euler, RK4, Dormand–Prince 5(4), each with speed/accuracy labels; fusion of equation Blocks into one system (`crates/simsandbox-eq`).
- Example systems: Lorenz, coupled oscillator.
- `simsandbox lorenz` CLI writing CSV, with `--method euler|rk4|dopri5`.
- 17 tests, including verification against analytic solutions and convergence order.

## Done recently

- 2026-10-16: kernel prep for model files and the UI (parameter access; snapshots include parameters; suggested rather than enforced parameter ranges); ADRs 0007 (JSON model files), 0008 (egui + wgpu UI), 0009 (hobbyists first), 0010 (live loop pacing) and 0011 (parameter entry).

## In progress

- Nothing.

## Next up

In priority order. Each item should end with tests and updated docs.

1. **Model file format** per [ADR 0007](decisions/0007-json-model-files.md): block-type registry in the kernel, `simsandbox_eq::register`, a JSON reader/writer crate with a JSON Schema, round-trip tests for Lorenz and the coupled oscillator, and `simsandbox run <file>` in the CLI.
2. **Desktop app shell.** New crate `simsandbox-app` (eframe + egui + wgpu, per [ADR 0008](decisions/0008-native-egui-ui.md)): window, run/pause/rewind controls, a live time-series plot, paced by wall-clock time with a speed control ([ADR 0010](decisions/0010-live-loop-pacing.md)).
3. **Auto-generated parameter panel** per [ADR 0011](decisions/0011-parameter-entry.md): a typed entry field with units for every `ParamSpec` (via `Simulator::model()`), plus opt-in sliders with user-set ranges, wired to `Simulator::set_param`.
4. **Phase-portrait / 3D trajectory view** for the Lorenz system.
5. **Symbolic equation entry.** Parse `dx/dt = sigma*(y - x)` into an `OdeSystem`; choose the parser approach via ADR.

## Known limitations

- Fused equation groups require outputs that depend only on state and parameters (no direct feedthrough), so algebraic loops are not supported ([ADR 0002](decisions/0002-layered-block-contract.md)).
- Physical (acausal) ports are not implemented.
- The master uses a fixed step with no coupling-error estimate, iteration or rollback.
- `FusedSystem::rhs` allocates per call; fine for small models, revisit before real-time use with large groups.
- No desktop UI yet.

## Blockers

- None.
