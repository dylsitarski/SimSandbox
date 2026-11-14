# 0090 — One engine for models that use models: the merged simulator

- **Status:** Accepted (the owner left the choice to the agent, 2026-11-14); not built yet
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

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none.
