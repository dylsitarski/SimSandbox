# 0090 — One engine for models that use models: the merged simulator

- **Status:** Accepted (the owner left the choice to the agent, 2026-11-14); points 1 and 2 built 2026-11-14
- **Date:** 2026-11-14
- **Builds on:** [ADR 0066](0066-models-nest-as-blocks.md), [ADR 0088](0088-activation.md); resolves [Q58](../open-questions.md); [review](../research/2026-11-14-code-review.md) W7

## Context

A model that uses models on their own clocks runs two ways. `simsandbox run` and Python's `run` use the parts runner (`simsandbox-session/src/compose.rs`, `run_in_parts`, about 1 200 lines): one `Simulator` per clock domain, exact ticks, probe links, threads. The app, `Session`, studies, `render` and `start-from` use the merged simulator (`simsandbox_file::compose::load_merged`): one `Simulator` with clocks. Equivalence rests on one test helper called five times. Every feature of nested models must be built and verified twice; activation (ADR 0088) lives in the kernel's `Simulator::gate` and the parts runner has no code for it.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| A. Merged only | The engine the app needs anyway (rewind, snapshots, live edits); already has activation; studies and render use it; removes about 1 200 lines and the double load of a composition | Headless loses the parts runner's run-ahead for models linked one way (measured 2026-11-06: a chain of five 0.61 → 0.24 s, a driver and two springs 0.30 → 0.18 s) until the merged engine gains it |
| B. Parts only | Fastest headless | The app cannot use it: no single state to snapshot, rewind or edit live |
| C. Keep both, compared over every example | No speed lost | Twice the work for every feature, for ever; the comparison only covers what examples exercise |

## Decision

A. The merged simulator is the one engine.

1. `simsandbox run` and Python's `run` of a composition go through the merged simulator, writing the same output (results by model, `--model`, `--signals`, `--every`).
2. Until the parts runner is deleted, a test compares both engines bit for bit over every example and fixture that uses models; a difference found is fixed in the merged engine or recorded as a deliberate change in the changelog.
3. The parts runner is then deleted, with `load_composition`'s separate domains where nothing else needs them.
4. Run-ahead for parts linked one way is added to the merged engine only when a measured model needs it.

## Consequences

- One place for clocks, links, activation and events within a step.
- Headless runs of compositions linked one way may be 1.5–2.5× slower until point 4; models that feed each other already meet at every instant in both engines.
- Loading a composition once instead of twice (review, section 4).
- Documents to update when built: [architecture overview](../architecture/overview.md) (data flow, point 9), [headless guide](../guides/headless.md), [performance](../design/performance.md).

## As built (2026-11-14, points 1 and 2)

- `simsandbox run` and Python's `run` of a model that uses models step the merged simulator (`simsandbox-session/src/compose/merged.rs`) and write the same output as before. The runner in parts (`compose/parts.rs`) is compiled for tests only, where every fixture and example that uses models runs on both engines and is compared bit for bit: times, values, populations, labels, notes.
- **A correction to the Context above:** activation did work in the runner in parts, each part being a `Simulator` with the kernel's gate. It was not a gap in behaviour.
- **The engines differ in three cases**, all recorded in [ADR 0066](0066-models-nest-as-blocks.md) ("Difference from the headless runner") and each now held by a test. Headless runs give the app's results in them from now on:
  1. A loop of links through outputs between models on their own clocks is solved within the instant, where the runner in parts took it a step late and warned.
  2. The same between two models on new data.
  3. A model on new data that reads `t`, read by a faster model, is evaluated at every instant, not only at deliveries.
- Every example gives the same bytes headless (two use models on other clocks; neither has such a loop).
- **Speed**, best of three: models linked one way are slower as expected (a chain of five 0.045 → 0.114 s, one feeding two 0.033 → 0.068 s); models feeding each other and the examples within 0.01 s of before. Point 4 (run-ahead in the merged engine) waits for a model that needs it.
- Left: point 3, deleting the runner in parts, when the owner is content with the comparison.

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none.
