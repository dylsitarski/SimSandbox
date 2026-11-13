# 0061 — Verified is earned per case, exact methods included

- **Status:** Accepted
- **Date:** 2026-10-28
- **Refines:** [ADR 0004](0004-method-classification.md) (method classification)

## Context

ADR 0004 defines the accuracy labels. Verified means "error estimated for this case; benchmarks pass", it applies to a specific case and is lost when the model changes. The [verification policy](../guides/verification-policy.md) adds: "Never declare `Verified`; it is earned per case by the planned verification tooling."

On 2026-10-28 the owner asked why the heat-rod examples show "Accuracy: Quantitative" now that the heat equation is verified against its exact solution ([ADR 0060](0060-arrays-in-typed-equations.md), step 3). That work verified the method (the stencil and RK4) on reference problems, not the run on screen: raising α past RK4's stability limit, editing the equations or coarsening the grid gives wrong numbers with the same method.

Checking this showed the code breaking the rule. Three method labels declared themselves Verified:
- `units.convert` (since 2026-10-23);
- the input blocks (since 2026-10-26);
- a constant-grass block in the grazing tests.

Each has no discretisation error, so it was taken as exact and therefore Verified. Maps and formula blocks, which are just as exact, were labelled Quantitative. The labels were inconsistent.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| **Exact methods are Quantitative too; Verified only from a case's convergence check** | One meaning for Verified: this case's error was checked; matches ADR 0004 and the policy as written | A model of only exact blocks shows Quantitative although nothing is approximated |
| Exact methods may declare Verified (amend ADR 0004) | Exact blocks show what they are | Verified then means two things (exact method, checked case); a model's combined label hides which |

## Decision

The owner chose to relabel and to plan the convergence check, on 2026-10-28.

- **No method declares Verified,** exact ones included. `units.convert` and the input blocks are labelled Quantitative, like maps and formulas. A comment at each says why.
- **Verified comes only from a check of the case:** the run is repeated at half the step (and, for grids, with twice the cells), and the change is shown as an error estimate. The label lasts until the model, its parameters or its settings change.
- **The convergence check is a roadmap item** in Phase 3 (Research), beside provenance reports. The heat-rod examples are its first cases.

## Consequences

- The toolbar of models with conversions or inputs may now show Quantitative where it showed Verified; no model was Verified as a whole before, since every one also had an integrator.
- [Method classification](../architecture/method-classification.md) lists exact methods as Quantitative.
- The convergence check needs its own design: what is refined (step, cells, tolerances), how the estimate is shown on plots and views, and how it is recorded with run files ([ADR 0059](0059-run-files.md)) and provenance.
