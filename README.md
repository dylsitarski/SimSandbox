# SimSandbox

A desktop environment for building, connecting and exploring simulations of all kinds — for fun, for learning and for research.

SimSandbox is a small, solver-agnostic kernel that only knows how to **connect, schedule and observe** models. Every kind of physics, math and ML lives behind plugins. Users build models from deliberately chosen methods, and every method is labelled for **speed** (real-time, responsive, batch) and **accuracy** (qualitative, quantitative, verified), so it is always clear what a model can be trusted for.

> **Status: Phase 0 (foundations).** The Block contract, units, the co-simulation master, a state store with rewind, and the first equation-domain methods exist and are tested. There is no desktop UI yet.

## Core ideas

- **Blocks.** Every simulation is wrapped as a Block with typed, unit-checked ports, named parameters (future sliders) and kernel-owned state.
- **Model definition vs method.** A Block separates *what* is simulated (equations, ports, parameters) from *how* it is computed (the chosen integrator or solver). Switching RK4 for an adaptive solver doesn't touch the model.
- **Layered coupling.** Blocks in the same domain are fused and solved together; different domains meet only through an FMI-style step interface run by a co-simulation master. See [ADR 0002](docs/decisions/0002-layered-block-contract.md).
- **Honest labels.** A model built from several methods is as slow as its slowest method and capped by its least accurate one.

Why fusion matters: in the test suite, two coupled oscillators solved as one fused system with RK4 stay within about 2×10⁻⁹ of the exact solution over 10 s. The same equations split into two co-simulated Blocks with a 0.01 s exchange step drift by about 1×10⁻² (`crates/simsandbox-eq/tests/verification.rs`).

## Repository layout

| Path | What it is |
| --- | --- |
| `crates/simsandbox-core` | Kernel: Block contract and capability traits, SI units, model graph, co-simulation master, snapshots |
| `crates/simsandbox-eq` | Equation domain: ODE systems, integrators (Euler, RK4, Dormand–Prince 5(4)), fusion of equation Blocks |
| `crates/simsandbox-cli` | `simsandbox` headless runner (CSV output) |
| `docs/decisions` | Architecture decision records |

## Getting started

Requires a stable Rust toolchain (1.86 or newer; `rustup` picks it up from `rust-toolchain.toml`).

```sh
cargo test --workspace
cargo run -p simsandbox-cli -- lorenz --t-end 20 --dt 0.01 --method rk4 > lorenz.csv
```

`--method` is one of `euler`, `rk4` or `dopri5`; the chosen method's speed and accuracy labels are printed to stderr.

## Roadmap

| Phase | Goal | Gate |
| --- | --- | --- |
| 0 · Foundations | One equation, live | Lorenz system runs live on the desktop with sliders |
| 1 · Composition | Connect and reuse | ODE + agents in one graph |
| 2 · Breadth | Many domains (MuJoCo, agents, ONNX, FMI 3.0, GPU fluids) | Verification suite passes |
| 3 · Research | Trustworthy results (CFD/FEM batch, sweeps, provenance, cluster runs) | Reproduce a published result |

Phase 0 still to do: model file format, desktop app shell (egui + wgpu), auto-generated sliders and plots, symbolic equation entry.

## Decisions

- [0001 — Rust core](docs/decisions/0001-rust-core.md)
- [0002 — Layered Block contract](docs/decisions/0002-layered-block-contract.md)
- [0003 — Desktop-first](docs/decisions/0003-desktop-first.md)
- [0004 — Method classification](docs/decisions/0004-method-classification.md)

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
