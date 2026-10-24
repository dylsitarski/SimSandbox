# 0009 — Hobbyists are the first audience

- **Status:** Accepted
- **Date:** 2026-10-16

## Context

The prototype needs one audience to shape its first examples, UI defaults and documentation ([open question Q2](../open-questions.md#resolved)). Candidates were students, hobbyists and researchers in the owner's field. The [vision](../vision.md) serves all three pillars eventually; this decision only sets the order.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Students | Clear learning goals; course-shaped examples | Pulls towards curriculum content and classroom distribution before the tool is stable |
| Hobbyists | Matches the owner's own current use; rewards the fun and learning pillars that Phase 0 already targets; tolerant of an early tool | Less pressure to prove research-grade accuracy early |
| Researchers | Strongest test of the research pillar | Needs verified methods, provenance and heavy solvers that arrive in Phases 2–3 |

## Decision

Hobbyists are the first audience: curious people who want to build and play with simulations on their own machine, with some maths but no requirement for publication-grade results. The owner chose this because it matches their own purposes and sits between the other two audiences in focus.

## Consequences

- First examples are playful and visual (Lorenz, pendulums, predator–prey, coupled oscillators) rather than course exercises or published benchmarks.
- UI defaults favour immediacy: a model runs as soon as it opens, sliders and plots appear without setup, and parameter descriptions use plain language with units shown.
- Speed and accuracy labels stay visible, so hobbyists learn what a result can be trusted for; the [verification policy](../guides/verification-policy.md) is unchanged.
- Students and researchers remain target audiences for later phases; revisit with a new ADR when Phase 1 or Phase 3 starts.
