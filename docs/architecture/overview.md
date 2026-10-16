# Architecture overview

_Last updated: 2026-10-16_

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
| `crates/simsandbox-core` | Kernel | Exists | `block.rs` (contract, labels), `units.rs`, `graph.rs` (model graph), `sim.rs` (master, snapshots), `error.rs` |
| `crates/simsandbox-eq` | Backend: equations | Exists | `ode.rs` (`OdeSystem`, `OdeBlock`), `integrators.rs`, `group.rs` (fusion), `systems.rs` (examples) |
| `crates/simsandbox-cli` | Runtime: headless | Exists | `simsandbox` binary |
| `crates/simsandbox-app` | Runtime: desktop | Planned | eframe + egui + wgpu shell ([ADR 0008](../decisions/0008-native-egui-ui.md)) |
| `crates/simsandbox-py` | Interface: Python | Planned | PyO3 bindings |
| further domain crates | Backends | Planned | e.g. `simsandbox-rigid`, `simsandbox-agents`, `simsandbox-ml`, `simsandbox-fmi` |

Dependency rule: domain crates depend on `simsandbox-core`; the kernel depends on no domain crate. Runtimes and interfaces depend on whatever they host.

## Data flow of a run

1. Blocks are added to a `Model` with default parameters; connections are checked for direction, shape and unit dimension, and a conversion factor is stored.
2. `Simulator::new` checks every input is connected, lets each Block initialise its kernel-owned state, computes initial outputs and copies them to inputs.
3. `Simulator::step(dt)` calls `do_step` on every Block with the inputs from the previous exchange (Jacobi), then exchanges outputs to inputs again.
4. `Simulator::set_param` changes a parameter between steps; values must be finite and within the `ParamSpec` range. `Simulator::model()` exposes Block specs and current parameter values to interfaces.
5. Snapshots copy time, all states, outputs and parameter values; `restore` rewinds to one, so a rewind also undoes parameter changes made after the snapshot.

Same-domain fusion happens before the master sees the Blocks: e.g. `EquationGroup` builds a `FusedSystem`, which is wrapped in one `OdeBlock`. See [block contract](block-contract.md).

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
| Model files | JSON with a block-type registry ([ADR 0007](../decisions/0007-json-model-files.md)) | Diffable, shareable, AI-generatable |
| Distribution | Desktop installer, headless CLI, containers | Same model file everywhere |
| Sharing | Standalone player app and recorded runs | Recovers most of the value of link-sharing |
