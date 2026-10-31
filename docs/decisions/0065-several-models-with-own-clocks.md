# 0065 — Several models, each with its own clock, referenced from their files and linked by probes

- **Status:** Proposed
- **Date:** 2026-10-31

## Context

[ADR 0063](0063-probes-on-wires.md) step 5: the owner wants probes to feed "a model that runs on different time steps", and to update "a time-independent model with new data", for maximum flexibility (2026-10-29). Steps 3 and 4 linked a probe into its own model (a sample-and-hold, no added delay) and into another model through its file (`input.file`). Step 5 runs several models together and needs its own ADR first, answering: how clocks interleave, how runs replay exactly across models, and how the canvas shows several models.

What exists and must keep working:

- A model file is one `Simulator` with one communication step `run.dt` ([ADR 0013](0013-run-section.md)). Time advances by `t += dt`; blocks exchange values at each step (Jacobi, feedthrough in dependency order, [ADR 0040](0040-feedthrough-and-algebraic-loops.md)).
- A model without states is swept instead of run ([ADR 0021](0021-maps-formulas-and-sweeps.md), [ADR 0028](0028-sweeps-as-samples.md)).
- Subsystems share their model's clock. Their uses expand into blocks named `use/inner`, with declared ports and a node you can open ([ADR 0043](0043-subsystems.md)).
- Values from outside a running model enter through input blocks (`input.value`), driven by sliders, keys, schedules or replay ([ADR 0058](0058-inputs-live-scheduled-replayed.md)).
- Views read several sources, each at its own times, and pair them at common times (ADR 0063).
- One recorder per model ([ADR 0064](0064-recorder-node.md)). Random draws depend on the seed, the block's name and the step ([ADR 0034](0034-agent-domain.md)). Blocks read data files from their model file's folder, and run hashes cover those files' contents (ADR 0063 step 4).

The first draft of this ADR (2026-10-31) kept the file's own blocks as an unnamed main model and added inline `models` beside it.

## Decided by the owner (2026-10-31)

- **Every model is named.** No model is special. (The owner set aside the renaming this costs. With models referenced from their files, below, no existing file is renamed anyway.)
- **Models are referenced from their own files**, so one model can be reused in many places.
- **Editing a referenced model from inside another file offers a choice**: copy its contents, or edit the original.

## Options considered

**Where models live**

