# 0025 — View settings that depend on the index

- **Status:** Accepted
- **Date:** 2026-10-18

## Context

Views were written when every history was indexed by time. [ADR 0021](0021-maps-formulas-and-sweeps.md) let a sweep's parameter stand in for `t`, so settings defined in seconds now act on a parameter: `trail_length` trims a resonance curve to the last stretch of `w`. Other settings appear where they do nothing (`time_axis` and `position` for 2D grids). The view contract has no way to say when a slot or setting applies ([Q19](../open-questions.md#resolved)).

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| The history states its index; slots and style properties declare where they apply | Settings make sense wherever shown; one mechanism for every view | Contract change for every view type |
| Explain in each setting's description | No code | Users still see settings that do nothing or something odd |

## Decision

- A history states its **index**: time (seconds), or a sample number ([ADR 0028](0028-sweeps-as-samples.md)). Views receive it with what the model offers.
- **Slots and style properties declare when they apply,** as a condition on the index and on what is bound (e.g. the shape of a bound field). The side panel shows only settings that apply; values of hidden settings are kept and saved, so switching back restores them.
- A setting's **unit** may come from the index: `trail_length` is in seconds and applies only to time; in a sweep the x–y plot has no trail and shows every sample.
- New settings must state where they apply when they are added.

## Consequences

- `StyleProperty` and `Slot` gain an applicability condition; `ViewInput` and `Available` carry the index.
- The rule in [ADR 0015](0015-pluggable-views.md) for adding options still holds; this adds where they apply.
