# Open questions

_Last updated: 2026-10-17_

Decisions not yet made. When one is resolved, write an ADR, link it in the **Resolved** table and remove the row from **Open**.

## Open

| ID | Question | Options | Needed by | Notes |
| --- | --- | --- | --- | --- |
| Q5 | Algebraic loops in fused groups | Causalisation and tearing; Newton solve per RHS evaluation; forbid and warn | Phase 1 | Current v0 restriction: no direct feedthrough |
| Q12 | Equations without time and without states | (a) stateless typed blocks (only `name = …` lines) explored by sweeping a parameter; (b) discrete maps `x[n+1] = …` stepped by iteration; (c) implicit equations and steady states (`f(x) = 0`) solved by Newton's method; (d) an independent variable other than `t` (`dy/dx`) | Before Phase 1 composition work on typed blocks | Raised 2026-10-17. Today typed blocks need at least one `dx/dt` line and `name = …` must be explicit. Recommendation: (a) and (b) first (cheap, hobbyist favourites such as the logistic map); (c) together with Q5 (algebraic loops); (d) later |
| Q13 | Merge the time-series and phase-portrait views into one x–y plot | Keep `plot.time` and `plot.phase`; one `plot.xy` with an `x` slot (default `t`) and a `y` slot taking several signals | Before views step 4 | Raised by the owner 2026-10-17. Recommendation: merge; trail length on a time plot then gives a scrolling "last N seconds" window, and old files convert like ADR 0014's `view` did. `plot.3d` stays separate because it draws differently |
| Q14 | Choosing the method for operations inside equations | Fixed implementations only; a method choice, with speed and accuracy labels ([ADR 0004](decisions/0004-method-classification.md)), for operations that approximate (derivatives for Jacobians, integrals, root finding for implicit equations, smoothing or differentiating recorded signals, spectra) | When the first approximating operation lands (Jacobians or Q12 c) | Raised by the owner 2026-10-17. Recommendation: exact operations (`+`, `sin`, `exp`) have no method; approximating ones get a method with a sensible default and labels, chosen like integrators are |

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
| Q4 | Symbolic equation parsing | [ADR 0017](decisions/0017-typed-equations.md) (proposed) |
