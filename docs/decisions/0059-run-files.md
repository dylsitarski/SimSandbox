# 0059 — Run files: recordings and scenarios beside the model

- **Status:** Accepted
- **Date:** 2026-10-26
- **Amends:** [ADR 0058](0058-inputs-live-scheduled-replayed.md) (where schedules and recordings are kept; the rest stands)

## Context

[ADR 0058](0058-inputs-live-scheduled-replayed.md) put scheduled changes in the model file (`run.inputs`) and had "Save recording as schedule" write a played session there. The owner asked what happens when several runs of one model should be kept (2026-10-26). One `run.inputs` holds one schedule, so runs would overwrite each other or pile up in the model.

The model file also mixes three things:
- **The model:** structure, equations and parameter defaults.
- **A scenario:** what is done to the model, such as timed changes, settings and a seed.
- **A run:** one recorded session and where it came from.

Recordings are data: a minute of dragging is thousands of events, and saving one would rewrite the model each time. A recording also outlives edits to its model, so it must say which version it was made with. Established tools keep these apart: Modelica keeps experiments apart from models, and FMI's SSP standard has its own scenario files. Phase 3's provenance reports need a record of what was run, on which model version, with which seed.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Everything in the model file, as ADR 0058 decided | One file to share | One schedule per model; recordings bloat and churn the model; no record of which model version a recording fits |
| A list of named scenarios and recordings inside the model file | Several runs, still one file | Same bloat and churn; model and data still mixed |
| **Run files beside the model; the model keeps at most one optional default schedule** | Any number of runs; models stay small and stable; each run records the model version it was made with; a step towards provenance | A run needs its model to replay; sharing a recording means two files |

## Decision

The owner chose run files and kept the optional default schedule on 2026-10-26.

### The model file

The model keeps `run.inputs`, an optional default schedule (ADR 0058). Use it only for timed changes that are part of what the model demonstrates, for example a shared demo that pushes the cart at t = 5 s. Recordings never go in the model file.

### Run files

A run file sits beside its model and is named `<model>.<run>.run.json`, e.g. `cart-pole.push-test.run.json`:

```json
{
  "format": 1,
  "model": "cart-pole.ssbx.json",
  "model_hash": "sha256:…",
  "name": "push test",
  "note": "Pole recovers from a 40 N push",
  "created": "2026-10-26T14:03:00Z",
  "seed": 1,
  "run": {"dt": 0.01, "t_end": 20},
  "inputs": [{"t": 5, "set": "push.y", "to": 40}, {"t": 5.2, "set": "push.y", "to": 0}],
  "track": [[0.37, "hand.x", 0.12], [0.37, "hand.down", 1]]
}
```

- **`model`** is the model file's path, relative to the run file.
- **`model_hash`** is a SHA-256 of the model as saved, leaving out `controls`. Controls never change results ([ADR 0012](0012-controls-in-model-files.md)), so rearranging panes or pinning values does not make a run stale. As built (2026-10-26) it also leaves out the seed, `dt`, `t_end` and the schedule, which the run records itself, and is taken over the model with every parameter written out, so a file that leaves defaults implicit and the same model saved by the app hash alike.
- **`inputs` and `track`** hold everything applied in the run: the schedule in effect (copied, so later edits to the model's default schedule do not change the run) and the recorded live input. Both are events of ADR 0058, applied at step boundaries.
- **`seed` and `run`** hold the seed and the run settings that were used, overriding the model's.
- **`name`, `note` and `created`** describe the run; they do not affect results.

Results are not stored. A run is reproduced by replaying it: the same model, events and seed give the same run bit for bit. Links to saved results can come with Phase 3's provenance work.

### In the app

- **"Save run…"** asks for a name and a note and writes the current run (the schedule in effect and the recorded track up to now) beside the model. A model that has never been saved is saved first.
- **A Runs menu** lists the run files beside the model that name it. Opening one restarts and replays it, showing "Replaying input" (ADR 0058); live input during the replay takes over, as after a rewind.
- **"Use as the model's schedule"** copies a run's events into `run.inputs`, for a run that should become the model's demo.
- **A stale run** (its `model_hash` differs) opens with a warning that the model changed since it was recorded. Events naming keys that no longer exist, or values out of range, are reported, not silently skipped.

### Headless

- `simsandbox run RUN.run.json` replays a run: it loads the model it names, warns on a hash mismatch, and uses the run's seed, settings and events in place of the model's.
- `simsandbox run MODEL --inputs RUN.run.json` applies a run's events to another model, for example an edited version of it; there is no hash check, but keys and ranges are checked. `render` accepts the same.
- `simsandbox check RUN.run.json` reports a missing model, a hash mismatch, unknown keys and out-of-range values.
- Python:
  - `ss.Run.load(path)` returns its model, events and seed, and `Run.replay()` returns `Results`;
  - `ss.Run(model, inputs=..., track=..., seed=..., name=...).save(path)` writes one;
  - `Model.run(inputs=...)` takes a list of events or a `Run`.
- `simsandbox schema` includes the run file's schema.

## Consequences

- ADR 0058's track file (`{"format": 1, "events": [...]}`) and "Save recording as schedule" are replaced by run files and "Save run…". Its input blocks, input track, replay after rewind and the default schedule stand.
- Run files follow the format rules for models ([ADR 0044](0044-unsaved-changes-and-older-files.md)): a format number, older forms read and converted, unknown fields refused with a hint.
- Phase 3's provenance reports build on run files.
- The session gains loading and saving runs; the file crate gains the run format and the model hash.
