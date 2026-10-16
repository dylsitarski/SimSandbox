# 0011 — Typed parameter entry; ranges are suggestions

- **Status:** Accepted
- **Date:** 2026-10-16

## Context

`ParamSpec` carried `min` and `max`, and earlier on 2026-10-16 `set_param` began rejecting values outside them. The owner pointed out that this removes flexibility: the range only needs to be reasonable for a slider, and users should be able to try any value. Sliders also mainly make sense for real-time models, where the result reacts while the slider moves.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Hard range limits | Guards against nonsense values | Blocks legitimate exploration beyond the author's guess; the range has to serve two purposes |
| Suggested range, typed entry by default, optional slider | Any value can be tried; slider ranges fit the user's current exploration | Blocks must cope with unusual values themselves |

## Decision

- `ParamSpec` has `suggested_min` and `suggested_max`: a hint used as the initial slider range, never a limit.
- `set_param` accepts any finite value and rejects only NaN and infinities (`SimError::InvalidParam`). A Block whose maths needs a narrower domain reports that itself through `SimError::Block`.
- The generated parameter panel shows a typed entry field, with units, for every parameter by default.
- A slider is opt-in per parameter, with a range the user sets (initially the suggested range); a typed value outside the slider range is kept and the range widens to include it. Sliders are aimed at real-time models.

## Consequences

- Model files ([ADR 0007](0007-json-model-files.md)) accept any finite parameter value; slider choices and ranges are UI state that may be saved with the model later.
- Supersedes the hard-range rule recorded in the [2026-10-16 session note](../sessions/2026-10-16-kernel-prep-and-adrs.md), which was not an ADR.
