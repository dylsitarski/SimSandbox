# 0050 — The canvas's data layer, promotion, and undo

- **Status:** Accepted
- **Date:** 2026-10-23

## Context

[ADR 0032](0032-filters-as-a-graph.md) required the node canvas to show filters as observer nodes and views as sink nodes, on a layer that can be hidden, with wires that read recorded data drawn differently, and to refuse wiring an observer into the model, promoting it to a block instead. The canvas ([ADR 0042](0042-node-canvas.md)) shipped without that layer and without undo. The owner chose (2026-10-23) to add both before planning Phase 2: observers at the model level only, a refused filter-to-model wire offering "Promote and connect", and undo for canvas edits.

## Options considered

| Question | Options | Chosen |
| --- | --- | --- |
| Where observers live | **Model level only**; also inside subsystems (instrumentation that travels with a reusable part) | Model level: model-level filters and views already read signals inside uses by their flat names; per-subsystem observers raise questions (one tile per use or one for all, which use the inside shows) worth answering with subsystem libraries |
| Where a node's place is saved | `at`, as for blocks; **`canvas`** | `canvas`: cut filters already use `at` for where they cut, and filter settings share the entry |
| A filter wired into a block input | Allow it; refuse; **refuse and offer to promote and connect** | Filters run on recorded data after each step, with no method or accuracy label, may look across the history, and live in the lenient `controls` (dropped with a warning if unusable); feeding one back would make results depend on display settings and add a hidden delay |
| How data edits apply | Rebuild and restart, like model edits; **apply to the running model** | Filters and views never change results; restarting for a new plot would lose the run |
| What undo restores | The whole file; **what the canvas edits** | The canvas's parts (blocks, wires, subsystems, declared parameters, groups, filters, views and places), keeping parameter values, sliders, speed and run settings changed since |

## Decision

- **Data layer.** On the model level the Diagram tab shows each filter as a `filter:NAME` node (outputs: its signals) and each view as a `view:K` node (inputs: its type's slots), with dashed outlines in the link colour, wired by dashed wires from the outputs they read: model outputs, a use's output for signals inside it, the whole vector for an element, the swept block for a sweep's grid. What has no port (`t`, parameters, swept values) is named beside the slot (`x · t`) and in the hover help. "Hide data" hides the layer; headless rendering shows it unless asked not to (`--no-data`, `data=False`).
- **Places.** Filters and views gain an optional `canvas: [x, y]` (format additive, [ADR 0044](0044-unsaved-changes-and-older-files.md)); without it they are placed to the right of the model, filters before views.
- **Editing the layer** applies to the running model without a restart: dropping an output on a view's slot adds the signal (or replaces it, for a slot that takes one); deleting a data wire unbinds it; filters are added (an expression, edited in Data…), renamed (views and filters reading them follow), moved and deleted (views stop showing their signals); views are added by type (bound from the type's first preset) and deleted. Double-clicking opens a filter in Data… or shows a view's tile. Filters are evaluated over the history again only when they changed.
- **Filters never feed the model.** Wiring a filter into a block input is refused with the reason and a "Promote and connect" offer. **Promoting** turns an expression filter into an `eq.equations` formula block with an input for each output it reads, declared in that output's unit and wired from it, defining the filter's name; views and filters reading the filter then read the block. Only expression filters that read scalar model-level outputs (and `t`) can be promoted; one reading parameters, elements, sweep values or other filters says why. A promoted block between two blocks that were solved together (ADR 0023) separates them.
- **Undo and redo** (buttons, Ctrl+Z, Ctrl+Shift+Z or Ctrl+Y) step through the canvas's edits, up to 100, by restoring the file as it was before each one in the parts the canvas edits. A change of places or of filters and views applies to the running model; anything else rebuilds it (or becomes the draft, if it does not load). Opening a model, or rebuilding it outside the canvas (equations, side panel), starts the history afresh.
- **Headless** ([ADR 0039](0039-ai-agent-access.md)): filters and views are in the file; `simsandbox render --diagram` draws the layer; Python `Model.promote(filter, to=None)` promotes (and connects).

## Consequences

- ADR 0032's canvas requirements are met; its list UI (Data…) stays and shows the same filters.
- Undo covers the canvas only: parameter changes, equation edits and side-panel changes are not undone by it.
- Observers inside subsystems, promoting cuts, and inlining filters read by a promoted one are left for later.
