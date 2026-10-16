# Status

_Last updated: 2026-10-16_

**Phase:** 0 · Foundations ([roadmap](roadmap.md#phase-0--foundations)). **Gate:** the Lorenz system runs live on the desktop with sliders. **Gate met:** no.

## What works

- Cargo workspace with four crates: `simsandbox-core`, `simsandbox-eq`, `simsandbox-file`, `simsandbox-cli`; CI on Windows, macOS and Linux.
- Block contract: required `Block` trait plus optional `Equations`, `Rollback`, `Derivatives`, `Realtime` capability traits (`crates/simsandbox-core/src/block.rs`).
- SI units with dimension checking and automatic conversion at connect time (`units.rs`).
- Model graph with unit-, shape- and direction-checked signal connections (`graph.rs`).
- Co-simulation master: fixed communication step, Jacobi exchange; kernel-owned state; snapshots and rewind that also restore parameter values; live parameter changes (`sim.rs`).
- Parameter access: `Model::param`, `Model::params`, `Model::block_ids`, `Simulator::model`, `Simulator::param`; `set_param` accepts any finite value and rejects unknown names (`SimError::UnknownParam`) and NaN or infinite values (`SimError::InvalidParam`); `ParamSpec` ranges are suggestions ([ADR 0011](decisions/0011-parameter-entry.md)).
- Equation domain: Euler, RK4, Dormand–Prince 5(4), each with speed/accuracy labels; fusion of equation Blocks into one system (`crates/simsandbox-eq`).
- Example systems: Lorenz, coupled oscillator; example files `examples/lorenz.ssbx.json` and `examples/coupled-oscillators.ssbx.json`.
- Model files ([ADR 0007](decisions/0007-json-model-files.md), [ADR 0012](decisions/0012-controls-in-model-files.md)): block-type registry (`crates/simsandbox-core/src/registry.rs`), equation types `eq.lorenz` and `eq.coupled_oscillator` (`simsandbox_eq::register`), JSON load/save with errors located by line/column or path, lenient `controls`, JSON Schema (`crates/simsandbox-file/model.schema.json`).
- CLI: `simsandbox run FILE` writes every output as CSV; `simsandbox lorenz` with `--method euler|rk4|dopri5`.
- 30 tests, including verification against analytic solutions and convergence order.

## Done recently

- 2026-10-16: model file format (registry, `simsandbox-file` crate, examples, `simsandbox run`).
- 2026-10-16: kernel prep for model files and the UI (parameter access; snapshots include parameters; suggested rather than enforced parameter ranges); ADRs 0007 (JSON model files), 0008 (egui + wgpu UI), 0009 (hobbyists first), 0010 (live loop pacing), 0011 (parameter entry) and 0012 (controls saved in model files).

## In progress

- Nothing.

## Next up

In priority order. Each item should end with tests and updated docs.

1. **Desktop app shell.** New crate `simsandbox-app` (eframe + egui + wgpu, per [ADR 0008](decisions/0008-native-egui-ui.md)): window, open a model file (start with `examples/`), run/pause/rewind controls, a live time-series plot, paced by wall-clock time with a speed control ([ADR 0010](decisions/0010-live-loop-pacing.md)).
2. **Auto-generated parameter panel** per [ADR 0011](decisions/0011-parameter-entry.md): a typed entry field with units for every `ParamSpec` (via `Simulator::model()`), plus opt-in sliders with user-set ranges, wired to `Simulator::set_param` and saved in the file's `controls` section.
3. **Phase-portrait / 3D trajectory view** for the Lorenz system.
4. **Symbolic equation entry.** Parse `dx/dt = sigma*(y - x)` into an `OdeSystem`; choose the parser approach via ADR.

## Known limitations

- Fused equation groups require outputs that depend only on state and parameters (no direct feedthrough), so algebraic loops are not supported ([ADR 0002](decisions/0002-layered-block-contract.md)).
- Physical (acausal) ports are not implemented.
- The master uses a fixed step with no coupling-error estimate, iteration or rollback.
- `FusedSystem::rhs` allocates per call; fine for small models, revisit before real-time use with large groups.
- No desktop UI yet.
- Model files cannot describe fused equation groups yet, and a model built in code cannot be saved (Blocks do not know their type name); files are the source for saving.
- Integer method options are written as numbers with a decimal point (`"substeps": 4.0`).

## Blockers

- None.
