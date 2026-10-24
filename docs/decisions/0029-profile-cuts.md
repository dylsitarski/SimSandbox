# 0029 — Profiles as cuts in any direction

- **Status:** Superseded by 0031
- **Date:** 2026-10-18

## Context

[ADR 0026](0026-profile-slices.md) draws a 2D grid's profile as slices along one chosen axis. The owner asked for cuts not tied to one axis. Curves in one plot share a horizontal axis, and cuts along x and along y run over different quantities (in a sweep, `w` and `c` even have different units), so the horizontal axis needs deciding too.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Straight cuts between two points, horizontal axis chosen per view | Any direction; axis-aligned slices are a special case; one view for a family of cuts | Interpolation between cells; the shared axis must be chosen |
| Slices along both axes, one tile per direction | No interpolation | No oblique cuts; more tiles |
| Keep ADR 0026 | Built soonest | Owner's request unmet |

## Decision

- A **cut** is a straight segment between two points in the grid's coordinates, e.g. from (w = 0, c = 0.1) to (w = 3, c = 0.8). Each cut draws one curve; cuts in one view may run in any directions. Axis-aligned slices are cuts across the whole grid at a fixed value. Polylines may come later.
- The **horizontal axis** is chosen per view and saved:
  - **fraction** along the cut, 0 to 1 (default; works for any cuts);
  - the **x** or **y** coordinate, offered only when every cut runs monotonically in it;
  - **distance** along the cut, offered only when both coordinates share a unit (e.g. metres on a spatial grid).
- **Interpolation:** values between cell centres are interpolated, an approximation with a method choice ([ADR 0020](0020-methods-for-approximations.md)): `nearest` or `bilinear` (default), shown on the plot. Points per cut default to the grid's resolution along the cut; a cut lying on a row or column reads it exactly.
- **Saved** as the view's `selection`, e.g. `{"cuts": [{"from": [0, 0.1], "to": [3, 0.8]}], "against": "fraction", "interpolation": "bilinear"}`. Default for a new view: four horizontal cuts spread over y. Edited as endpoints in the side panel; later by dragging a line on a linked heat map.
- Line fields are drawn as before. Cuts apply to grids, including grid sweeps ([ADR 0028](0028-sweeps-as-samples.md)); cuts through unstructured samples wait for triangulation.
- Supersedes ADR 0026.

## Consequences

- `ViewEntry` gains an optional `selection` object (as 0026 planned).
- Interpolation is labelled like other approximations; the verification policy applies (a test against a known field, e.g. a linear field reproduced exactly by bilinear interpolation).