| Option | Pros | Cons |
| --- | --- | --- |
| **A composition: a file whose `models` reference model files (owner's choice)** | Reuse; each model still opens, runs and is tested on its own; existing model files stay exactly as they are | A composition depends on other files: paths, files changed or missing, edits that reach other compositions |
| Models written inline in one file, beside an unnamed main model (first draft) | One self-contained file | No reuse; the main model is a special case in every rule |
| A clock on subsystem uses | Smallest change | Rate changes hidden in ordinary wires; time-independent models fit badly |

**Where a link enters a model**

| Option | Pros | Cons |
| --- | --- | --- |
| **Its input blocks** (`input.value`): a link drives one as a slider would | No new kind of port. The model runs alone with its slider, schedule or recorded input, and in a composition with a link. Units and ranges are already declared | Scalars only until input blocks for grids and points exist |
| Declared input ports, as subsystems have | Familiar from subsystems | Unconnected ports when the model runs alone: it needs defaults anyway, which is what an input block is |

**How links sample** and **how clocks are ordered**: as in the first draft. Links go only through a probe, so sampling is visible. Clocks are ordered in exact ticks, not floating-point times compared with a tolerance.

## Decision

**1. Two kinds of file.** A **model file** is today's file, unchanged: blocks, connections, its `run.dt`, its own `controls`. Opened alone it runs as now, with the same names and the same hash. A **composition** is a file with `models` and no `blocks` (a file with both is refused):

```json
{"format": 1, "run": {"t_end": 60, "seed": 1},
 "models": {
   "plant":      {"file": "car.ssbx.json"},
   "controller": {"file": "cruise.ssbx.json", "params": {"pi.kp": 900}, "run": {"dt": 0.5}}},
 "connections": [{"from": "probe:sensor.plant/car.v", "to": "controller/speed"}],
 "controls": {"probes": [{"name": "sensor", "record": ["plant/car.v"], "every": {"seconds": 0.5}}]}}
```

- **A use** names a model file. Paths are relative to the composition. The same file can be used twice under two names (two cars with one plant). A use may also give:
  - `params`: values for this use only. Setting them is not an edit of the model.
  - `run.dt`: this use's clock. By default it is the model file's `run.dt`. A model without states has no clock.
- **Names** inside a use are prefixed with it: `plant/car.v`, as for subsystem uses. Views, filters, probes, sliders, run files and headless results use these names. Random draws differ between uses because block names do.
- **Each use reads its own data files** (`input.file`) from its own model file's folder, not the composition's.
- **What is used and what is not:** the composition has its own `controls` (views, probes, recorder, sliders). A referenced model's own `controls` are not used, so its views and probes stay its own when it runs alone.
- **Compositions do not reference compositions** (one level for now). A model file referencing itself, or a missing file, is an error that names the path.

**2. Links.** A link goes from a probe output to an input block of another use: `"to": "controller/speed"` drives `controller`'s `input.value` block `speed`, in place of its slider or schedule. Units are checked as on any wire. A probe records from one use and fires on that use's steps. Direct wires between uses are refused, with a message to put a probe between. On the canvas, dropping a use's output on another use's input block adds a probe that records every step. Probes feeding links are part of the composition's hash.

**3. Clocks interleave in exact ticks.**
- Each clock's `dt` is read as the decimal it is written as. The session's tick is the largest time dividing every `dt` exactly, and the session advances through the ticks where any use has a communication point.
- **Between points, uses step independently.** Each uses its link inputs as they were at the start of its step, held over the step. So uses may step in any order, or at once on several cores, with the same bits.
- **At a point,** uses reaching it evaluate outputs and probes sample in link order, feeding uses first. A reading use takes the latest sample at or before that tick: no delay when both have a point there.
- **A loop of links** between uses sharing a point is broken in the order the uses are written, with one step of delay at the later use. `check` reports where.
- Blocks still see `t` advance by `t += dt`, so a model gives the same bits in a composition of one use as alone.

**4. Time-independent models** (no states) are evaluated when a link delivers a new sample. The other link inputs keep their last values (NaN until each has delivered once). Their outputs carry those times. A swept model runs its whole sweep again at each delivery, its views show the latest sweep, and its outputs do not feed links.

**5. Editing a used model.** Parameter values (`params`) and the use's clock are part of the use, so they are edited with no question. A structural edit inside a use asks once, before anything changes. Structural edits are blocks, equations, wires and the model's own declared parameters. The choices:
- **Edit the original:** the edit goes to the model file, which every composition using it sees. The question says how many uses this composition has of that file. It cannot list other compositions, since nothing indexes them. Save writes the model file and the composition, and the unsaved marker names each file with changes. Undo covers both.
- **Make a local copy:** the use's `file` becomes the model's contents written inline under the use (`"models": {"plant": {"blocks": [...], ...}}`), detached from the file, with its clock and names unchanged. Then the edit applies.
- **Save as a new model file…:** the contents go to a new file, the use references it, then the edit applies. This makes a variant that other compositions can reuse.

The app remembers the choice for that use until the composition is closed.

**6. Changes to a model file.** A composition reads its models when it is built (opened, or edited on the canvas), as `input.file` reads data (Q43). Its hash covers each used file's hash, data files included, so run files report a composition whose models changed. A model file whose input block a link drives is renamed or removed → the composition fails to load with an error at that connection; in the app the edit waits as a draft. A missing file → "Locate…" in the app.

**7. Recording, rewind and replay.** Each use's every-step recording, probes and views work at its own times. Views pair across uses at common ticks. The composition's recorder (one, ADR 0064) snapshots every use at ticks where all clocked uses have a point. `check` warns when such ticks are far apart. Rewind restores the latest common snapshot and replays in tick order, giving the same bits. Held link values are state of the reading use, so snapshots include them. Run files record inputs by full name (`controller/speed`).

**8. The canvas.** A composition's top level shows its uses as nodes, titled with their clock ("every 0.5 s", "on new data") and the file they use. Each node shows its input blocks as inputs and its outputs. Double-click opens a use's inside, with the breadcrumb back. A use of a file draws a link mark; a local copy does not. The data layer sits at the top level. The menu has "Add model…" (choose a file) and "New model file…". From a model file, "New composition from this model" makes a composition using it.

**9. Headless** ([ADR 0039](0039-ai-agent-access.md)):
- `check` lists uses (files, clocks, parameter values), links, the tick and any loop it breaks.
- `run` gives each use's outputs at its own times. JSON has one `t` per use. CSV needs `--model NAME`, or `--out DIR` for one file per use.
- `render --diagram --model NAME` draws a use's inside.
- Python: `Composition` with `use(name, file, params=, dt=)`, `link(probe, to)`, `run()` and results by use.
- `localize NAME` writes a use inline, for agents as for the app.

**Order of work:**
1. Compositions in the file, the loader and a headless run: uses of files, names, exact ticks, links through probes into input blocks, link order and loops, data files per use, the hash over used files. Two tests:
   - a cruise control split into a plant file and a controller file, used with the same `dt` and linked every step, gives the same bits as the model in one file with the step-3 hold;
   - clocks of 0.1 s and 0.3 s match a hand-computed multi-rate reference.
2. Time-independent models evaluated on delivery, sweeps included.
3. The canvas and the app: use nodes, opening a use, the probe added when wiring, the edit question with its three choices, saving several files.
4. Recorder and rewind at common ticks; run files of compositions.
5. Uses stepping on several cores between points (with ADR 0062 step 3), tested for the same bits.

## Open points for the owner

These suggestions were added to the owner's choices and wait for confirmation:
- **Links enter through input blocks**, not new ports, so every model also runs alone (above).
- **Parameter values and clocks per use** are not edits of the model and ask nothing.
- **A third choice, "Save as a new model file…"**, beside copying and editing the original.
- **"Make a local copy", not "make a subsystem":** a subsystem shares its containing model's clock, so turning a use into a subsystem would change its timing. Using a model file as a subsystem inside another model, on that model's clock, is a separate and also useful kind of reuse. It is proposed for later, with subsystem libraries.
- **Rewinding one use alone** (per-use recorders) is left for later. With links both ways it would not reproduce the coupled run.

## Consequences

- Model files are unchanged. A composition is the same file format with `models` instead of `blocks`, so the loader, `check`, the canvas, run files, Python and the CLI learn names prefixed by use, times per use and edits that reach several files.
- A session holds several `Simulator`s and a scheduler over exact ticks. One model alone runs exactly as before (tested bit for bit).
- Coupling between models is explicit and sampled. Error from coupling across clocks is the reading use's hold, as in co-simulation ([block contract](../architecture/block-contract.md)).
- Reuse brings shared-file risks: edits to a model reach every composition using it, and paths can break. Hashes, the edit question and "Locate…" make these visible rather than silent.
- Supersedes nothing. ADR 0064's per-model recorders and the open points above stay for later.
