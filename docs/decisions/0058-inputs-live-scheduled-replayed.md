# 0058 — Inputs: live, scheduled and replayed

- **Status:** Accepted; where schedules and recordings are kept is amended by [0059](0059-run-files.md) (run files beside the model)
- **Date:** 2026-10-26

## Context

Phase 2 item 1 ([ADR 0057](0057-phase-2-plan.md)) lets people act on a running model: drag a pendulum, steer the cruise-control car, push the cart-pole of the gate. It settles [Q36](../open-questions.md).

What exists today:
- Parameters change between steps (`Simulator::set_param`). Sliders apply at once on real-time models and on release otherwise ([ADR 0037](0037-when-parameter-changes-apply.md)).
- Snapshots save parameter values, so a rewind restores them. Running on after a rewind starts a new branch: the recorded history after that time is dropped.
- Nothing records when values changed. A run steered by hand cannot be replayed, saved or reproduced headless, against the Research pillar ("every run is seeded and traceable").
- Scheduled changes (steps and disturbances at set times, [ADR 0042](0042-node-canvas.md)) are planned but not built.
- Views only display: none sends anything back to the model.

The kernel contains no UI or physics, and every feature must be usable headless ([ADR 0039](0039-ai-agent-access.md)).

## Options considered

**How input enters a model**

| Option | Pros | Cons |
| --- | --- | --- |
| **Input blocks: blocks whose outputs are their own values, changed by timed events between steps** | No new kernel concept: an input block is a stateless block (it fuses, [ADR 0051](0051-stateless-blocks-join-fused-groups.md)), its values are parameters (so snapshots and rewind already cover them), and it is wired like any block, with units | A device must be bound to a block before it does anything |
| Views and devices call `set_param` directly | Nothing to add | Nothing recorded; no replay; input reaches only parameters, not ports |
| A separate event channel into the kernel | General | A second way for data to enter the model; the kernel learns about devices |

**What is recorded**

| Option | Pros | Cons |
| --- | --- | --- |
| **An input track: each change of a value during a run, as (time, key, value), applied at a step boundary** | Small; exact (applied by step, not by wall-clock time); the same list serves live, scheduled and replayed input; covers sliders and typed values too | Sub-step input (faster than `dt`) is not seen, as today |
| Record input blocks' outputs every step | Simple | Large; duplicates the history; misses parameter changes |
| Nothing (as today) | — | No replay, so the gate cannot be met |

**After a rewind**

| Option | Pros | Cons |
| --- | --- | --- |
| **Keep the track ahead and replay it; new live input takes over from then (a new branch)** | Rewind-and-watch reproduces the run bit for bit (the gate); touching anything still branches, as today | One more state to show: "replaying" |
| Drop the track ahead at the rewind, as the history is dropped today | Simplest | No replay without saving and reloading |

**Where scheduled changes live**

| Option | Pros | Cons |
| --- | --- | --- |
| **In the model file, `run.inputs`: a list of timed changes** | A scenario is one shareable file; headless runs use it with no flags | Long recordings make files large |
| Only in separate track files | Files stay small | A scenario is two files |

## Decision

The owner chose input blocks, replay after a rewind and schedules in the model file on 2026-10-26.

### Input blocks

New block types in the equation domain, beside `units.convert`. Each is stateless, its outputs are its values, and the values are parameters. So sliders, formulas, snapshots and rewind work unchanged.

- `input.value`: one scalar output `y` with a declared unit and a range. It is driven by a slider, by keys (two keys move it down and up at a rate, or step it), or by a typed value. A steering angle or a throttle is an `input.value`.
- `input.pointer`: `x` and `y` in a view's coordinates (with that view's units) and `down` (0 or 1). It is fed by a view that sends input.
- `input.gamepad`: axes and buttons. It is added when the first demo needs it.

A model turns input into physics itself. To drag the pendulum, wire `input.pointer` into a typed-equation block that applies a spring force towards the pointer while `down` is 1. This keeps "what a drag does" in the model, where it is visible, typed and unit-checked.

### Bindings

How keys, sliders and views feed input blocks is saved in `controls.inputs`, like the rest of `controls`, and never changes results ([ADR 0012](0012-controls-in-model-files.md)). Examples:
- `{"steer": {"keys": ["ArrowLeft", "ArrowRight"], "rate": 0.5}}` (keyed by the input block's name; built as such on 2026-10-26)
- `{"hand": {"view": 2}}`

A view that sends input declares it, and `plot.xy` and the pendulum view send the pointer first.

### The input track

Every change of a value during a run is an event (time, key, value). This covers an input block's value, a slider, a typed parameter and a scheduled change. Events are applied at the first step boundary at or after their time, in the order given, and held over the step (zero-order hold), as parameter changes are today. The session keeps the track beside the history.
- **Live input** appends to it.
- **Running forward** applies whatever events lie ahead.
- **Restart** keeps the scheduled part and drops the recorded part.

### Scheduled changes

`run.inputs` in the model file is a list of timed changes: `{"t": 10, "set": "car.F", "to": 500}`, and later ramps. They are the first events of every run, in the app and headless. "Save recording as schedule" turns a run's recorded track into `run.inputs`, so a played session becomes a reproducible scenario.

### Rewind and replay

After a rewind, the recorded track ahead is kept, and running on replays it. The app shows "Replaying input" until the track ends. Any new live input during a replay removes the track from that time on and continues live, which is a new branch, as today. The rewind slider shows where recorded input lies.

### Headless

- `simsandbox run FILE --inputs TRACK.json` adds a track's events to `run.inputs`; `render` accepts it too.
- Python: `Model.run(inputs=[(t, key, value), ...])`, and `Model.schedule(t, key, value)` to write `run.inputs`.
- A track file is `{"format": 1, "events": [[t, "key", value], ...]}`. The app saves one with "Save input…".
- `describe` lists the input types; `check` reports events that name an unknown key or fall outside a value's range.

### Determinism

The same file, track and seed give the same run bit for bit, live, replayed or headless. A test runs a model with recorded input, rewinds, replays and compares every sample. A second test runs the same track headless.

## Consequences

- Q36 is settled as option (a). Scheduled changes, listed in [ADR 0042](0042-node-canvas.md), are built here.
- The session gains the track and the replay state; the kernel is unchanged apart from the new block types in `simsandbox-eq`.
- ADR 0037's rule (when slider changes apply) holds; the change it makes is now recorded.
- The first demos are dragging the pendulum (a new example or the pendulum example with a hand) and steering the cruise-control car with keys.
- Sub-step input and event-driven blocks wait for events in the master (Phase 2's later adaptive master).
- The scene view of item 3 sends its pointer and drags through the same `input.pointer`.
