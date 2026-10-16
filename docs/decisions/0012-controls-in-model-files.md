# 0012 — Exploration controls are saved in model files

- **Status:** Accepted
- **Date:** 2026-10-16

## Context

[ADR 0011](0011-parameter-entry.md) made sliders opt-in per parameter with user-set ranges and left open whether those choices are saved. [ADR 0007](0007-json-model-files.md) defines the model file but says nothing about presentation. For hobbyists ([ADR 0009](0009-hobbyists-first.md)), opening a shared model with the author's controls already set up is the closest a desktop app gets to a click-to-play link (see [risks](../risks.md)).

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Not saved | Model files stay purely physical | Every reader rebuilds the controls; the author's intent ("watch rho from 0 to 30") is lost |
| Saved in the model file, in their own section | Shared files open ready to explore; presentation stays apart from the model in diffs | The file carries some non-physical data |
| Saved in a sidecar file or app settings | Model file untouched | Easily lost when sharing; two files to keep together |

## Decision

This extends ADR 0007; it does not replace it.

- Model files gain an optional top-level `controls` section, next to `blocks` and `connections`.
- `controls` holds how the model is meant to be explored:
  - per parameter (`"block.param"`): whether it has a slider and the slider's range; later, scale (linear or log) and step;
  - the default playback speed ([ADR 0010](0010-live-loop-pacing.md));
  - the outputs plotted by default.
- Results never depend on `controls`. The CLI and headless runs ignore it, and a missing or invalid `controls` section produces a warning, never a load failure.
- Controls are written only on an explicit save, never while sliders move.
- Per-person and per-machine state (window layout, plot zoom, panel sizes, recent files) is not stored in model files; it belongs in app settings.
- Current parameter values stay in `blocks[].params`, not in `controls`.

## Consequences

- The first file-format implementation includes `controls`, and the JSON Schema covers it.
- Diffs separate changes to the model from changes to its presentation.
- The parameter panel (STATUS item 3) reads and writes `controls`.
