# 0062 — Probes: what a run records, saves and replays

- **Status:** Proposed (draft for the owner; the choices marked **Owner** are open)
- **Date:** 2026-10-28
- **Settles, once accepted:** [Q38](../open-questions.md) (recording large grids), [Q39](../open-questions.md) (several cores and exact replay)

## Context

Large grids ([ADR 0060](0060-arrays-in-typed-equations.md)) showed that recording is not under anyone's control. Gray–Scott at 128 × 128 grew towards 10 GB in minutes until a fixed safety net (2026-10-28: about 256 MB each for grid history and rewind snapshots) capped it at about 510 MB. On 2026-10-28 the owner asked whether history and replay could be part of a general capability: probing and saving chosen fields, states or values at chosen times, as nodes on the canvas that fire at set intervals or times. If history and saving become the user's to control, the user should also choose between exactness and speed (Q39).

Today one mechanism does three jobs:

- **Recording for views.** Every output (or those `controls.plots` names) is recorded at every step into the history, which thins itself when long (every other sample dropped). Views and filters read the history. Headless runs (`simsandbox run`, Python) keep every output at every step, unthinned.
- **Replay state.** A kernel snapshot every 10 steps (states, tables, outputs, parameters) lets the app rewind. Running on after a rewind replays the input track exactly ([ADR 0058](0058-inputs-live-scheduled-replayed.md)).
- **Saving.** There is none beyond headless results. Run files ([ADR 0059](0059-run-files.md)) store what was done to a model, not what it produced.

Runs are deterministic: the same start, inputs and seed give the same numbers bit for bit. So anything not recorded can be recomputed from the nearest snapshot. A step's arithmetic runs on one thread, so the Gray–Scott example (2.4 ms a step, almost all cell-by-cell evaluation) uses one of this machine's 4 cores.

## Proposal

### Three roles, kept apart

1. **Replay state:** what makes rewind exact. Snapshots and the input track; nothing a user chooses.
2. **Probes:** what is recorded, when, and for how long. Views read probes.
3. **Saving:** a probe that also writes to a file.

### Probes

A probe is a node on the canvas's data layer, beside filters and views ([ADR 0050](0050-data-layer-and-canvas-undo.md)), wired from the signals and fields it records. It is saved in `controls.probes`: like filters and views, it never changes results ([ADR 0012](0012-controls-in-model-files.md)).

```json
{"name": "pattern", "record": ["gs.v"], "every": {"seconds": 50}, "keep": {"mb": 100}, "to": "gs-pattern.csv"}
```

- **What:** any signals, fields and filter outputs, by name.
- **When** (`every`): every N steps, every Δt of simulated time, or at listed times. Later, possibly when a condition holds ("when v_mean > 0.01"), in the expression language.
- **How long** (`keep`): everything, the last N samples or seconds, or a memory budget; past it, the probe thins (every other sample dropped, as now) or forgets the oldest. **Owner:** whether a probe thins or forgets by default.
- **Where** (`to`): memory (default), or memory and a file beside the model, named like run files ([ADR 0059](0059-run-files.md)). CSV first; a binary format for grids (NumPy `.npy`, later HDF5 or Parquet with Phase 3).
- **Views read probes.** A view shows what the probes have recorded of its signals. A model without probes gets one implicit probe recording what its views and filters need at every step within the app's memory budget, which is today's behaviour. `controls.plots` becomes that implicit probe's list.
- **Headless:** `simsandbox run` and Python record what the probes say: by default every output at every step, as now; `--probe NAME` or `Model.run(probes=...)` selects. A probe with a file writes it headless too, so a scripted run produces the same data files as the app.

### Rewind and replay

- **Snapshots** keep the replay state on their own budget (an app setting, default 256 MB). Past it they thin, so rewind stays possible over the whole run, coarser further back.
- **Snapshots without outputs** would be about a third the size for grids. Recomputing outputs on rewind is exact for most models but can differ in the last bits where algebraic loops are solved by iteration (ADR 0040). **Owner:** keep outputs (exact, larger) or recompute them (smaller, exact except for such loops, which would keep outputs).
- **Recompute on demand:** when a view shows a time a probe did not record (between its samples, or after thinning), the session can rerun from the nearest snapshot to fill it in, exactly. This is how "record little, see everything" works. **Owner:** whether this is part of the first build or later.

### Exact or fast (Q39)

A run is either **exact** or **fast**, set with the model's run settings (`run.mode`), shown in the toolbar and recorded in run files.

- **Exact** (default): replay, rewind and headless runs give the same bits on any computer. Cell-by-cell evaluation runs on several cores (identical in any split). Sums (`sum`, `mean`) and factorisations add in a fixed order that does not depend on the machine: work is cut into a fixed number of chunks, combined in the same order.
- **Fast:** every core, any order, the sparse LU's own parallelism. Results may differ in the last bits between runs and machines; replaying a fast run says "close, not exact".
- **Owner:** whether exact runs use several cores from the first build (the fixed-chunk split), or stay on one thread until fast mode exists.

### Memory

One app setting, the **memory budget** (default 1 GB), shared by snapshots and in-memory probes; a probe's own `keep` is a share of it. When a model would exceed it the toolbar says so and which probe is largest. The current safety net becomes this budget's default split.

## Options considered

**How recording is declared**

| Option | Pros | Cons |
| --- | --- | --- |
| **Probe nodes in the model (`controls.probes`)** | Visible on the canvas, shareable with the model, same in the app and headless; general (intervals, times, files) | A new node type and file section |
| A global recording interval and budget (app settings) | Simplest | One setting for all signals; nothing saved with the model; no files |
| Per view: each view records what it shows | No new node | Two views of one field record it twice; nothing for saving data no view shows |

**Exactness and speed**

| Option | Pros | Cons |
| --- | --- | --- |
| **Per run, exact by default, fixed-chunk parallelism for exact runs** | Exact replay everywhere by default; several cores still used; the user picks speed when exactness does not matter | Writing reductions and solvers to a fixed split takes care |
| Always one thread | Simple, exact | Leaves cores idle on large grids |
| Always fast | Fastest | Breaks exact replay, the Phase 2 gate and the verification policy's determinism rule |

**Snapshots**

| Option | Pros | Cons |
| --- | --- | --- |
| **On their own budget, thinning** (as the safety net) | Rewind over the whole run | Coarser further back |
| A fixed interval set by the user | Predictable | Runs out of memory unless the user sets it right |
| Only the last N | Bounded | Cannot rewind to the start of a long run |

## Consequences (if accepted)

- `controls.probes` in the file format (with the schema and the `upgrade` rules for older files), probe nodes on the canvas, views bound to probes, `controls.plots` folded into the implicit probe.
- The session's history becomes a set of probe recordings; headless results come from probes.
- Data files beside the model join run files; Phase 3's provenance reports build on both.
- `run.mode` in model and run files; reductions and the instruction list's cell loops split into fixed chunks for exact runs; fast runs may use `faer`'s own parallelism.
- Q38 and Q39 are settled; the safety net's constants become the memory budget's defaults.
- Order of work: probes in memory with the implicit probe (no behaviour change), then probe nodes and files, then exact multi-core, then fast mode, then recompute on demand.
- Where it goes in Phase 2: after item 2 (arrays) and before item 3 (rigid bodies), since the cart-pole gate's replay and saved data use it. **Owner:** confirm the place in the plan ([ADR 0057](0057-phase-2-plan.md)).
