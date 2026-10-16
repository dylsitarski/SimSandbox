# 0010 — Pace the live loop by wall-clock time

- **Status:** Accepted
- **Date:** 2026-10-16

## Context

The simulator advances in fixed communication steps of `dt` simulated seconds; the desktop app redraws about 60 times per wall-clock second, depending on the monitor. The app must decide how many steps to run per frame, which sets how fast a model appears to play ([open question Q6](../open-questions.md#resolved)).

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Fixed steps per frame | Simplest | Playback speed depends on the refresh rate (144 Hz runs 2.4× faster than 60 Hz); no notion of real time |
| Wall-clock pacing with a speed control and a per-frame budget | Same speed on every machine; natural speed control; slow models degrade to slow motion | Needs an accumulator and a budget |
| Simulation on its own thread | UI stays smooth under heavy models | Shared-state synchronisation; more complex than Phase 0 needs |

## Decision

- Each frame, add `elapsed wall time × speed` to a time accumulator, then call `Simulator::step(dt)` while the accumulator holds at least `dt`, subtracting `dt` each time.
- The user sets `speed` (simulated seconds per wall-clock second); pause is speed 0.
- Stepping stops when its share of the frame is used (start with 8 ms, tuned later). Any backlog left over is dropped rather than carried forward, so a slow model runs in slow motion instead of falling ever further behind and freezing the UI.
- When the budget cuts stepping short, the app shows the achieved speed next to the requested one.
- In Phase 0 the simulation runs on the UI thread.

## Consequences

- Playback is the same on every display, and the speed control doubles as a fast-forward for slow-moving models.
- A model whose method is not real-time still runs, visibly slower than requested, consistent with the speed labels ([ADR 0004](0004-method-classification.md)).
- Moving the simulation to a worker thread later needs a new ADR; the pacing rule stays the same there.
