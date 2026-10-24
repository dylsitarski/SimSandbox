# 0014 — The opening view is saved in `controls`

- **Status:** Accepted
- **Date:** 2026-10-16

## Context

The app shows a time series, a phase portrait or a 3D trajectory. Whether a model file should say which one it opens in was [open question Q8](../open-questions.md#resolved). The view is presentation, not results, so it fits the rules of [ADR 0012](0012-controls-in-model-files.md); for hobbyists ([ADR 0009](0009-hobbyists-first.md)), Lorenz opening straight into its 3D butterfly is the point of the example.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Save the view in `controls` | Shared files open as the author intended | One more section to validate |
| Always open on the time series | Nothing to store | Every reader has to find the interesting view themselves |

## Decision

- `controls.view` = `{"kind": "time" | "phase" | "3d", "axes": [...], "camera": {"yaw", "pitch", "zoom"}}`.
- Axes name output elements: `"block.port"` for a scalar output, `"block.port[i]"` for an element of a vector output; two axes for `phase`, three for `3d`, none for `time`. `camera` applies to `3d` only.
- Like the rest of `controls`, an invalid view is dropped with a warning and never fails a load or affects the other controls. An axis that is valid but not among the plotted outputs makes the app open on the time series with a warning.
- Save writes the view currently shown, with the 3D camera. A `--view` flag on the app overrides the file.

## Consequences

- The Lorenz example opens in 3D.
- When the app gains several views at once (open question Q9), this single `view` becomes a list; the format version is still 1 with no released reader, so that change can be made without a migration.
