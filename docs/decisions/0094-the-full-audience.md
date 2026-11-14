# 0094 — The audience is hobbyists, researchers and engineers together

- **Status:** Accepted (owner, 2026-11-14)
- **Date:** 2026-11-14
- **Supersedes:** [ADR 0009](0009-hobbyists-first.md); resolves [Q61](../open-questions.md)

## Context

[ADR 0009](0009-hobbyists-first.md) (2026-10-16) put hobbyists first, to shape the prototype's first examples and defaults, and asked for a new ADR when later phases began. Since then typed equations became the primary way to build models ([ADR 0091](0091-equations-first-hosting-for-existing-codes.md)), and the work serves research as much as play: implicit methods on large grids, meshes, studies and fitting, exact replay. The [review of 2026-11-14](../research/2026-11-14-code-review.md) asked which audience is true now. The owner (2026-11-14): "The focus is now on the full audience: hobbyists, researchers, and perhaps now even engineers."

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Hobbyists first (ADR 0009) | One audience to design for | No longer describes the work or the owner's aim |
| **Hobbyists and researchers together, engineers where the same features serve them** | Matches what is built; the three pillars already map to them (fun, learning, research) | No audience breaks a tie; every feature must stay approachable and defensible at once |
| Researchers first | Sharpest accuracy pressure | Gives up the immediacy the app is built around |

## Decision

SimSandbox is for the full audience: hobbyists, researchers, and engineers where the same capabilities serve them. No audience comes first.

- Priorities are set by the roadmap's gates and by models that need a capability, not by an audience.
- A feature is not finished for one audience only: it opens and runs at once with plain-language defaults (hobbyists), and it is labelled, verified and reproducible (researchers, engineers).
- Nothing is planned for engineers alone yet (certified methods, standards, reports); such work needs its own decision.

## Consequences

- ADR 0009's consequences for defaults stay as practice: a model runs as soon as it opens; labels stay visible.
- Examples span play and research; the [verification policy](../guides/verification-policy.md) and honest labels carry more weight, so the review's W1 (a wrong result under a `Quantitative` label) and W5 come first among its work items.
- [Vision](../vision.md) updated (the audience line).

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none.
