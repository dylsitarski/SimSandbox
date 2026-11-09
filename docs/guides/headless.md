# Using SimSandbox headless (CLI and Python)

How people and AI agents describe, check, run and render models without the window ([ADR 0039](../decisions/0039-ai-agent-access.md), [ADR 0041](../decisions/0041-headless-interface.md)). The model file is the one input: see the [JSON Schema](../../crates/simsandbox-file/model.schema.json) and the [examples](../../examples), which double as templates.

## Workflow for an agent

1. `simsandbox describe` (or `ss.describe()`): every block type with ports, units, parameters, methods and labels; every view and filter type.
2. Write a model file (or build a `Model` in Python), starting from the closest example.
3. `simsandbox check FILE`: fix each error at its `path` (`blocks[0].method`) or `line`/`column`, until `"ok": true`.
4. `simsandbox run FILE --format json` (or `Model.run()`): the numbers, with speed and accuracy labels and notes on algebraic loops.
5. `simsandbox render FILE --out view.png`: the views the app would show.
6. To act on a run (push, steer, drag), write the changes as events: a schedule in the model (`run.inputs`) or a run file beside it, and replay it with `simsandbox run RUN.run.json` (see [Inputs and run files](#inputs-and-run-files)).

## CLI

Build with `cargo build --release -p simsandbox-cli`; the binary is `target/release/simsandbox`.

| Command | Output |
| --- | --- |
| `simsandbox describe [TYPE]` | JSON: `block_types`, `view_types`, `filter_types`, `units`; or one block or view type; `describe units` lists every unit symbol, its SI form and factor, and the prefixes ([ADR 0047](../decisions/0047-unit-vocabulary.md)) |
| `simsandbox schema [run]` | The model file's JSON Schema, or the [run file's](../../crates/simsandbox-file/run.schema.json) |
| `simsandbox check FILE` | JSON report; exit status 1 if the model cannot run. For a run file: its model is found and checked with the run's settings and events, and a model changed since the run was made is a warning, as is a run made by another SimSandbox version (its `made_with`) |
| `simsandbox run FILE [--t-end S] [--dt S] [--format csv\|json] [--inputs RUN]` | Every output at every step (or, for a model whose only runs are its studies, every run); JSON output names the SimSandbox that made it (`"made_with": "simsandbox 0.0.1"`). `FILE` may be a run file, which is replayed; `--inputs RUN` applies a run's events to a model file. `render` takes `--inputs` too |
| `simsandbox run FILE --signals NAMES --every N [...]` | Only the outputs named (comma-separated: `block.port`, `block` for all its ports; in a model using models also a model's name, `.` the top level, or a path in it, `plant/car.v`) and only every `N`-th step and the last: the same values, less memory and time ([ADR 0076](../decisions/0076-headless-runs-keep-what-is-asked.md)). A name that keeps nothing is an error listing the outputs |
| `simsandbox run FILE --probe NAME [--save-probes DIR] [...]` | What probe `NAME` recorded ([Probes](#probes)) instead of every output: CSV of `t` and its signals, or with `--format json` also its fields. `--save-probes DIR` (with or without `--probe`) writes the files of the probes that have one (`to`) into `DIR` |
| `simsandbox render FILE --out IMAGE.png [--t-end S] [--view N] [--width P] [--height P] [--scale S] [--light]` | PNG of the views after running to `t_end`, placed as the file's `controls.layout` arranges them ([ADR 0053](../decisions/0053-docked-panes-inspector-and-node-values.md)) or tiled in a grid |
| `simsandbox render FILE --diagram [--inside PATH] [--no-data] [--values] --out IMAGE.png [--width P] [--height P] [--scale S] [--light]` | PNG of the block diagram, or the inside of the model used at `PATH` (`car1`, `vehicle/plant`; `--subsystem` is its older name); `--expand car1,vehicle/plant` draws those uses expanded in place, as `controls.expand` does, as the app's Diagram tab shows it (also for a file that cannot run), with the filters and views as its data layer unless `--no-data` ([ADR 0050](../decisions/0050-data-layer-and-canvas-undo.md)); the nodes `controls.open` lists show their values, every node with `--values` ([ADR 0055](../decisions/0055-open-nodes-subsystem-values-deferred-windows.md)) |
| `simsandbox start-from FILE --t S [--out NEW.json]` | The model file that starts from the state reached at `t` ([ADR 0042](../decisions/0042-node-canvas.md)): initial-condition parameters where they say it exactly, else a `start` entry per block |
| `simsandbox upgrade FILE [--out NEW.json]` | The file in the current form of the format; what was converted on standard error ([ADR 0044](../decisions/0044-unsaved-changes-and-older-files.md)) |

Notes, warnings and labels go to standard error; data to standard output.

### `check` report

| Key | Meaning |
| --- | --- |
| `ok` | The model loads and can run |
| `errors` | Each `{path, message}` (file structure) or `{line, column, message}` (JSON syntax) or `{message}` |
| `warnings` | Problems that were worked around, e.g. a saved view that cannot be shown |
| `conversions` | Older forms of the format the file was written in, converted on reading; `simsandbox upgrade` writes the current form |
| `params` | Every parameter: `key` (`block.param`; a model parameter's `name`, a used model's `use.name`; blocks inside uses `use/inner.param`), `value`, `formula` (set when a formula computes it), `unit`, `suggested` range, `initial_condition`, `description` |
| `definitions`, `blocks` | Each model defined in the file, with its input blocks, declared outputs and uses ([ADR 0068](../decisions/0068-one-kind-of-nested-model.md)); every block after flattening, as signals and sliders name them |
| `labels` | Speed and accuracy of the model's methods |
| `recorded` | `signals` (scalars) and `fields` (grids, point sets, populations) that views and probes can take, recorded now or not |
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

A model's `blocks` may use other model files, each running on its own clock, linked by probes. Uses nest to any depth ([ADR 0066](../decisions/0066-models-nest-as-blocks.md)). A use has a `file` in place of a `type`:

```json
{"format": 1, "run": {"t_end": 60},
 "blocks": [{"name": "plant", "file": "car.ssbx.json"},
            {"name": "controller", "file": "cruise.ssbx.json", "params": {"cruise.kp": 600}, "clock": {"dt": 0.5}}],
 "connections": [{"from": "probe:speed.plant/car.v", "to": "controller/speed"},
                 {"from": "probe:force.controller/cruise.F", "to": "plant/force"}],
 "controls": {"probes": [{"name": "speed", "record": ["plant/car.v"]},
                         {"name": "force", "record": ["controller/cruise.F"]}]}}
```

- **Uses:**
  - `file` is found from this file's folder, and data files inside a use from its own file's folder.
  - `params` set values for this use only, by path at any depth (`car.m`, `plant/car.m`, a declared `m`, `plant/m`). Each value is a number or a formula of the declared parameters of the file it is written in. On a shared clock the formula is live. Into a model on its own clock it is evaluated once, when the model is built.
  - `clock` is `{"dt": step}`, `{"times": [0.5, 1.2, 4]}` to step from one listed instant to the next (positive, ascending; [ADR 0067](../decisions/0067-experiments-and-time-from-data.md) step 4), or absent for the file's own `run.dt`.
  - `"clock": "shared"` puts the use on this model's clock instead, flattened into it (fused with its neighbours where possible). Wire straight into its input blocks (`{"from": "controller/cruise.F", "to": "plant/force"}`) and from outputs (`plant/car.v`), at any depth of models on a shared clock (`vehicle/plant/force`). Any model file can be used shared: models on their own clocks inside it keep their clocks under its path (`vehicle/engine`), and its probes, links and groups come along.
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
- **Python:** `Model.add_model(name, file, params=, dt=, shared=)` and `Model.link("plant/car.v", "controller/speed", every=)`. `Model.run()` returns `Composed`, read with `.model("plant")`, `.t("plant")` or `composed["vehicle/plant/car.v"]`.
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
- **Points from inputs and search strategies** ([ADR 0078](../decisions/0078-adaptive-studies.md)): `"design": "inputs", "count": 5` takes the points through the study's inputs, one array input of `count` cells per swept parameter (named as its output would be: `cruise.kp` is the input `cruise_kp`), wired from another model. The swept parameters are not outputs then; the results are. Each point's results are kept by its values: when the inputs change, only points whose values (or the model's other inputs and parameters) changed run again, under their own ids, so seeds and results are those of running every point again. A strategy is a model that chooses the points from earlier results: a map (`x[n+1] = …`, one step a round) reading the study's results as arrays and sending it its next points. `examples/oscillator-fit.ssbx.json` fits a damped oscillator's `k` and `c` to measurements by Levenberg–Marquardt: four points a round (the centre, a step on each parameter for the Jacobian, the candidate), residuals `at_samples(osc.x - data.x, data)`, and `solve` for the damped step; it reaches the least-squares fit SciPy finds. `examples/bistable-continuation.ssbx.json` is continuation: one run a round, at the next value of a parameter, starting from the state the last run settled at (the study sweeps the starting value `bi.x0` too), which traces a hysteresis loop. `examples/bistable-refinement.ssbx.json` is refinement: the starting values as a table with edges between neighbours, a study with `"design": "rows"` running one per row, and `split` of every interval whose ends end on different branches, until it brackets the boundary between the basins within a tolerance. Python writes these strategy models for any number of parameters: `Model.minimize(target, params, objective)`, `fit(target, params, residuals)`, `refine(target, param, range, result)` and `continue_(target, param, range, step, state, settled)`, the target a study block, a used model or `None` (the whole model); the app's study form has the same under "Search…" ([ADR 0078](../decisions/0078-adaptive-studies.md) step 5). `examples/cruise-search.ssbx.json` is pattern search over two gains: the centre and a step either way on each, moving to the best point or halving the step, holding still once the step is below a tolerance (and then nothing runs again). Maps may use grids for this (`grid k = 5`, `input cost[k]`, array states declared `state c[k] = …`).
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

## Recorder

`"recorder": {"every": 10, "mb": 256}` in `controls` (both optional) lets the model be rewound in the app ([ADR 0064](../decisions/0064-recorder-node.md)): a snapshot of its state every `every` steps, within `mb` megabytes. Without it the app shows no rewind bar. It changes no results; headless runs ignore it.

## Inputs and run files

Input blocks ([ADR 0058](../decisions/0058-inputs-live-scheduled-replayed.md)) are values set from outside a running model: `input.value` (one output `y`) and `input.pointer` (`x`, `y`, `down`), with settings `unit`, `min` and `max`. Their outputs are their own parameters, so they are wired like any block.

A change during a run is an event `(t, key, value)`, applied at the first step boundary at or after `t` and held over the step. `key` is any parameter key (`push.y`, `cruise.kp`). Events come from:
- the model's default schedule, `run.inputs`: `[{"t": 5, "set": "push.y", "to": 40}]` (Python `Model.schedule(5, "push.y", 40)`); every run applies it;
- a run file `<model>.<run>.run.json` beside the model ([ADR 0059](../decisions/0059-run-files.md)): the schedule in effect (`inputs`) and recorded live input (`track`, `[[t, "key", value], ...]`), with the seed, `run` settings and `model_hash` of the model it was made with. The hash leaves out `controls`, the seed, `dt`, `t_end` and the schedule, and is taken over the model with every parameter written out, so saving from the app does not make a run stale.

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

- Units ([ADR 0045](../decisions/0045-units-in-typed-equations.md)): write them in the equations (`input F [N]`, `param m [kg] = 2`, `state x [m] = 0`, `dv/dt = F/m [m/s^2]`; a unit at the end of a line is its left side's, [ADR 0049](../decisions/0049-state-declarations-and-rate-units.md)); `check` then reports lines that do not balance, and each parameter's `unit`. Connections need the same unit; convert with `m.add_block("to_m", "units.convert", settings={"from": "mm", "to": "m"})`.
- Arrays ([ADR 0060](../decisions/0060-arrays-in-typed-equations.md)): `grid x [m] = 0 .. length, 50` declares a line of cells; `state T[x] [K] = expression`, `input q[x] [W/m^3]` and any quantity using them have a value per cell and are grid outputs, with the axis `x` as their coordinate; `edge T: x- = value, x+ gradient = 0` (or `x periodic`) sets the ends, insulated where not given; `lap(T)` and `ddx(T)` are stencils; `sum`, `mean`, `minof`, `maxof` and `at(T, i)` give one value (`sum` and `mean` over more than 4096 cells add in fixed chunks, the same bits on any machine: [ADR 0077](../decisions/0077-sums-in-fixed-chunks.md)), `argmaxof(T)` and `argminof(T)` the cell (in storage order) of the greatest or least, and `at(T, i)` with a computed `i` the cell found so ([ADR 0070](../decisions/0070-studies-are-blocks.md)); `grid k = 5` declares five cells numbered 0 to 4, with no unit (a study's runs, say); `spacing(x)` is the cell width; `prev(v)` and `next(v)` (`prev(v, y)` for an array over two axes, which must name the axis) are the neighbouring cells along an axis, beyond an end the edge's held value itself, the end cell (insulated) or the far end (periodic) ([ADR 0069](../decisions/0069-repetition-in-blocks.md)); `param m[x] [kg] = 1000` gives a value per cell, each set as its own parameter `m[0]`, `m[1]`, … (`"params": {"m[2]": 2500}`). Each `grid` line is an axis of its own ([ADR 0080](../decisions/0080-named-axes.md)): declared quantities name theirs in the order of the grid lines (`state T[x, y]`, x fastest in the cells; `param m[k]`; `input r[i, k]`), a defined quantity is over the axes its right side uses, and an axis's own output is over itself alone. Quantities over different axes combine over all of them (`a[i] + b[k]` is over `[i, k]`); `sum(T, k)` (and `mean`, `minof`, `maxof`, `argminof`, `argmaxof`) works along one axis, leaving the others; `on(T, a, b)` is `T` with its axis `a` read as `b` (the same length), so `sum(J*on(J, p, q), r)` is JᵀJ over `[p, q]`; `solve(A, b)`, with `A` over two axes of one length and `b` over one of them, is the `x` over the other with `sum(A*x, j) = b` (NaN if `A` is singular); `dd(T, x)` is `ddx(T)`; `rcond(A)` is how far `A` is from singular, the reciprocal condition number in the 1-norm (1 for the identity, 0 when singular). A quantity over three or more axes (up to 8) is a port of that many axes ([ADR 0081](../decisions/0081-shapes-are-lists-of-axes.md)): wired to an input over as many axes of the same lengths, recorded, written in JSON with its axes named, in CSV by places (`q[1,2,3]`), and in Python as an array `(rows, nk, …, n1)` (`Results.axes(name)`); views draw them later. A **table** is an axis whose rows come and go between steps ([ADR 0079](../decisions/0079-tables-of-members.md), in maps): `table pts max 200`, `start pts = 5`, `state age[pts] = id(pts)`, then rules after each step's update, in order: `remove pts where age > 6`, `add pts from pts where age < 1.5: age = 0` (one new row for each row, or each place of another axis, where it holds; without `from`, one row). Each row keeps its id, never reused: `id(pts)`, `rows(pts)`, `of(age, 3)` (NaN once row 3 is gone). **Edges** (stage 2): `edges segs of pts` declares rows that join two rows of `pts`, `start segs = chain` joins each starting row to the next; `from_end(x, segs)` and `to_end(x, segs)` are `x` at each edge's ends; `split segs where c: x = mean` puts a new row in the middle of each edge where `c` holds (the edge becomes two), `merge segs where c` puts an edge's two rows into one; each column is `copy` (the `from` end's value, the default on split), `mean` (the default on merge) or an expression over the edges; removing a row removes its edges. **Meshes** ([ADR 0085](../decisions/0085-meshes-are-tables-of-cells.md)): `cells c of pts: triangle` (or `segment`, `quad`, `tetra`, `hexa`, `prism`, `pyramid`, `simplex d`, `box d`) are rows of corners over the points, `start c = grid x 0 .. 2 by 3, y 0 .. 1 by 2` makes points and cells (simplices by the Kuhn construction, or boxes), `sides f of c` the sides between cells, kept with stable ids; `corner(x, c, k)`, `centre(x, c)`, `measure(c, x, y)`, and for sides `normal(f, k, x, y)` (outwards from the inner cell), `inner(u, f)`, `outer(u, f)`, `boundary(f)`; cells, sides and edges may hold states. A table with its own `start`, rules or edges takes an input over it by id (NaN for a row the input lacks), so a strategy can split its points and read a study's results for them: refinement over one parameter. A quantity over rows is a port carrying the rows with their ids and the table's version (stage 1b): wire `pop.age` to a block with `table q` and `input age[q]`, and `q` follows those rows by id, at the same instant (no `start` or rules of its own; its states keep each row's value by id, a new row starting from the state's starting value). Rows are recorded and written by id (stage 1c): `run` JSON `over_rows`, CSV columns `pop.age#7`, probe files `t,version,id,value`, Python `Results.over_rows` (`Rows`: `ids`, `values`, `versions`, `of(id)`, `by_id()`); views do not draw them yet. Over two axes, `lap` sums along both, `ddy` differentiates along y, edges are per axis (`y- = 0`, `y periodic`) and `at(T, i, j)` reads one cell. Fine grids are stiff: the methods `trbdf2` (second order) and `backward_euler` (first) take steps far beyond what `rk4` can. Array ports connect to array inputs of the same number of cells and unit, and to built-in grids such as `eq.heat_rod.temperature`.
- `Model.add_block(..., canvas=(x, y))` places a block on the canvas (laid out automatically otherwise). `Model.connect("a.y", "b.u")` wires blocks; `Model.as_study(("spring.w", 0, 3, 600))` studies the whole model over parameter values ([Studies](#studies)).
- `Results.outputs` shapes: scalar `(rows,)`, vector `(rows, n)`, grid `(rows, ny, nx)`, points `(rows, count, dim)`; `Results.populations[name]` is a list of `(n, dim)` arrays; `Results.over_rows[name]` (and `Probe.over_rows`) is a `Rows`: `ids` and `values` per row, `versions`, `of(id)` (that row through the run, NaN where absent) and `by_id()` (every id, and a `(rows, ids)` matrix).
- Problems raise `simsandbox.ModelError`, listing every error `check` finds.

Tests: `python -m pytest python/tests` after installing (CI runs them on Windows, macOS and Linux).
