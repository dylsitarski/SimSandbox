# Status

_Last updated: 2026-10-17_

**Phase:** 0 · Foundations ([roadmap](roadmap.md#phase-0--foundations)). **Gate:** the Lorenz system runs live on the desktop with sliders. **Gate met:** yes, 2026-10-16: Lorenz runs live in the desktop app with rho and sigma sliders (checked headless in the cloud; to confirm on the owner's Windows machine). Phase 0 continues with the remaining roadmap items before Phase 1.

## What works

- Cargo workspace with seven crates: `simsandbox-core`, `simsandbox-expr`, `simsandbox-eq`, `simsandbox-file`, `simsandbox-cli`, `simsandbox-views`, `simsandbox-app`; CI on Windows, macOS and Linux.
- Block contract: required `Block` trait plus optional `Equations`, `Rollback`, `Derivatives`, `Realtime` capability traits (`crates/simsandbox-core/src/block.rs`).
- SI units with dimension checking and automatic conversion at connect time (`units.rs`).
- Model graph with unit-, shape- and direction-checked signal connections (`graph.rs`).
- Co-simulation master: fixed communication step, Jacobi exchange; kernel-owned state; snapshots and rewind that also restore parameter values; live parameter changes; `Simulator::restart` re-initialises from current parameters (`sim.rs`). `ParamSpec::initial_condition` marks parameters that only apply on restart.
- Parameter access: `Model::param`, `Model::params`, `Model::block_ids`, `Simulator::model`, `Simulator::param`; `set_param` accepts any finite value and rejects unknown names (`SimError::UnknownParam`) and NaN or infinite values (`SimError::InvalidParam`); `ParamSpec` ranges are suggestions ([ADR 0011](decisions/0011-parameter-entry.md)).
- Equation domain: Euler, RK4, Dormand–Prince 5(4), each with speed/accuracy labels; fusion of equation Blocks into one system (`crates/simsandbox-eq`).
- Typed equations ([ADR 0017](decisions/0017-typed-equations.md)): `simsandbox-expr` parses and evaluates expressions with column-accurate errors; `eq.equations` blocks (`EquationSet`) turn lines like `dx/dt = sigma*(y - x)` and `E = …` into states, computed outputs, parameters (default 1) and initial values `x0`; discrete maps `x[n+1] = …` (method `iterate`) and formula-only blocks (method `evaluate`), [ADR 0021](decisions/0021-maps-formulas-and-sweeps.md); stored in model files as `equations`; verified against the built-in Lorenz (bit for bit), exponential decay, the Lotka–Volterra invariant and the exact logistic map at r = 4.
- Sweeps: a model with no states is swept instead of run: outputs evaluated across one parameter's range, recomputed on every slider change, saved as `controls.sweep`; sweep controls replace Run/Pause in the app.
- Example systems: Lorenz, coupled oscillator, damped pendulum (`eq.pendulum`), heat along a rod (`eq.heat_rod`, temperature and position at 50 points as grid outputs, rod length a parameter) and orbiting test particles (`eq.orbits`, a 200-point set), each verified against known solutions; example files `examples/lorenz.ssbx.json`, `examples/coupled-oscillators.ssbx.json`, `examples/predator-prey.ssbx.json` (typed equations), `examples/logistic-map.ssbx.json` (a map), `examples/resonance.ssbx.json` (formulas, swept), `examples/pendulum.ssbx.json`, `examples/heat-rod.ssbx.json` and `examples/orbits.ssbx.json`.
- Port shapes: scalar, vector, grid (`Shape::Grid { nx, ny }`) and point set (`Shape::Points { count, dim }`).
- Model files ([ADR 0007](decisions/0007-json-model-files.md), [ADR 0012](decisions/0012-controls-in-model-files.md), [ADR 0023](decisions/0023-automatic-fusion.md)): automatic fusion of connected equation blocks with identical methods, `fusion: off`, explicit `groups`; block-type registry (`crates/simsandbox-core/src/registry.rs`), equation types `eq.lorenz` and `eq.coupled_oscillator` (`simsandbox_eq::register`), JSON load/save with errors located by line/column or path, lenient `controls`, a `run` section with `dt` and optional `t_end` ([ADR 0013](decisions/0013-run-section.md)), JSON Schema (`crates/simsandbox-file/model.schema.json`).
- Desktop app `simsandbox-app` ([ADR 0008](decisions/0008-native-egui-ui.md)): opens a model file (native Open dialog, drag and drop, command-line argument) or a built-in example; starts running at once; "New model…" and "Edit equations…" open an equation editor whose Apply runs the equations, keeping parameter values that still exist; run/pause, Restart (re-initialise from current parameters), rewind slider that branches, pause at the file's `t_end`; parameter panel with a typed field per parameter, units, initial-condition markers and opt-in sliders with editable ranges ([ADR 0011](decisions/0011-parameter-entry.md)); Save (back to the opened file) and Save as (native dialog, asks before overwriting), with Ctrl+O, Ctrl+S and Ctrl+Shift+S; saving writes parameters, step, sliders, speed and view ([ADR 0012](decisions/0012-controls-in-model-files.md), [ADR 0013](decisions/0013-run-section.md)); speed control and editable step with wall-clock pacing, an 8 ms per-frame budget and an achieved-speed warning ([ADR 0010](decisions/0010-live-loop-pacing.md)); speed and accuracy labels; pluggable views ([ADR 0015](decisions/0015-pluggable-views.md), `crates/simsandbox-views`) of the file's `controls.plots` (all outputs if none; grids and point sets are recorded whole as fields): `plot.xy` (any signals against time or each other, [ADR 0018](decisions/0018-one-xy-plot.md)), `plot.3d` (rotatable, zoomable), `plot.heatmap` (a grid; a line of cells over time, placed by an optional `position` field, time across or upwards), `plot.profile` (line fields now against a coordinate field) and `plot.scatter` (a point set); infinite and NaN values leave gaps instead of crashing the plot, plus the domain view `eq.pendulum`, offered only for models with a pendulum (`applies_to`, registered by `simsandbox-eq` behind its `views` feature), shown together as tiles: the model's own views first, more from the presets under "More views…" (plot against time, phase portrait, 3D trajectory) ([ADR 0016](decisions/0016-model-curated-views.md)), each with slot pickers and a close button; a colour slot (fading by `t` by default, or colour by any signal) and a `trail_length` style property for the selected tile under "View" in the parameter panel (seconds of trail; 0 shows none, -1 the whole run; against time it is a scrolling window); the scroll wheel zooms 2D plots around the pointer; derived signals defined in a "Signals…" window and saved as `controls.signals`; the tiles are read from and saved to `controls.views` (the older `controls.view` is still read); the examples list curated views (Lorenz: 3D and time series; oscillators: time series and phase portrait); a log tile left of the rewind bar (the newest notice, warning or error on one line, dimmed after 8 s; "Log" opens every message, newest first) and a one-line model description with the full text on hover; `--view time|phase|3d`, `--window signals|equations` and `--screenshot` for headless checks.
- CLI: `simsandbox run FILE` writes every output as CSV, using the file's `run` section unless `--dt`/`--t-end` override it; `simsandbox lorenz` with `--method euler|rk4|dopri5`.
- 112 tests, including verification against analytic solutions and convergence order.

## Done recently

- 2026-10-17: automatic fusion (ADR 0023): fused groups in the master (`Model::fuse`, `GroupSolver`), `fusion` and `groups` in model files, "Solving" section in the app with an on/off switch; the oscillator example's energy now holds to 10⁻¹² over 100 s (was +11.5 %). Q17 (implicit and IMEX methods) raised.
- 2026-10-17: owner feedback: crash when a sweep reaches infinity fixed (all views skip non-finite values; picking a sweep parameter uses its slider range); wider Derived signals text boxes; messages in a log tile beside the rewind bar (2026-10-17, replacing a fixed-height status line); description on one line; heat rod length and `position` output, `plot.profile`, heat map `position` slot and `time_axis`. Q15 (multi-parameter sweeps) and Q16 (coupling error) raised.
- 2026-10-17: ADR 0022 (sweeps over two parameters) and ADR 0023 (automatic fusion with opt-out and explicit groups) accepted, resolving Q15 and Q16.
- 2026-10-17: views steps 5–6: grid and point-set shapes, `plot.heatmap`, `plot.scatter`, the `eq.pendulum` domain view; heat-rod, orbits and pendulum examples. The pluggable-views migration is complete.
- 2026-10-17: views step 4: derived signals (`controls.signals`, "Signals…" window); the oscillator example plots its total energy.
- 2026-10-17: typed models without time (ADR 0021): discrete maps, formula blocks, parameter sweeps; logistic-map and resonance examples.
- 2026-10-17: ADR 0017 accepted; ADRs 0018 (one x–y plot), 0019 (typed models without time) and 0020 (method choice only for approximations); `plot.time` and `plot.phase` merged into `plot.xy` with view presets; old files converted on load.
- 2026-10-17: symbolic equation entry: `simsandbox-expr`, `eq.equations`, `equations` in model files, equation editor in the app, predator–prey example; ADR 0017 (proposed). All Phase 0 roadmap items are done.
- 2026-10-17: ADR 0016 (models choose the views shown) and views step 3: several views at once in tiles, "More views…", curated example views.
- 2026-10-17: `trail_length` semantics changed at the owner's request: 0 shows no trail, -1 (any negative) the whole run, default -1.
- 2026-10-16: pluggable views, migration steps 1–2: `simsandbox-views` crate, view registry, `controls.views`, colour slot and `trail_length`.
- 2026-10-16: native Open, Save and Save as dialogs in the app (`rfd`).
- 2026-10-16: ADR 0015 (pluggable views, style options) and [visualization architecture](architecture/visualization.md).
- 2026-10-16: opening view saved in `controls` (ADR 0014); scroll-to-zoom in 2D plots; Lorenz example opens in 3D.
- 2026-10-16: phase-portrait and 3D trajectory views in the app.
- 2026-10-16: parameter panel, Restart and Save in the app; `run` section in model files (ADR 0013).
- 2026-10-16: desktop app shell (`crates/simsandbox-app`).
- 2026-10-16: model file format (registry, `simsandbox-file` crate, examples, `simsandbox run`).
- 2026-10-16: kernel prep for model files and the UI (parameter access; snapshots include parameters; suggested rather than enforced parameter ranges); ADRs 0007 (JSON model files), 0008 (egui + wgpu UI), 0009 (hobbyists first), 0010 (live loop pacing), 0011 (parameter entry) and 0012 (controls saved in model files).

## In progress

- Nothing.

## Next up

In priority order. Each item should end with tests and updated docs.

1. **Two-parameter sweeps** ([ADR 0022](decisions/0022-two-parameter-sweeps.md)): `controls.sweep` with two parameters, results as grid fields with parameter coordinates, heat map first, then a surface view, then contours.
2. **Plan Phase 1 with the owner:** order the [Phase 1 roadmap](roadmap.md#phase-1--composition) items (node canvas, Python API, subsystems, agent domain, direct feedthrough).

## Known limitations

- Fused equation groups require outputs that depend only on state and parameters (no direct feedthrough), so algebraic loops are not supported ([ADR 0002](decisions/0002-layered-block-contract.md)).
- Physical (acausal) ports are not implemented.
- The master uses a fixed step with no coupling-error estimate, iteration or rollback.
- `FusedSystem::rhs` and the master's fused-group right-hand side allocate per call; fine for small models, revisit before real-time use with large groups.
- Inside a fused group, inputs from blocks outside the group are held over each step (no extrapolation yet, ADR 0023). Changing the fusion switch in the app rebuilds the model and restarts the run.
- Views beyond `plot.xy`, `plot.3d`, `plot.heatmap`, `plot.profile`, `plot.scatter` and `eq.pendulum` are not built; heat maps place cells only along evenly spaced lines (2D grids are numbered by cell); derived signals cannot use grids or point sets; `plot.scatter` shows only the newest frame, without trails. Tiles are laid out in an automatic grid; they cannot be resized or reordered. The colour-blind-safe palette is fixed; viewer theme settings are not built ([ADR 0015](decisions/0015-pluggable-views.md)).
- The 3D view projects on the CPU and draws with the egui painter, at most 20 000 points per frame; it suits trajectories, not meshes or fields, which need wgpu paint callbacks ([ADR 0008](decisions/0008-native-egui-ui.md)).
- The app does not track unsaved changes: opening another model or closing the window discards them without asking.
- On Linux the file dialogs need the XDG desktop portal (standard on GNOME and KDE desktops); without it Open and Save as do nothing, and drag and drop or a command-line path still work.
- The simulation runs on the UI thread ([ADR 0010](decisions/0010-live-loop-pacing.md)); a heavy model slows the window's controls as well as the plot.
- Plotted series and rewind history are thinned (every other point dropped) beyond 100 000 points and 10 000 snapshots, so very long runs lose resolution in the past.
- A model built in code cannot be saved (Blocks do not know their type name); files are the source for saving.
- Only models with no states can be swept; sweeping models with states (bifurcation diagrams) is a later feature. Maps have no step-number variable `n` yet.
- Typed equations are dimensionless and have no inputs, so typed blocks cannot be wired to other blocks yet; there are no symbolic Jacobians yet ([ADR 0017](decisions/0017-typed-equations.md)). Applying edited equations restarts the run from t = 0.
- Integer method options are written as numbers with a decimal point (`"substeps": 4.0`).

## Blockers

- None.
