# Open questions

_Last updated: 2026-10-29_

Decisions not yet made. When one is resolved, write an ADR, link it in the **Resolved** table and remove the row from **Open**.

## Open

| ID | Question | Options | Needed by | Notes |
| --- | --- | --- | --- | --- |
| Q41 | Whether rewind's state recording becomes a node | (a) A recorder node per model holding the replay state (snapshots and the input track) and its settings (how often, memory budget, on or off); one is implicit, as the implicit probe is, so rewind works without adding it; (b) the same, but rewind only where a recorder is added; (c) recorders also on subsystems, rewinding them alone against their recorded inputs | With ADR 0063 step 5 (several models in one session) | Raised 2026-10-29 by the owner. Recommendation (not decided): (a) now or with step 5, where each model has its own clock and recorder and models linked by probes can be rewound apart and replayed exactly from the probes' data; (c) later, as replaying a part against what it saw, since rewinding a subsystem inside a tightly coupled model cannot change what the rest did. A recorder is not wired to outputs: it attaches to a model or subsystem |
| Q37 | How users define their own views without rebuilding SimSandbox | (a) Through the same plugin interface as blocks ([Q28](#open)): a module receives signals and returns a draw list (lines, shapes, meshes, text) plus input events, rendered by the app with egui and wgpu; (b) views stay built in, written in Rust; (c) export to an external viewer (Rerun, ParaView) for custom pictures | Phase 2 item 4, with Q28 ([ADR 0057](decisions/0057-phase-2-plan.md)) | Raised 2026-10-26 by the owner, as generalising views. Today a view is compiled Rust (`View` in `crates/simsandbox-views`, [ADR 0015](decisions/0015-pluggable-views.md)). Sandboxing matters as for blocks, since views travel in shared models |
| Q28 | How users define their own blocks without rebuilding SimSandbox | (a) Python, in process through PyO3; (b) WebAssembly plugins (sandboxed, any compiled language); (c) Julia, out of process; (d) extended typed languages only | Phase 2 item 4, after a WebAssembly spike ([ADR 0057](decisions/0057-phase-2-plan.md)) | Raised 2026-10-22 with [ADR 0038](decisions/0038-phase-1-revised.md), which deferred Python-defined blocks ([ADR 0036](decisions/0036-python-blocks.md)). Any choice keeps state in kernel memory and runs under the worker thread. Recommendation (2026-10-26, not decided): WebAssembly as the main path for shareable custom blocks (near-native, any language, sandboxed so a shared model cannot harm the machine), one plugin interface shared with user-defined views ([Q37](#open)); Python blocks later for research convenience (numpy, not sandboxed, too slow per step for live play unless vectorised). Settle after input blocks ([Q36](#open)) and arrays in typed equations ([Q35](#open)) show what plugins still need to cover |
| Q29 | When to add an MCP server and AI control of the running app | (a) With the headless interface; (b) after the node canvas, so an agent can build models in front of the user; (c) with the in-app AI assistant (Phase 3) | After Phase 1 item 5 | Raised 2026-10-22 with [ADR 0039](decisions/0039-ai-agent-access.md) |
| Q27 | How the app finds Python | (a) The user's installed Python (minimum version to choose); (b) a Python shipped with the app (e.g. python-build-standalone), packages in an app-managed environment; (c) both, preferring the user's | Only if Q28 chooses Python | Raised 2026-10-19 with [ADR 0036](decisions/0036-python-blocks.md); settled by an embedding spike on Windows and Linux |
| Q26 | Typed agent rules | (a) Per-agent update expressions with conditions for birth and death, using the equation parser; (b) a small rule language with neighbourhood queries; (c) agent types in Rust only, with Python blocks later | After the Phase 2 gate ([ADR 0060](decisions/0060-arrays-in-typed-equations.md)) | Raised 2026-10-19 with [ADR 0034](decisions/0034-agent-domain.md): the grazing sheep are a built-in type (`agents.grazers`) first, so the rule language is designed from real models |

## Resolved

| ID | Question | Resolution |
| --- | --- | --- |
| Q40 | How views choose what they draw from, and what the implicit probe records | Probes have outputs wired into views, filters and models; views mix sources, paired at common times; the implicit history records only what is wired ([ADR 0063](decisions/0063-probes-on-wires.md)) |
| Q39 | How simulations use several cores without losing exact replay | Runs are exact (default: several cores, fixed-order sums and factorisations) or fast (any order), chosen per run ([ADR 0062](decisions/0062-probes-recording-and-replay.md)) |
| Q38 | How often large grids are recorded | Probes: canvas nodes recording chosen values at chosen times within a budget, thinning by default; snapshots on their own budget ([ADR 0062](decisions/0062-probes-recording-and-replay.md)) |
| Q35 | Whether and how to speed up typed equations | A compact instruction list over whole arrays; machine code only if measurements call for it ([ADR 0060](decisions/0060-arrays-in-typed-equations.md)) |
| Q17 | Implicit and IMEX methods for stiff systems | One-step L-stable methods, `backward_euler` and `trbdf2`, Newton with a sparse finite-difference Jacobian ([ADR 0060](decisions/0060-arrays-in-typed-equations.md)) |
| Q36 | How views and devices send input to a running model | Input blocks, an input track recorded at step boundaries, replay after rewind, an optional default schedule in `run.inputs` ([ADR 0058](decisions/0058-inputs-live-scheduled-replayed.md)); runs saved as run files ([ADR 0059](decisions/0059-run-files.md)) |
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
