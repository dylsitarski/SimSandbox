# Open questions

_Last updated: 2026-10-24_

Decisions not yet made. When one is resolved, write an ADR, link it in the **Resolved** table and remove the row from **Open**.

## Open

| ID | Question | Options | Needed by | Notes |
| --- | --- | --- | --- | --- |
| Q28 | How users define their own blocks without rebuilding SimSandbox | (a) Python, in process through PyO3; (b) WebAssembly plugins (sandboxed, any compiled language); (c) Julia, out of process; (d) extended typed languages only | After the typed agent rules (Q26) | Raised 2026-10-22 with [ADR 0038](decisions/0038-phase-1-revised.md), which deferred Python-defined blocks ([ADR 0036](decisions/0036-python-blocks.md)). Any choice keeps state in kernel memory and runs under the worker thread |
| Q29 | When to add an MCP server and AI control of the running app | (a) With the headless interface; (b) after the node canvas, so an agent can build models in front of the user; (c) with the in-app AI assistant (Phase 3) | After Phase 1 item 5 | Raised 2026-10-22 with [ADR 0039](decisions/0039-ai-agent-access.md) |
| Q27 | How the app finds Python | (a) The user's installed Python (minimum version to choose); (b) a Python shipped with the app (e.g. python-build-standalone), packages in an app-managed environment; (c) both, preferring the user's | Only if Q28 chooses Python | Raised 2026-10-19 with [ADR 0036](decisions/0036-python-blocks.md); settled by an embedding spike on Windows and Linux |
| Q26 | Typed agent rules | (a) Per-agent update expressions with conditions for birth and death, using the equation parser; (b) a small rule language with neighbourhood queries; (c) agent types in Rust only, with Python blocks later | After two or three built-in agent models | Raised 2026-10-19 with [ADR 0034](decisions/0034-agent-domain.md): the grazing sheep are a built-in type (`agents.grazers`) first, so the rule language is designed from real models |
| Q17 | Implicit and implicit–explicit (IMEX) methods for stiff systems | (a) An implicit method (e.g. backward differentiation or an L-stable Runge–Kutta) for whole systems, with Jacobians by finite differences first; (b) named IMEX methods that treat a declared stiff part implicitly; (c) explicit methods with substeps only | Before stiff fields or large fused groups | Raised 2026-10-17 with ADR 0023, which rules out free per-block method mixing inside one fused system: arbitrary pairs of methods fall to first order. First case: `eq.heat_rod`, whose explicit step limit (≈0.7 h²/α for RK4) forces 10 substeps in the example |

## Resolved

| ID | Question | Resolution |
| --- | --- | --- |
| Q34 | How the app declares model parameters and writes formulas | `= expression` in any value field, "+ parameter" in the Model group ([ADR 0056](decisions/0056-formulas-and-parameters-in-the-app.md)) |
| Q33 | Whether popped-out tabs need deferred viewports | Yes: popped-out tabs draw on their own ([ADR 0055](decisions/0055-open-nodes-subsystem-values-deferred-windows.md)) |
| Q32 | Whether which nodes are open on the diagram is saved | Saved as `controls.open` ([ADR 0055](decisions/0055-open-nodes-subsystem-values-deferred-windows.md)) |
| — | Rust core or Julia-first core | [ADR 0001](decisions/0001-rust-core.md) |
| — | Shape of the Block contract | [ADR 0002](decisions/0002-layered-block-contract.md) |
| — | Desktop-only or browser | [ADR 0003](decisions/0003-desktop-first.md) |
| — | "One model, three modes" or per-method classification | [ADR 0004](decisions/0004-method-classification.md) |
| — | Where project knowledge lives | [ADR 0005](decisions/0005-repository-as-source-of-truth.md) |
| — | License | [ADR 0006](decisions/0006-apache-2-license.md) |
| Q5 | Algebraic loops in fused groups | [ADR 0040](decisions/0040-feedthrough-and-algebraic-loops.md) |
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
| Q20 | Profiles of 2D grids | [ADR 0026](decisions/0026-profile-slices.md), superseded by [ADR 0029](decisions/0029-profile-cuts.md), then [ADR 0031](decisions/0031-cut-filters.md) |
| Q21 | Where run versus sweep is decided | [ADR 0027](decisions/0027-explicit-experiments.md) |
| Q22 | Unstructured sweeps | [ADR 0028](decisions/0028-sweeps-as-samples.md) |
| Q23 | A simpler way to choose profile cuts | [ADR 0031](decisions/0031-cut-filters.md) |
| Q24 | Pipeline of filters versus views doing their own transforms | [ADR 0030](decisions/0030-filters-and-native-visualization.md), amended by [ADR 0032](decisions/0032-filters-as-a-graph.md) |
| Q25 | Filters and views on the node canvas | [ADR 0032](decisions/0032-filters-as-a-graph.md) |
| Q31 | How data from outside sources enters a model with units | [ADR 0048](decisions/0048-units-of-external-data.md): units declared where data enters; `units.convert` for fixed factors; typed equations to scale by a reference quantity |
| Q30 | Parameters shared by several blocks | [ADR 0043](decisions/0043-subsystems.md): declared parameters with formulas |
