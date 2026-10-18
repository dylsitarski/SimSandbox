# Open questions

_Last updated: 2026-10-18_

Decisions not yet made. When one is resolved, write an ADR, link it in the **Resolved** table and remove the row from **Open**.

## Open

| ID | Question | Options | Needed by | Notes |
| --- | --- | --- | --- | --- |
| Q5 | Algebraic loops in fused groups | Causalisation and tearing; Newton solve per RHS evaluation; forbid and warn | Phase 1 | Current v0 restriction: no direct feedthrough |
| Q17 | Implicit and implicit–explicit (IMEX) methods for stiff systems | (a) An implicit method (e.g. backward differentiation or an L-stable Runge–Kutta) for whole systems, with Jacobians by finite differences first; (b) named IMEX methods that treat a declared stiff part implicitly; (c) explicit methods with substeps only | Before stiff fields or large fused groups | Raised 2026-10-17 with ADR 0023, which rules out free per-block method mixing inside one fused system: arbitrary pairs of methods fall to first order. First case: `eq.heat_rod`, whose explicit step limit (≈0.7 h²/α for RK4) forces 10 substeps in the example |
| Q18 | Heat map axes and coordinates | (a) Replace `position` and `time_axis` with `x` and `y` slots that each take the index (`t`) or a coordinate field, so flipping axes is swapping bindings; coordinates are a declared role with the grid axis they run along, and pickers list only coordinates for that axis; (b) keep `position`/`x`/`y` and add a flip setting; (c) a separate space–time view | Before more field views | Raised by the owner 2026-10-18: `position` duplicates `y`; binding a non-coordinate (e.g. `spring.amplitude`) silently falls back to cell numbers; `spring.lag` is accepted but its uneven spacing is drawn as even |
| Q19 | Settings whose meaning depends on the index | (a) The history states its index (time, a swept parameter, sample number) and slots and style properties declare where they apply, so the panel hides or relabels the rest; (b) per-view text only | With Q18 | `trail_length` (seconds) trims a one-parameter sweep's curve to the last stretch of the parameter; `time_axis` and `position` show for 2D grids where they do nothing |
| Q20 | Profiles of 2D grids | (a) `plot.profile` draws a 2D grid as curves along one axis at chosen values of the other (slices, saved with the view), e.g. resonance curves at several dampings; later linked to a cursor on a heat map; (b) profiles take line fields only | With Q18 | Today a 2D grid is drawn as one line through every cell row by row, a zigzag |
| Q21 | Where run versus sweep is decided | (a) An explicit experiment in the model file: parameter sampling (outer) × run in time (inner) × reduction, with today's behaviour as defaults; (b) keep deciding from the model (no states anywhere: sweep) | Before sweeping models with states | Today the whole model is either run or swept: `Session` sweeps when no block has states; blocks only declare their time semantics |
| Q22 | Unstructured sweeps | (a) Record every sweep as samples (index = sample number, swept parameters as signals), with grid structure as optional metadata that enables heat maps; random, Latin hypercube, Sobol or listed points; shown as coloured scatter, later triangulated contours; also allows more than two parameters; (b) keep grids only | Before sensitivity studies | Would change ADR 0022's representation (grid fields), so it needs a superseding ADR |

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
| Q15 | Sweeps over several parameters | [ADR 0022](decisions/0022-two-parameter-sweeps.md) |
| Q16 | Reducing coupling error between co-simulated Blocks | [ADR 0023](decisions/0023-automatic-fusion.md) |
