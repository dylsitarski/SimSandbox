# Open questions

_Last updated: 2026-11-14_

Decisions not yet made. When one is resolved, write an ADR, link it in the **Resolved** table and remove the row from **Open**.

## Open

| ID | Question | Options | Needed by | Notes |
| --- | --- | --- | --- | --- |
| Q61 | Whether hobbyists are still the first audience ([ADR 0009](decisions/0009-hobbyists-first.md)), now that typed equations are the primary way to build models (ADR 0091) and recent work (meshes, implicit methods on large grids) serves research | (a) Yes: the gate and scenes are for them, research features wait for models that need them; (b) hobbyists and researchers together; (c) researchers first | Before the next roadmap revision | Raised by the [review](research/2026-11-14-code-review.md) and ADR 0091 |
| Q52 | How a study keeps each run's series, now that experiments are studies (ADR 0071) and `keep: "series"` is refused | (a) A 2D output per signal, runs × samples, on a common time grid; (b) a probe inside the studied model writing a file per run; (c) not at all: reduce, or run the model alone at that point | When someone needs whole runs of a study | Experiments kept them headless (`Results.series`) until 2026-11-04; nothing used them besides tests |
| Q43 | Re-reading a data file that changes while its model is open (`input.file`, ADR 0063) | (a) Read when the model is built only (as built 2026-10-30): reopen or edit to re-read; (b) re-read on Restart; (c) watch the file and offer to reload | When the owner reads files that change, e.g. another model's streamed probe | Step 5 (several models in one session) links models by probes directly, which may make this rare |
| Q42 | Whether probe CSV files write units in their header (`car.v [m/s]`) | (a) No (as now): reading one back needs `unit.COLUMN` settings; (b) yes: `input.file` reads them with no settings, but pandas column names carry the unit; (c) a sidecar file with units and field shapes | Before probe files are read back often | `input.file` already reads `name [unit]` headers (ADR 0048: units from the source when it has any). The session would need each recorded signal's unit |
| Q37 | How users define their own views without rebuilding SimSandbox | (a) Through the same plugin interface as blocks ([Q28](#open)): a module receives signals and returns a draw list (lines, shapes, meshes, text) plus input events, rendered by the app with egui and wgpu; (b) views stay built in, written in Rust; (c) export to an external viewer (Rerun, ParaView) for custom pictures | Phase 2 item 4, with Q28 ([ADR 0057](decisions/0057-phase-2-plan.md)) | Raised 2026-10-26 by the owner, as generalising views. Today a view is compiled Rust (`View` in `crates/simsandbox-views`, [ADR 0015](decisions/0015-pluggable-views.md)). Sandboxing matters as for blocks, since views travel in shared models |
| Q28 | How users define their own blocks without rebuilding SimSandbox | (a) Python, in process through PyO3; (b) WebAssembly plugins (sandboxed, any compiled language); (c) Julia, out of process; (d) extended typed languages only | Phase 2 item 4, after a WebAssembly spike ([ADR 0057](decisions/0057-phase-2-plan.md)) | Raised 2026-10-22 with [ADR 0038](decisions/0038-phase-1-revised.md), which deferred Python-defined blocks ([ADR 0036](decisions/0036-python-blocks.md)). Any choice keeps state in kernel memory and runs under the worker thread. Recommendation (2026-10-26, not decided): WebAssembly as the main path for shareable custom blocks (near-native, any language, sandboxed so a shared model cannot harm the machine), one plugin interface shared with user-defined views ([Q37](#open)); Python blocks later for research convenience (numpy, not sandboxed, too slow per step for live play unless vectorised). Settle after input blocks ([Q36](#open)) and arrays in typed equations ([Q35](#open)) show what plugins still need to cover |
| Q29 | When to add an MCP server and AI control of the running app | (a) With the headless interface; (b) after the node canvas, so an agent can build models in front of the user; (c) with the in-app AI assistant (Phase 3) | After Phase 1 item 5 | Raised 2026-10-22 with [ADR 0039](decisions/0039-ai-agent-access.md) |
| Q27 | How the app finds Python | (a) The user's installed Python (minimum version to choose); (b) a Python shipped with the app (e.g. python-build-standalone), packages in an app-managed environment; (c) both, preferring the user's | Only if Q28 chooses Python | Raised 2026-10-19 with [ADR 0036](decisions/0036-python-blocks.md); settled by an embedding spike on Windows and Linux |
| Q26 | Typed agent rules | (a) Per-agent update expressions with conditions for birth and death, using the equation parser; (b) a small rule language with neighbourhood queries; (c) agent types in Rust only, with Python blocks later | After the Phase 2 gate ([ADR 0060](decisions/0060-arrays-in-typed-equations.md)) | Raised 2026-10-19 with [ADR 0034](decisions/0034-agent-domain.md): the grazing sheep are a built-in type (`agents.grazers`) first, so the rule language is designed from real models. 2026-11-03 ([ADR 0070](decisions/0070-studies-are-blocks.md)): an agent type will be a model definition whose states and parameters are a table's columns, run as rows; the open part is the rule syntax |

## Resolved

| ID | Question | Resolution |
| --- | --- | --- |
| Q60 | Whether views built from primitive parts come before the Phase 2 gate | Owner, 2026-11-14: the primitives are designed first and only what the cart-pole needs is built before the gate; the design is [ADR 0093](decisions/0093-scenes-from-primitive-layers.md), Proposed |
| Q59 | Whether a model file may read paths outside its own folder | [ADR 0092](decisions/0092-what-a-model-file-may-read.md) (owner, 2026-11-14): inside its project folder, and folders the user grants outside the file; not built yet |
| Q56 | Whether SimSandbox is an equation sandbox that later hosts other solvers, or a host first | [ADR 0091](decisions/0091-equations-first-hosting-for-existing-codes.md) (owner, 2026-11-14): typed equations are the primary way to build models; hosting is for codes that exist |
| Q58 | Which engine runs models that use models | The merged simulator ([ADR 0090](decisions/0090-one-engine-for-nested-models.md), 2026-11-14, the owner leaving the choice to the agent); not built yet |
| Q57 | Whether the Phase 2 gate starts before any further typed-equation feature | Yes (owner, 2026-11-14), as [ADR 0057](decisions/0057-phase-2-plan.md) orders; whether views from primitives come first is Q60 |
| Q55 | Whether typed equations run compiled phases (Cranelift in the process) now | Yes (owner, 2026-11-13): `simsandbox-compiled`, vector code, the interpreter as reference and fallback ([ADR 0084](decisions/0084-where-typed-equations-stop.md) As built); native code too, switching on when a toolchain is found ([ADR 0089](decisions/0089-native-code-for-typed-equations.md)) |
| Q54 | Where typed equations stop and wrapped libraries, FMI or other backends begin | [ADR 0084](decisions/0084-where-typed-equations-stop.md) (owner, 2026-11-08): blocks wrap anything; typed equations a causal language of exact, verified primitives; acausal physics from Modelica through FMI; speed from compiling our own program, the route measured 2026-11-13 (Cranelift; whether now: Q55) |
| Q53 | How a model or study runs or restarts when an event fires (activation) | [ADR 0088](decisions/0088-activation.md) (2026-11-13): an `activate` input on used models and studies with modes `rise` (default), `fall`, `change` and `while` (held outputs keep their last values); a `restart` input on used models; events within a step later, as a capability of the signal's source |
| Q51 | Whether `controls.plots` leaves the format | Removed (owner, 2026-11-04): no file relies on it, so nothing converts it; a stray `plots` is ignored like any unknown `controls` key |
| Q50 | Where repetition and recursion of structure belong | [ADR 0069](decisions/0069-repetition-in-blocks.md) (owner, 2026-11-02): in blocks (arrays extended); no self-use; Duplicate; `count` waits |
| Q49 | Whether nested models supersede subsystems | [ADR 0068](decisions/0068-one-kind-of-nested-model.md) (owner, 2026-11-02): one kind; subsystems become models defined in the file |
| Q48 | Making a new copy of a model used deep inside another used model | [ADR 0066](decisions/0066-models-nest-as-blocks.md) step 4c (owner, 2026-11-02): the using file replaces the deep use's file by path (`files`); editing the file in between, or copying every file on the way down, offered too |
| Q47 | Showing used models and subsystems in place, as outlined groups | [ADR 0066](decisions/0066-models-nest-as-blocks.md) step 4d (owner, 2026-11-02): "Expand in place" per node, one level at a time, after 4c |
| Q46 | Ports of used models crowd their nodes | [ADR 0066](decisions/0066-models-nest-as-blocks.md) step 4b (owner, 2026-11-02): declared `outputs` in model files; a node shows inputs, declared outputs and what is wired; the rest from a searchable list by path; parameters as a tree in the Inspector; nested inputs hidden unless wired |
| Q45 | Define compositions like models, so models nest | [ADR 0066](decisions/0066-models-nest-as-blocks.md) (owner, 2026-11-02): one recursive kind of model; uses of model files in `blocks`, on their own clocks by default or shared; links at any depth |
| Q44 | Suggestions in ADR 0065 beside the owner's choices | [ADR 0065](decisions/0065-several-models-with-own-clocks.md): all accepted by the owner (2026-11-02): links through input blocks, parameter values and clocks per use without asking, "Save as a new model file…", local copies keep their own clock, rewinding one use alone later |
| Q41 | Whether rewind's state recording becomes a node | Yes: rewind is off without a recorder node, which holds the snapshots' settings; the rewind bar is hidden until one is added; subsystem replay later ([ADR 0064](decisions/0064-recorder-node.md)) |
| Q40 | How views choose what they draw from, and what the implicit probe records | Probes have outputs wired into views, filters and models; views mix sources, paired at common times; the implicit history records only what is wired ([ADR 0063](decisions/0063-probes-on-wires.md)) |
| Q39 | How simulations use several cores without losing exact replay | Runs are exact on several cores, with fixed-order sums ([ADR 0062](decisions/0062-probes-recording-and-replay.md), [ADR 0077](decisions/0077-sums-in-fixed-chunks.md)); code that is not exact comes as blocks labelled so, not as a per-run fast mode ([ADR 0082](decisions/0082-exactness-is-labelled.md), 2026-11-08) |
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
