# 0065 — Several models in one file, each with its own clock, linked by probes

- **Status:** Proposed
- **Date:** 2026-10-31

## Context

[ADR 0063](0063-probes-on-wires.md) step 5: the owner wants probes to feed "a model that runs on different time steps", and to update "a time-independent model with new data", for maximum flexibility (2026-10-29). Steps 3 and 4 linked a probe into its own model (a sample-and-hold, no added delay) and into another model through its file (`input.file`). Step 5 runs several models together and needs its own ADR first, answering: how clocks interleave, how runs replay exactly across models, and how the canvas shows several models.

What exists and must keep working:

- A model file is one `Simulator` with one communication step `run.dt` ([ADR 0013](0013-run-section.md)). Time advances by `t += dt`; blocks and fused groups exchange values at each step (Jacobi, feedthrough in dependency order, [ADR 0040](0040-feedthrough-and-algebraic-loops.md)).
- A model without states is swept instead of run ([ADR 0021](0021-maps-formulas-and-sweeps.md), [ADR 0028](0028-sweeps-as-samples.md)).
- Subsystems are expanded into blocks named `instance/inner`, with declared `inputs` and `outputs` and a node you can open on the canvas ([ADR 0043](0043-subsystems.md)).
- Views read several sources, each at its own times, and pair them only at times both recorded (ADR 0063 step 1).
- One recorder per model holds the snapshots that rewind uses ([ADR 0064](0064-recorder-node.md)). Random draws depend on the seed, the block's name and the step ([ADR 0034](0034-agent-domain.md)).
- One worker thread per open model ([ADR 0035](0035-simulation-worker-thread.md)). ADR 0062 step 3 plans exact runs on several cores.

## Options considered

**Where several models live**

| Option | Pros | Cons |
| --- | --- | --- |
| **One file: the file's blocks are the main model; `models` adds others, each with its own clock** | One file to share, check, hash and replay; existing files are unchanged; models appear like subsystem uses on the canvas, which people already know | A model cannot be reused in another file without copying it (until models can be referenced, below) |
| A separate "composition" file that references model files | Each model also opens alone; reuse | A second kind of file; several files to keep in sync, save and hash together; the app edits several files at once |
| A clock on subsystem uses (multi-rate inside one model) | Smallest change | Rate changes would be hidden in ordinary wires; a time-independent model fits badly; snapshots and recorders per clock become unclear |

**How links between models sample**

| Option | Pros | Cons |
| --- | --- | --- |
| **Only through a probe; the reading model holds the value over its step** | When values are sampled is visible and set on the probe (ADR 0063: probes are the links); one rule for every pair of clocks | One extra node per link (added automatically when wiring) |
| Direct wires with an implicit rate transition | Fewer nodes | Sampling is hidden; a 1 s model reading a 0.01 s model silently drops 99 of 100 values |

**How clocks are ordered**

| Option | Pros | Cons |
| --- | --- | --- |
| **Exact ticks: each `dt` read as a decimal fraction, steps ordered by whole multiples of their common divisor** | Times compare exactly, so ties, samples and replays never depend on rounding | A clock whose `dt` has many digits gives a small tick (counted in 128 bits, so no overflow) |
| Floating-point times compared with a tolerance | Simple | Order and pairing depend on the tolerance: 3 × 0.1 ≠ 0.3 |

## Decision

**1. The file.** The file's blocks, as today, are the **main model**, stepping every `run.dt`. A new section `models` adds other models by name, each with `run.dt` (its own clock) or no `run` (time-independent), and with `blocks`, `connections`, `inputs` and `outputs` as a subsystem has (ADR 0043). Subsystem definitions and the file's declared `params` are shared by all models. A file without `models` is one model, unchanged, with the same hash. The session has one time, one `run.t_end` and one `run.seed`.

- Names inside a model are written like a subsystem use's: `plant/car.v`. Views, filters, probes, run files, sliders and headless results use them.
- Models do not contain models.

**2. Links.** A model's input port is fed only from a probe output: `{"from": "probe:sensor.plant/car.v", "to": "controller.v"}`. A probe records from one model and fires on that model's steps (`every` as now). A wire straight from one model to another is refused, with a message to put a probe between. On the canvas, dropping a model's output on another model's input adds a probe that records every step, which you can then edit. Units are checked as on any wire. A probe feeding a model is part of the file's model hash, as in step 3.

**3. Clocks interleave in exact ticks.** Each `dt` is read as the decimal it is written as (`0.1` is 1/10). The session's tick is the largest time that divides every `dt` exactly; each model's communication points are whole numbers of ticks. The session runs tick by tick through the points where any model has one:

