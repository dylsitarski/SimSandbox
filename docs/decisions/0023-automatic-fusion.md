# 0023 — Automatic fusion of equation blocks, with opt-out and explicit groups

- **Status:** Accepted
- **Date:** 2026-10-17

## Context

[ADR 0002](0002-layered-block-contract.md) fuses blocks of the same domain into one system, and `EquationGroup` implements it, but model files cannot ask for it, so every block in a file is co-simulated. The coupled-oscillator example gains about 11 % energy by t = 100 with dt = 0.01: each block holds the other's position over a step, delaying the coupling by about dt/2, which acts as negative damping ([Q16](../open-questions.md#resolved)). Fused, the same model stays within about 2×10⁻⁹ of the exact solution (test `cosimulation_adds_coupling_error`). Hobbyists come first ([ADR 0009](0009-hobbyists-first.md)), so the accurate result should not require knowing what co-simulation is; the owner also wants control when it matters.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Fuse automatically, with an opt-out and optional explicit groups | Right answer by default; control when wanted | One method per fused group; loader must decide what fuses |
| Explicit groups only | Full control, nothing implicit | Users must know about coupling error to avoid it |
| Extrapolate inputs over each step (first-order hold) | Cheap; error O(dt²) instead of O(dt); works across domains | Still an error; can overshoot on stiff coupling |
| Iterate each step until exchanged values settle | Accurate for any blocks | Needs `Rollback`; several times the cost |

## Decision

- **Automatic fusion is the default.** When a model loads, connected blocks that are continuous equation blocks (they implement `Equations`), have no direct feedthrough and use **identical method settings** are fused into one system solved by that method. Blocks whose methods differ stay separate and are co-simulated; this is how a user keeps different methods. Maps and formula blocks are not fused.
- **Opt-out:** a model file may set `"fusion": "off"` (default `"auto"`), and the app offers the same switch; then only explicit groups are fused.
- **Explicit groups are optional:** `"groups": [{"blocks": ["a", "b"], "method": {...}}]` fuses exactly those blocks with that method, overriding their own methods, and they are left out of automatic fusion. Several groups may use different methods. One fused system always has one method; different methods inside one system (multirate or implicit–explicit schemes) are not planned.
- **Visible:** the app shows which blocks are solved together and with which method (e.g. "a, b solved together with RK4"); speed and accuracy labels are the fused group's.
- **Names do not change:** outputs and parameters keep their block names (`a.x`, `a.kappa`), so views, sliders and saving work the same; the file still lists the blocks separately.
- **Across domains and for blocks left separate,** input extrapolation (first-order hold) is added later as a master setting, off by default at first; the gap between held and extrapolated inputs becomes a coupling-error estimate for the accuracy label. Iterating each step waits for `Rollback` use cases. A smaller step remains the general fallback.

## Consequences

- The coupled-oscillator example becomes accurate without changes; its description is updated when this lands.
- The loader builds an `EquationGroup` per fused set; the simulator and the app must map member names to the fused block's `<member>.<name>` ports and parameters, and snapshots, rewind and `set_param` must keep working through that mapping.
- The no-direct-feedthrough restriction (Q5) decides what can fuse; blocks with feedthrough stay co-simulated.
- Docs to update when built: [block contract](../architecture/block-contract.md), model file schema, [STATUS](../STATUS.md).
