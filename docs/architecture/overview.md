# Architecture overview

_Last updated: 2026-10-23_

SimSandbox has five layers. Only the kernel and the model graph are new code; interfaces and backends are replaceable around them. The kernel never contains physics: it schedules Blocks, checks units, stores state and records provenance, which is what lets a CFD solver, an ODE and a neural network sit in one graph.

```mermaid
flowchart TB
  subgraph I["Interfaces"]
    canvas["Visual canvas"] --- py["Python / Jupyter"] --- ai["AI assistant"] --- views["Live views"]
  end
  subgraph M["Model graph"]
    graph["Blocks = model definitions + chosen methods,<br/>unit-aware ports, parameters, state; one plain-text file"]
  end
  subgraph K["Kernel · Rust, no physics"]
    sched["Co-simulation master"] --- units["Units and types"] --- state["State store"] --- prov["Provenance"]
  end
  subgraph B["Backend plugins · wrap mature solvers"]
    eq["Equations and ODEs"] --- phys["Physical networks"] --- rigid["Rigid body"] --- cfd["CFD and FEM"]
    agents["Agents and events"] --- ml["ML models"] --- fmi["FMI and external"]
  end
  subgraph R["Runtimes"]
    desk["Desktop app"] --- cli["Headless CLI"] --- cluster["Cluster or cloud"]
  end
  I -->|edit and drive| M -->|compiled into a schedule| K -->|step and exchange data| B -->|runs on| R
```

## Crate map

