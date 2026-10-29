# 0063 — Probes on wires: what views, filters and models read from

- **Status:** Accepted
- **Date:** 2026-10-29 (proposed and accepted)
- **Amends:** [ADR 0062](0062-probes-recording-and-replay.md) (how views read probes, what the implicit probe records)
- **Settles:** [Q40](../open-questions.md)

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

## Owner's answers (2026-10-29)

1. **Probes feed models in every way that connects them**, for maximum flexibility, including interactions that are not obvious otherwise:
   - **Into the same model:** a probe output wired into a block input, as a sampled signal: the value is held between samples and changes results like any wire. No delay is added (owner's question, 2026-10-29): such a probe is a sample-and-hold element in the model's graph, evaluated at each communication point in dependency order like an output with direct feedthrough ([ADR 0040](0040-feedthrough-and-algebraic-loops.md)), so a block reads the value sampled at the same instant. Its held value is kernel state, kept in snapshots. It never joins a fused group: blocks it feeds see a value held over each step, as for any input from outside a group. It counts as feedthrough in loop detection, so a loop through it is solved like any other.
   - **Into another model through its file:** a block that reads a probe's saved file (CSV, `.npy`) as an input over time. It also serves external data ([ADR 0048](0048-units-of-external-data.md)).
   - **Into a model on different time steps**, and **into a time-independent model** (formulas, a sweep) that is evaluated again whenever the probe delivers new data. Probes are the links between models with their own clocks.
2. **Recompute on demand comes later**, as ADR 0062 planned, and only when asked for (an action on a view or probe), never automatically. A signal wired mid-run starts empty; Restart or that action fills it.

## Decision

As above: probes have outputs; the implicit history records only what is wired; views and filters may read several sources, paired only at times both recorded; bindings name their source (`probe:NAME/signal`); probes feed views, filters and models; snapshots drop outputs that copy a state; recompute on demand later and by request.

**A probe that feeds a model is part of the model.** Its settings then change results, so it counts in the model's hash ([ADR 0059](0059-run-files.md)) and editing it rebuilds and restarts the run like any model edit; a probe feeding only views and filters stays a control that never changes results.

**Order of work:**
1. Probe outputs; sources in bindings; the implicit history records only what is wired; views and filters with several sources; the "from" menu and `ViewEntry::probe` go (converted).
2. Snapshots without outputs that copy a state.
3. A probe into a block of the same model: a sample-and-hold element in the graph, with no added delay.
4. A block reading a probe's file (or any CSV or `.npy`) as an input over time.
5. Several models in one session, each with its own clock, linked by probes; a time-independent model evaluated again on new data. Its own ADR first: how clocks interleave, exact replay across models, how the canvas shows several models.

Then ADR 0062 step 3 (exact runs on several cores). Recompute on demand after ADR 0062, as an action.

## As built

**Step 1 (2026-10-29): probe outputs, recording what is wired, several sources.** Built in three parts, each committed.

- **Views read named recordings.** A binding is `car.v` (the model, every step) or `probe:speed/car.v` (`simsandbox_views::history::split_source`, `from_probe`, `label`). `Recorded` holds the model's history and each probe's; `ViewInput` resolves every binding in its own recording (`Bound`, `one_any`, `many_any`), while `one` and `many` keep giving the bindings in the view's main recording for views that read one. `paired` pairs signals at the times all of them recorded. `plot.xy` draws each curve from its own recording (against `t` at its own times; against another signal or with colour, paired) and `plot.3d` pairs its axes; field views read one recording. `ViewEntry::probe` (built earlier the same day) is read and converted into `probe:` bindings, never written; the tile's "from" menu is gone.
- **Probe nodes have outputs**, one per signal recorded, wired into views (`Edit::Bind` writes `probe:NAME/signal`) and filters. Renaming or deleting a probe, or no longer recording a signal, follows into views and filters; a probe cannot record another probe's recording or feed the model yet. The data layer's columns: model filters, probes, filters on probes, views.
- **The history records what is wired.** `Session::set_view_reads` (the app calls it whenever its tiles' bindings change) sets what views read from the model directly; the history keeps those and what model filters read (with the coordinates cuts need), nothing else, and keeps what it had of them. A signal wired mid-run has no past: NaN for signals, an absent (empty) frame for fields, which views draw as nothing and `.npy` files write as NaN. **Difference from the decision:** until views are known (a model loaded without saved views and no app, e.g. `simsandbox render` of a file without views, whose default views are chosen when drawn) the history records everything. Sweeps record everything, as before.
- **Probes take their samples from everything a step yields** (the session's layout: every output and model filter output), not from the history. `controls.plots` no longer limits what can be recorded, bound or probed; it is still read and saved but has no effect. Views, the Data window and `check`'s `recorded` list everything a step yields.
- **Filters on probes.** `FilterEntry::probe` names the probe a filter runs on. It is compiled with that probe (reading only what the probe records, parameters and `t`) and computed whenever the probe records; its outputs join the probe's recording, bound as `probe:NAME/output` (`Available::probed`). Wiring a probe output into a filter puts it on the probe (views reading its outputs follow); deleting the probe puts its filters back on the model. A filter whose probe is missing computes nothing, with a warning. A filter reads one probe; pairing two probes' samples in a filter is not built.

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

## Consequences

- Probe nodes get outputs; data wires from probes carry a source; the session records per wired source; views and filters take per-signal times (`simsandbox-views` reads bindings through sources, not one `History`).
- Bindings in the file name their source (`probe:NAME/signal`); `ViewEntry::probe` is converted and dropped.
- Headless `run --probe` is unchanged; `render` follows the wires.
- Snapshot outputs that copy states are dropped from snapshots, restored on rewind.
- A probe feeding a model moves it into the model's part of the file for hashing; the file form is settled when step 3 is built.
- Before ADR 0062 step 3 (exact multi-core runs).
