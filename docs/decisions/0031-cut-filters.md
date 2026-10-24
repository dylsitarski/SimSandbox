# 0031 — Cuts as filters with simple modes

- **Status:** Accepted
- **Date:** 2026-10-18

## Context

[ADR 0029](0029-profile-cuts.md) made profile cuts two end points per cut with a chosen horizontal axis. In use it is confusing: most cuts hold one coordinate constant, yet each needs four numbers, and the rule for when x or y can be the horizontal axis is invisible. The owner proposed choosing per coordinate "constant" or "free", and a "whole field" toggle so a constant-coordinate cut is one number ([Q23](../open-questions.md#resolved)). Under [ADR 0030](0030-filters-and-native-visualization.md) cutting is a filter, not part of the profile view.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| One mode per cut filter: along x at fixed y, along y at fixed x, or free lines | Common case is one number per cut; the horizontal axis follows from the mode | Mixed families need two filters |
| Per cut, each coordinate constant or free, with a whole-field toggle | Flexible per cut | Cuts in one plot can lose a common horizontal axis |
| Keep ADR 0029's end points | Built | Confusing |

## Decision

- A **cut filter** takes a 2D grid and makes one line field per cut, each with its coordinate, so profiles and x–y plots show them like any line field.
- Each cut filter has one **mode**:
  - **along x at fixed y:** each cut is one y value; it samples the whole field across x by default, or a chosen x range when "whole field" is off; its coordinate is x;
  - **along y at fixed x:** the same the other way round;
  - **free lines:** each cut is two end points; with "whole field" on (default), the line is extended to the edges of the grid; its coordinate is the fraction along the line, or the distance when both directions share a unit.
- Interpolation between cells stays a method choice, `bilinear` (default) or `nearest`, as in ADR 0029; a fixed value on a row or column reads it exactly.
- Mixing modes means two filters; their line fields can share a view when their coordinates agree.
- Cuts are edited in the filter's settings; later, clicking a heat map of the same grid adds a cut in the filter's mode.
- Supersedes ADR 0029 (its interpolation choice is kept).

## Consequences

- The profile view loses its cut code and selection; saved profile selections from ADR 0029 are converted into a cut filter on load.
- Filter outputs need names (e.g. `<filter>[k]`) and legend labels that show the cut's value (e.g. `spring.c = 0.05`); settled when built.
