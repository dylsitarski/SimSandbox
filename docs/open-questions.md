# Open questions

_Last updated: 2026-10-18_

Decisions not yet made. When one is resolved, write an ADR, link it in the **Resolved** table and remove the row from **Open**.

## Open

| ID | Question | Options | Needed by | Notes |
| --- | --- | --- | --- | --- |
| Q5 | Algebraic loops in fused groups | Causalisation and tearing; Newton solve per RHS evaluation; forbid and warn | Phase 1 | Current v0 restriction: no direct feedthrough |
| Q17 | Implicit and implicit–explicit (IMEX) methods for stiff systems | (a) An implicit method (e.g. backward differentiation or an L-stable Runge–Kutta) for whole systems, with Jacobians by finite differences first; (b) named IMEX methods that treat a declared stiff part implicitly; (c) explicit methods with substeps only | Before stiff fields or large fused groups | Raised 2026-10-17 with ADR 0023, which rules out free per-block method mixing inside one fused system: arbitrary pairs of methods fall to first order. First case: `eq.heat_rod`, whose explicit step limit (≈0.7 h²/α for RK4) forces 10 substeps in the example |
| Q23 | A simpler way to choose profile cuts | (a) A mode per profile: "along x at fixed y values", "along y at fixed x values" (each cut one number, the whole field by default, optionally a sub-range; plotted against the free coordinate) or "free lines" (two end points, plotted against fraction or distance); (b) per cut, each coordinate "constant at" or "free", with a "whole field" toggle (owner's proposal); (c) keep ADR 0029's end points | Before more field views | Raised by the owner 2026-10-18: the cut editor is confusing; the rule for when x or y is offered (every cut not constant in it) is not visible. Would supersede ADR 0029's editing and horizontal-axis rules |
| Q24 | Pipeline of filters versus views doing their own transforms | (a) Lightweight dataflow: named filters (cut, slice, reshape, FFT, reduction, derived expressions) saved in the model's controls, each making new signals or fields that any view can show; views stay simple; (b) a full node-editor pipeline like ParaView; (c) keep transforms inside views; plus export (VTK, HDF5) for ParaView or Python, and evaluate embedding a ready-made viewer (e.g. Rerun) | Before more views | Raised by the owner 2026-10-18, with whether native visualization is worth the effort |

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
| Q18 | Heat map axes and coordinates | [ADR 0024](decisions/0024-heat-map-axes-and-coordinates.md) |
| Q19 | Settings whose meaning depends on the index | [ADR 0025](decisions/0025-index-aware-view-settings.md) |
| Q20 | Profiles of 2D grids | [ADR 0026](decisions/0026-profile-slices.md), superseded by [ADR 0029](decisions/0029-profile-cuts.md) |
| Q21 | Where run versus sweep is decided | [ADR 0027](decisions/0027-explicit-experiments.md) |
| Q22 | Unstructured sweeps | [ADR 0028](decisions/0028-sweeps-as-samples.md) |
