# Using SimSandbox headless (CLI and Python)

How people and AI agents describe, check, run and render models without the window ([ADR 0039](../decisions/0039-ai-agent-access.md), [ADR 0041](../decisions/0041-headless-interface.md)). The model file is the one input: see the [JSON Schema](../../crates/simsandbox-file/model.schema.json) and the [examples](../../examples), which double as templates.

## Workflow for an agent

1. `simsandbox describe` (or `ss.describe()`): every block type with ports, units, parameters, methods and labels; every view and filter type.
2. Write a model file (or build a `Model` in Python), starting from the closest example.
3. `simsandbox check FILE`: fix each error at its `path` (`blocks[0].method`) or `line`/`column`, until `"ok": true`.
4. `simsandbox run FILE --format json` (or `Model.run()`): the numbers, with speed and accuracy labels and notes on algebraic loops.
5. `simsandbox render FILE --out view.png`: the views the app would show.

## CLI

Build with `cargo build --release -p simsandbox-cli`; the binary is `target/release/simsandbox`.

| Command | Output |
| --- | --- |
| `simsandbox describe [TYPE]` | JSON: `block_types`, `view_types`, `filter_types`, `units`; or one block or view type; `describe units` lists every unit symbol, its SI form and factor, and the prefixes ([ADR 0047](../decisions/0047-unit-vocabulary.md)) |
| `simsandbox schema` | The model file's JSON Schema |
| `simsandbox check FILE` | JSON report; exit status 1 if the model cannot run |
| `simsandbox run FILE [--t-end S] [--dt S] [--format csv\|json]` | Every output at every step (or sweep sample) |
| `simsandbox render FILE --out IMAGE.png [--t-end S] [--view N] [--width P] [--height P] [--scale S] [--light]` | PNG of the views after running to `t_end` |
| `simsandbox render FILE --diagram [--subsystem NAME] [--no-data] --out IMAGE.png [--width P] [--height P] [--scale S] [--light]` | PNG of the block diagram, or a subsystem's inside, as the app's Diagram tab shows it (also for a file that cannot run), with the filters and views as its data layer unless `--no-data` ([ADR 0050](../decisions/0050-data-layer-and-canvas-undo.md)) |
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
| `recorded` | `signals` (scalars) and `fields` (grids, point sets, populations) that views can bind |
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
```

- Units ([ADR 0045](../decisions/0045-units-in-typed-equations.md)): write them in the equations (`input F [N]`, `param m [kg] = 2`, `state x [m] = 0`, `dv/dt = F/m [m/s^2]`; a unit at the end of a line is its left side's, [ADR 0049](../decisions/0049-state-declarations-and-rate-units.md)); `check` then reports lines that do not balance, and each parameter's `unit`. Connections need the same unit; convert with `m.add_block("to_m", "units.convert", settings={"from": "mm", "to": "m"})`.
- `Model.add_block(..., at=(x, y))` places a block on the canvas (laid out automatically otherwise). `Model.connect("a.y", "b.u")` wires blocks; `Model.sweep(("spring.w", 0, 3, 600))` evaluates a model with no states over parameter values.
- `Results.outputs` shapes: scalar `(rows,)`, vector `(rows, n)`, grid `(rows, ny, nx)`, points `(rows, count, dim)`; `Results.populations[name]` is a list of `(n, dim)` arrays.
- Problems raise `simsandbox.ModelError`, listing every error `check` finds.

Tests: `python -m pytest python/tests` after installing (CI runs them on Windows, macOS and Linux).
