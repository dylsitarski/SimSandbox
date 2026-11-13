# 0088 — Activation: a used model or study acts when a signal rises

- **Status:** Accepted (owner, 2026-11-13: the four modes with `rise` by default, held outputs keep their last values, `restart` as an input of its own, events within a step later on the same wire, studies first; time going on is mine)
- **Date:** 2026-11-13
- **Resolves:** [Q53](../open-questions.md)
- **Builds on:** [ADR 0075](0075-parameters-are-inputs-with-defaults.md) decisions 3 and 6 (driven starting values and study parameters), [ADR 0066](0066-models-nest-as-blocks.md) (models use models; one flat runtime), [ADR 0070](0070-studies-are-blocks.md) and [ADR 0071](0071-sweeps-are-studies.md) (studies run when their inputs change), [ADR 0067](0067-experiments-and-time-from-data.md) step 4 (listed instants)

## Context

ADR 0075 lets a wire drive a starting value (read when the block starts and at Restart) and a study's parameter (the study runs again when it changes). Both are most useful when a part of a model starts again *because something happened*: the cart-pole of the Phase 2 gate starts a new attempt when the pole falls; a controller is retuned by a study each time a condition holds, not at every change of its inputs; a batch is restarted each time a tank empties. Today a model starts once (and at Restart, for the whole model), and a study runs whenever any input changes.

**What exists:**
- A study runs from its model's start to its end at each change of its inputs and holds its results between.
- A model that uses models is one flat runtime: one list of blocks and links, each used model's blocks on its parent's clock or its own.
- Signals are numbers; comparisons give 0 or 1 (ADR 0070 step 3a). There is no event kind.
- Clocks can step at listed instants, known before the run.

## Options considered

| What starts it | Pros | Cons |
| --- | --- | --- |
| **A signal wired to an `activate` input, with a mode: on its rise, fall or change, or while it is high (proposed; first written as on its rise only)** | One wire, any condition the equations can compute (`fell = angle > 0.5`); seen and rewired on the canvas; replays exactly, since it is a signal like any other | Detected at communication instants only: a condition true for less than a step can be missed |
| An event kind of signal, blocks that fire and blocks that are fired | Exact event times (zero crossings located within a step) | A new kind of wire, of port and of block behaviour; zero-crossing location is a solver feature the kernel does not have |
| Listed instants only | Exists (clocks at listed times) | Only events known before the run; not the cart-pole |

| What an activated model does | Pros | Cons |
| --- | --- | --- |
| **Starts again: its states and tables back to their starting values, reading driven starting values then; time goes on (recommended)** | Matches Restart for a part of the model; time stays one for the whole model, so links, probes and replay need nothing new | A used model reading `t` sees the whole model's time, not time since it started |
| Starts again with its own time from 0 | Episodes read naturally (`t` since the attempt began) | Only for a used model on its own clock; its clock jumps back, so probes and links across clocks see time go backwards |
| Runs to its end within the instant, outputs held (an episode) | One evaluation per event | That is a study of one point: a study with `activate` already does it |

## Proposal

Revised 2026-11-13 after the owner's answers: activation covers steps as well as impulses (toggle control), with four modes, `rise` by default (owner); a held part's outputs keep their last values (owner); whether a part resumes or starts again may depend on a condition (owner), so starting again is an input of its own, not a setting; studies first, then used models (owner); time goes on for the whole model (the owner had no preference; mine).

1. **`activate`, an input any used model and any study may have,** with a mode, `on`. Wired, it decides when the part acts; unwired, nothing changes from today. Written as any wire (`{"from": "pole.fell", "to": "cart.activate"}`), the mode on the use or study (`"activate": {"on": "while"}`; `rise` when not given).
2. **The signal is high above 0.5,** read at each communication instant of the part's clock. Modes:

   | `on` | The part | Typical use |
   | --- | --- | --- |
   | `rise` | acts once when the signal goes high | run a study once when a condition starts |
   | `fall` | acts once when it goes low | act when a condition ends |
   | `change` | acts once at each change | a toggle |
   | `while` | runs only while the signal is high; while low it holds its states, and its outputs keep their last values | a controller switched on and off by a step |

   An edge is a change between two instants; a pulse shorter than a step can be missed, as for any sampled signal (point 7). At the start nothing is an edge; under `while` a part starting low holds its starting values.
