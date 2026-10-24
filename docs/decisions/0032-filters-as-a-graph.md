# 0032 — Filters stored as a graph, shown on the node canvas later

- **Status:** Accepted
- **Date:** 2026-10-18

## Context

[ADR 0030](0030-filters-and-native-visualization.md) put a filter layer between recorded data and views, as "ordered entries" shown as "a list, not a node graph". The owner asked whether filters, and even views, would work better on the node canvas planned for Phase 1 ([Q25](../open-questions.md#resolved)). A canvas gives one mental model (Simulink wires signals into scope blocks) and shows chains of filters at a glance; the risks are observers silently changing results, wires that mean different things (a model wire carries the current value each step, a filter reads recorded history), and clutter. The canvas does not exist yet, so filters must not wait for it.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Store filters as a graph now; list UI first; canvas display later as a distinct, hideable observer layer, with views as sinks linked to their tiles | Nothing redone when the canvas lands; chains explicit; one model of data flow | The canvas display is deferred |
| A list only (ADR 0030 as written) | Simplest | Order-dependent entries do not map cleanly onto a canvas; chains stay hidden |
| Canvas only | One editor | Blocks filters on Phase 1; heavy for one-off plots |

## Decision

- **Filters form a graph.** Each filter has a unique name, a type, settings, and **names its inputs** explicitly: recorded signals or fields, or other filters' outputs (e.g. `cut1[0]`). The order of filters in the file carries no meaning; the app evaluates them in dependency order. A cycle or a missing input drops the filters involved with a warning, like other bad controls ([ADR 0012](0012-controls-in-model-files.md)).
- **Views are sinks of that graph.** Their bindings already name their inputs; nothing changes in how views are saved.
- **The UI starts as a list** (the "Data" panel of ADR 0030), each filter showing its inputs. **When the node canvas exists**, filters appear on it as **observer nodes** and views as **sink nodes** linked to their tiles (block diagram and front panel, as in LabVIEW or Simulink). The list and the canvas show the same graph.
- **Observers are visibly distinct and on a layer that can be hidden:** their own colour and shape, and wires that read recorded data drawn differently from model wires that carry current values each step.
- **Observers never feed the model.** Wiring an observer into a model block's input is refused; to use a transformation inside the model it is **promoted to a model block** (e.g. a derived expression becomes an `eq.equations` formula block), which then steps with the model and carries a method label.
- **Quick paths stay:** plotting an output from its context menu, presets under "More views…" and curated default views ([ADR 0016](0016-model-curated-views.md)) create views without wiring.
- Supersedes [ADR 0030](0030-filters-and-native-visualization.md) in two points only — filters are "ordered entries" and the UI is "a list, not a node graph". Everything else in ADR 0030 stands: sources → filters → views, views only draw, filters live in `controls` and never change results, approximating filters have method choices, native visualization grows with need including 3D rendering, and export complements it.

## Consequences

- The filter file format gives every filter `name`, `type`, `inputs` and settings; converted derived signals name their inputs from their expressions.
- Whether hobbyists prefer wiring observers or the list for simple cases is judged once both exist; both stay available.
- The node canvas design (Phase 1) must include the observer layer, distinct wire styles and the refusal of observer-to-model wires.
