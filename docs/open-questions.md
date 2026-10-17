# Open questions

_Last updated: 2026-10-17_

Decisions not yet made. When one is resolved, write an ADR, link it in the **Resolved** table and remove the row from **Open**.

## Open

| ID | Question | Options | Needed by | Notes |
| --- | --- | --- | --- | --- |
| Q5 | Algebraic loops in fused groups | Causalisation and tearing; Newton solve per RHS evaluation; forbid and warn | Phase 1 | Current v0 restriction: no direct feedthrough |
| Q15 | Sweeps over several parameters | (a) `controls.sweep` takes a list of parameters; two give a grid field over (p1, p2) shown by `plot.heatmap`, contours or a surface; (b) also allow sweeping models with states (each grid point a run to `t_end`, then a reduction such as final value or amplitude); (c) keep one parameter | Before bifurcation diagrams | Raised by the owner 2026-10-17. Cost grows as points^dims (100 × 100 = 10 000 evaluations, instant for formulas; runs with states need a background thread). Needs a contour/surface view and a 2D coordinate pair for grids |
| Q16 | Reducing coupling error between co-simulated Blocks | (a) Model files describe fused groups (one integrator, no coupling error); (b) the master extrapolates inputs over each step (first-order hold), error O(dt²) instead of O(dt); (c) iterate each step until the exchanged values settle (needs `Rollback`); (d) smaller communication step only | Phase 1 (with subsystems) | Raised by the owner 2026-10-17 about the oscillator example's energy rising: holding the neighbour's position fixed over a step delays the coupling force by about dt/2, which acts as negative damping on the mode where the masses move in opposite directions (energy grows at about κ·dt/2 per second there); measured +11.5 % at t = 100 with dt = 0.01, halving with dt |

## Resolved

| ID | Question | Resolution |
| --- | --- | --- |
| — | Rust core or Julia-first core | [ADR 0001](decisions/0001-rust-core.md) |
| — | Shape of the Block contract | [ADR 0002](decisions/0002-layered-block-contract.md) |
| — | Desktop-only or browser | [ADR 0003](decisions/0003-desktop-first.md) |
| — | "One model, three modes" or per-method classification | [ADR 0004](decisions/0004-method-classification.md) |
| — | Where project knowledge lives | [ADR 0005](decisions/0005-repository-as-source-of-truth.md) |
| — | License | [ADR 0006](decisions/0006-apache-2-license.md) |
| Q1 | Model file format | [ADR 0007](decisions/0007-json-model-files.md) |
| Q3 | Desktop UI approach | [ADR 0008](decisions/0008-native-egui-ui.md) |
| Q2 | First audience for the prototype | [ADR 0009](decisions/0009-hobbyists-first.md) |
| Q6 | Pacing of the live simulation loop | [ADR 0010](decisions/0010-live-loop-pacing.md) |
| Q7 | Where run settings live | [ADR 0013](decisions/0013-run-section.md) |
| Q8 | Save the default view in `controls` | [ADR 0014](decisions/0014-view-in-controls.md) |
| Q9 | Visualization architecture for many outputs and model-specific pictures | [ADR 0015](decisions/0015-pluggable-views.md) |
| Q10 | Which draw-style options to expose | [ADR 0015](decisions/0015-pluggable-views.md) |
| Q11 | Which views a model offers | [ADR 0016](decisions/0016-model-curated-views.md) |
| Q4 | Symbolic equation parsing | [ADR 0017](decisions/0017-typed-equations.md) |
| Q12 | Equations without time or states | [ADR 0019](decisions/0019-equations-without-time.md) |
| Q13 | Merge time-series and phase-portrait views | [ADR 0018](decisions/0018-one-xy-plot.md) |
| Q14 | Methods for operations inside equations | [ADR 0020](decisions/0020-methods-for-approximations.md) |
