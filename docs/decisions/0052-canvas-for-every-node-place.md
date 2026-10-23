# 0052 — `canvas` for every node's place

- **Status:** Accepted
- **Date:** 2026-10-23

## Context

Blocks and subsystem ports kept their place on the node canvas as `at` ([ADR 0042](0042-node-canvas.md), [ADR 0043](0043-subsystems.md)). Filters and views got `canvas` instead ([ADR 0050](0050-data-layer-and-canvas-undo.md)), because cut filters already use `at` for where they cut. The owner asked (2026-10-23) for one name for every node's place.

## Decision

- Blocks and subsystem inputs and outputs write their place as `canvas: [x, y]`, like filters and views; the Rust fields are `canvas` too.
- The older `at` is still read (ADR 0044's rule: converted on reading, reported in `conversions`, written in the new form); a fixture (`crates/simsandbox-file/tests/old/adr-0052-places-at.json`) pins it.
- Python's `Model.add_block` takes `canvas=(x, y)`.

## Consequences

- One spelling for places across the file; cut filters' `at` keeps its own meaning.
- Files saved since ADR 0042 convert silently in meaning and visibly in the log.
