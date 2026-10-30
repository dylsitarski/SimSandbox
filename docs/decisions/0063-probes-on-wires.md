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

## Filters with several sources (owner, 2026-10-29)

After step 1 the owner asked for filters to read several probes, as views do, either updating whenever one of their inputs updates or running when their inputs pair.

- **Each input of a filter has a source:** the model (every step), a probe, or another filter that has its own clock. `from` maps an input to a probe (`"from": {"car.v": "slow", "cruise.F": "fast"}`); `probe` stays as the source of every input not in `from`. Wiring a probe output into a filter's input sets that input's source.
- **When it runs** (`when`): `paired` (the default) runs at the times all its sources recorded, as views pair signals; `any` runs whenever one of its sources records, taking each other input's latest recorded value (held), and gives NaN until every source has recorded once. `any` with an input from the model runs at every step.
- **A filter with any input not from the model has a clock and a recording of its own.** Its outputs are bound by their names alone (`power`, as model filters' are): output names are unique in a model, so the name says where to find them. The `probe:NAME/output` form written on 2026-10-29 for filters on one probe is read and converted. Filters may read such filters; they run in dependency order within a step.
- Its held inputs are part of what it recorded: after a rewind they are taken again from its sources' recordings at that time, so replay stays exact.

## Outputs recomputed on rewind (owner, 2026-10-29)

ADR 0062 decided that snapshots keep outputs, because recomputing them could differ in the last bits where algebraic loops are solved by iteration. After step 2 the owner left it to us whether rewind should recompute outputs. **Decided:** a snapshot leaves out a Block's outputs when they depend on its state and parameters only (no output with feedthrough, no population outputs) and computing them again from the state reproduces them bit for bit, checked when the snapshot is taken; rewind computes them again. Every other Block keeps its outputs, less any declared copies of its state, so loops (which always involve feedthrough) are never recomputed and rewind stays exact for every model.

## As built

**Step 1 (2026-10-29): probe outputs, recording what is wired, several sources.** Built in three parts, each committed.

- **Views read named recordings.** A binding is `car.v` (the model, every step) or `probe:speed/car.v` (`simsandbox_views::history::split_source`, `from_probe`, `label`). `Recorded` holds the model's history and each probe's; `ViewInput` resolves every binding in its own recording (`Bound`, `one_any`, `many_any`), while `one` and `many` keep giving the bindings in the view's main recording for views that read one. `paired` pairs signals at the times all of them recorded. `plot.xy` draws each curve from its own recording (against `t` at its own times; against another signal or with colour, paired) and `plot.3d` pairs its axes; field views read one recording. `ViewEntry::probe` (built earlier the same day) is read and converted into `probe:` bindings, never written; the tile's "from" menu is gone.
- **Probe nodes have outputs**, one per signal recorded, wired into views (`Edit::Bind` writes `probe:NAME/signal`) and filters. Renaming or deleting a probe, or no longer recording a signal, follows into views and filters; a probe cannot record another probe's recording or feed the model yet. The data layer's columns: model filters, probes, filters on probes, views.
- **The history records what is wired.** `Session::set_view_reads` (the app calls it whenever its tiles' bindings change) sets what views read from the model directly; the history keeps those and what model filters read (with the coordinates cuts need), nothing else, and keeps what it had of them. A signal wired mid-run has no past: NaN for signals, an absent (empty) frame for fields, which views draw as nothing and `.npy` files write as NaN. **Difference from the decision:** until views are known (a model loaded without saved views and no app, e.g. `simsandbox render` of a file without views, whose default views are chosen when drawn) the history records everything. Sweeps record everything, as before.
- **Probes take their samples from everything a step yields** (the session's layout: every output and model filter output), not from the history. `controls.plots` no longer limits what can be recorded, bound or probed; it is still read and saved but has no effect. Views, the Data window and `check`'s `recorded` list everything a step yields.
- **Filters on probes** (superseded the same day by the section above). `FilterEntry::probe` names the probe a filter runs on. It is compiled with that probe (reading only what the probe records, parameters and `t`) and computed whenever the probe records; its outputs join the probe's recording, bound as `probe:NAME/output` (`Available::probed`). Wiring a probe output into a filter puts it on the probe (views reading its outputs follow); deleting the probe puts its filters back on the model. A filter whose probe is missing computes nothing, with a warning.
- **Filters with several sources (2026-10-29, as decided above).** `FilterEntry::{from, when}` with `source_of`, `on_probes`, `on_any`. The session keeps every filter with an input from a probe, or reading such a filter's outputs (`observers::clocked`), as a `Derived`: its inputs each with a source (`Src::Model`, `Probe`, `Derived`), compiled on those inputs alone, run after the probes at each step (`run_derived`, in dependency order) when all or any of its sources recorded, holding each input's latest value; its own history, bound by output names (`Recorded::derived`, `Available::derived`, `Session::derived_histories`). Rewind truncates it and takes the held values again from its sources' recordings (`reseed`), so a run replays bit for bit (tested). On the canvas each filter input shows its source (`from.NAME` on the node); wiring a probe or model output into the input of that name sets it, disconnecting puts it back on the model, and "Run when any input records" sets `when` (`Edit::SetFilterWhen`). Filters with clocks of their own sit after the probes. A probe cannot record such a filter's outputs.

**Step 2 (2026-10-29): snapshots without outputs that copy a state.** `PortSpec::state` (`from_state(offset)`) declares an output equal to the Block's state from an offset. `Simulator` keeps, per Block, the declared copies (output offset, state offset, length); `snapshot` checks each against the state bit for bit and, when all match, stores only the other outputs (`Snapshot::compact`); `restore` puts the copies back from the state. Declared by typed equations (every state, in state order), fused groups (offsets shifted by member), `eq.lorenz`, `eq.coupled_oscillator`, `eq.pendulum` (angle and omega), `eq.heat_rod` and `eq.grass`. Tested: every example restores every output bit for bit; a wrongly declared copy is kept. Gray–Scott's snapshot falls from 7 to 5 grids of 128 × 128 (about 115 000 to 82 000 numbers): its computed `reaction` and the axes `x`, `y` stay, since only copies of a state are left out.

**Outputs recomputed (2026-10-29).** `Simulator::recomputable` (no feedthrough, no populations) and `Simulator::recompute` (the Block's `compute_outputs` on copies of its state and tables, at the snapshot's time and parameters). `Snapshot::kept` per Block: `None` (recomputed on restore), `ButCopies` (step 2) or `All`. Tested: every example restores every output bit for bit; a Block whose step writes outputs differently from `compute_outputs` keeps them. Gray–Scott's snapshot is its two state grids (about 33 000 numbers, from 115 000 before step 2); the check costs nothing measurable (1.67 ms a step in a session, 1.73 stepping alone).

**Step 3 (2026-10-29): a probe into a block of the same model.** A model-level connection may come from a probe: `{"from": "probe:sensor.car.v", "to": "cruise.v"}`. The loader (`ModelFile::build`, `hold`) puts a `simsandbox_core::hold::SampleHold` between the probe's signal and the input, one per probe signal: input `u`, output `y` with feedthrough on `u` (so it is evaluated in dependency order at each communication point and counts in loop detection), state `[due, next time, next index, held…]` (so snapshots, rewind and replay are exact). Its `do_step` decides with the probe's own trigger (`simsandbox_core::schedule`, moved from the session so both decide alike) whether it samples at the end of the step; `compute_outputs` takes the input then, else gives the held value. It is not an `Equations` Block, so it never joins a fused group, and it is not a recorded output. The probe must exist and record the signal, a whole output of a block on the model's level (not a population, not inside a subsystem). On the canvas, dropping a probe output on a block input makes the connection (a model edit: rebuild and restart) and draws a solid wire from the probe; deleting the probe, renaming it or no longer recording the signal follow. A data-layer edit that changes such a probe's schedule rebuilds the model (`undo::change`, and the app's `handle` checks it). Run files' model hash includes the feeding probes' names, records and schedules, not their files or how long they keep samples. Tested: the controller of cruise control reading the speed every second gets the speed of that very step and holds it (no delay); sampled at every step it equals a direct wire between blocks solved apart, bit for bit; a run rewound and replayed gives the same bits; the hash follows the schedule only. Found on the way: `Controls::is_empty` ignored the newer sections (layout, open nodes, inputs, probes, recorder), so a file whose controls held only those wrote none of them; fixed.

**Probe files and memory (2026-10-29, owner's request).** `ProbeEntry::stream` writes the probe's file as it records (`probe::Stream`: a buffered CSV and one `.npy` per field whose fixed 128-byte header is rewritten with the sample count on each flush, so numpy reads it at any flush). It flushes about every 2 s, on `Session::flush` and on drop; restart empties it and rewind cuts it back to the rewound time (an index of time and byte offset, kept only while a recorder is present). No batch-size setting: the buffered writer already batches writes, so a knob would add nothing. `keep` gained `{"samples": N}`, and the default became "thinned past 100 000 samples", shown and editable in the settings window. A probe keeps samples in memory only when a view or filter reads it, it writes a file at the end, or a headless run asks for all probes (`Session::update_probe_io`, `record_all_probes`); otherwise `keep` has no effect and nothing is held. Hiding the data layer keeps probes that feed blocks (`observers::model_part`, `probe_feeds_model`), drawn with a solid outline: they belong to the model.

**Step 4 (2026-10-30): a block reading a probe's file or any CSV or `.npy`.** `input.file` (`simsandbox_eq::file_input::FileInput`): the data is read when the model is built and the outputs are a function of time alone, so snapshots, rewind and replay need nothing new. A CSV's first column is the time and each other column a scalar output named by its header (non-identifier characters become `_`, since `.` and `[` mean paths and indices in connections); a `.npy` (`f8` or `f4`, C order, read by `read_npy`) is one output `y`, scalar, vector, grid or, with `shape: points`, a point set, with its times from `times` (a CSV's first column, e.g. the probe's CSV) or `dt`. Units, as ADR 0048 asks, from the source when it says (`name [unit]` in a CSV header), else `unit.COLUMN` or `unit`. `interpolate: hold` (default, what a probe feeding a block gives) or `linear`; before the first sample the first, after the last the last. It never joins a fused group, so a held value stays constant over a step, as a probe's hold does. **Where files are read from:** the kernel's `BlockConfig::folder` (the model file's folder) and `BlockConfig::file(setting)`; `TypeInfo::file` names the settings that are files, which `describe` lists (`files`). `simsandbox_file::load_in` and `Session::load_in` take the folder (the app passes the saved model's); headless tools wrap a command in `registry::in_folder` (the CLI, for the model or a run file's model) or `registry::set_folder` (Python, from `Model.folder`, set by `Model.load` and `save`), so every function that loads a model's text finds its data without a new argument. A relative name in a model never saved is an error that says to save it or give a full path. **The data is part of the model:** `Loaded::data` holds the SHA-256 of each file read, by `block.setting`, and `runs::model_hash_with` adds them to the hash (a model without files hashes as before). Inspector fields name the file (typed, or chosen in the system dialog, kept relative to the model's folder), choose hold or linear, the unit, and a `.npy`'s times. Tested: hold and linear on a CSV with units in its header; a `.npy` grid with times from a CSV and as points; bad files and settings refused; **a probe's streamed file read back in place of the probe drives the cruise control bit for bit as the probe did**; the hash follows the file's contents; Python and the app (a block added on the canvas waits as a draft until its file is named, then runs). Not built: re-reading a file that changes while the model is open (the file is read when the model is built; reopen or edit it), and units written into probe CSV headers ([Q42](../open-questions.md), [Q43](../open-questions.md)).

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
