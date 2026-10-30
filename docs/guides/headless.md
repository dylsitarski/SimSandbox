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
| `simsandbox check FILE` | JSON report; exit status 1 if the model cannot run. For a run file: its model is found and checked with the run's settings and events, and a model changed since the run was made is a warning |
| `simsandbox run FILE [--t-end S] [--dt S] [--format csv\|json] [--inputs RUN]` | Every output at every step (or sweep sample). `FILE` may be a run file, which is replayed; `--inputs RUN` applies a run's events to a model file. `render` takes `--inputs` too |
| `simsandbox run FILE --probe NAME [--save-probes DIR] [...]` | What probe `NAME` recorded ([Probes](#probes)) instead of every output: CSV of `t` and its signals, or with `--format json` also its fields. `--save-probes DIR` (with or without `--probe`) writes the files of the probes that have one (`to`) into `DIR` |
| `simsandbox render FILE --out IMAGE.png [--t-end S] [--view N] [--width P] [--height P] [--scale S] [--light]` | PNG of the views after running to `t_end`, placed as the file's `controls.layout` arranges them ([ADR 0053](../decisions/0053-docked-panes-inspector-and-node-values.md)) or tiled in a grid |
| `simsandbox render FILE --diagram [--subsystem NAME] [--no-data] [--values] --out IMAGE.png [--width P] [--height P] [--scale S] [--light]` | PNG of the block diagram, or a subsystem's inside, as the app's Diagram tab shows it (also for a file that cannot run), with the filters and views as its data layer unless `--no-data` ([ADR 0050](../decisions/0050-data-layer-and-canvas-undo.md)); the nodes `controls.open` lists show their values, every node with `--values` ([ADR 0055](../decisions/0055-open-nodes-subsystem-values-deferred-windows.md)) |
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
| `params` | Every parameter: `key` (`block.param`; a model parameter's `name`, a subsystem use's `instance.name`; blocks inside uses `instance/inner.param`), `value`, `formula` (set when a formula computes it), `unit`, `suggested` range, `initial_condition`, `description` |
| `subsystems`, `blocks` | Each subsystem's parameters, ports and uses ([ADR 0043](../decisions/0043-subsystems.md)); every block after flattening, as signals and sliders name them |
| `labels` | Speed and accuracy of the model's methods |
| `recorded` | `signals` (scalars) and `fields` (grids, point sets, populations) that views and probes can take, recorded now or not |
| `fused_groups`, `swept`, `filters`, `views` | How blocks are solved together, the swept parameters, filter and view names |

### `run --format json`

| Key | Meaning |
| --- | --- |
| `rows` | Number of steps (including t = 0) or sweep samples |
| `index` | `t`, or `sample` and each swept parameter, as lists |
| `outputs` | `block.port` → `{shape, values}`; a scalar's values are a list, others a list per row in the port's element order (grids row by row, x fastest) |
| `populations` | `block.port` → `{dim, frames}`: per row, a list of points |
| `labels`, `notes`, `warnings` | As in `check` and on standard error |

JSON has no infinities or NaN; they are written as `null`. CSV leaves populations out.

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

A view binds a probe's recording as `probe:NAME/signal` (`"y": ["car.v", "probe:speed/car.v"]`): the model's every step and the probe's samples on one plot. Where a view pairs signals from different sources (x against y, colour), it uses only the times both recorded. A filter's inputs may come from probes: `"from": {"car.v": "fast", "cruise.F": "slow"}` (or `"probe": "NAME"` for all of them); it then reads only what those probes record and runs where their times pair, or with `"when": "any"` whenever one of them records, with the others' latest values. Its outputs are bound by name, as any filter's ([ADR 0063](../decisions/0063-probes-on-wires.md)). The model's every-step recording keeps only what views and filters read from the model; `controls.plots` no longer limits anything.

A probe keeps its samples in memory only when something reads them: a view or filter wired to it, a file written at the end (`to` without `stream`), or `run --probe` (which keeps every probe it is given). A probe that only streams, or that nothing reads, holds nothing, so `keep` has no effect on it. `render --diagram --no-data` hides the data layer but keeps probes that feed blocks: they are part of the model.


A probe with bad settings, or recording something the model does not record, is dropped with a warning. `simsandbox run FILE --probe NAME` prints one; `--save-probes DIR` writes the files. A swept model records no probes. JSON output of a probe: `rows`, `t`, `signals` (name → list), `fields` (name → `{shape, frames}`).

A model-level connection may come from a probe: `{"from": "probe:sensor.car.v", "to": "cruise.v"}` feeds `cruise.v` with `car.v` as probe `sensor` samples it, held between samples ([ADR 0063](../decisions/0063-probes-on-wires.md)). The probe must record that signal, a whole output of a block on the model's level. Such a probe changes results: it counts in a run file's model hash.

## Data files as inputs

An `input.file` block reads recorded data as an input over time: a probe's file, or any CSV or `.npy` ([ADR 0063](../decisions/0063-probes-on-wires.md)). File names are read from the model file's folder (absolute names work anywhere); the CLI and Python (`Model.load`, `Model.save`) use the model's folder, and a model never saved can name files only by full path.

```json
{"name": "recorded", "type": "input.file", "settings": {"file": "speed.csv", "unit.car.v": "m/s"}}
```

| Setting | Meaning |
| --- | --- |
| `file` | A `.csv`: first column the time, each other column a scalar output named by its header with characters other than letters, digits and `_` made `_` (`car.v` gives `recorded.car_v`). Or a `.npy` (`float64` or `float32`): one output `y`, its first axis the samples, `(samples)` a scalar, `(samples, n)` a vector, `(samples, a, b)` a grid of `a` rows of `b` |
| `interpolate` | `hold` (default): the last sample, as a probe feeding a block gives it; `linear`: a straight line between samples. Before the first sample, the first; after the last, the last |
| `unit`, `unit.COLUMN` | Units of the outputs, unless a CSV header gives one as `name [unit]`; dimensionless otherwise |
| `times` or `dt` | For a `.npy`: a CSV whose first column gives the times (a probe's CSV beside its `.npy` files), or seconds between samples from 0 |
| `shape` | For a `.npy` of 3 axes: `grid` (default) or `points` (`a` points of `b` coordinates) |

The file is read when the model is built (opened, or changed on the canvas). Its contents count in a run file's model hash: the same model with a different data file is a different model. `describe input.file` lists the settings that name files (`files`). The value is taken at each step's end and held over the next step, like any block's output; the block never joins a fused group.

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
m.set("decay.k", 1.0)                            # edits are file edits: m.data, m.save(path)
m.render("decay.png")                            # PNG of the views
m.render("diagram.png", diagram=True)             # PNG of the block diagram
m.declare("rate", 0.5)                            # a model parameter (ADR 0043)
m.set("decay.k", "2*rate")                        # a block parameter as a formula of it
m.group(["decay"])                                # make a subsystem of blocks
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
- Arrays ([ADR 0060](../decisions/0060-arrays-in-typed-equations.md)): `grid x [m] = 0 .. length, 50` declares a line of cells; `state T[x] [K] = expression`, `input q[x] [W/m^3]` and any quantity using them have a value per cell and are grid outputs, with the axis `x` as their coordinate; `edge T: x- = value, x+ gradient = 0` (or `x periodic`) sets the ends, insulated where not given; `lap(T)` and `ddx(T)` are stencils; `sum`, `mean`, `minof`, `maxof` and `at(T, i)` give one value; `spacing(x)` is the cell width. A second `grid y ...` line makes the grid 2D: arrays are `T[x, y]` (x fastest in the cells), `lap` sums along both axes, `ddy` differentiates along y, edges are per axis (`y- = 0`, `y periodic`) and `at(T, i, j)` reads one cell. Fine grids are stiff: the methods `trbdf2` (second order) and `backward_euler` (first) take steps far beyond what `rk4` can. Array ports connect to array inputs of the same number of cells and unit, and to built-in grids such as `eq.heat_rod.temperature`.
- `Model.add_block(..., canvas=(x, y))` places a block on the canvas (laid out automatically otherwise). `Model.connect("a.y", "b.u")` wires blocks; `Model.sweep(("spring.w", 0, 3, 600))` evaluates a model with no states over parameter values.
- `Results.outputs` shapes: scalar `(rows,)`, vector `(rows, n)`, grid `(rows, ny, nx)`, points `(rows, count, dim)`; `Results.populations[name]` is a list of `(n, dim)` arrays.
- Problems raise `simsandbox.ModelError`, listing every error `check` finds.

Tests: `python -m pytest python/tests` after installing (CI runs them on Windows, macOS and Linux).
