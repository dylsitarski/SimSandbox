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
| [0027](0027-explicit-experiments.md) | Explicit experiments: sampling, run and reduction | Accepted | 2026-10-18 |
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
| [0043](0043-subsystems.md) | Subsystems defined in the file and flattened on load; declared parameters with formulas (resolves Q30); open and group on the canvas | Accepted; amended by 0056 | 2026-10-23 |
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
