# 0072 — Models with nothing in time are evaluated, not run

- **Status:** Accepted
- **Date:** 2026-11-05
- **Amends:** [ADR 0071](0071-sweeps-are-studies.md) decision 2 (a model with no states and no study runs in time)

## Context

Since ADR 0071 a model with no states runs in time in the app, like any other: the clock advances, the same values are recorded at every step, and Run, Pause, the step, the speed and the rewind bar do nothing useful. The resonance examples, the tuning example and any model made of formulas and studies are such models. Headless, `run` already evaluates a model whose only runs are its studies once and writes a row per run (ADR 0070 step 2). On 2026-11-05 the owner asked why studies sweeping a parameter run in time.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| **Evaluate, not run: once, and again when a value it reads changes** | Nothing pretends to change over time; the toolbar shows only what applies; a study runs once per change | Two kinds of model in the app, told apart by what they hold |
| Run in time as now | One kind of model | Steps that change nothing; time and rewind that mean nothing |
| Run in time but record only changes | Fewer samples | Still a clock and rewind for nothing |

## Decision

1. A model has **nothing in time** when no block has states or tables or reads recorded data (`Block::instants`), the model has no clocks (no uses on clocks of their own, no data times), and the file schedules no inputs (`run.inputs`). Studies, formulas, input blocks and unit conversions are all in this class.
2. **In the app** such a model is **evaluated, not run**: once when opened and again whenever a value it reads changes (a parameter, an input, a slider, a pointer). Time stays at 0, the history holds one sample, the newest evaluation. The toolbar shows neither Run/Pause, the time, the step nor the speed, and there is no rewind bar; it says the model is evaluated. The worker evaluates it, as it runs studies (ADR 0071 step 2): with progress, and a newer change gives an evaluation in progress up and starts it again.
3. **Headless** is unchanged: a model whose only runs are its studies is evaluated once and written as a row per run; given `--t-end` or `--dt` any model runs in time.
4. A model that gains something in time (an edit adding a state, a clock, data or a schedule) runs in time again when rebuilt.

## Consequences

- Views of such a model show its newest values: profiles, heat maps and cell-by-cell plots already do; a view of plain numbers follows (ADR 0071 follow-up, item 2 of 2026-11-05).
- A change at t = 0 no longer restarts the model inside the command for these models; the worker evaluates it, so it can be given up.

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none; "evaluated, not run" is how the app treats a model that has nothing in time, a property it already had headless.
