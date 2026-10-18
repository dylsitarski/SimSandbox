# 0026 — Profiles of 2D grids as slices

- **Status:** Accepted
- **Date:** 2026-10-18

## Context

`plot.profile` draws the newest frame of a field as one line. For a 2D grid it runs through every cell row by row, a meaningless zigzag. The useful picture is a family of curves, e.g. resonance curves (amplitude against `w`) at several dampings ([Q20](../open-questions.md#resolved)).

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Slices: curves along one axis at chosen values of the other | Classic family-of-curves plot; works for any 2D grid | Needs view state beyond numeric style values |
| Line fields only | Nothing to build | 2D results cannot be cut |

## Decision

- For a 2D grid, `plot.profile` draws curves **along** one axis (`x` or `y`) at **slices** of the other, given as coordinate values; each slice uses the nearest row or column and the legend shows its actual value. By default four slices are spread evenly over the other axis.
- The axis and slice values are saved with the view as a `selection` (e.g. `"selection": {"along": "x", "at": [0.05, 0.1, 0.2, 0.5]}`), next to `bind`, `style` and `camera`, and edited in the side panel.
- A line field is drawn as now. A 2D grid is never drawn as one line through every cell.
- Later, clicking on a heat map of the same field may set a slice (linked views).

## Consequences

- `ViewEntry` gains an optional `selection` object; `ViewState` keeps it and views read it.
- Uses the coordinate fields of [ADR 0024](0024-heat-map-axes-and-coordinates.md) for slice values.
