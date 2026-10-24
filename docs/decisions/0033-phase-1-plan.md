# 0033 — Phase 1 order and scope

- **Status:** Superseded by 0036
- **Date:** 2026-10-19

## Context

Phase 0 is done ([roadmap](../roadmap.md)). The Phase 1 gate is an ODE model and an agent model running in one graph. Of the eight Phase 1 roadmap items, pluggable views and pause, rewind and branch were built during Phase 0, and shareable model files are mostly built. The remaining items depend on each other: subsystems need the node canvas to be useful, and the agent domain and direct feedthrough both change the Block contract. The owner chose the order and scope on 2026-10-19.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Agents first, then feedthrough, canvas, subsystems, Python | Meets the gate early through a model file; tests the Block contract with a second, non-equation domain before more UI rests on it; the contract changes once while there are two domains | The most visible change (the canvas) comes third |
| Canvas first | Most visible improvement first | Contract tested late; canvas may need rework after agents and feedthrough |
| Interleave | Spreads visible progress | Contract changes in two separate rounds |

## Decision

- **Order:** (1) agent domain with a first agent model, (2) direct feedthrough and algebraic loops in fused groups ([Q5](../open-questions.md)), (3) visual node canvas, (4) subsystems collapsed into reusable Blocks, (5) Python API, (6) the rest of shareable model files (asking before discarding unsaved changes; a stated rule for converting older files).
- **First agent model: grazing.** Grass regrows per cell of a grid by an ODE (logistic growth); sheep agents wander, eat and reproduce; coupled both ways, shown as a point set over a heat map with population counts. It is the gate's demonstration.
- **Python API scope:** headless scripting only: load or build models, run and sweep them, and read results as numpy arrays. Blocks written in Python are out of Phase 1.
- **Threading:** the simulation stays on the UI thread ([ADR 0010](0010-live-loop-pacing.md)) until the grazing model is measured; if it makes the window sluggish, move the simulation to a worker thread with a new ADR.
- Pluggable views and pause, rewind and branch count as done for Phase 1.

## Consequences

- The agent domain gets its own ADR (storage of agents, ECS or plain arrays, stepping and seeding, how populations become outputs) before code.
- The roadmap lists Phase 1 in this order; STATUS takes its **Next up** from it.
- Feedthrough follows agents so that both contract changes land together.
