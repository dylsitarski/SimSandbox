# 0024 — Heat map axes and coordinate fields

- **Status:** Accepted
- **Date:** 2026-10-18

## Context

`plot.heatmap` grew three ways of placing cells: `position` for a line of cells over time, `x` and `y` for 2D grids, and a `time_axis` setting to flip a space–time picture. Any grid of the right shape can be bound as a coordinate; one that does not increase along the axis silently falls back to cell numbers, and one that increases unevenly (`spring.lag`) is drawn as if evenly spaced, which misplaces the cells. Axes cannot be swapped for 2D grids ([Q18](../open-questions.md#resolved)).

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| `x` and `y` slots that each take the index or a coordinate field; coordinates a declared role | One way to place cells; flipping is swapping bindings; pickers can offer only what works | New slot kind; blocks must declare coordinates |
| Keep `position`, `x`, `y` and add a flip setting | Small change | Three overlapping slots; still accepts grids that cannot place cells |
| A separate space–time view | Simple per view | Two views for one idea: a field drawn over two axes |

## Decision

- `plot.heatmap` draws a field over two axes, with slots `field`, `x` and `y`. Each axis slot takes either the **index** (`t`: time, or later the sample axis) or a **coordinate field**. A line of cells over time is `x = t`, `y = rod.position`; time upwards is the same with the bindings swapped. `position` and `time_axis` are removed; files using them are converted on load (`position` becomes `y` with `x = t`, or `x` with `y = t` when `time_axis` was 1).
- **Coordinates are a declared role.** An output port may declare that it gives the positions of the cells of another output of the same block along one grid axis (e.g. `rod.position` along the only axis of `rod.temperature`). The swept parameters of a structured sweep are coordinates along their sweep axes ([ADR 0028](0028-sweeps-as-samples.md)). Coordinates must be monotonic along their axis; uneven spacing is drawn correctly, each cell reaching halfway to its neighbours' centres.
- **Pickers offer only what fits:** an axis slot lists the index (where the field varies over it) and the coordinates of that field along a free axis. A file binding anything else is dropped with a warning naming why, never a silent fallback.
- A **swap axes** button in the view swaps the `x` and `y` bindings; for a 2D grid the picture transposes.

## Consequences

- The kernel's `PortSpec` gains an optional coordinate declaration; `eq.heat_rod` declares `position`. The view contract gains an axis slot kind accepting the index or coordinates.
- Heat maps of uneven coordinates are drawn as meshes of rectangles (or resampled textures) instead of one evenly stretched image.
- [Visualization](../architecture/visualization.md) and [block contract](../architecture/block-contract.md) are updated when this is built.
