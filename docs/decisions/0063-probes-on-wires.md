# 0063 — Probes on wires: what views, filters and models read from

- **Status:** Proposed
- **Date:** 2026-10-29
- **Amends:** [ADR 0062](0062-probes-recording-and-replay.md) (how views read probes, what the implicit probe records)
- **Settles:** [Q40](../open-questions.md) once accepted

## Context

Probes were built on 2026-10-29 ([ADR 0062](0062-probes-recording-and-replay.md), steps 1 and 2): canvas nodes with one `record` input, copying their samples out of the implicit history, and a "from" menu on each view tile choosing between every step and one probe. On 2026-10-29 the owner asked for probes to have outputs wired into views instead, so the wiring shows when a view's data was recorded, and asked whether recording what the model already records is redundant.

It is, in two ways: probes copy what the implicit history also holds (for grids, twice the memory), and the implicit history records every output any view could show, at every step, whether or not anything reads it. Because probes read from the implicit history, they also cannot record outputs left out of `controls.plots`.

## Decided by the owner (2026-10-29)

- **Probes have outputs.** One output per signal or field a probe records. A view, filter or other node wired from a probe output reads the probe's recording; wired from the model, it reads every step. The tile's "from" menu goes.
- **The implicit history records only what is wired.** Signals and fields read by views and filters wired directly to the model, scalars included. Nothing is recorded "in case".
- **Views may read from several sources at once**, e.g. one plot against time of scalars from probes with different recording rates.
- **Probes may feed filters, and models.** (What "models" means is asked below.)
- **Probes take their samples from each step's outputs**, not from the implicit history; `controls.plots` no longer limits them.

## Proposal for the rest

### Views with several sources

Each bound signal keeps its own times. Against `t` (plot.xy's default, the heat map's time axis), every series is drawn at its own samples: a probe every 2 s and the model's every step on one plot. Where a view pairs values (x against y, a 3D trajectory, colour by a signal), signals from different sources are paired **only at times both recorded**: never interpolated, never held. Probes record at actual step times, so a probe every 2 s pairs with every-step data every 2 s, and probes every 2 s and every 3 s pair every 6 s. A pairing with no common times draws nothing and says so.

In the file a binding names its source: `"y": "probe:speed/car.v"` (probe `speed`'s recording of `car.v`), and `"car.v"` alone is the model's every step. View entries lose `probe` (built 2026-10-29, never released; read and converted).

### Filters fed from probes

A filter computes on its inputs' samples. Wired from one probe, it runs on that probe's samples (an expression every 2 s, a cut every 2 s); from several sources, it pairs them like a view, at common times. Its output carries its inputs' times and can feed views, probes and other filters.

### Newly wired signals have no past

With nothing recorded in case, a signal wired to a view mid-run has no past until the run continues. Restart fills it. Recompute on demand ([ADR 0062](0062-probes-recording-and-replay.md): rerun from the nearest snapshot) fills it in exactly, and becomes more valuable now.

### Snapshots and probes

Snapshots (every 10 steps, within their budget) hold the full replay state: states, tables, outputs, parameters, step count. Probes hold chosen outputs at chosen times. The overlap is outputs at the same times, but the roles differ: snapshots are the minimum from which anything can be recomputed exactly; probes are what the user chose to keep at hand. Two savings follow without changing either role:

- **Outputs that copy a state** (most typed equations' outputs, Gray–Scott's `u` and `v`) are stored twice in every snapshot. They can be restored from the states exactly, with no iteration, so snapshots need not hold them; outputs computed by iteration (algebraic loops) stay, as ADR 0062 decided.
- **With recompute on demand**, probes become caches: one can keep little and still show any time, at the cost of recomputing.

## Questions for the owner

1. **"Probes may feed other models"** means: (a) a probe output wired into a block input of the same model, as a sampled sensor (the value held between samples, changing results like any wire); (b) a probe's saved file read by another model file as an input (a table or file input block); or (c) both?
2. **Recompute on demand:** build it with this change, so a view wired mid-run shows the past at once, or keep it after the rest of ADR 0062 (a newly wired signal starts empty until Restart)?

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| **Probes with outputs, sources shown by wires** | The canvas shows where every view's data comes from; one rule for views, filters and models | Views and filters must handle several time bases |
| A "from" menu per view (built 2026-10-29) | Simple; one time base per view | Hidden in the tile; cannot mix sources; probes cannot feed filters |

| Implicit history records | Pros | Cons |
| --- | --- | --- |
| **Only what is wired (owner's choice)** | No memory spent on what nothing reads; simple rule | A newly wired signal has no past until recompute on demand or Restart |
| Every scalar, grids only when wired | Newly bound scalars show their past | Still records unread scalars |
| Everything, as before | Any binding shows the past | Grids recorded whether or not they are shown |

## Consequences (if accepted)

- Probe nodes get outputs; data wires from probes carry a source; the session records per wired source; views and filters take per-signal times (`simsandbox-views` reads bindings through sources, not one `History`).
- Bindings in the file name their source (`probe:NAME/signal`); `ViewEntry::probe` is converted and dropped.
- Headless `run --probe` is unchanged; `render` follows the wires.
- Snapshot outputs that copy states are dropped from snapshots, restored on rewind.
- Before ADR 0062 step 3 (exact multi-core runs).
