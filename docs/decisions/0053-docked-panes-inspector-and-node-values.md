# 0053 — Docked panes, an inspector, and values on nodes

- **Status:** Accepted; the Controls, pins and values on nodes are amended by [0054](0054-controls-pane-pins-and-node-toggles.md)
- **Date:** 2026-10-23

## Context

The app had a fixed layout: a parameter panel on the left and a central area switching between Views (all plots tiled in a grid) and Diagram. On larger models, the platoon first, the panel grew long and repetitive: subsystem uses repeat their parameters, values set by formulas are listed beside the ones that set them, and solving and view style sit among parameters. Plots could not be arranged individually, and the diagram and the plots could not be seen together. The owner asked (2026-10-23) for tabs that can be moved, split, tiled and popped out, plots as their own windows or tiled together, and considered parameters on the canvas nodes.

## Options considered

| Question | Options | Chosen |
| --- | --- | --- |
| Arranging panes | Fixed layout; **dockable tabs** (`egui_dock`) | Dockable: drag, stack, split; a view alone or several tiled in one tab; pop out into windows of their own |
| Popping out | egui's in-app windows (what `egui_dock` offers); **native windows** (egui viewports) | Native windows, so a plot can go on a second screen; egui shows them inside the app where native windows are not available |
| Where the arrangement is saved | App settings; **the model file** | The file (owner): a curated model's presentation is part of it ([ADR 0016](0016-model-curated-views.md)); window positions on screen stay with the machine |
| Parameters | One long panel; on nodes only; **inspector plus values on nodes** | An inspector for the model's own parameters and the selection; nodes show key values, editable in place when zoomed in (owner) |

## Decision

- **Docking.** The inspector, the diagram and the views are tabs in a dock. A view is shown alone in a tab or tiled with others in a "Views" tab ("own tab" and "tile" on each view; "Tile all views here" on a Views tab). Tabs are dragged to stack or split; "Pop out" (a tab's context menu) shows one in a native window, docked back by "Dock" or by closing the window. Closing a view's own tab closes the view; the inspector, the diagram and Views tabs stay. "More views…" and "Data…" are in the toolbar. Popped-out tabs stay popped out when the model is rebuilt by a canvas edit.
- **The arrangement in the file.** `controls.layout` (optional, additive under [ADR 0044](0044-unsaved-changes-and-older-files.md)): splits (`left_right`, `top_bottom`, a `fraction` for the first part) of stacks of `tabs` with an `active` one; a tab is `"inspector"`, `"diagram"`, `"view:K"` (a view of `controls.views`) or `{"views": [K, …]}`. Lenient like all of `controls`: unknown tabs and missing views are dropped with a warning, empty parts collapse, views not placed join the first Views tab, and the inspector and diagram are added if missing. The standard arrangement (inspector left, views and diagram right) is not written. Popped-out tabs are saved as tabs of the first stack.
- **Headless** ([ADR 0039](0039-ai-agent-access.md)): `simsandbox render` and `Model.render` draw the views as the arrangement places them, each stack's front tab, with parts that show no views (inspector, diagram) giving their space to the rest; `--view N` still draws one view.
- **The inspector** (next): the model's own parameters and pinned controls, then everything about the selected node (from the diagram or a plot): each value once, where it is set (values set by formulas only noted at their source), a table of a subsystem's uses, initial values grouped, with a search box. Solving moves out of it.
- **Values on nodes** (after): nodes show key parameter values, editable in place when zoomed in, hidden when zoomed out, with a switch to hide them.

## Consequences

- The app depends on `egui_dock` (0.21.1, built against egui 0.36.2); egui and egui_dock are upgraded together.
- Native windows behave differently on Windows, X11 and Wayland; they were checked here only in tests and an X11 virtual display, and are for the owner to confirm.
- Every tab change counts as an unsaved change, since the arrangement is saved.
