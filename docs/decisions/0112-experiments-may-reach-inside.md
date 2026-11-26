# 0112 — Experiments may set values inside a used model, marked

- **Status:** Accepted (owner, 2026-11-26)
- **Date:** 2026-11-26
- **Answers:** [Q66](../open-questions.md)
- **Amends:** [ADR 0110](0110-encapsulated-models.md) point 1 (what goes through a model's interface)

## Context

[ADR 0110](0110-encapsulated-models.md) made a model's interface its contract: a use changes its model, and anything that feeds a result reads from it, only through that interface. Observers may still read inside. Building its step 1 found three more things that set values inside a used model by path (`plant/car.m`):

- a study's points (`sample.params`, [ADR 0070](0070-studies-are-blocks.md));
- the run's schedule (`run.inputs`, [ADR 0058](0058-inputs-live-scheduled-replayed.md));
- pinned controls (`controls.params`).

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| **Allowed, marked** | No housekeeping before a sweep or a poke; varying any value is what sensitivity studies are for; the card still shows the dependence | Two rules: uses go through the interface, experiments need not |
| Declared parameters only | One rule for everything | Every value to sweep or poke must be exposed first, level by level |
| Allowed, unmarked (as before) | No work | Nothing shows what depends on a model's inside |

## Decision

1. **Experiments are the experimenter's hands, like observers.** A study's points, the run's schedule and pinned controls may set any value inside a used model by path.
2. **They are marked.** The card lists each under `reaching_inside` with `sets: true` and where it is written. The Overview and Markdown say "sets … inside …".
3. **A path that breaks is an error at its reference**, as for observers.
4. **Exposing is one step from the mark.** The value becomes a declared parameter of the used model (`expose`), and the experiment sets that. In the app this comes with ADR 0110 step 3, the Inspector's table of uses.
5. **Uses stay strict**, as ADR 0110 decided: a use's `params`, wires and links.

## Consequences

- Nothing in older files converts for experiments; they run as before.
- The difference between a use and an experiment is the line between building a model and trying it.

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none; experiments join observers under "reaching inside".

## Built

- 2026-11-26:
  - `card_at` marks the schedule's and pinned controls' paths into a use at the top level, and studies' points into a use inside the studied model, as `sets`. Markdown and the Overview say so.
  - Tested in `card.rs`.
- 2026-11-26: **Expose** from the Overview's mark and from the Inspector (`Edit::ExposeInside`, [ADR 0110](0110-encapsulated-models.md) step 3) rewrites the schedule's and pins' paths to the parameter's name.
