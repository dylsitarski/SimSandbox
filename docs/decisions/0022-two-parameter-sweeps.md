# 0022 — Sweeps over two parameters

- **Status:** Superseded by 0028
- **Date:** 2026-10-17

## Context

[ADR 0021](0021-maps-formulas-and-sweeps.md) sweeps a model with no states across one parameter. The owner asked for sweeps across several parameters, giving contour plots or (coloured) surfaces ([Q15](../open-questions.md#resolved)). Cost grows as points^dimensions, and nothing can show a 3D grid of results well.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| One or two swept parameters, results as grid fields | Reuses grid fields and `plot.heatmap`; 100 × 100 formula evaluations are instant | Needs 2D coordinates for heat maps and new surface and contour views |
| Any number of parameters | General | No way to show 3 or more dimensions; 100³ evaluations per slider move |
| Also sweep models with states (one run per point, then a reduction) | Bifurcation diagrams, parameter studies | Needs reductions (final value, amplitude, period) and a background thread; a different feature |
| Keep one parameter | Nothing to build | Owner's request unmet |

## Decision

- `controls.sweep` sweeps **one or two** parameters, each with its own range and number of points. One parameter behaves as in ADR 0021. The one-parameter form `{"param", "range", "points"}` stays valid and is still what is saved for one parameter; two are saved as a list of two such objects.
- With two parameters, every output is recorded as a **grid field** over (first parameter along x, second along y), and the two parameters' values as coordinate grids, so views place the cells by parameter value. Any slider change recomputes the whole grid, as for one parameter.
- Views: `plot.heatmap` shows the result first (gaining x and y coordinate slots for 2D grids); a coloured **surface** view (CPU projection like `plot.3d`) follows, then **contours**.
- Three or more parameters are not supported.
- Sweeping models with states stays a separate, later feature, planned with bifurcation diagrams (ADR 0021).

## Consequences

- A 2D sweep's history has a single sample (the whole grid); the rewind bar does not apply, as for one-parameter sweeps.
- The default is 100 × 100 points; heavier formula models may need fewer, and the points field stays editable.
- Docs to update when built: [visualization](../architecture/visualization.md), model file schema, `controls.sweep` in the file format.
