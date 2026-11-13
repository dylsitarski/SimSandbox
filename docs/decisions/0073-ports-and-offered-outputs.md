# 0073 — One place to offer outputs, one list of a used model's ports

- **Status:** Accepted; amended by [0074](0074-model-interfaces.md) (toggles replace the block-menu offer and the port list)
- **Date:** 2026-11-05
- **Amends:** [ADR 0066](0066-models-nest-as-blocks.md) step 4b (declared outputs and "Show a port") as the app presents them

## Context

A model declares outputs (`outputs`, ADR 0066 step 4b): names that models using it read and wire to, kept valid when its insides change. In the app this showed in two places that did not explain each other (owner, 2026-11-05):
- every block's context menu offered "Offer x as an output", even in a model nothing uses, where it changes nothing visible, and listed a line per output of the block;
- a used model's node offered "Show a port", a search over every inner signal by path, listing `car.v` beside the declared output `v` that shows the same signal, so the same value could be put on the node twice. Its arrow marks did not draw in the app's font.

## Options considered

| Where outputs are offered | Pros | Cons |
| --- | --- | --- |
| **The model's own place: the Inspector's Model section; block menus only inside a used model** | Offered where the model's interface is described; the block menu says it only where it means something | One more section in the Inspector |
| Every block's menu, as now | Close to the signal | Clutter; unclear in a model nothing uses |

| A used model's ports | Pros | Cons |
| --- | --- | --- |
| **Declared outputs always on the node; the list holds only signals no declared output shows** | One port per value; the declared name wins | A signal behind a declared output is reached by that name only |
| Every signal listed, declared ones marked | Complete | Two ports for one value |

## Decision

1. **Offering outputs.** A model's offered outputs are listed and edited in the Inspector's Model section ("Offered to models using this one": name, the signal it shows, remove; add one by picking a block output). Block context menus offer an output only on the canvas of a used model's inside (open or expanded), worded "Offer x to models using this one".
2. **A used model's ports.** Its input blocks and declared outputs are always on its node. "Show a port" lists only the inner signals no declared output already shows, inputs and outputs marked in words (`in`, `out`), with units.
3. Nothing changes in the file format: `outputs` and wiring by path are as ADR 0066 step 4b built them.

## Consequences

- The Platoon's followers offer `v`; their nodes show `v` and no longer list `car.v`.
- Ports shown from the list are still forgotten on reload (Known limitations).

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none.
