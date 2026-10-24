# 0013 — Run settings are saved in a `run` section

- **Status:** Accepted
- **Date:** 2026-10-16

## Context

Results depend on the communication step `dt`, but model files ([ADR 0007](0007-json-model-files.md)) did not store it; the CLI took `--dt` and the app defaulted to 0.01 s ([open question Q7](../open-questions.md#resolved)). `controls` cannot hold it, because controls never affect results ([ADR 0012](0012-controls-in-model-files.md)). A shared file should reproduce the same run.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| A `run` section in the model file | Shared files reproduce the run; one place for future run settings | Another section to validate |
| In `controls` | No new section | Breaks the rule that `controls` never changes results |
| Per user only (CLI flags, app settings) | Model files stay purely about the model | The same file gives different results on different machines |

## Decision

- Model files gain an optional top-level `run` section: `{"dt": 0.01, "t_end": 20.0}`.
- `dt` (simulated seconds per communication step) defaults to 0.01 when absent; `t_end` is optional.
- Unlike `controls`, `run` affects results, so invalid values (non-finite or not positive) and unknown fields are load errors reported at `run.dt` or `run.t_end`.
- Saving always writes `run`, so a saved file states the step it was run with.
- The CLI uses `run.dt` and `run.t_end` (default 20 s when absent); `--dt` and `--t-end` override them for that invocation.
- The app uses `run.dt`; when `t_end` is set it pauses there, and Run continues past it. A step edited in the app is saved to `run.dt`.
- The format version stays 1: the section is optional and no released reader exists.

## Consequences

- `run` is the home for later run settings (start time, seed, solver tolerances that apply to the whole model).
- Example files carry a `run` section.