- **Between points, models step independently.** A model's step from point *a* to point *b* uses its link inputs as they were at *a*, held over the step, like any input from outside a fused group. So models can step in any order, or at once on several cores, with the same bits.
- **At a point**, each model that reaches it evaluates its outputs, and its probes sample, in link order: a model that feeds another goes first. Then a reading model's link input takes the latest sample at or before that tick: the value at that very tick if the feeding model has a point there (no delay, as in step 3), else the last one before.
- **A loop of links between models that share a point** is broken in the order the models are written, with the main model first. The model written later reads the other's previous sample (one step of delay at that link). `check` reports where the loop is broken.
- Each model's blocks still see `t` advance by `t += dt` as today, so a model run alone gives the same bits. Ticks only order steps and match samples.

**4. Time-independent models** (no states, no `run`) are evaluated when a link delivers a new sample, at that tick, in link order. The other link inputs keep their last values. Until every link input has delivered once, they give NaN, as filters with `when: any` do. Their outputs carry those times, so they can feed views, probes and further links. A time-independent model with a `sample` section (a sweep) runs its whole sweep again at each delivery, and its views show the latest sweep. Its outputs cannot feed links, since they are not one value per time.

**5. Recording, rewind and replay.** Each model's every-step recording, probes and views work as today, at that model's own times. Views pair sources from different models at common ticks (exact, from point 3). The recorder stays one per file (ADR 0064). It takes snapshots of every model at ticks where all clocked models have a point: the first such tick after every `every` steps of the main model. If those common ticks are far apart (the clocks barely share points, e.g. 0.1 s and 0.07 s), `check` warns that rewind will be coarse. Rewind restores every model to the latest common snapshot and replays forward in tick order, which gives the same bits as the first run. Held link values are kernel state of the reading model, so snapshots include them. Run files record inputs by full name (`controller/push.y`), and the hash covers the whole file.

**6. The canvas.** The main model's level shows each other model as a node with its ports (like a subsystem use, with a heavier outline), titled with its clock: "every 0.1 s", or "on new data" for a time-independent one. Double-click opens its inside, with the breadcrumb back. The data layer stays at the main level, and probes linking models sit between the model nodes. "Add model" in the canvas menu adds an empty model, or one copied from a model file ("Import model…").

**7. Headless** ([ADR 0039](0039-ai-agent-access.md)):
- `check` lists `models` with their clocks, links, the tick and any loop it breaks.
- `run` gives each model's outputs at its own times. JSON has one `t` per model. CSV needs `--model NAME` when clocks differ, or `--out DIR` for one file per model.
- `render --diagram --model NAME` draws a model's inside.
- Python: `Model.add_model(name, dt=None)`; results by model (`r.model("plant").t`).

**Order of work:**
1. File, loader and headless run: `models`, exact ticks, links through probes, link order and loops. Two tests: a model split in two with the same `dt`, linked by a probe every step, gives the same bits as the single model with the step-3 hold; two clocks (0.1 s and 0.3 s) match a hand-computed multi-rate reference.
2. Time-independent models evaluated on delivery, sweeps included.
3. The canvas and the app: model nodes, opening a model, the probe added when wiring, views across models.
4. Recorder and rewind at common ticks; run files.
5. Models stepping on several cores between points (with ADR 0062 step 3), tested for the same bits.

## Open points for the owner

- **The main model is special:** it is unnamed and its blocks sit at the top level. This keeps every existing file unchanged. The alternative is every model named, with existing files read as one model called `main`, which renames every signal (`main/car.v`).
- **Referencing model files** instead of copying them (`"file": "plant.ssbx.json"`) is left for later. It would reuse a model across files; editing it from inside another file raises questions this ADR does not settle.
- **Rewinding one model alone** (per-model recorders, ADR 0064) is left for later. With links in both directions it would not reproduce the coupled run.
- **Links into parameters** (a probe setting a parameter, e.g. a fitted constant): this ADR feeds only input ports, and a typed block makes a parameter an input with an `input` line.

## Consequences

- A session holds several `Simulator`s and a scheduler over exact ticks. A file without `models` runs exactly as before (tested bit for bit).
- The loader, `check`, the canvas, run files, the Python results and the CLI output learn names prefixed by model and times per model.
- Coupling between models is explicit and sampled; error from coupling across clocks is the reading model's hold, as in co-simulation ([block contract](../architecture/block-contract.md)).
- Stepping models on several cores becomes possible without changing results.
- Supersedes nothing. ADR 0063's step 5 points here; ADR 0064's per-model recorders and the open points above stay for later.
