# 0028 — Sweeps recorded as samples

- **Status:** Accepted
- **Date:** 2026-10-18

## Context

[ADR 0022](0022-two-parameter-sweeps.md) records a two-parameter sweep as grid fields, and a one-parameter sweep indexes signals by the swept parameter (standing in for `t`). Neither fits sampling that is not a grid (random, Latin hypercube, Sobol, a list of points, adaptive refinement), nor more than two parameters. The owner asked how unstructured sweeps would work ([Q22](../open-questions.md#resolved)).

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Every sweep is a list of samples; parameters are signals; grid structure is optional metadata | One representation for every design and any number of parameters; scalar views work at once | Grid views must reshape signals; changes 0022's storage |
| Grids for structured sweeps, something else for unstructured | Grid views stay simple | Two representations, two sets of views |
| Grids only | Already built | No unstructured or higher-dimensional sweeps |

## Decision

- A sweep records **samples**: the index is the sample number; every swept parameter is recorded as a scalar signal under its key (`spring.w`), beside the outputs and derived signals. Views bind parameters by name; `t` no longer stands for a swept parameter (files that bind `t` in a sweep are converted to the first swept parameter).
- **Designs:** `grid` (one or more parameters, each with range and points, evenly spaced), `list` (given points), `random` (uniform, with a saved seed), `latin_hypercube` and `sobol`. Any number of parameters; at most 250 000 samples in total.
- **Structure is metadata:** a `grid` design states its shape, and the history offers each signal reshaped as a grid with the parameters as coordinates ([ADR 0024](0024-heat-map-axes-and-coordinates.md)), so heat maps, surfaces, contours and profile slices ([ADR 0026](0026-profile-slices.md)) work on structured sweeps.
- **Unstructured samples** are shown as points: `plot.xy` with a parameter on each axis and colour bound to an output, drawing markers instead of lines (a style property that applies to sampled histories, [ADR 0025](0025-index-aware-view-settings.md)); triangulated contours later.
- This **supersedes [ADR 0022](0022-two-parameter-sweeps.md)'s storage** (grid fields). What 0022 decided otherwise still holds as the `grid` design: two parameters give a grid, shown by heat map, then a surface, then contours.

## Consequences

- `Session` records sweeps as samples; the two-parameter code path of 0022 becomes a reshaping of sample signals.
- One-parameter sweeps plot against the parameter by name; `Available::sweep` gives way to the grid metadata.
- Sampling belongs to the experiment of [ADR 0027](0027-explicit-experiments.md).
