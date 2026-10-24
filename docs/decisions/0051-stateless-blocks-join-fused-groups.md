# 0051 — Stateless equation blocks join fused groups

- **Status:** Accepted
- **Date:** 2026-10-23

## Context

Automatic fusion ([ADR 0023](0023-automatic-fusion.md)) solves connected continuous equation blocks with the same method as one system. A stateless block between two of them, such as a formula block or a `units.convert` ([ADR 0045](0045-units-in-typed-equations.md)), was not a candidate: it has no states and its method (`evaluate`, `exact`) is not theirs. It therefore split the chain, and its neighbours were co-simulated with a one-step coupling lag. A filter promoted between a car and its controller ([ADR 0050](0050-data-layer-and-canvas-undo.md)) changed the run by 0.2 %, and a unit conversion between two springs did the same. The owner asked (2026-10-23) why such a block separates the solve rather than joining it. Nothing requires it to: a fused group already evaluates its members' direct-feedthrough outputs at every right-hand-side evaluation, in dependency order and with loops solved ([ADR 0040](0040-feedthrough-and-algebraic-loops.md)).

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Keep the rule; tell people to switch fusion off or group explicitly | No change | A conversion or formula silently costs accuracy, which makes explicit conversion ([ADR 0045](0045-units-in-typed-equations.md)) a penalty |
| **Stateless equation blocks join the group they connect** | Formulas and conversions are free inside a solve; the drive of a formula source is evaluated at every stage | Groups contain blocks without states |

## Decision

- A block can join a fused group if it implements `Equations` and is continuous or stateless (`Model::is_fusable`). Stateless ones (`Model::is_algebraic`: formula blocks of `eq.equations`, `units.convert`) have no states and are evaluated inside the group's right-hand side like any feedthrough member.
- Automatic fusion unions connected candidates of one domain. A stateless block takes the method of the set it joins and never merges two sets whose blocks with states have different methods. A set is fused if it has two or more blocks with states; its method is theirs.
- An explicit group (`groups`) may include stateless blocks but needs at least one with states.
- Maps (`x[n+1]`) and blocks without `Equations` are unchanged.

## Consequences

- Promoting a filter between fused blocks leaves the run bit-identical (tested on the cruise control), and conversions cost nothing inside a solve.
- Models where a formula or conversion fed a fused block now evaluate it at every stage instead of holding it over the step, which is more accurate; results of such models change slightly (the units example's shaker now drives the springs inside each RK4 step).
- ADR 0050's note that a promoted block separates fused neighbours no longer holds.
