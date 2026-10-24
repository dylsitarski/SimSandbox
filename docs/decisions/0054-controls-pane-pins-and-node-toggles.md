# 0054 — A Controls pane, pins, and values opened per node

- **Status:** Accepted; older sliders, open nodes and pop-out windows are amended by [0055](0055-open-nodes-subsystem-values-deferred-windows.md)
- **Date:** 2026-10-23

## Context

The owner tried the inspector and node values of [ADR 0053](0053-docked-panes-inspector-and-node-values.md) on the platoon (2026-10-23) and listed problems:

- Whether a value appeared among the Controls depended on its slider; pinning did not visibly matter, and a control could not be unpinned.
- Notes such as "used by 3", "starts at ↻", and the subsystem line "used by car1, car2, car3: editing changes every use" took space or misled. The last one reads as if editing a value inside the subsystem shares it across uses. It does not: every use keeps its own values, and only blocks and wires drawn inside apply to every use.
- One "Values" switch showed values on every node at once. Nodes also listed values that formulas set.
- Selecting several blocks showed one of them.
- A block added on the diagram did not appear in the inspector until the model loaded with it. A conversion block with nothing wired in does not load, so its units could not be set anywhere. A value typed on such a node was dropped.
- Other problems: units shown as `m·s^-1`, the log listed newest first, turning the only plot into its own tab removed the Views tab, and popping out a tab stalled the main window on the owner's Linux desktop.

The owner also asked whether the inspector should be split into separate tabs.

## Options considered

| Question | Options | Chosen |
| --- | --- | --- |
| What puts a value among the Controls | Its slider (as before); **its pin** | The pin; a slider is offered only for pinned values, off by default |
| Where the Controls go | At the top of the inspector; **a pane of their own**; also split out the selection | A **Controls** pane above the inspector by default; the selection stays at the top of the inspector, next to the tree it belongs with |
| Showing values on nodes | One switch for all; **a toggle on each node** | A triangle in each node's header; open nodes list all their values |
| Values that formulas set | Shown greyed; **not shown on nodes** | Not shown (the inspector lists them with "computed") |
| Editing a block not yet in the run | Wait until it loads; **edit the draft** | Values and settings are edits in the draft, through the inspector or the node |
| Pop-out stall | Deferred viewports (a rewrite of how panes are drawn); **no vsync while a tab is popped out** | No vsync while popped out, with the worker's redraw requests capped at 120 a second |

## Decision

- **Pins.** `controls.params.K.pin` puts a value in the **Controls**. A 📌 button on every row of the inspector pins or unpins it; so does right-clicking a name, or a row of a uses table, per use. A pinned value can show a slider (a checkbox in the Controls, off by default); its range is set by right-clicking its name there. In older files, `slider: true` without `pin` counts as pinned. Unpinning removes the slider and keeps its range. Pinned values stay in the inspector's tree too.
- **The Controls pane.** It is a tab like the others, saved as `"controls"` in `controls.layout`. The standard arrangement places it above the inspector. An arrangement saved without it gets it there.
- **Inspector text.** The "used by" rows and the "starts at" heading are gone. Starting values come last, each marked ↻. A name's hover lists the formulas that read it. The subsystem breadcrumb no longer says "editing changes every use". Its hover says that blocks and wires drawn inside apply to every use and values are set per use.
- **Selection.** Every selected block is shown, each with its values. A conversion block shows its `from` and `to` units, also in its group of the tree. The separate "Unit conversions" section is gone.
- **Drafts.** While canvas edits wait as a draft (the model does not load yet), blocks that are not in the running model appear in the tree as "not running yet" and in the selection, with their values. Values and units typed there or on their nodes are `SetParam` and `SetSetting` edits to the draft. They can be undone like other canvas edits.
- **Nodes.** Each node with values has a triangle in its header, with the count while closed. Clicking it opens or closes the node. Which nodes are open is not saved. Headless diagrams show every node open. Values set by formulas are not on nodes. A conversion block shows its units beside its ports (`u [mm]`, `[m] y`); double-click one, zoomed in, to change it. That edit restarts the run, as any model edit does.
- **Units** are shown with a slash: `m/s`, `kg·m/s^2`, `kg/(m·s^2)`, `1/s`. Named scaled units keep their names (`km/h`). Every shown unit reads back.
- **Smaller fixes.** The log lists oldest first and scrolls to the newest. A tab of views always stays, empty if every view has its own tab. Messages use `->` and "to" in place of `→`, which the app's font lacks.
- **Pop-out.** Popped-out tabs are drawn in the same pass as the main window. Waiting on vsync for a window the compositor is not showing (Wayland) would stall both, so presentation does not wait for vsync while any tab is popped out.

## Consequences

- The Controls can sit beside the plots, or be popped out, while the inspector shows the selection and the tree.
- Under Xvfb (X11, software rendering), the main window kept drawing with a tab popped out, more slowly. The stall itself could not be reproduced here. If it remains on the owner's desktop, the next step is deferred viewports, which draw each window on its own (an [open question](../open-questions.md)).
- Changing a conversion's unit goes through canvas edits now, so it can be undone. A unit that does not fit (a different kind of quantity) leaves the edit as a draft instead of an error.