| Crate | Layer | Status | Contents |
| --- | --- | --- | --- |
| `crates/simsandbox-core` | Kernel | Exists | `block.rs` (contract, labels), `units.rs`, `graph.rs` (model graph), `sim.rs` (master, snapshots), `table.rs` (populations of rows, ADR 0034), `random.rs` (counter-based random numbers), `sampling.rs` (sweep designs), `registry.rs` (block types by name), `error.rs` |
| `crates/simsandbox-expr` | Backend support: expressions | Exists | Parser and evaluator for typed equations, parameter formulas and derived signals ([ADR 0017](../decisions/0017-typed-equations.md)); `units.rs`: the dimension of an expression and evaluation in SI, shared by equations and formulas ([ADR 0045](../decisions/0045-units-in-typed-equations.md), [ADR 0046](../decisions/0046-units-of-declared-parameters.md)); depends on `simsandbox-core` for units |
| `crates/simsandbox-eq` | Backend: equations | Exists | `ode.rs` (`OdeSystem`, `OdeBlock`), `integrators.rs`, `group.rs` (fusion), `systems.rs` (examples), `symbolic.rs` (typed equations, `eq.equations`), `registry.rs` (file types) |
| `crates/simsandbox-file` | Model graph: files | Exists | JSON model files, `controls`, JSON Schema ([ADR 0007](../decisions/0007-json-model-files.md)); `subsystems.rs`: subsystems and declared parameters, flattened on load, formulas as a `ParamPlan` ([ADR 0043](../decisions/0043-subsystems.md)); depends on `serde`, `serde_json`, `simsandbox-expr` |
| `crates/simsandbox-cli` | Runtime: headless | Exists | `simsandbox` binary: `run`, `check`, `describe`, `schema`, `render` ([guide](../guides/headless.md)) |
| `crates/simsandbox-session` | Interfaces: shared | Exists | The open model as every interface uses it ([ADR 0041](../decisions/0041-headless-interface.md)): `session.rs` (simulator, shared history, rewind, sweeps evaluated in slices), `filters.rs`, `views.rs` (view tiles from `controls.views`), `diagram.rs` (the model graph, its automatic layout and the canvas's edits as file changes) and `canvas.rs` (the node canvas drawn and edited with egui, [ADR 0042](../decisions/0042-node-canvas.md)), `describe.rs` (type catalogue and model checks as JSON), `run.rs` (headless runs), `render.rs` and `raster.rs` (views to PNG on the CPU), `examples.rs` |
| `crates/simsandbox-app` | Runtime: desktop | Exists | eframe + egui + wgpu ([ADR 0008](../decisions/0008-native-egui-ui.md)): `live.rs` (the open model on its worker thread: commands, published model and status, [ADR 0035](../decisions/0035-simulation-worker-thread.md)), `pacing.rs` (wall-clock pacing, [ADR 0010](../decisions/0010-live-loop-pacing.md)), `app.rs` (window and views), `diagram.rs` (the Diagram tab: drafts that do not load yet, edits applied by rebuilding); the open model itself comes from `simsandbox-session` |
| `crates/simsandbox-agents` | Backend: agents | Exists | Agent block types on kernel tables ([ADR 0034](../decisions/0034-agent-domain.md)): `update.rs` (update orders and sharing), `grazers.rs` (`agents.grazers`) |
| `crates/simsandbox-views` | Interface: views | Exists | History of recorded signals, view contract and registry, built-in views, 3D projection ([visualization](visualization.md)) |
| `python/` (`simsandbox-py`) | Interface: Python | Exists | The `simsandbox` Python package: PyO3 extension over `simsandbox-session` plus a Python layer (`Model`, `Results`); its own Cargo workspace, built with maturin |
| further domain crates | Backends | Planned | e.g. `simsandbox-rigid`, `simsandbox-ml`, `simsandbox-fmi` |

Dependency rule: domain crates depend on `simsandbox-core`; the kernel depends on no domain crate. Runtimes and interfaces depend on whatever they host.

## Data flow of a run

1. A model file is parsed, its subsystems are expanded into their blocks (`instance/inner`) and parameter formulas evaluated ([ADR 0043](../decisions/0043-subsystems.md)), and each block is built by name from a `Registry` that domain crates fill (`simsandbox_file::load`), or Blocks are added in code. Blocks start with default parameters, then the file's values are applied; connections are checked for direction, shape and unit (both ends the same unit; conversions are `units.convert` blocks, [ADR 0045](../decisions/0045-units-in-typed-equations.md)).
2. `Simulator::new` checks every input is connected, lets each Block initialise its kernel-owned state, computes initial outputs and copies them to inputs.
3. `Simulator::step(dt)` calls `do_step` on every Block with the inputs from the previous exchange (Jacobi), then exchanges outputs to inputs again.
4. `Simulator::set_param` changes a parameter between steps; any finite value is accepted; the `ParamSpec` range is only a suggestion ([ADR 0011](../decisions/0011-parameter-entry.md)). `Simulator::model()` exposes Block specs and current parameter values to interfaces.
5. `Simulator::restart` re-initialises every Block from the current parameter values, so changed initial conditions take effect; a Block with a saved start (`Model::set_start`, a model file's `start` entry) then takes that state and those table rows instead ([ADR 0042](../decisions/0042-node-canvas.md)).
6. Snapshots copy time, all states, outputs and parameter values; `restore` rewinds to one, so a rewind also undoes parameter changes made after the snapshot.

Same-domain fusion: the master advances fused groups of equation Blocks as one system with one method; model files form them automatically ([ADR 0023](../decisions/0023-automatic-fusion.md)). In code, `EquationGroup` can also build a composite `FusedSystem` wrapped in one `OdeBlock`. See [block contract](block-contract.md).

## Tech stack

| Layer | Choice | Why |
| --- | --- | --- |
| Kernel | Rust, native ([ADR 0001](../decisions/0001-rust-core.md)) | Predictable real-time performance with no GC pauses; one binary per OS |
| Desktop UI | egui (eframe) for panels and node editor, wgpu for viewports ([ADR 0008](../decisions/0008-native-egui-ui.md)) | One process with direct GPU access |
| GPU | wgpu (Vulkan, Metal, DX12); CUDA inside specific backends | Full desktop GPU |
| Scripting and research API | Python via PyO3 | Where scientists and ML users already are |
| Equation and DAE backend | Rust-native ODE path now; Julia (ModelingToolkit, DifferentialEquations.jl) as an optional out-of-process plugin | Real-time stays in Rust; heavy DAE work can use Julia |
| Heavy solvers | OpenFOAM, FEniCSx, MuJoCo as local binaries or containers | The desktop app launches and manages them |
| Interop | FMI 3.0, ONNX, gRPC | Standards keep the ecosystem open |
| Model files | JSON with a block-type registry, a `run` section and an optional `controls` section ([ADR 0007](../decisions/0007-json-model-files.md), [ADR 0012](../decisions/0012-controls-in-model-files.md), [ADR 0013](../decisions/0013-run-section.md)) | Diffable, shareable, AI-generatable |
| Distribution | Desktop installer, headless CLI, containers | Same model file everywhere |
| Sharing | Standalone player app and recorded runs | Recovers most of the value of link-sharing |
