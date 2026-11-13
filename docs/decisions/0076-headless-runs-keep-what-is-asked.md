# 0076 — Headless runs keep what is asked for

- **Status:** Accepted
- **Date:** 2026-11-06

## Context

`simsandbox run`, the Python `run` and runs of models that use models keep every output of every block at every step until the run ends. For large grids this dominates memory and costs time: a 128 × 128 Gray–Scott run to t = 600 held 392 MB and spent about a fifth of its time in the system, faulting in fresh memory ([performance](../design/performance.md)). Often only a few signals, or a coarser time step, are wanted. Probes ([ADR 0062](0062-probes-recording-and-replay.md)) already choose what to record, but they live in the model file and their output is a probe's, not `run`'s.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| **Choose what a run keeps when running it: outputs by name, every few steps** | Same results format, the chosen columns and rows only, the same bits; no file edit; works for every model | Two more options on `run` |
| Write CSV rows as they come instead of keeping them | No memory for the default CSV output | Only for CSV; JSON and Python need everything at the end; a failing run leaves half a table on standard output |
| Default to less (e.g. the outputs views show) | Gains without asking | Changes what `run` prints for every model; headless files have no views |
| Probes only | Already built | Must be written into the model; one probe per question |

## Decision

A run takes a **keep** setting (`simsandbox_session::run::Keep`):

- **Signals:** a list of names. Each keeps the outputs it names: `block.port`, `block` (all its ports), and in a model that uses models a model's name (all its outputs; `.` the top level's own blocks) or a path in one (`plant/car`, `plant/car.v`). A name that keeps nothing is an error listing the outputs. Absent: every output, as before.
- **Every:** keep every n-th step from the first, and always the last. In a model that uses models, each model counts its own steps (or evaluations, for one on new data). Default 1: every step.
- What is kept is the same values, bit for bit, as the same rows and outputs of a whole run. Default runs are unchanged.
- **CLI:** `simsandbox run FILE --signals a.x,b --every N`. With models on their own clocks, `--model NAME` (CSV of one model) now records only that model unless `--signals` is given. `--probe` and `--save-probes` refuse `--signals` and `--every` (a probe records what it is set to).
- **Python:** `Model.run(..., signals=[...], every=N)`, for single models and models that use models.
- A model whose only runs are its studies keeps a row per run; `signals` chooses its outputs, `every` does not apply.

## Consequences

- The 128 × 128 grid keeping one summary signal runs in 0.80 s instead of 1.04 s and holds 17 MB instead of 392 MB; two grids keeping one signal each 0.29 s instead of 0.39 s, 28 MB instead of 279 MB.
- Streaming CSV rows was not done: keeping less covers the same need for every format.
- `profile_run` takes `--signals` and `--every` too.

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none; signals and steps are existing concepts, chosen per run.
