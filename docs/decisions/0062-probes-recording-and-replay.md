# 0062 — Probes: what a run records, saves and replays

- **Status:** Accepted
- **Date:** 2026-10-28 (proposed), 2026-10-29 (accepted)
- **Amends:** [ADR 0057](0057-phase-2-plan.md) (a new item after item 2)
- **Settles:** [Q38](../open-questions.md) (recording large grids), [Q39](../open-questions.md) (several cores and exact replay)

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
- **How long** (`keep`): everything, the last N samples or seconds, or a memory budget; past it, the probe thins (every other sample dropped, as now) or forgets the oldest. **Decided:** a probe thins by default; forgetting the oldest is the other choice of `keep`.
- **Where** (`to`): memory (default), or memory and a file beside the model, named like run files ([ADR 0059](0059-run-files.md)). CSV first; a binary format for grids (NumPy `.npy`, later HDF5 or Parquet with Phase 3).
- **Views read probes.** A view shows what the probes have recorded of its signals. A model without probes gets one implicit probe recording what its views and filters need at every step within the app's memory budget, which is today's behaviour. `controls.plots` becomes that implicit probe's list.
- **Headless:** `simsandbox run` and Python record what the probes say: by default every output at every step, as now; `--probe NAME` or `Model.run(probes=...)` selects. A probe with a file writes it headless too, so a scripted run produces the same data files as the app.

### Rewind and replay

- **Snapshots** keep the replay state on their own budget (an app setting, default 256 MB). Past it they thin, so rewind stays possible over the whole run, coarser further back.
- **Snapshots without outputs** would be about a third the size for grids. Recomputing outputs on rewind is exact for most models but can differ in the last bits where algebraic loops are solved by iteration (ADR 0040). **Decided:** snapshots keep outputs, so rewind is exact for every model. (Amended 2026-10-29 by [ADR 0063](0063-probes-on-wires.md): outputs that depend only on a Block's state and parameters, and recompute bit for bit, are left out and recomputed; all others, loops included, are kept.)
- **Recompute on demand:** when a view shows a time a probe did not record (between its samples, or after thinning), the session can rerun from the nearest snapshot to fill it in, exactly. This is how "record little, see everything" works. **Decided:** later, after the rest of this ADR is built.

### Exact or fast (Q39)

A run is either **exact** or **fast**, set with the model's run settings (`run.mode`), shown in the toolbar and recorded in run files.

- **Exact** (default): replay, rewind and headless runs give the same bits on any computer. Cell-by-cell evaluation runs on several cores (identical in any split). Sums (`sum`, `mean`) and factorisations add in a fixed order that does not depend on the machine: work is cut into a fixed number of chunks, combined in the same order.
- **Fast:** every core, any order, the sparse LU's own parallelism. Results may differ in the last bits between runs and machines; replaying a fast run says "close, not exact".
- **Decided:** exact runs use several cores from the first build (the fixed-chunk split).

### Memory

One app setting, the **memory budget** (default 1 GB), shared by snapshots and in-memory probes; a probe's own `keep` is a share of it. When a model would exceed it the toolbar says so and which probe is largest. The current safety net becomes this budget's default split.

## Decision

The owner accepted the proposal on 2026-10-29 with these choices: probes thin by default; snapshots keep outputs; recompute on demand comes later; exact runs use several cores from the start; probes are built after item 2 (arrays) and before item 3 (rigid bodies).

## As built

**Step 1 (2026-10-29): probes in memory.** `crates/simsandbox-session/src/probe.rs`: `Every` (`Steps`, `Seconds`, `Times`), `Keep` (`Thin { mb }`, the default; `Last { samples }`; `All`), `ProbeSpec` with its checks, and the trigger that decides when a probe records. It follows rewinds: after rewinding to t, it fires again exactly where it did. The session keeps its history as the implicit probe, unchanged, and holds further probes (`Session::set_probes`, `probe_names`, `probe`).
- **What they record:** a subset of what the implicit probe records, signals and grid fields by name (with their coordinates when recorded too), taken from the same samples. A probe's values are the implicit history's, bit for bit.
- **When they reset:** they clear on restart, truncate and record the same samples again after a rewind, and start afresh when filters change, dropping (with a warning) names no longer recorded.
- **Where they don't record:** probes record in time only, not in sweeps.
- **`History` gains:** `thin_past`, `keep_last` and `values_per_sample`.
- **Not yet:** headless results and views are unchanged; they read the implicit probe as before. Probes reach the file, the canvas, the CLI and Python in step 2.

**Step 2, first part (2026-10-29): the file, files and headless.** The canvas nodes and the app follow in the second part.
- **In the file:** `controls.probes` entries (`ProbeEntry`): `name`, `record`, `every` (one of `steps`, `seconds`, `times`; every step if absent), `keep` (at most one of `mb`, `last`, `all`; thinned if absent), `to` and `canvas`. Like other controls they never stop a file loading: an entry with bad settings or a name used twice is dropped with a warning (`controls.probes[k]: …; ignored`), and so is one that records something the session does not record. Saving writes the probes back with defaults left out; a swept model keeps the file's probes as they are.
- **Files:** `to` is a plain file name ending in `.csv`, written beside the model: `t` and the probe's signals, one row per sample. Each grid or point field goes to `<stem>.<field>.npy` beside it (NumPy format 1.0, little-endian `f64`), shaped `(samples, ny, nx)` or `(samples, count, dim)`; populations are not written (a note says so). HDF5 or Parquet wait for Phase 3.
- **Headless:** `run::run_probes` runs a model through a `Session`, as the app does, so a probe's samples are the app's, bit for bit (tested against `simsandbox run`). `simsandbox run FILE --probe NAME` prints that probe (CSV of its signals, or JSON with its fields), and `--save-probes DIR` writes the files of the probes that have one into `DIR`. Python: `Model.add_probe(name, record, every=, keep=, to=)` and `Model.run_probes(t_end, dt, inputs, probes=, save=)`, returning `ProbeRun` and `Probe` (numpy arrays).
- **Step 2, second part (2026-10-29): the canvas and the app.** Probes are `probe:NAME` nodes on the data layer, in the views' column, with one input, `record`, taking any number of signals (`observers.rs`). "Add probe" in the canvas menu adds one recording nothing yet; wiring an output (or a filter's) into `record` adds it, deleting a wire removes it; renaming a filter renames it in probes, deleting one drops it from them. Double-clicking opens the probe's settings (every, keep, file; `Edit::SetProbe`). These edits, and undo, apply to the running model without a restart: a probe whose recording is unchanged keeps its samples, a new or changed one records from the current time on (`Session::set_probe_entries`, `place_probe`). Runs › "Save probe data" writes the files of probes that have one beside the saved model, as recorded so far. A view entry may name a `probe`; its tile draws what that probe recorded, chosen with the tile's "from" menu, also in `simsandbox render`. A probe may record nothing (an empty `record`), so one just added survives saving.
- **Differences from the proposal:** files are written only where asked (`--save-probes DIR`, `save=`), never as a side effect of `simsandbox run`. A probe records only what the implicit probe records: with `controls.plots` set, the outputs listed there and the filters.

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
- Order of work: (1) probes in memory with the implicit probe (no behaviour change); (2) probe nodes on the canvas and files; (3) exact runs on several cores; (4) fast mode; (5) the memory budget as an app setting. Recompute on demand comes after.
- Where it goes in Phase 2: after item 2 (arrays) and before item 3 (rigid bodies), since the cart-pole gate's replay and saved data use it. **Decided:** as proposed, a new item between items 2 and 3 of [ADR 0057](0057-phase-2-plan.md).
