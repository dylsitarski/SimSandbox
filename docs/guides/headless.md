# Using SimSandbox headless (CLI and Python)

How people and AI agents describe, check, run and render models without the window ([ADR 0039](../decisions/0039-ai-agent-access.md), [ADR 0041](../decisions/0041-headless-interface.md)). The model file is the one input: see the [JSON Schema](../../crates/simsandbox-file/model.schema.json) and the [examples](../../examples), which double as templates.

## Workflow for an agent

1. `simsandbox describe` (or `ss.describe()`): every block type with ports, units, parameters, methods and labels; every view and filter type. `simsandbox describe eq.equations` gives the whole equation language (`reference`), which most models are written in, and its functions with their signatures and units (`functions`).
2. Write a model file (or build a `Model` in Python), starting from the closest example.
3. `simsandbox check FILE`: fix each error at its `path` (`blocks[0].method`) or `line`/`column`, until `"ok": true`.
4. `simsandbox run FILE --format json` (or `Model.run()`): the numbers, with speed and accuracy labels and notes on algebraic loops and on loops of equation blocks stepped apart ([ADR 0095](../decisions/0095-one-block-with-states-fuses-with-its-formulas.md): each then reads the others a step late, which the labels do not show).
5. `simsandbox render FILE --out view.png`: the views the app would show.
6. Explain it to the person who will read it ([ADR 0107](../decisions/0107-model-cards-and-agent-notes.md)):
   - Mark what you make or change with `"agent": true`: the model (its description, run, views, declarations), blocks (with their values) and wires. Python's `ss.agent_mode()`, or `SIMSANDBOX_AGENT=1`, marks what a `Model` adds or changes.
   - Write what cannot be derived:
     - the model's `description`;
     - each block's `notes` (`purpose`, `assumptions`, `reasons` for the values you set; `Model.note()` in Python);
     - `# …` after each typed-equations line, and `as ROLE` on parameters ([ADR 0109](../decisions/0109-parameter-roles-key-parameters-port-groups.md)).
   - Run `simsandbox check FILE --agent` (`Model.check(agent=True)`) until it is ok: it names each missing note at its path.
   - Read `simsandbox explain FILE` (`Model.explain()`), the model's card, as the person will. A person who edits a part, or marks it reviewed in the app, clears its mark.
