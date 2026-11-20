# 0102 — Scene bindings may name parameters

- **Status:** Accepted (owner, 2026-11-19)
- **Date:** 2026-11-19
- **Builds on:** [ADR 0093](0093-scenes-from-primitive-layers.md) (scenes of layers bound to constants, signals and rows), [ADR 0058](0058-inputs-live-scheduled-replayed.md) (parameter changes are input, recorded and replayed), [ADR 0063](0063-probes-on-wires.md) (the history records what views read)

## Context

A scene binds an attribute or a frame's pose to a constant, a scalar signal or a quantity over a table's rows (ADR 0093 point 2). The pendulum example's rod was therefore a constant 1 m: the rod length `L` is a parameter, not an output, so a changed `L` slider moved the bob in the `eq.pendulum` view but not in the scene. ADR 0093's stage 3 left "binding a model parameter (`bob.L`) in a scene" waiting on the owner's decision.

Two things must hold for a parameter binding. The picture at a drawn instant must use the parameter's value at that instant, so that a rewound or replayed run draws as it was (scenes are functions of recorded data). And a name must mean one thing: `input.pointer` has both an output `hand.x` and a parameter `hand.x`.

Parameters change only as input: the session applies a change at a step boundary and records it in the run's input track (ADR 0058), which rewinds and replays apply again. The history, which views draw from, held outputs and filters, not parameters; expression filters already read every parameter's current value each step.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| A. A parameter binding reads the parameter's current value | Nothing recorded | A rewound or replayed run draws today's value, not the value then; the picture stops being a function of recorded data |
| B. The view reconstructs the value at the drawn instant from the input track (the model's own value, then the schedule and recorded events up to that time) | Nothing new in the history | The track would have to reach the window and every headless render; events are applied at step boundaries and through formulas (a declared parameter sets block parameters, ADR 0043), which the view would have to repeat; a second way to find a value at an instant beside the history |
| **C. The session records the parameters views bind as signals in the history, every step, under their names, the way it records the signals views read (`view_reads`)** | Rewind, replay, run files, recompute and headless renders work as they do for signals, with nothing new: the history is truncated, filled and replayed already; the value recorded is the one the model used at that step, after formulas; a scene reads it like a scalar signal | One more recorded column per bound parameter (only those bound); names must not clash with signals |

## Decision

C.

1. **A binding may name a parameter** `block.param` of the model (any block parameter, including an `input.*` block's), wherever it may name a scalar signal: an attribute or a frame's `x`, `y` or `angle`. It names a value; it computes nothing.
2. **The name rule: an output wins.** A name is a scalar signal (an output, or an expression filter's) if the model has one; else a quantity over rows; else a parameter. `hand.x` is the pointer's output. A parameter that an output of any shape has the name of (a study's swept `curve.w` is a field) is never offered, recorded or bound as a parameter; bind the output, which for an input block carries the same value.
3. **Values over time.** The session records each parameter a view binds, and no output or expression filter shadows, as a scalar in the history, at every step, under its name, after the signals (`Catalogue::bound_params`). So a scene draws a parameter at the value the model used at the drawn sample: rewound, the history is truncated with everything else; replayed, the input track sets it again and it is recorded again; a run file replayed headless records it the same way. A history made before a scene bound it lacks it for earlier samples, a gap as for any signal wired later, filled by "recompute". Recorded parameters are not offered as signals (`bindable_scalars`, the slot pickers and probes leave them out); they appear only where a scene binds them.
4. **Units** are checked as for signals: a parameter's unit comes from its block's declaration (the history knows every parameter's unit, recorded or not); lengths must be in the scene's unit, angles in rad or dimensionless, screen sizes dimensionless.
5. **Discovered** like signals: `Available::params` lists every block parameter no output has the name of; `describe plot.scene` names the `parameter` binding; `check` reports a wrong one at its path ("no signal or parameter 'bob.M'"); the Inspector's binding picker lists them under "parameters"; the Python `Model.add_scene` accepts them as it accepts any name starting with one of the model's blocks.
6. **On the canvas** a parameter binding is not a wire: parameters are not outputs, and wires carry recorded signals. The scene's input at that path names it beside the input (`layers[1].bind.x2 · bob.L`), as a view's slot names `t`. Renaming a block renames its parameters in a scene, as for every dotted name in `controls`.

Declared parameters (ADR 0043, keys without a block) are not bound yet; bind the block parameter a declared parameter sets.

## Consequences

- `examples/pendulum.ssbx.json` binds the rod's end and the bob's centre to `bob.L`: its scene follows the `L` slider. Its run output is unchanged (the headless `run` does not use the session's history); its render, diagram (the inputs name `bob.L`) and `check` output change.
- Results stay bit-identical: recording reads a parameter, it never sets one.
- `simsandbox-session`: the catalogue knows every block parameter, records those views bind (`set_param_reads`, from `Session::set_view_reads` and on load), and sets their units in the history; recomputing filters keeps their recorded values.
- `simsandbox-views`: `Available::params`, `spec::Source::Param`; the view reads it from the history like a scalar.
- Documents: [visualization](../architecture/visualization.md#scenes), the [headless guide](../guides/headless.md#scenes), [python/README.md](../../python/README.md), ADR 0093's note "Parameters, frame renames and errors".

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none. A parameter was already a concept and a scene binding names values; this lets one name a parameter.
