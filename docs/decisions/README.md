# Decision records

Architecture decision records (ADRs) capture one significant decision each: the context, the decision and its consequences. They are the project's memory of *why*.

## Index

| ADR | Title | Status | Date |
| --- | --- | --- | --- |
| [0001](0001-rust-core.md) | Rust core | Accepted | 2026-10-15 |
| [0002](0002-layered-block-contract.md) | Layered Block contract | Accepted | 2026-10-15 |
| [0003](0003-desktop-first.md) | Desktop-first | Accepted | 2026-10-15 |
| [0004](0004-method-classification.md) | Method classification instead of "one model, three modes" | Accepted | 2026-10-15 |
| [0005](0005-repository-as-source-of-truth.md) | Repository as single source of truth | Accepted | 2026-10-15 |
| [0006](0006-apache-2-license.md) | Apache-2.0 license | Accepted | 2026-10-15 |
| [0007](0007-json-model-files.md) | JSON model files with a block-type registry | Accepted | 2026-10-16 |
| [0008](0008-native-egui-ui.md) | All-native desktop UI with egui and wgpu | Accepted | 2026-10-16 |
| [0009](0009-hobbyists-first.md) | Hobbyists are the first audience | Accepted | 2026-10-16 |
| [0010](0010-live-loop-pacing.md) | Pace the live loop by wall-clock time | Superseded by 0035 | 2026-10-16 |
| [0011](0011-parameter-entry.md) | Typed parameter entry; ranges are suggestions | Accepted | 2026-10-16 |
| [0012](0012-controls-in-model-files.md) | Exploration controls are saved in model files | Accepted | 2026-10-16 |
| [0013](0013-run-section.md) | Run settings are saved in a `run` section | Accepted | 2026-10-16 |
| [0014](0014-view-in-controls.md) | The opening view is saved in `controls` | Accepted | 2026-10-16 |
| [0015](0015-pluggable-views.md) | Pluggable views with slots; style options limited to encodings and readability | Accepted | 2026-10-16 |
| [0016](0016-model-curated-views.md) | Models choose the views shown; other fitting views stay one click away | Accepted | 2026-10-17 |
| [0017](0017-typed-equations.md) | A built-in expression parser for typed equations | Accepted | 2026-10-17 |
| [0018](0018-one-xy-plot.md) | One x–y plot replaces the time-series and phase-portrait views | Accepted | 2026-10-17 |
| [0019](0019-equations-without-time.md) | Typed models without time: formulas and discrete maps first | Accepted | 2026-10-17 |
| [0020](0020-methods-for-approximations.md) | Only approximating operations have a method choice | Accepted | 2026-10-17 |
| [0021](0021-maps-formulas-and-sweeps.md) | Maps, formula blocks and parameter sweeps | Accepted | 2026-10-17 |
| [0022](0022-two-parameter-sweeps.md) | Sweeps over two parameters | Superseded by 0028 | 2026-10-17 |
| [0023](0023-automatic-fusion.md) | Automatic fusion of equation blocks, with opt-out and explicit groups | Accepted | 2026-10-17 |
| [0024](0024-heat-map-axes-and-coordinates.md) | Heat map axes and coordinate fields | Accepted | 2026-10-18 |
| [0025](0025-index-aware-view-settings.md) | View settings that depend on the index | Accepted | 2026-10-18 |
| [0026](0026-profile-slices.md) | Profiles of 2D grids as slices | Superseded by 0029 | 2026-10-18 |
| [0027](0027-explicit-experiments.md) | Explicit experiments: sampling, run and reduction | Superseded by 0071 | 2026-10-18 |
| [0028](0028-sweeps-as-samples.md) | Sweeps recorded as samples | Accepted | 2026-10-18 |
| [0029](0029-profile-cuts.md) | Profiles as cuts in any direction | Superseded by 0031 | 2026-10-18 |
| [0030](0030-filters-and-native-visualization.md) | Filters feed views; native visualization grows with need | Superseded by 0032 | 2026-10-18 |
| [0031](0031-cut-filters.md) | Cuts as filters with simple modes | Accepted | 2026-10-18 |
| [0032](0032-filters-as-a-graph.md) | Filters stored as a graph, shown on the node canvas later | Accepted | 2026-10-18 |
| [0033](0033-phase-1-plan.md) | Phase 1 order and scope: agents first, grazing model, headless Python | Superseded by 0036 | 2026-10-19 |
| [0034](0034-agent-domain.md) | Agent domain: kernel-owned tables, update order per block, counter-based randomness | Accepted | 2026-10-19 |
| [0035](0035-simulation-worker-thread.md) | The simulation runs on a worker thread; pacing as before | Accepted | 2026-10-19 |
| [0036](0036-python-blocks.md) | Python-defined blocks in Phase 1, in process, state in the kernel; revised Phase 1 order | Superseded by 0038 | 2026-10-19 |
| [0037](0037-when-parameter-changes-apply.md) | Sliders apply while dragged for real-time models, on release otherwise | Accepted | 2026-10-22 |
| [0038](0038-phase-1-revised.md) | Phase 1 revised: user-defined blocks decided later; headless interface before the canvas | Accepted | 2026-10-22 |
| [0039](0039-ai-agent-access.md) | AI agents are first-class users: no window-only features; discover, check, run and render headless | Accepted | 2026-10-22 |
| [0040](0040-feedthrough-and-algebraic-loops.md) | Direct feedthrough per output and input; loops detected and solved by Newton (or as `run.loops` says); typed `input` lines | Accepted | 2026-10-22 |
| [0041](0041-headless-interface.md) | Headless interface: shared session crate, `describe`/`check`/`run --format json`/`render`, views rasterised on the CPU, Python package in its own workspace | Accepted | 2026-10-22 |
| [0042](0042-node-canvas.md) | Node canvas as a Diagram tab; per-block `at`; edits rebuild and restart (drafts until they load); "Start from here" saves a reached state as the start | Accepted | 2026-10-22 |
| [0043](0043-subsystems.md) | Subsystems defined in the file and flattened on load; declared parameters with formulas (resolves Q30); open and group on the canvas | Superseded by 0068 (subsystems); formulas stand, amended by 0056 | 2026-10-23 |
| [0044](0044-unsaved-changes-and-older-files.md) | Ask before discarding unsaved changes; older files convert on reading, are reported and tested; a new format version only when adding is impossible | Accepted | 2026-10-23 |
| [0045](0045-units-in-typed-equations.md) | Units in brackets in typed equations, every line checked; connections need the same unit; `units.convert` is the only conversion | Accepted, amended by 0049 | 2026-10-23 |
| [0046](0046-units-of-declared-parameters.md) | Declared parameters' units are checked; formulas balance and give their target's unit, computed in SI | Accepted | 2026-10-23 |
| [0047](0047-unit-vocabulary.md) | SI prefixes, common non-SI and exact US/imperial units; left-to-right grammar with groups; `a/b*c` and °C/°F refused | Accepted | 2026-10-23 |
| [0048](0048-units-of-external-data.md) | Outside data: units declared where it enters; `units.convert` for fixed factors; typed equations to scale by a reference quantity | Accepted | 2026-10-23 |
| [0049](0049-state-declarations-and-rate-units.md) | `state x [m] = 0` declares a state; a unit at the end of a line is its left side's (`dv/dt = F/m [m/s^2]`) | Accepted | 2026-10-23 |
| [0050](0050-data-layer-and-canvas-undo.md) | Filters and views on the canvas (model level), applied live; a filter on a block input offers "Promote and connect"; undo for canvas edits | Accepted, amended by 0051 | 2026-10-23 |
| [0051](0051-stateless-blocks-join-fused-groups.md) | Formula and conversion blocks join the fused group they connect, taking its method; never merging two methods | Accepted | 2026-10-23 |
| [0052](0052-canvas-for-every-node-place.md) | Every node's place is `canvas` (blocks and ports too); `at` still read and converted | Accepted | 2026-10-23 |
| [0053](0053-docked-panes-inspector-and-node-values.md) | Inspector, diagram and views as dockable tabs, poppable into windows; arrangement saved as `controls.layout`; an inspector and editable values on nodes next | Accepted; amended by 0054 | 2026-10-23 |
| [0054](0054-controls-pane-pins-and-node-toggles.md) | Pins decide a Controls pane (sliders only for pinned values); every selected block in the inspector; drafts editable; values opened per node; units with a slash | Accepted; amended by 0055 | 2026-10-23 |
| [0055](0055-open-nodes-subsystem-values-deferred-windows.md) | Open nodes saved as `controls.open`; older sliders gain pins on reading; values inside subsystems as every use's default; popped-out tabs as deferred viewports | Accepted | 2026-10-24 |
| [0056](0056-formulas-and-parameters-in-the-app.md) | `= expression` in any value field makes a formula; "+ parameter" declares the model's parameters; failing edits are refused | Accepted | 2026-10-24 |
| [0057](0057-phase-2-plan.md) | Phase 2: interactive cart-pole gate; inputs, typed arrays and speed, rigid bodies and a scene view, one plugin interface; the rest after the gate | Accepted | 2026-10-26 |
| [0058](0058-inputs-live-scheduled-replayed.md) | Inputs: input blocks wired like any block; every change recorded as a track applied at step boundaries; replay after rewind; `run.inputs` schedules; headless tracks | Accepted; amended by 0059 | 2026-10-26 |
| [0059](0059-run-files.md) | Run files beside the model: recorded tracks and scenarios with the model hash, seed and settings; the model keeps an optional default schedule | Accepted | 2026-10-26 |
| [0060](0060-arrays-in-typed-equations.md) | Arrays in typed equations: 1D and 2D grids, whole-array equations with stencils and declared edges, an instruction-list evaluator, `backward_euler` and `trbdf2` with a sparse Jacobian | Accepted | 2026-10-27 |
| [0061](0061-verified-is-earned-per-case.md) | Verified is earned per case by a convergence check; no method declares it, exact ones included (conversions, inputs relabelled Quantitative) | Accepted | 2026-10-28 |
| [0067](0067-experiments-and-time-from-data.md) | Experiments over whole models: the opened file's `sample` runs the whole model per point (any model, by path, designs incl. `file`), final values by default, `reduce` and series opt-in, common seeds; models without states also run on their own data's sample times; nothing in time is one evaluation | Accepted; amended by 0070 | 2026-11-02 |
| [0068](0068-one-kind-of-nested-model.md) | One kind of nested model: subsystems become models defined in the file (`models`, used by `model`), ports as input blocks and declared outputs, default clock by `run.dt`, local copies as definitions, one editor; older `subsystems` converted (resolves Q49) | Accepted | 2026-11-02 |
| [0069](0069-repetition-in-blocks.md) | Repetition and recursion of structure belong in blocks: no self-use; arrays extended (array parameters, neighbours by index, network grids); Duplicate for a few copies; `count` on uses waits (resolves Q50) | Accepted | 2026-11-02 |
| [0070](0070-studies-are-blocks.md) | A study is a block (a model and a `sample` as its definition, inputs from the model's input blocks, arrays by run as outputs); a use always runs its model plain; arrays, agents and studies are models over sets of members (rows with ids, edges as tables, structure changed between steps by labelled transfers); a concept list kept in the overview | Accepted | 2026-11-03 |
| [0071](0071-sweeps-are-studies.md) | Sweeps and experiments are studies: the `sample` section, the default sweep and the app's sweep removed; study outputs shaped by their design (2D grids for two-parameter grids); studies run within their step in the app, with progress, given up for newer changes | Accepted | 2026-11-04 |
| [0072](0072-models-with-nothing-in-time.md) | Models with nothing in time (no states, tables, data, clocks or schedule) are evaluated in the app, not run: once and on every change, no clock or rewind | Accepted | 2026-11-05 |
| [0073](0073-ports-and-offered-outputs.md) | Offered outputs edited in the Inspector's Model section (block menus offer only inside a used model); a used model's port list holds only signals no declared output shows | Accepted | 2026-11-05 |
| [0074](0074-model-interfaces.md) | A model's interface: parameters (defaults in the model, values per use or study), inputs (signals, no defaults) and outputs, each exposed by a toggle where it is; studies take parameters; exposed things drawn apart | Accepted | 2026-11-05 |
| [0075](0075-parameters-are-inputs-with-defaults.md) | Parameters are inputs with defaults: every block's parameters may be wired (read when their owner reads them: each evaluation, at start, when a study runs); which outputs a driven parameter reaches is worked out for typed equations, declared by code blocks with a safe default, and checked by a test; activation left to its own ADR | Accepted | 2026-11-05 |
| [0076](0076-headless-runs-keep-what-is-asked.md) | Headless runs keep what is asked for: outputs by name (`block.port`, `block`, a model or a path in it) and every n-th step and the last; the same bits; default unchanged | Accepted | 2026-11-06 |
| [0077](0077-sums-in-fixed-chunks.md) | Sums in fixed chunks: up to 4096 cells in order; past it chunks of 4096 in four lanes, added in order; several cores from 2^20 cells; the same bits on any machine; reductions run before a phase when they can | Accepted | 2026-11-06 |
| [0078](0078-adaptive-studies.md) | Adaptive studies: strategies are models (a map on a clock) that choose a study's points through its inputs; runs remembered by id; `solve`; refinement on ADR 0079's tables; strategies shipped as example models with helpers | Proposed | 2026-11-06 |
| [0079](0079-tables-of-members.md) | Tables of members: rows with stable ids in typed equations, edge tables, rules (`add`, `remove`, `split`, `merge`) between steps, transfers, ports with ids and a structure version; one structure for adaptive studies, AMR, unstructured meshes and agents, built in five stages | Proposed | 2026-11-06 |
| [0066](0066-models-nest-as-blocks.md) | Models nest: a model's blocks may be uses of model files, on their own clocks (default) or their parent's; links through probes at any depth; 128-bit exact ticks; rewind per clock with recorded link values | Accepted | 2026-11-02 |
| [0065](0065-several-models-with-own-clocks.md) | Several models with their own clocks: compositions reference model files as named uses; links only through probes into input blocks; clocks ordered in exact ticks; editing a used model asks: edit the original, copy locally or save as new | Superseded by 0066 | 2026-11-02 |
| [0064](0064-recorder-node.md) | Rewind is opt-in: a recorder node holds the replay state's settings; without one no snapshots are taken and the rewind bar is hidden | Accepted | 2026-10-29 |
| [0063](0063-probes-on-wires.md) | Probes on wires: probe outputs feed views, filters and models; the implicit history records only what is wired; views read several sources, paired at common times; probes link models | Accepted | 2026-10-29 |
| [0062](0062-probes-recording-and-replay.md) | Probes: canvas nodes that record and save chosen values at chosen times (thinning by default); snapshots on their own budget, with outputs; exact runs on several cores, or fast; built after Phase 2 item 2 | Accepted | 2026-10-29 |

## When to write one

Write an ADR when a choice is hard to reverse, affects more than one crate or document, or settles an item in [open questions](../open-questions.md). Small, local choices belong in code comments or the pull request.

## How

1. Copy [`0000-template.md`](0000-template.md) to the next number: `NNNN-short-title.md`.
2. Fill it in; keep it to one page. Status starts as `Proposed` if the owner has not agreed yet, otherwise `Accepted`.
3. Add a row to the index above (the docs check fails otherwise) and to the map in [`docs/README.md`](../README.md).
4. Update the documents the decision affects, and remove the item from open questions if it resolves one.

## Rules

- Accepted ADRs are not rewritten. To change a decision, write a new ADR and set the old one's status to `Superseded by NNNN`.
- Statuses: `Proposed`, `Accepted`, `Superseded by NNNN`, `Rejected`.