3. **For a study,** acting is running: on an edge it runs once, reading its inputs and driven parameters then, and holds its results; under `while` it runs as today (when its inputs change) while high, and holds its results while low.
4. **For a used model,** acting under an edge mode is starting again (point 5); under `while`, it steps only while high and resumes from its held states when the signal goes high again.
5. **`restart`, a second input of a used model:** on its rise the model starts again: its blocks' states and tables go back to their starting values, driven starting values read then (ADR 0075 decision 3), and initial outputs computed in dependency order; parameters keep their current values. Whether a part resumes or starts again is then wiring, not a setting: wire `activate` alone to resume; wire the same signal to both to start again each time it is switched on; wire a condition (`again = switch*(angle > 0.5)`) to `restart` to start again only when it holds. Time goes on for the whole model; a model needing time since it last started declares a state for it (`dτ/dt = 1`).
6. **Exact and replayable:** edges and levels come from recorded signals at fixed instants, so a run replays bit for bit; rewinds restore each input's previous value with the state; run files need nothing new.
7. **Events within a step come later as a capability of a signal's source, not as a new kind of wire.** Contacts, switches and thresholds whose time matters physically (an impact, a relay, a thermostat) need the instant located within the step, and the step shortened or rolled back to it (the block contract's `Rollback`). When needed, a block will declare that an output is an event it can locate (a crossing it can find by root finding), and the master will cut the step there for every part whose `activate` or `restart` it reaches. The wire, the modes and models already built stay as they are. Rigid-body engines locate their own contacts inside their domain, so the Phase 2 gate does not need this.
8. **On the canvas:** the `activate` and `restart` ports first among a node's inputs, the mode beside `activate`; a part held, or started again, says so on its node (held; the count of its starts).

## Order of work

1. **Studies:** `activate` with its four modes on the study block, headless (file, `check`, `run`, Python) and in the app.
2. **Used models:** `activate` and `restart` on uses; the kernel holds or starts again a used model's blocks mid-run.
3. Events located within a step, when a model needs them (point 7).

## Consequences

- One new concept: activation (an input with a meaning, not a new kind of signal). Restart of the whole model is unchanged.
- The kernel resets a subset of its blocks mid-run (their states, tables and initial outputs) as Restart does for all.
- Studies gain an input (`activate`), used models two (`activate`, `restart`); files without them are unchanged.
- Updated with the work: the concept list, [headless guide](../guides/headless.md), STATUS, Q53 resolved.

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): **activation**, because no existing concept says *when* a part starts: wires carry values, clocks say when steps happen, and Restart applies to the whole model.

## As built

**Step 1 (2026-11-13): studies.** `crates/simsandbox-session/src/study.rs` (`Activate`, `StudyBlock`), `BlockEntry::activate`.
- **File:** `"activate": "while"` on a study (a mode as a string: `rise`, `fall`, `change`, `while`), passed to the study type as a setting; an unknown mode is refused, and so is `activate` on any block but a study (used models come with step 2). Point 1 as built: the `activate` input exists when a mode is written (the kernel has no optional inputs), and must then be wired, as any input; without a mode the study is as before. Files without the field are unchanged.
- **The study:** a dimensionless `activate` input after its own; states: the level last read, whether it holds results, and the inputs and parameters it holds. At each instant its outputs come from the current inputs when it acts (or holds nothing yet, at the start) and from those it holds otherwise; the step commits that choice. Its runs are the same pure function of what it runs with as before (cached by their bits), so results held are bit for bit those of the run that made them, and a rewind restores what it holds. Over a table's rows it is not activated yet.
- **In the app:** "Runs" in the study's Inspector form: when its inputs change, or on `activate` with each mode (`Edit::SetActivate`); turning it off removes the input and its wire.
- **Tested:** a study scaling its input `u = t`, gated high at t = 3, 4, 5, runs with u = 0, 0, 0, 3, 3, … (`rise`), 0 until 6 then 6 (`fall`), 0, 0, 0, 3, 3, 3, 6, 6, 6 (`change`), 0, 0, 0, 3, 4, 5, 5, 5, 5 (`while`), and 0 … 8 without activation; both runs hold alike; a rewind into a held stretch replays the same bits; an unknown mode, an unwired `activate` and activation off a study are refused; the edit adds the port, a dimensionless switch wires to it (a speed is refused for its unit), and turning it off removes port and wire.