7. To act on a run (push, steer, drag), write the changes as events: a schedule in the model (`run.inputs`) or a run file beside it, and replay it with `simsandbox run RUN.run.json` (see [Inputs and run files](#inputs-and-run-files)).

## CLI

Build with `cargo build --release -p simsandbox-cli`; the binary is `target/release/simsandbox`.

| Command | Output |
| --- | --- |
| `simsandbox describe [TYPE]` | JSON: `block_types`, `view_types`, `filter_types`, `units`; or one block or view type; `describe units` lists every unit symbol, its SI form and factor, and the prefixes ([ADR 0047](../decisions/0047-unit-vocabulary.md)) |
| `simsandbox schema [run]` | The model file's JSON Schema, or the [run file's](../../crates/simsandbox-file/run.schema.json) |
| `simsandbox check FILE [--agent]` | JSON report; with `--agent`, also an error at each path where a part an agent made (`"agent": true`) lacks its note (a model's description, a block's `notes.purpose`, a reason for each value set on it, a description for each typed-equations parameter and input; [ADR 0107](../decisions/0107-model-cards-and-agent-notes.md)), and `agent.missing` counts them; exit status 1 if the model cannot run, 3 if the file cannot be read. For a run file: its model is found and checked with the run's settings and events, and a model changed since the run was made is a warning, as is a run made by another SimSandbox version (its `made_with`) |
| `simsandbox run FILE [--t-end S] [--dt S] [--format csv\|json] [--inputs RUN]` | Every output at every step (or, for a model whose only runs are its studies, every run); JSON output names the SimSandbox that made it (`"made_with": "simsandbox 0.0.1"`). `FILE` may be a run file, which is replayed; `--inputs RUN` applies a run's events to a model file. `render` takes `--inputs` too |
| `simsandbox run FILE --signals NAMES --every N [...]` | Only the outputs named (comma-separated: `block.port`, `block` for all its ports; in a model using models also a model's name, `.` the top level, or a path in it, `plant/car.v`) and only every `N`-th step and the last: the same values, less memory and time ([ADR 0076](../decisions/0076-headless-runs-keep-what-is-asked.md)). A name that keeps nothing is an error listing the outputs |
| `simsandbox run FILE --probe NAME [--save-probes DIR] [...]` | What probe `NAME` recorded ([Probes](#probes)) instead of every output: CSV of `t` and its signals, or with `--format json` also its fields. `--save-probes DIR` (with or without `--probe`) writes the files of the probes that have one (`to`) into `DIR` |
| `simsandbox render FILE --out IMAGE.png [--t-end S] [--view N] [--width P] [--height P] [--scale S] [--light]` | PNG of the views after running to `t_end`, placed as the file's `controls.layout` arranges them ([ADR 0053](../decisions/0053-docked-panes-inspector-and-node-values.md)) or tiled in a grid |
| `simsandbox render FILE --diagram [--inside PATH] [--no-data] [--values] --out IMAGE.png [--width P] [--height P] [--scale S] [--light]` | PNG of the block diagram, or the inside of the model used at `PATH` (`car1`, `vehicle/plant`; `--subsystem` is its older name); `--expand car1,vehicle/plant` draws those uses expanded in place, as `controls.expand` does, as the app's Diagram tab shows it (also for a file that cannot run), with the filters and views as its data layer unless `--no-data` ([ADR 0050](../decisions/0050-data-layer-and-canvas-undo.md)); the nodes `controls.open` lists show their values, every node with `--values` ([ADR 0055](../decisions/0055-open-nodes-subsystem-values-deferred-windows.md)) |
| `simsandbox explain FILE [--format markdown\|json] [--part PATH] [--depth N]` | The model's card ([ADR 0107](../decisions/0107-model-cards-and-agent-notes.md)), generated from the file, one level at a time ([ADR 0110](../decisions/0110-encapsulated-models.md)): each model used is one part with its interface for that use (declared parameters and their values, inputs, outputs, what is wired to each) and what it holds; `--part car1` gives the card of a used model by its path of uses, `--depth 2` nests each use's own card. Its own blocks are parts by role (interaction, system, controller, formulas, study, inferred from the graph), each part's summary, equations or bodies, method with its labels and what it is solved with, its parameters by role (drive parameters at 0 that nothing drives are counted, not listed, in Markdown) and its inputs and outputs with their descriptions; how the parts connect, the wires between two parts as one bundle; the key parameters (declared, then pinned, else suggested by the types); what each view shows; what reads inside a used model by path (`reaching_inside`); the trust labels. Exit status 1 if the model cannot load or has no such part. Python: `ss.explain(model, part=, depth=)`, `Model.explain(format="json")` |
| `simsandbox start-from FILE --t S [--out NEW.json]` | The model file that starts from the state reached at `t` ([ADR 0042](../decisions/0042-node-canvas.md)): initial-condition parameters where they say it exactly, else a `start` entry per block |
| `simsandbox upgrade FILE [--out NEW.json]` | The file in the current form of the format; what was converted on standard error ([ADR 0044](../decisions/0044-unsaved-changes-and-older-files.md)) |

Notes, warnings and labels go to standard error; data to standard output.

### Help, exit status and errors

`simsandbox --help` (or `-h`, `help`) lists the commands; `simsandbox COMMAND --help` (or `help COMMAND`) gives a command's options. How a command fails sets its exit status ([ADR 0099](../decisions/0099-cli-exit-codes-and-json-errors.md)):

| Status | Kind | When |
| --- | --- | --- |
| 0 | | Success |
| 1 | `model`, `run` | The model cannot load or `check` found errors (`model`); the run or the drawing failed (`run`) |
| 2 | `usage` | An unknown command or flag, a missing or bad argument; also no arguments at all |
| 3 | `io` | A file could not be read (the model, a run file or its model, `--inputs`) or written (`--out`, `--save-probes`) |

`--errors json`, anywhere on the line, makes every line on standard error one JSON object, for every command:

```json
{"error": {"kind": "model", "message": "m.json cannot run (see \"errors\")", "file": "m.json", "errors": [{"path": "blocks[0].method", "message": "..."}]}}
{"warning": {"message": "...", "file": "m.json"}}
{"note": {"message": "..."}}
{"labels": {"speed": "RealTime", "accuracy": "Quantitative"}}
```

A failure has `kind` and `message`, and where they exist `file`, `errors` (as in the `check` report: `run`, `render` and `start-from` of a model that does not check give them too) and, for a usage error, `usage` (the command's synopsis). The other objects are `warning` (with `file` when known), `note` (with `model` in a model using models), `labels`, `converted` (from `upgrade`) and `wrote` (a probe's file). Without the option (`--errors text`, the default) the same lines are text: `error: ...`, `warning: ...`, `note: ...`.

### `check` report

| Key | Meaning |
| --- | --- |
| `ok` | The model loads and can run |
| `errors` | Each `{path, message}` (file structure) or `{line, column, message}` (JSON syntax) or `{message}`. Every block's error and every wire's (between blocks that were built) are listed; other stages, and a typed-equations block's own lines, stop at the first ([ADR 0099](../decisions/0099-cli-exit-codes-and-json-errors.md)) |
| `warnings` | Problems that were worked around, e.g. a saved view that cannot be shown |
| `conversions` | Older forms of the format the file was written in, converted on reading; `simsandbox upgrade` writes the current form |
| `params` | Every parameter: `key` (`block.param`; a model parameter's `name`, a used model's `use.name`; blocks inside uses `use/inner.param`), `value`, `formula` (set when a formula computes it), `unit`, `suggested` range, `initial_condition`, `description` |
| `definitions`, `blocks` | Each model defined in the file, with its input blocks, declared outputs and uses ([ADR 0068](../decisions/0068-one-kind-of-nested-model.md)); every block after flattening, as signals and sliders name them |
| `labels` | Speed and accuracy of the model's methods |
| `recorded` | `signals` (scalars) and `fields` (grids, point sets, populations) that views and probes can take, recorded now or not |
| `code` | How each block made from equations runs, by name: `interpreted` (small arrays, or `SIMSANDBOX_INTERPRET`), `compiled` (Cranelift), `native` (a library built from it), or `compiled (native code building)`, `compiled (native code not built: …)`, `compiled (native code differed)`; when its programs differ, each named (`rates: native, outputs: compiled`). Blocks written in Rust are not listed. A model that uses models gives each model's under `models[].code` ([speed of typed equations](#speed-of-typed-equations)) |
| `fused_groups`, `swept`, `filters`, `views` | How blocks are solved together, the swept parameters, filter and view names |

### `run --format json`

| Key | Meaning |
| --- | --- |
| `rows` | Number of steps (including t = 0) or runs |
| `index` | `t`, or `run`, as lists |
| `outputs` | `block.port` → `{shape, values}`; a scalar's values are a list, others a list per row in the port's element order (grids row by row, x fastest) |
| `populations` | `block.port` → `{dim, frames}`: per row, a list of points |
| `over_rows` | A quantity over a table's rows ([ADR 0079](../decisions/0079-tables-of-members.md)): `block.port` → `{dim, versions, ids, values}`, each a list over rows (the table's version, the rows' ids, their values in that order) |
| `labels`, `notes`, `warnings` | As in `check` and on standard error |

JSON has no infinities or NaN; they are written as `null`. CSV leaves populations out, and writes a table's rows by id: a column per id any row had (`pop.size#7`), NaN where that row is not there.

## Speed of typed equations

Large grids in typed equations run compiled, with the same results bit for bit whichever way ([ADR 0084](../decisions/0084-where-typed-equations-stop.md), [ADR 0089](../decisions/0089-native-code-for-typed-equations.md)). Environment variables choose how:

| Variable | Effect |
| --- | --- |
| `SIMSANDBOX_INTERPRET=1` | The interpreter only (the reference), for comparing |
| `SIMSANDBOX_NATIVE=off` | No native code: Cranelift's code only |
| `SIMSANDBOX_NATIVE=cache` | Native libraries already built are used, none built (the CLI's and Python's default) |
| `SIMSANDBOX_NATIVE=build` | Missing libraries are built in the background with the installed Rust toolchain (the app's default); a run uses one once it is built and has given the same bits |
| `SIMSANDBOX_CACHE=DIR` | Where native libraries are kept (default: the platform's cache folder, under `simsandbox/native`): a folder of your own. On Unix nothing is loaded from a cache that other users can write to; the 200 libraries used most recently are kept ([ADR 0096](../decisions/0096-the-native-cache.md)) |

`check` reports how each block runs (`code`). Moving parameters (sliders, `--set`, driven parameters) never compiles anything; editing an equation, a grid's cells or an edge kind does (milliseconds for Cranelift; for a native library, in the background while the run goes on).

## Probes

Probes ([ADR 0062](../decisions/0062-probes-recording-and-replay.md)) record chosen signals and fields on their own schedule, and can write files. They are saved in `controls.probes`:

```json
{"name": "pattern", "record": ["gs.v"], "every": {"seconds": 50}, "keep": {"all": true}, "to": "gs-pattern.csv"}
```

| Key | Meaning |
| --- | --- |
| `record` | Signals and fields by the names views bind (`car.v`, `gs.v`, a model filter's name); may be empty |
| `every` | One of `{"steps": N}`, `{"seconds": dt}`, `{"times": [...]}`: at the first step at or after each; every step if absent |
| `keep` | One of `{"samples": N}` (thinned past N samples), `{"mb": size}` (thinned past it), `{"last": N}`, `{"all": true}`; thinned past 100 000 samples if absent. Applies only to samples held in memory |
| `to` | A file name ending in `.csv`: `t` and the signals, one row per sample; each grid or point field in `<stem>.<field>.npy` beside it (`(samples, ny, nx)` or `(samples, count, dim)`, read with `numpy.load`) |
| `stream` | `true`: write `to` as it records (buffered, flushed about every 2 s and when the run stops); the file holds every sample, `keep` does not thin it, and rewind cuts it back. Needs `to`; the app writes it once the model is saved |
| `canvas` | Where its node sits on the diagram's data layer, where `render --diagram` draws it |

A view binds a probe's recording as `probe:NAME/signal` (`"y": ["car.v", "probe:speed/car.v"]`): the model's every step and the probe's samples on one plot. Where a view pairs signals from different sources (x against y, colour), it uses only the times both recorded. A filter's inputs may come from probes: `"from": {"car.v": "fast", "cruise.F": "slow"}` (or `"probe": "NAME"` for all of them); it then reads only what those probes record and runs where their times pair, or with `"when": "any"` whenever one of them records, with the others' latest values. Its outputs are bound by name, as any filter's ([ADR 0063](../decisions/0063-probes-on-wires.md)). The model's every-step recording keeps only what views and filters read from the model.

A probe keeps its samples in memory only when something reads them: a view or filter wired to it, a file written at the end (`to` without `stream`), or `run --probe` (which keeps every probe it is given). A probe that only streams, or that nothing reads, holds nothing, so `keep` has no effect on it. `render --diagram --no-data` hides the data layer but keeps probes that feed blocks: they are part of the model.


A probe with bad settings, or recording something the model does not record, is dropped with a warning. `simsandbox run FILE --probe NAME` prints one; `--save-probes DIR` writes the files; `--memory MB` sets the app's memory budget (default 1024 MB; probes keeping what they record by default thin past a quarter of it, ADR 0062 step 5), as Python's `Model.run_probes(memory=MB)` does, whose `ProbeRun.memory` reports what was held. A swept model records no probes. JSON output of a probe: `rows`, `t`, `signals` (name → list), `fields` (name → `{shape, frames}`; a table's rows as `{version, ids, values}` per sample). A probe recording a table's rows writes them beside its CSV as `<stem>.<field>.csv`, a line per row per sample (`t,version,id,value`).

A model-level connection may come from a probe: `{"from": "probe:sensor.car.v", "to": "cruise.v"}` feeds `cruise.v` with `car.v` as probe `sensor` samples it, held between samples ([ADR 0063](../decisions/0063-probes-on-wires.md)). The probe must record that signal, a whole output of a block on the model's level. Such a probe changes results: it counts in a run file's model hash.

## Data files as inputs

An `input.file` block reads recorded data as an input over time: a probe's file, or any CSV or `.npy` ([ADR 0063](../decisions/0063-probes-on-wires.md)). File names are read from the model file's folder (absolute names work anywhere); the CLI and Python (`Model.load`, `Model.save`) use the model's folder, and a model never saved can name files only by full path.

```json
{"name": "recorded", "type": "input.file", "settings": {"file": "speed.csv", "unit.car.v": "m/s"}}
```

| Setting | Meaning |
| --- | --- |
| `file` | A `.csv`: first column the time, each other column a scalar output named by its header with characters other than letters, digits and `_` made `_` (`car.v` gives `recorded.car_v`). Or a `.npy` (`float64` or `float32`): one output `y`, its first axis the samples, `(samples)` a scalar, `(samples, n)` a vector, `(samples, a, b)` a grid of `a` rows of `b` |
| `data` | Instead of `file`: the samples as CSV text in the model itself (`"t,x\n0.5,0.59\n1,-0.23\n"`), so a small data set travels with the model (the fitting example) |
| `interpolate` | `hold` (default): the last sample, as a probe feeding a block gives it; `linear`: a straight line between samples. Before the first sample, the first; after the last, the last. A step's time within rounding of a sample's (steps summed from `dt` drift: 2.4999999999998 for 2500 steps of 0.001) counts as that sample's |
| `unit`, `unit.COLUMN` | Units of the outputs, unless a CSV header gives one as `name [unit]`; dimensionless otherwise |
| `times` or `dt` | For a `.npy`: a CSV whose first column gives the times (a probe's CSV beside its `.npy` files), or seconds between samples from 0 |
| `shape` | For a `.npy` of 3 axes: `grid` (default) or `points` (`a` points of `b` coordinates) |

The file is read when the model is built (opened, or changed on the canvas). Its contents count in a run file's model hash: the same model with a different data file is a different model. `describe input.file` lists the settings that name files (`files`). The value is taken at each step's end and held over the next step, like any block's output; the block never joins a fused group.

**Time from data** ([ADR 0067](../decisions/0067-experiments-and-time-from-data.md) step 4): a model with no states that reads data, and writes no `run.dt` of its own, runs at the data's own sample times, even irregular ones: each sample time is an instant (exact, as decimals join the clocks' common tick), and without `run.t_end` the run ends at the last sample. It is not swept. A model that writes a `run.dt`, or has states, runs every `dt` as before, the data held (or interpolated) between samples.

## Models that use model files

A model's `blocks` may use other model files, each running on its own clock, linked by probes. Uses nest to any depth ([ADR 0066](../decisions/0066-models-nest-as-blocks.md)). A use has a `file` in place of a `type`. Here `car.ssbx.json` offers `"outputs": [{"name": "v", "from": "car.v"}]`, and `cruise.ssbx.json` declares `kp` and offers `F`:

```json
{"format": 1, "run": {"t_end": 60},
 "blocks": [{"name": "plant", "file": "car.ssbx.json"},
            {"name": "controller", "file": "cruise.ssbx.json", "params": {"kp": 600}, "clock": {"dt": 0.5}}],
 "connections": [{"from": "probe:speed.plant/v", "to": "controller/speed"},
                 {"from": "probe:force.controller/F", "to": "plant/force"}],
 "controls": {"probes": [{"name": "speed", "record": ["plant/v"]},
                         {"name": "force", "record": ["controller/F"]}]}}
```

**Encapsulation** ([ADR 0110](../decisions/0110-encapsulated-models.md)): what changes a used model, or feeds a result from it, goes through its interface. A use sets only its model's declared parameters, and wires and links read only the outputs it offers. Views, probes that only record, filters and `run --signals` may still read inside by path (`plant/car.v`). To make the interface:

- **Python:** `m.expose(model, "car.m")` declares a value inside `model` (a model defined in the file, or `None` for this one) as its parameter and returns the name; `m.offer(model, "car.v")` offers an output. Setting a value inside a use by path (`add_model(..., params={"car.m": 2})`, `set("plant.car.m", 2)`) or wiring or linking from an inner output (`plant/car.v`) is refused, naming the fix.
- **Older files** convert as they are read, with results unchanged, each reported by `check` and `upgrade` under `conversions`. For a model defined in the file: a value a use set by path (`car.m`, `plant/car.m`) becomes a declared parameter of that model, its default the value the model had, read by formula inside; a wire or link from an inner output becomes an output the model offers (`car1/car.v` → `car1/v`). Paths several levels deep convert level by level. A value set inside by a formula (other than a declared parameter's name) stays, with a warning.
- **Separate model files** are not rewritten because another file uses them: what reaches inside one still loads, and `check` warns at its path with the fix (expose or offer it in that file).

- **Uses:**
  - `file` is found from this file's folder, and data files inside a use from its own file's folder.
  - `params` set the used model's declared parameters for this use only (`m`). Each value is a number or a formula of the declared parameters of the file it is written in. On a shared clock the formula is live. Into a model on its own clock it is evaluated once, when the model is built.
  - `clock` is `{"dt": step}`, `{"times": [0.5, 1.2, 4]}` to step from one listed instant to the next (positive, ascending; [ADR 0067](../decisions/0067-experiments-and-time-from-data.md) step 4), or absent for the file's own `run.dt`.
  - `"clock": "shared"` puts the use on this model's clock instead, flattened into it (fused with its neighbours where possible). Wire straight into its input blocks (`{"from": "controller/F", "to": "plant/force"}`) and from the outputs it offers (`plant/v`), at any depth of models on a shared clock (`vehicle/plant/force`). Any model file can be used shared: models on their own clocks inside it keep their clocks under its path (`vehicle/engine`), and its probes, links and groups come along.
  - Everything in a use is named with the path of uses leading to it: `plant/car.v`, `vehicle/plant/car.v`. The top level's own blocks keep plain names, and a model may have blocks of its own beside its uses.
  - A used file may use files itself; a file that ends up using itself is refused.
- **What is read from a used file:** only `run.dt`, plus the probes that feed its links or its own blocks. The top level's `run` governs, and inner models draw random numbers from seeds mixed from the top level's and the use's path.
- **Links:** a link goes from a probe's output to an `input.value` block in another model, at any depth (`vehicle/controller/speed`), which takes the probe's samples in place of its slider. Units must match. A probe records from one model.
- **How clocks interleave:**
  - Clocks count exact ticks (0.1 and 0.3 s share a tick of 0.1 s; 0.1 and 0.3333333333333333 s one of 10⁻¹⁶ s, with 128-bit counts).
  - Each model runs to its last step at or before `t_end`.
  - Between its steps a model advances with its linked inputs held. At an instant several models share, a model that feeds another evaluates first.
  - In a loop, the model written first evaluates first and takes the other's sample right after. Only an output depending on it at that same instant is late, which is reported.
- **`check`:** lists each model (`name`, which is its path, or `.` for the top level's own blocks; `file`, `dt`, its outputs and the `inputs` links can drive), the links, the tick, the order, the loops and `model_hash`.
- **`run`:** `--format json` gives every model at its own times. CSV needs `--model NAME`, which then records only that model unless `--signals` says what to keep.
- **Python:** `Model.add_model(name, file, params=, dt=, shared=)`, `Model.link("plant/v", "controller/speed", every=)`, `Model.expose` and `Model.offer`. `Model.run()` returns `Composed`, read with `.model("plant")`, `.t("plant")` or `composed["vehicle/plant/car.v"]`.
- **Models without states** (formulas, input blocks) run on new data. Given no clock, such a model is evaluated at the start, whenever a link into it delivers a sample, and at its own recorded data's sample times, at that time. When only such data drives a run (no clock every `dt`) and no `t_end` is given, it ends at the last sample. A model that feeds it is evaluated first, and its answer reaches others in the same instant. Given a clock, it is evaluated at each step instead. A use runs its model as written; a study block runs it at many points ([Studies](#studies)). With nothing in time anywhere, the run is one evaluation of every model at t = 0, in link order: a pipeline such as features → an ML model → post-processing ([ADR 0067](../decisions/0067-experiments-and-time-from-data.md)).
- **Exposed inputs** ([ADR 0074](../decisions/0074-model-interfaces.md)): a model may expose its blocks' inputs, `"inputs": [{"name": "lead", "to": ["control.lead"], "unit": "m/s"}]`, without an input block. They are signals with no default: every use wires them (`car1/lead`) and every study is wired to them (`s.lead`), or the model using it is refused, naming the input; the model itself does not run on its own. Input blocks still act as inputs, with their own value, as before.
- **Driven parameters** ([ADR 0075](../decisions/0075-parameters-are-inputs-with-defaults.md)): a connection may end at a parameter, `{"from": "clock.k", "to": "decay.k"}`, a used model's declared parameter (`car1/mass`) or a study's (`curve.m`); the signal then replaces the value. An ordinary parameter follows the signal at every evaluation, a starting value takes it when the block starts (and at Restart), a study runs again when it changes. A wired parameter keeps its own value in `params` (or its default), unused while the wire drives it and used again once the wire goes. Driving a used model's parameter drives the inner parameters set to it alone (`"m": "mass"`); one read in another formula (`"2*mass"`) cannot be driven, and `check` says which. `check` lists what is driven (`driven`, and `driven_by` on each parameter row); `render --diagram` draws the wire to the value it drives, listed first on its node with its source (`from push.F`). The forced spring example shows one spring model with its force set in one use and driven in another. A model's inputs, outputs and declared parameters share one set of names. Not yet: driving a parameter of a model on its own clock, or a whole array parameter from one vector signal (each element `m[2]` can be driven).
- **Declared outputs:** a model file may declare what it offers, `"outputs": [{"name": "speed", "from": "car.v"}]` (`from` may also be a path into a model it uses). Users then wire and link to `plant/speed`, which stays valid when the file's insides change. `check` lists them as `declared_outputs`.
- **Replacing files inside a use:** `{"name": "vehicle", "file": "vehicle.ssbx.json", "files": {"plant": "car-2.ssbx.json"}}` runs the vehicle with that car in place of the one its file names, for this use only; keys are paths of uses inside it (`plant/engine`), files are found from this file's folder. `check` lists them as `replaced_files`.
- **Models defined in the file** ([ADR 0068](../decisions/0068-one-kind-of-nested-model.md)): `"models": {"wheel": {"blocks": [...], "outputs": [...]}}`, used as `{"name": "front", "model": "wheel"}`, as often as needed; a definition inside a definition finds the outer ones. Its input blocks are its inputs, its declared outputs its outputs. Without `clock`, a wired model that writes no `run.dt` runs on its user's clock. `starts` sets start states inside by path. `check` lists them under `definitions`. Python: `Model.define("wheel", model)` and `add_model("front", model="wheel")`. The app's "Make a local copy" writes one.
- **`render --diagram`** draws uses as nodes with their clock and file. A node shows the used model's input blocks, its declared outputs, and whatever is wired by path. Every other port stays reachable by path; a signal a declared output already shows is reached by that name only ([ADR 0073](../decisions/0073-ports-and-offered-outputs.md)). Exposed ports are drawn ringed. In the app, a click on a port or a parameter's ring of a block inside a model (opened, or expanded in place) exposes it or hides it again ([ADR 0074](../decisions/0074-model-interfaces.md)); offered outputs are also listed in the Inspector's Model section. To see inside a used model, render its own file (the app's "Open" shows the same diagram).
- **Inputs, probes and run files** ([ADR 0066](../decisions/0066-models-nest-as-blocks.md) step 5): `run --inputs RUN`, `--probe NAME`, `--save-probes DIR` and run files work, the models then running as one on their clocks, with every instant of any clock as a row; keys and signals are full names (`slow/u.y`, `car3/car.v`). `--dt` is refused (each model keeps its own step). Python: `run(inputs=[(t, "slow/u.y", v)])` returns `Results`.

## Studies

A model is run at many points only by a **study block** ([ADR 0070](../decisions/0070-studies-are-blocks.md), [ADR 0071](../decisions/0071-sweeps-are-studies.md)): a file's top-level `sample` is refused, saying to put the model under `models` with a study over it, and the older `controls.sweep` is dropped with a warning. A model with no states and no study is not swept: it runs in time like any other, its formulas constant unless their inputs change. A study's `sample`:

```json
"sample": {"design": "list", "params": [{"param": "cruise.kp"}], "values": [[200], [400], [800]],
           "seeds": "common", "replicates": 1}
```

- **Points:** `grid`, `list`, `random`, `latin_hypercube`, `sobol`, or `"design": "file", "file": "points.csv"`, a CSV whose header names the parameters; its contents are part of the run hash. A column named `id` gives each point an id (whole numbers from 0, each once), the study's `id` output; without one, a point's id is its place. A parameter is `block.param` inside the studied model by full name (`plant/car.m`) or a declared parameter (`kp`).
- **Points from inputs and search strategies** ([ADR 0078](../decisions/0078-adaptive-studies.md)): `"design": "inputs", "count": 5` takes the points through the study's inputs, one array input of `count` cells per swept parameter (named as its output would be: `cruise.kp` is the input `cruise_kp`), wired from another model. The swept parameters are not outputs then; the results are. Each point's results are kept by its values: when the inputs change, only points whose values (or the model's other inputs and parameters) changed run again, under their own ids, so seeds and results are those of running every point again. A strategy is a model that chooses the points from earlier results: a map (`x[n+1] = …`, one step a round) reading the study's results as arrays and sending it its next points. `examples/oscillator-fit.ssbx.json` fits a damped oscillator's `k` and `c` to measurements by Levenberg–Marquardt: four points a round (the centre, a step on each parameter for the Jacobian, the candidate), residuals `at_samples(osc.x - data.x, data)`, and `solve` for the damped step; it reaches the least-squares fit SciPy finds. `examples/bistable-continuation.ssbx.json` is continuation: one run a round, at the next value of a parameter, starting from the state the last run settled at (the study sweeps the starting value `bi.x0` too), which traces a hysteresis loop. `examples/bistable-refinement.ssbx.json` is refinement: the starting values as a table with edges between neighbours, a study with `"design": "rows"` running one per row, and `split` of every interval whose ends end on different branches, until it brackets the boundary between the basins within a tolerance. Python writes these strategy models for any number of parameters: `Model.minimize(target, params, objective)`, `fit(target, params, residuals)`, `refine(target, params, result)` (`params` maps each parameter to its range, one or more: a mesh of simplices over them, segments over one, refined where the result changes) and `continue_(target, param, range, step, state, settled)`, the target a study block, a used model or `None` (the whole model); the app's study form has the same under "Search…" ([ADR 0078](../decisions/0078-adaptive-studies.md) step 5). `examples/cruise-search.ssbx.json` is pattern search over two gains: the centre and a step either way on each, moving to the best point or halving the step, holding still once the step is below a tolerance (and then nothing runs again). Maps may use grids for this (`grid k = 5`, `input cost[k]`, array states declared `state c[k] = …`).
- **A point per row of a table** ([ADR 0079](../decisions/0079-tables-of-members.md) stage 1d): `"design": "rows"` (no `count`, no `replicates`) takes each swept parameter's values as a table's rows through the study's input of its name (`input` port carrying rows, wired from a quantity over a table such as `pop.x0`); each row is a point, and each result leaves as rows with the same ids (`halves.half`, read downstream by a block with `table q` and `input half[q]`). Each row's results are kept by its id: only rows that are new, or whose values (or the study's other inputs and parameters) changed, run again, under their ids, so seeds and results are those of running every row. Several swept parameters are joined by id (NaN where one lacks a row).
- **Runs:** each point (times its replicates) runs the whole model from its start to its `t_end`, on all cores (the results do not depend on how many), or is one evaluation if the model has nothing in time.
- **Seeds:** `"common"` (default) gives every point the run's seed; `"per point"` mixes in the point's id and the replicate ([ADR 0070](../decisions/0070-studies-are-blocks.md)), so with ids from the file a point draws the same however the file is ordered or cut. `"replicates": n` runs each point n times, replicate j drawing alike at every point.
- **Reductions:** `"reduce": {"v_max": "max(plant/car.v)", "settle": "first_time(abs(cruise.e) < 0.1)"}` gives each run these named values instead of final ones. The reductions are `final`, `mean` (over time), `min`, `max`, `amplitude` (half of max − min), `period` (mean time between upward crossings of the mean) and `first_time(e)` (the first time `e` holds, is not 0; NaN if never), each over an expression of signals by full name and `t`. A result is any expression of reductions: `max(car.v) - final(car.v)`, `final(a.x)/final(b.x)`; a signal outside a reduction is refused. `min(a, b)` and `max(a, b)` with two arguments stay functions of two numbers. Two reductions give a vector per run ([ADR 0080](../decisions/0080-named-axes.md)): `at_times(e, 1, 2.5)`, `e` at each given time, and `at_samples(e, data)`, `e` at each sample time of the recorded-data block `data` (an `input.file` in the model) within the run, for model-minus-data residuals (`at_samples(car.v - data.v, data)`). Each takes the first recorded step at or after the time, within rounding, as a data block takes a sample's value (NaN past the end). Scalar reductions in the same formula go with every element; vectors in one formula have one length. A study's vector result is over two axes, its own (fastest) and the runs (`grid obs = 3`, `grid run = 5`, `input res[obs, run]` reads it), not over a grid of two parameters; headless runs and Python give it as a vector per row.
- **Comparisons:** `<`, `<=`, `>`, `>=` give 1 or 0 in every expression (typed equations, formulas, filters, reductions), both sides in one unit, at most one per pair of parentheses: `max(car.v > 30)` is 1 if the car ever went above 30.
- `"keep": "series"` (each whole run) is refused: a study keeps one value per run ([Q52](../open-questions.md)).
- **Rows:** a model whose only runs are its studies (no states, no clocks, no scheduled inputs) is evaluated once, and `run` writes one row per run, indexed by `run`, with a column per study output (`kp.v_max`); every study must make the same number of runs, and other outputs are printed as notes. Giving `--t-end` or `--dt` runs it in time instead.

### Study blocks

A **study** is a block that runs a model at each point of its own `sample` and outputs the results as arrays ([ADR 0070](../decisions/0070-studies-are-blocks.md)). It is how a model is studied without changing its file, and how studies nest.

```json
"models": {"cruise": {"blocks": [...], "connections": [...], "run": {"dt": 0.05, "t_end": 60}}},
"blocks": [{"name": "kp", "type": "study", "model": "cruise",
            "sample": {"design": "list", "params": [{"param": "cruise.kp"}], "values": [[200], [800]],
                       "reduce": {"v_max": "max(car.v)", "settle": "first_time(abs(cruise.e) < 0.5)"}}}]
```

- **Model:** named as a use names it: `"model"`, a definition in scope, or `"file"`, a model file found from this file's folder (its contents are part of the run hash). In the app, "Open" on a study's node goes inside that model to see and edit it, with the same choices as a used model's (the definition, the file, or a copy); renaming the definition renames it in the study. Its `sample` is as above.
- **Parameters:** the model's declared parameters (`params` of its model) are the study's own parameters ([ADR 0074](../decisions/0074-model-interfaces.md)): the model's values are their defaults, the study sets its own in its entry's `params` (`"params": {"c": 0.2}`), shown on its node and in the Inspector and pinned as sliders like any parameter; a change runs it again. Its points may also vary them by name (`"param": "w"`, output `w`).
- **Inputs:** the model's input blocks (`input.value`), one scalar input each, named as the block; every point takes their values. An input block the sample sweeps is not an input.
- **Outputs:** lines of cells with one value per run (points times replicates): each swept parameter (`cruise.kp` becomes `kp.cruise_kp`), then each `reduce` result, or without `reduce` each scalar output's final value (`car.v` becomes `car_v`) except its input blocks'. Units: a parameter's or output's own; `period` and `first_time` in seconds; a reduction of one signal in its unit; other expressions dimensionless. Their shape follows the design ([ADR 0071](../decisions/0071-sweeps-are-studies.md)): a grid over one parameter, run once per point, gives that parameter's values as the coordinate of every result; a grid over two parameters, run once per point, gives 2D grids of `n1 × n2` cells (the first parameter fastest), both parameters as coordinates placing them, so heat maps, surfaces, contours and cuts read them directly; any other design, or replicates, lines indexed by run. `plot.xy` draws lines of cells against each other cell by cell (`x`, `y` and `colour` bound to a study's outputs), the way to see an unordered design.
- **Reading them:** a typed-equations block numbers the runs with an index axis and reads them as arrays: `"grid k = 5", "input amp[k], w[k]", "w_peak = at(w, argmaxof(amp))"`. `argmaxof` and `argminof` give the cell of the greatest or least value (the first of equals; NaN if all are NaN), and `at(T, i)` takes a computed cell number (rounded; NaN outside). The axis's number of cells must match the study's runs; the wire is refused otherwise. 2D results are read with two index axes (`grid i = n1`, `grid j = n2`).
- **When it runs:** whenever its inputs change, each point from the model's start to the model's `t_end` (one evaluation if the model has nothing in time). The results are kept until the inputs change. Only the outermost study uses several cores; the results never depend on how many.
- **Studies of studies:** studies nest to any depth; a model file whose studies lead back to itself is refused, naming the chain. A definition holding a study can itself be studied: the outer study's points set the inner one's inputs through input blocks, and its `reduce` reads one run of the inner arrays by element (`final(inner.half[1])`).
- **Run hash:** a study's points file and the data files its model reads are part of the run hash of the model holding it, as `s:sample.file`, `s:ramp.file`.
- The app opens a model, and checks a diagram edit, without running its studies; its worker runs them first. A model with nothing in time (no states, tables, data, clocks or scheduled inputs; formulas and studies only) is evaluated, not run ([ADR 0072](../decisions/0072-models-with-nothing-in-time.md)): once, and again whenever a value it reads changes, with no clock or rewind. A model that runs in time runs each study within the step that needs it, showing its runs done of runs; changing a parameter or input, Pause or Run gives the study up, undoes the step and runs it again from the new values, so results never depend on timing. It opens such a model on a profile of the study's results (against the swept parameter for a grid over one parameter, else against the run's number), or on a heat map of its 2D results for a grid over two.
- **Making one:** "Study this model" on a used model's node (Python `Model.study("car1")`) adds a study block beside it over the same model: a grid of 20 points over its first parameter (a declared parameter, else the first block parameter), its inputs fed as the use's input blocks are. On the canvas background (Python `Model.as_study(("spring.w", 0, 3, 600))` or `as_study(sample={...})`) it studies the whole model: its blocks move into a new definition used in their place ("Make model"), with a study of it beside. The Inspector shows the model it studies and the number of runs, and edits its sample as a form (design, a row per parameter chosen from the model's, count and seed, replicates and seeds, results) or as JSON, applied when a field is left or a choice made; renaming the definition renames it in the study.

## Scenes

A `plot.scene` view draws a picture from layers of primitives (`points`, `segments`, `boxes`, `discs`) whose attributes are bound to numbers, signals, rows of tables or model parameters, in frames that chain ([ADR 0093](../decisions/0093-scenes-from-primitive-layers.md), [visualization](../architecture/visualization.md#scenes)). `simsandbox describe plot.scene` lists the primitives, each attribute's kind, unit and default, and the palette; `check` warns about a layer that does not fit, at its path (`controls.views[1]: layers[2].bind.x: 'car.v' is in m/s; the scene's lengths are in m; not drawn`); `render` draws it, or in its place `Not drawn:` and the error, and `render --diagram` marks its node and the input at that path. The pendulum example draws its pendulum this way beside the `eq.pendulum` view:

```json
{"type": "plot.scene", "unit": "m", "extent": {"x": [-1.3, 1.3], "y": [-1.3, 1.3]},
 "frames": {
   "pivot": {"angle": -1.5707963267948966},
   "rod": {"in": "pivot", "angle": "bob.angle"}},
 "layers": [
   {"draw": "boxes", "bind": {"colour": "grey", "height": 0.08, "width": 0.3, "x": 0.0, "y": 0.04}},
   {"draw": "segments", "in": "rod", "bind": {"colour": "grey", "width": 3.0, "x1": 0.0, "x2": "bob.L", "y1": 0.0, "y2": 0.0}},
   {"draw": "discs", "in": "rod", "bind": {"colour": "orange", "radius": 0.08, "x": "bob.L", "y": 0.0}}]}
```

The pendulum's angle is measured from straight down and a scene's counter-clockwise from +x, so the rod's frame sits in a fixed frame turned by −π/2: a constant pose, not a calculation in the model. A layer evaluates no expressions: a position that must be computed is a model output or an `expression` filter. Every length is in the scene's `unit`; a signal in another length unit is refused (convert it in the model). `simsandbox render examples/pendulum.ssbx.json --view 1 --out scene.png` draws it alone.

The rod's end and the bob are at `bob.L`, the rod length, a parameter ([ADR 0102](../decisions/0102-scene-bindings-may-name-parameters.md)): a binding names a scalar signal if the model has one of that name, else a quantity over rows, else a block parameter (`input.pointer`'s `hand.x` is its output). The session records each parameter a view binds every step, as it records signals, so the picture follows a change (a slider, a run file's `track`, a scheduled change in `run.inputs` such as `[{"t": 0.5, "set": "bob.L", "to": 1.25}]`) at the step it applies, and a rewound or replayed run draws it as it was. Its unit is checked as a signal's.

From Python, `Model.add_scene` adds one in the same form and refuses a wrong form, a binding that names no block, filter or probe of the model, or a `grab` that names no `input.pointer` block, when it is called; units and whether a block has that output are left to `check`. The cart-pole picture of ADR 0093 point 8, on a model whose blocks `cart` and `pole` give `cart.x` and `pole.theta`, with an `input.pointer` named `hand`:

```python
m.add_scene(
    unit="m",
    extent={"x": [-3, 3], "y": [-0.5, 2]},
    frames={"cart": {"x": "cart.x", "y": 0}, "pole": {"in": "cart", "angle": "pole.theta"}},
    layers=[
        {"draw": "segments", "bind": {"x1": -3, "y1": 0, "x2": 3, "y2": 0}},
        {"draw": "boxes", "in": "cart", "bind": {"x": 0, "y": 0, "width": 0.4, "height": 0.2}, "grab": "hand"},
        {"draw": "boxes", "in": "pole", "bind": {"x": 0, "y": 0.5, "width": 0.04, "height": 1.0}},
        {"draw": "discs", "in": "pole", "bind": {"x": 0, "y": 1.0, "radius": 0.05}, "grab": "hand"},
    ],
)
m.check()["warnings"]                    # a layer that does not fit, at its path
m.render("scene.png", t_end=1.0, view=0)
```

**Templates** ([ADR 0113](../decisions/0113-views-bind-to-interfaces.md)): a picture written once with an interface, and drawn wherever it is bound. A template is a view (here a scene) under the file's `templates`. It declares `inputs` and `params`, which it names with a `$`, and binds no signal of its own. A scene draws it as a layer, in a frame, binding its inputs. The cart-pole example draws its cart and pole this way:

```json
"templates": {"cartpole": {"type": "plot.scene", "unit": "m",
  "inputs": {"x": {"unit": "m"}, "theta": {"unit": "rad"}, "grab": {"kind": "pointer"}},
  "params": {"cart_colour": {"value": "blue"}, "pole_colour": {"value": "orange"}},
  "frames": {"cart": {"x": "$x", "y": 0.0}, "pole": {"in": "cart", "angle": "$theta"}},
  "layers": [{"draw": "boxes", "in": "cart", "bind": {"x": 0, "y": 0, "width": 0.4, "height": 0.2, "colour": "$cart_colour"}},
             {"draw": "boxes", "in": "pole", "grab": "$grab", "bind": {"x": 0, "y": 0.5, "width": 0.05, "height": 1.0, "colour": "$pole_colour"}}]}}
```

and in its scene's `layers`: `{"draw": "template", "name": "rig", "template": "cartpole", "bind": {"x": "cartpole.slider_position", "theta": "cartpole.hinge_angle", "grab": "hand"}}`. Its frames are named `rig/cart` and `rig/pole`; a parent may draw only in frames a template `offers`. A whole view can be a template too: `{"template": "phase", "bind": {"a": "osc.x", "b": "osc.v"}}`. A model that uses another may draw the templates it offers, by path: `{"draw": "template", "name": "left_rig", "template": "left/cartpole", "in": "left_track"}` binds `x` and `theta` to `left/x` and `left/theta`, the outputs the cart-pole offers, and its pointer input `hand` to the cart-pole's pointer block `left/hand`, unless `bind` says otherwise. Grabbing it sends the pointer in the frame the template is drawn in (`grab_in`), so the cart-pole reads it as in its own scene (`examples/two-cartpoles.ssbx.json`). `explain` lists a model's templates and each use's pictures.

A block type may bring pictures of itself ([ADR 0114](../decisions/0114-block-types-bring-default-pictures.md)): a block offers them as templates named `BLOCK/NAME`, whose inputs bind by name to the block's ports and parameters, so a scene draws `{"draw": "template", "name": "rig", "template": "cartpole/bodies"}` with no `bind`, except for a pointer to grab with. `check` lists them under `pictures`, `explain` with each block, and `describe TYPE` names them. A mechanism offers `bodies` ([reference](../reference/mechanisms.md#its-picture-bodies)). A model that writes no views opens, and renders, with one scene of its blocks' pictures beside the plot chosen by fit; it is saved in the file only if saved with it.

**View files** (`*.ssbv.json`, [ADR 0113](../decisions/0113-views-bind-to-interfaces.md)): templates and views without a model, written as a model file's `templates` and `controls.views` with `format` and nothing else. `simsandbox check FILE.ssbv.json` lists them. `simsandbox render MODEL_OR_RUN --views FILE --out IMAGE.png` draws them in place of the model's views. A run keeps its identity, since views and templates are outside the model hash. From Python: `m.render(views=FILE)` and `m.add_views(FILE)`. `examples/cartpole-sketch.ssbv.json` draws the cart-pole as a stick figure with the pole's phase portrait. A parent may draw in a frame a template `offers` (`"in": "left_rig/tip"`), and in no other of its frames. A template whose frame binds quantities over a table's rows is drawn once a row: the cart-pole's `body_axes` over the mechanism's pose rows draws axes on every body. `check` lists the file's `templates` and warns, at its path, about a use that does not fit (an input not bound, a unit, a frame not offered). Python: `m.define_template(name, view, inputs=, params=, offers=)`, `m.add_view(template=, bind=, params=)`, and template layers in `m.add_scene`.

On the diagram's data layer a scene is a view node with an input per bound signal and per grab, named by the path `check` uses (`frames.cart.x`, `layers[1].grab`), so `render --diagram` shows what it reads and which pointer it sends to; a parameter binding has no wire and is named beside its input (`layers[1].bind.x2 · bob.L`). Renaming a block renames it in the scene's bindings (parameters too), poses and grabs.

## Recorder

`"recorder": {"every": 10, "mb": 256}` in `controls` (both optional) lets the model be rewound in the app ([ADR 0064](../decisions/0064-recorder-node.md)): a snapshot of its state every `every` steps, within `mb` megabytes. Without it the app shows no rewind bar. It changes no results; headless runs ignore it.

## Inputs and run files

Input blocks ([ADR 0058](../decisions/0058-inputs-live-scheduled-replayed.md)) are values set from outside a running model: `input.value` (one output `y`) and `input.pointer` (`x`, `y`, `down`, `picked`), with settings `unit`, `min` and `max`. Their outputs are their own parameters, so they are wired like any block. `picked` is the row id of the scene instance a grab holds (0 for a layer of one instance) and −1 while nothing is held; files and runs made before it read −1 throughout.

A change during a run is an event `(t, key, value)`, applied at the first step boundary at or after `t` and held over the step. `key` is any parameter key (`push.y`, `cruise.kp`). Events come from:
- the model's default schedule, `run.inputs`: `[{"t": 5, "set": "push.y", "to": 40}]` (Python `Model.schedule(5, "push.y", 40)`); every run applies it;
- a run file `<model>.<run>.run.json` beside the model ([ADR 0059](../decisions/0059-run-files.md)): the schedule in effect (`inputs`) and recorded live input (`track`, `[[t, "key", value], ...]`), with the seed, `run` settings and `model_hash` of the model it was made with. The hash leaves out `controls`, the seed, `dt`, `t_end` and the schedule, and is taken over the model with every parameter written out, so saving from the app does not make a run stale.

A scene's grab ([ADR 0093](../decisions/0093-scenes-from-primitive-layers.md) point 6: a layer with `"grab": "hand"`) is recorded as the events it sends to that `input.pointer`: on the press `hand.x`, `hand.y` (in the scene's own frame), `hand.down` 1 and `hand.picked` (the row id); while held, `hand.x` and `hand.y` as they change; on release `hand.down` 0 and `hand.picked` −1. Headless, the same events come from a run file's `track`, written by the app or by hand, so `simsandbox run MODEL --inputs RUN` (or `run RUN`) replays a grab without drawing the scene: `[[0.2, "hand.x", 1.0], [0.2, "hand.y", 0.0], [0.2, "hand.down", 1.0], [0.2, "hand.picked", 0.0], [0.6, "hand.down", 0.0], [0.6, "hand.picked", -1.0]]` holds the bob of `examples/pendulum-grab.ssbx.json` from 0.2 s to 0.6 s.

The same model, events and seed give the same run bit for bit, in the app (live, after a rewind, or from Runs ▸ a saved run) and headless. Bindings of keys and views to input blocks (`controls.inputs`) affect only the app.

## Python

Install from a checkout (needs Rust and Python 3.10 or later):

```sh
pip install ./python            # or, while developing: cd python && maturin develop
```

```python
import simsandbox as ss

ss.describe("eq.equations")                      # a type, as a dict
m = ss.Model()                                   # or ss.Model.load(path), ss.Model.example("grazing")
m.add_block("decay", "eq.equations", "rk4", equations="dx/dt = -k*x", params={"k": 0.5})
m.set_run(dt=0.01, t_end=2.0)
m.check()["ok"]                                  # True
r = m.run()                                      # numpy arrays
r.t, r["decay.x"]
m.run(signals=["decay"], every=10)            # only these outputs, every 10th step and the last (ADR 0076)
m.set("decay.k", 1.0)                            # edits are file edits: m.data, m.save(path)
m.render("decay.png")                            # PNG of the views
m.render("diagram.png", diagram=True)             # PNG of the block diagram
m.declare("rate", 0.5)                            # a model parameter (ADR 0043)
m.set("decay.k", "2*rate")                        # a block parameter as a formula of it
m.group(["decay"])                                # make a model of blocks, defined in the file
m.study("car1")                                   # a study block of a used model (ADR 0070)
m.promote("energy", to="spring.u")               # an expression filter becomes a block (ADR 0050)
settled = m.start_from(10.0)                     # starts from the state at t = 10
m.schedule(5.0, "decay.k", 2.0)                  # a change at t = 5 in every run (run.inputs)
r = m.run(inputs=[(1.0, "decay.k", 0.1)])        # one-off changes during this run
run = ss.Run.load("cart-pole.push-test.run.json")  # a run file and its model
r = run.replay()                                 # the same numbers as when it was recorded
m.add_probe("speed", ["car.v"], every={"seconds": 0.5}, to="speed.csv")  # a probe (ADR 0062)
p = m.run_probes(save=".")["speed"]              # what it recorded; its files written to "."
p.t, p["car.v"]
ss.Run(m, track=[(1.0, "decay.k", 0.1)], name="slow", model_path="decay.ssbx.json").save("decay.slow.run.json")
```

- The lines of an `eq.equations` block (units, grids and stencils, tables, edges, meshes, rules, every function) are in the [typed-equations reference](../reference/typed-equations.md), which `simsandbox describe eq.equations` and `ss.describe("eq.equations")` give whole under `reference`. `check` reports lines that do not parse or balance, with line and column.
- `Model.add_block(..., canvas=(x, y))` places a block on the canvas (laid out automatically otherwise). `Model.connect("a.y", "b.u")` wires blocks; `Model.add_scene(layers, frames=..., unit=..., extent=..., view_from=...)` adds a scene view ([Scenes](#scenes)); `Model.as_study(("spring.w", 0, 3, 600))` studies the whole model over parameter values ([Studies](#studies)).
- `Results.outputs` shapes: scalar `(rows,)`, vector `(rows, n)`, grid `(rows, ny, nx)`, points `(rows, count, dim)`; `Results.populations[name]` is a list of `(n, dim)` arrays; `Results.over_rows[name]` (and `Probe.over_rows`) is a `Rows`: `ids` and `values` per row, `versions`, `of(id)` (that row through the run, NaN where absent) and `by_id()` (every id, and a `(rows, ids)` matrix).
- Problems raise `simsandbox.ModelError`, listing every error `check` finds.

Tests: `python -m pytest python/tests` after installing (CI runs them on Windows, macOS and Linux).
