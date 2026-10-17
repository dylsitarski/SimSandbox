# 0021 — Maps, formula blocks and parameter sweeps

- **Status:** Accepted
- **Date:** 2026-10-17

## Context

[ADR 0019](0019-equations-without-time.md) decided to support typed formulas (no states) and discrete maps first, and left how they run, how formulas are explored and how both are stored to be designed with them.

## Decision

- **Maps:** `x[n+1] = …` makes `x` a map state. On the right, `x[n]` and `x` both mean the current value; all states update together. A block is either continuous (`dx/dt`) or a map, never both. A map is iterated once per communication step, so `run.dt` sets how much `t` advances per iteration (the logistic-map example uses `dt = 1`, so `t` counts iterations). `n` is reserved in maps; the step number is not available as a variable yet.
- **Formulas:** a block with only `name = …` lines has no states and is evaluated from its parameters (and `t`).
- **Methods:** maps use the method `iterate` and formulas `evaluate`, with no options; neither approximates anything, so there is nothing to choose ([ADR 0020](0020-methods-for-approximations.md)). They are labelled real-time and quantitative (no discretisation error, but not benchmarked case by case). A mismatched method is a load error at `blocks[i].method`; the equation editor picks the right one.
- **Sweeps:** a model in which no block has states is swept instead of run. The app evaluates its outputs at `points` values of one parameter across a range and shows them against that parameter; any slider change recomputes at once. The sweep is saved as `controls.sweep = {"param": "block.param", "range": [a, b], "points": n}` (presentation, like the rest of `controls`, [ADR 0012](0012-controls-in-model-files.md)); without it the first parameter is swept over its slider or suggested range.
- **The index axis:** recorded signals are indexed by time or, in a sweep, by the swept parameter. In view bindings `t` always means this index ([ADR 0015](0015-pluggable-views.md)), and the app shows the swept parameter's name in its place.

## Consequences

- Sweeping models that have states (rerunning per value, e.g. a bifurcation diagram) is a separate, later feature (roadmap Phase 3, parameter sweeps).
- Mixing maps and continuous blocks in one model works step by step: each communication step iterates every map once.
