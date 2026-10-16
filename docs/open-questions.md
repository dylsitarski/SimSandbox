# Open questions

_Last updated: 2026-10-16_

Decisions not yet made. When one is resolved, write an ADR, link it in the **Resolved** table and remove the row from **Open**.

## Open

| ID | Question | Options | Needed by | Notes |
| --- | --- | --- | --- | --- |
| Q4 | Symbolic equation parsing | Rust-native parser and symbolic crate; SymPy via Python; Julia Symbolics service | Symbolic equation entry | Affects dependencies and startup |
| Q5 | Algebraic loops in fused groups | Causalisation and tearing; Newton solve per RHS evaluation; forbid and warn | Phase 1 | Current v0 restriction: no direct feedthrough |
| Q9 | Visualization architecture for many outputs and model-specific pictures | (a) keep a fixed set of views over scalar series; (b) a view registry like the block registry: views declare named slots (x, y, z, color, …) bound to outputs or derived signals, domain crates register their own views (pendulum, rigid-body scene, field heatmap), several views laid out at once | Before a second domain (agents) or a model with more than a handful of outputs | Raised 2026-10-16. Recommendation (b), with richer output shapes (grids, point sets) and derived signals (FFT, energy, projections, Poincaré sections) so high-dimensional models get suitable views; see the [session note](sessions/2026-10-16-kernel-prep-and-adrs.md) |
| Q10 | Which draw-style options to expose (trail fading, colour, line width, …) | Ad-hoc toggles per view; a small set of encodings (colour/opacity by time, speed, constant; trail length) plus a shared theme and palette, all saved in `controls` | With Q9 | Raised 2026-10-16. Recommendation: expose an option only when it changes what can be read (an encoding) or readability and accessibility (width, colour-blind-safe palette); everything else is a good fixed default |

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
