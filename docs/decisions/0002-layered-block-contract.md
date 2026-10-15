# 0002 — Layered Block contract

- **Status:** accepted
- **Date:** 2026-10-15

## Context

The Block contract decides how simulations exchange data and advance in time, and therefore how accurate coupled models can be and which kinds of simulation fit at all. SimSandbox must host equation models, physical networks, rigid bodies, agents, CFD/FEM and ML side by side.

## Options considered

| Option | Why it was not chosen as the contract | Where it lives on |
| --- | --- | --- |
| Step-based co-simulation only (FMI style) | Coupling error and instability for tightly coupled systems; no physical networks across Blocks | The cross-domain layer |
| Equation-based only (Modelica style) | Cannot express CFD codes, agents, ML or external binaries; would mean building a Modelica-class compiler | The equation domain |
| DEVS | Weak for stiff ODEs, PDEs and fields; niche ecosystem | Event semantics, via FMI 3.0 clocks |
| Fixed-tick dataflow (game engine / ECS) | Inaccurate for stiff and multi-rate systems | The agent domain and real-time methods |

## Decision

A layered hybrid:

- Every Block declares a **domain**. Blocks in the same domain are **fused** using that domain's native approach (equation Blocks become one ODE/DAE system with one integrator; agents share one ECS world; rigid bodies share one scene).
- Different domains meet only through an **FMI-style step interface** (set inputs, `do_step(dt)`, get outputs, save/restore state) coordinated by a **co-simulation master**.
- In Rust: one required `Block` trait plus optional capability traits — `Equations` (fusable), `Rollback`, `Derivatives`, `Realtime` — exposed through `as_*` accessors.
- The kernel owns native Block state as flat `f64` slices, so snapshots, rewind and saving work uniformly.
- Time is continuous with events and clocks (as in FMI 3.0); fixed-step, discrete-event and stateless are special cases.
- Out-of-process Blocks implement the same traits through a gRPC adapter.

## Consequences

- Tightly coupled equations have no coupling error. The test `cosimulation_adds_coupling_error` shows the gap: about 2×10⁻⁹ fused versus about 1×10⁻² co-simulated for two coupled oscillators.
- Cross-domain connections still carry coupling error; the UI must flag them and estimate it.
- v0 restriction: equation-Block outputs may depend on state and parameters only (no direct feedthrough), which rules out algebraic loops inside a fused group. Lifting it requires an algebraic-loop solver or causalisation.
- Acausal (physical) ports are deferred until the equation-domain compiler exists.
