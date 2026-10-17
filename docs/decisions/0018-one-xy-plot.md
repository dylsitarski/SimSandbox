# 0018 — One x–y plot replaces the time-series and phase-portrait views

- **Status:** Accepted
- **Date:** 2026-10-17

## Context

`plot.time` and `plot.phase` ([ADR 0015](0015-pluggable-views.md)) are the same picture with different axes: a time series is an x–y plot with `t` on the horizontal axis. The owner proposed merging them ([open question Q13](../open-questions.md#resolved)).

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Keep two view types | No migration | Two codes paths for one picture; a time plot cannot use trail length, a phase plot cannot show several curves |
| One `plot.xy` | Any signal on either axis; trail length on a time plot gives a scrolling window; several curves in a phase plot | Files must be converted; the phase portrait is less visible as its own entry |

## Decision

- One view type, `plot.xy` ("Plot"): slot `x` (one signal, default `t`), slot `y` (one or more signals), optional `colour` slot, and the `trail_length` style property.
- With one `y` signal the colour slot applies (fade by `t`, or colour by a signal); with several, each curve gets its own colour and a legend.
- Trail length applies on any axis; with `x = t` it shows the last N seconds as a scrolling window.
- View types offer **presets**: named starting bindings shown in "More views…". `plot.xy` has "Plot against time" (the default) and "Phase portrait"; `plot.3d` has "3D trajectory". Presets replace a type's single set of default bindings.
- Files: `plot.time` and `plot.phase` entries are converted to `plot.xy` on load, keeping their bindings and style; saving writes `plot.xy`. The format version stays 1.

## Consequences

- One 2D plot to maintain and extend (derived signals, step 4).
- `--view time` and `--view phase` select the matching `plot.xy` preset.
