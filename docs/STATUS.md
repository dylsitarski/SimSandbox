# Status

_Last updated: 2026-10-15_

**Phase:** 0 · Foundations ([roadmap](roadmap.md#phase-0--foundations)). **Gate:** the Lorenz system runs live on the desktop with sliders. **Gate met:** no.

## What works

- Cargo workspace with three crates: `simsandbox-core`, `simsandbox-eq`, `simsandbox-cli`; CI on Windows, macOS and Linux.
- Block contract: required `Block` trait plus optional `Equations`, `Rollback`, `Derivatives`, `Realtime` capability traits (`crates/simsandbox-core/src/block.rs`).
- SI units with dimension checking and automatic conversion at connect time (`units.rs`).
- Model graph with unit-, shape- and direction-checked signal connections (`graph.rs`).
- Co-simulation master: fixed communication step, Jacobi exchange; kernel-owned state; snapshots and rewind; live parameter changes (`sim.rs`).
- Equation domain: Euler, RK4, Dormand–Prince 5(4), each with speed/accuracy labels; fusion of equation Blocks into one system (`crates/simsandbox-eq`).
- Example systems: Lorenz, coupled oscillator.
- `simsandbox lorenz` CLI writing CSV, with `--method euler|rk4|dopri5`.
- 13 tests, including verification against analytic solutions and convergence order.

## In progress

- Nothing.

## Next up

In priority order. Each item should end with tests and updated docs.

1. **Model file format.** Resolve [open question Q1](open-questions.md) with an ADR, then implement load/save for the existing systems.
2. **Desktop app shell.** New crate `simsandbox-app` (egui + wgpu, per [ADR 0003](decisions/0003-desktop-first.md)): window, run/pause/rewind controls, a live time-series plot.
3. **Auto-generated parameter panel.** Sliders built from `ParamSpec` (name, unit, range), wired to `Simulator::set_param`.
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
