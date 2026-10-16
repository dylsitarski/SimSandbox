# Status

_Last updated: 2026-10-16_

**Phase:** 0 · Foundations ([roadmap](roadmap.md#phase-0--foundations)). **Gate:** the Lorenz system runs live on the desktop with sliders. **Gate met:** yes, 2026-10-16: Lorenz runs live in the desktop app with rho and sigma sliders (checked headless in the cloud; to confirm on the owner's Windows machine). Phase 0 continues with the remaining roadmap items before Phase 1.

## What works

- Cargo workspace with five crates: `simsandbox-core`, `simsandbox-eq`, `simsandbox-file`, `simsandbox-cli`, `simsandbox-app`; CI on Windows, macOS and Linux.
- Block contract: required `Block` trait plus optional `Equations`, `Rollback`, `Derivatives`, `Realtime` capability traits (`crates/simsandbox-core/src/block.rs`).
- SI units with dimension checking and automatic conversion at connect time (`units.rs`).
- Model graph with unit-, shape- and direction-checked signal connections (`graph.rs`).
- Co-simulation master: fixed communication step, Jacobi exchange; kernel-owned state; snapshots and rewind that also restore parameter values; live parameter changes; `Simulator::restart` re-initialises from current parameters (`sim.rs`). `ParamSpec::initial_condition` marks parameters that only apply on restart.
- Parameter access: `Model::param`, `Model::params`, `Model::block_ids`, `Simulator::model`, `Simulator::param`; `set_param` accepts any finite value and rejects unknown names (`SimError::UnknownParam`) and NaN or infinite values (`SimError::InvalidParam`); `ParamSpec` ranges are suggestions ([ADR 0011](decisions/0011-parameter-entry.md)).
- Equation domain: Euler, RK4, Dormand–Prince 5(4), each with speed/accuracy labels; fusion of equation Blocks into one system (`crates/simsandbox-eq`).
- Example systems: Lorenz, coupled oscillator; example files `examples/lorenz.ssbx.json` and `examples/coupled-oscillators.ssbx.json`.
- Model files ([ADR 0007](decisions/0007-json-model-files.md), [ADR 0012](decisions/0012-controls-in-model-files.md)): block-type registry (`crates/simsandbox-core/src/registry.rs`), equation types `eq.lorenz` and `eq.coupled_oscillator` (`simsandbox_eq::register`), JSON load/save with errors located by line/column or path, lenient `controls`, a `run` section with `dt` and optional `t_end` ([ADR 0013](decisions/0013-run-section.md)), JSON Schema (`crates/simsandbox-file/model.schema.json`).
- Desktop app `simsandbox-app` ([ADR 0008](decisions/0008-native-egui-ui.md)): opens a model file (path field, drag and drop, command-line argument) or a built-in example; starts running at once; run/pause, Restart (re-initialise from current parameters), rewind slider that branches, pause at the file's `t_end`; parameter panel with a typed field per parameter, units, initial-condition markers and opt-in sliders with editable ranges ([ADR 0011](decisions/0011-parameter-entry.md)); Save writes parameters, step, sliders and speed to the path in the File field ([ADR 0012](decisions/0012-controls-in-model-files.md), [ADR 0013](decisions/0013-run-section.md)); speed control and editable step with wall-clock pacing, an 8 ms per-frame budget and an achieved-speed warning ([ADR 0010](decisions/0010-live-loop-pacing.md)); speed and accuracy labels; live time-series plot of the file's `controls.plots` (all outputs if none); load errors and warnings shown in the window; `--screenshot` for headless checks.
- CLI: `simsandbox run FILE` writes every output as CSV, using the file's `run` section unless `--dt`/`--t-end` override it; `simsandbox lorenz` with `--method euler|rk4|dopri5`.
- 44 tests, including verification against analytic solutions and convergence order.

## Done recently

- 2026-10-16: parameter panel, Restart and Save in the app; `run` section in model files (ADR 0013).
- 2026-10-16: desktop app shell (`crates/simsandbox-app`).
- 2026-10-16: model file format (registry, `simsandbox-file` crate, examples, `simsandbox run`).
- 2026-10-16: kernel prep for model files and the UI (parameter access; snapshots include parameters; suggested rather than enforced parameter ranges); ADRs 0007 (JSON model files), 0008 (egui + wgpu UI), 0009 (hobbyists first), 0010 (live loop pacing), 0011 (parameter entry) and 0012 (controls saved in model files).

## In progress

- Nothing.

## Next up

In priority order. Each item should end with tests and updated docs.

1. **Phase-portrait / 3D trajectory view** for the Lorenz system: plot one output against another (x–z), then a rotatable 3D view (wgpu paint callback, [ADR 0008](decisions/0008-native-egui-ui.md)).
2. **Native file dialogs** for Open and Save (e.g. the `rfd` crate), replacing the path field.
3. **Symbolic equation entry.** Parse `dx/dt = sigma*(y - x)` into an `OdeSystem`; choose the parser approach via ADR.

## Known limitations

- Fused equation groups require outputs that depend only on state and parameters (no direct feedthrough), so algebraic loops are not supported ([ADR 0002](decisions/0002-layered-block-contract.md)).
- Physical (acausal) ports are not implemented.
- The master uses a fixed step with no coupling-error estimate, iteration or rollback.
- `FusedSystem::rhs` allocates per call; fine for small models, revisit before real-time use with large groups.
- Save writes to the path typed in the File field and overwrites without asking; built-in examples have no path until one is typed.
- The app has no native file dialog; files open from a path field, drag and drop or the command line.
- The simulation runs on the UI thread ([ADR 0010](decisions/0010-live-loop-pacing.md)); a heavy model slows the window's controls as well as the plot.
- Plotted series and rewind history are thinned (every other point dropped) beyond 100 000 points and 10 000 snapshots, so very long runs lose resolution in the past.
- Model files cannot describe fused equation groups yet, and a model built in code cannot be saved (Blocks do not know their type name); files are the source for saving.
- Integer method options are written as numbers with a decimal point (`"substeps": 4.0`).

## Blockers

- None.
