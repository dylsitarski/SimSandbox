# 0027 — Explicit experiments: sampling, run and reduction

- **Status:** Accepted
- **Date:** 2026-10-18

## Context

Whether a model runs in time or is swept is decided implicitly for the whole model: the app sweeps it when no block has states ([ADR 0021](0021-maps-formulas-and-sweeps.md)). Blocks only declare their time semantics. Sweeping models that have states (bifurcation diagrams, parameter studies) does not fit: it needs a run at each sample and a way to reduce the run to values. The sweep lives in `controls`, which by [ADR 0012](0012-controls-in-model-files.md) never changes results, yet batch runs need it ([Q21](../open-questions.md#resolved)).

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| An explicit experiment: sampling (outer) × run in time (inner) × reduction | One structure for runs, sweeps and parameter studies; batch runs can do sweeps | New file section; conversion of `controls.sweep` |
| Keep deciding from the model | Nothing to change | No place for sweeps of models with states |

## Decision

- An **experiment** belongs to the model file, at the top level next to `run`: an optional **sampling** of parameters ([ADR 0028](0028-sweeps-as-samples.md)), the **run** in time (the existing `run` section, [ADR 0013](0013-run-section.md)), and for sampled models with states a **reduction** per output that turns each run into values (final value, mean, minimum, maximum, amplitude, period; more as needed).
- **Defaults keep today's behaviour:** a model with states and no sampling runs in time; a model without states is sampled over its first parameter with a trivial run (formulas evaluated once per sample).
- The decision is made at the **model** level. Blocks keep declaring only their time semantics; a model mixing stateless and dynamic blocks runs in time, evaluating the stateless ones every step.
- `controls.sweep` moves to the experiment's sampling; files with `controls.sweep` are read and converted. `simsandbox run` performs the experiment, writing one row per sample for sampled models.
- Sampling models with states (reductions, background computation) is built later; this ADR fixes the structure so it fits.

## Consequences

- The file format gains a section (name and schema settled when built, e.g. `"sample"` beside `"run"`); [ADR 0021](0021-maps-formulas-and-sweeps.md)'s rule that a model without states is swept becomes the default of this structure.
- The app's sweep bar edits the experiment's sampling; the CLI gains sweeps.
