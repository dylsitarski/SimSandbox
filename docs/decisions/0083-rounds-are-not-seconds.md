# 0083 — Rounds are not seconds: models that iterate run as fast as they can

- **Status:** Accepted (owner, 2026-11-07)
- **Date:** 2026-11-07
- **Amends:** [ADR 0072](0072-models-with-nothing-in-time.md) (models with nothing in time are evaluated, not run), [ADR 0078](0078-adaptive-studies.md) (a strategy's rounds are its clock's steps)
- **Builds on:** [ADR 0035](0035-simulation-worker-thread.md) (the app's worker and its pacing)

## Context

A search strategy is a map on a clock (ADR 0078): each round is one step, `kp[n+1] = …`. The app paces every model with a state at its speed, in simulated seconds per wall-clock second (ADR 0010, ADR 0035). The pattern-search example sets `dt = 1` and no speed, so the app plays it at one round a second: about 22 seconds to hold still, against 0.24 s headless. The owner asked why: nothing in it depends on time.

Its steps are not seconds. The map never reads `t`, and nothing in the model happens at a time: no continuous state, data, clocks, scheduled inputs or pointer. Such a model's history is a sequence of rounds; pacing it is waiting for nothing, and the speed slider, the time and Run/Pause mean nothing. ADR 0072 handled the case of no state at all (evaluated once, again on every change). An iteration is the same idea with rounds: computed through on opening and on every change.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| A high speed in the example's file | No code | Still seconds that are not seconds; a slider and Run/Pause that mean nothing; it never stops by itself; every such model needs it |
| An "as fast as possible" speed for any model | General | Still asks the user to choose it; does not stop when the iteration holds still |
| **Models that iterate are found when loaded and run as fast as each tick allows, stopping when a round changes nothing; a change starts them again from round 0** | The model says what it is; nothing to set; the app shows rounds; extends ADR 0072 (no state is zero rounds) | Each block type must say whether it reads time; a model with a block that does not say is paced as now |

## Decision

**1. A model iterates** when it has state, a last round, and nothing in it happens at a time:
- a `t_end`, the round budget: an open-ended map (the logistic map, chaotic, never holding still) is watched as it goes and stays paced as now;
- no continuous states (every state belongs to a map), no tables;
- no clocks, recorded data or scheduled inputs;
- every block says it does not read time (`Block::reads_time` is `Some(false)`): typed equations that never use `t`, `input.value`, unit conversion, studies (whose runs have their own time). A block that does not say (the default, `None`) keeps the model paced as now; an `input.pointer` does not say, since it follows the hand.

**2. In the app such a model iterates by itself**, as an evaluated model is evaluated: when opened and whenever a value it reads changes, it starts again from round 0 and steps as many rounds as each tick of the worker allows, with no pacing. It stops at its `t_end` (the round budget), or when a round changes nothing: every state the same bits as before, so every later round would be the same. The toolbar shows the round and whether it is iterating, holds still or reached its last round, instead of Run, Pause, the time, the step and the speed. There is no rewind bar; its history (one sample per round) is shown in its views.

**3. Headless runs are unchanged**: they already run as fast as they can, to `t_end`, with the same bits.

**4. Views still plot rounds against `t`** (round × `dt`); a round axis is a matter for the views work (ADR 0081).

## Consequences

- The search example holds still in a fraction of a second in the app, and a slider on its limit runs the search again at once.
- Block types say whether they read time; new ones should, or models using them are paced.
- **Concepts added:** none. Iteration is how the app runs a model of maps, as evaluation is for a model of formulas.

## As built

**2026-11-07.**
- **Kernel:** `Block::reads_time() -> Option<bool>`, default `None`. `Some(false)` from typed equations that use `t` nowhere (equations, initial values, edge values: `EquationSet::reads_time`), `input.value` (not `input.pointer`), `units.convert` and studies.
- **Session:** `Session::iterates` (found when loaded: a `t_end`; no clocks or scheduled inputs; every block `reads_time() == Some(false)`, no tables or data, states only in maps), `rounds_pending`, `held_still`, `round`, `stop_rounds`. `step` compares every block's state bits before and after: a round that changes nothing holds still. A restart sets rounds pending again; in the app (`evaluate_later`) a parameter change restarts from round 0.
- **App:** the worker steps an iterating model while it has rounds pending, as many as its 7 ms tick budget allows, unpaced and without sleeping, stopping at `t_end` or when it holds still; `Status` carries `round`, `iterating` and `held_still`. The toolbar says "Iterates: round n — iterating / holds still / reached its last round" in place of Run, Pause, Restart, the step, the speed and the time; no rewind bar.
- **Which examples:** the pattern search iterates and holds still at round 29 of 40 (seen in the window under Xvfb, within seconds). The logistic map has no `t_end` and stays paced. Headless runs and every example's results are unchanged.
- **Tested:** which models iterate (the search; not the logistic map, the cruise control or a map reading `t`); Newton's square root of 2 holding still in under 20 of 50 rounds; a change restarting at round 0; the worker reaching the search's end state unpaced at 1×, and after a change the same bits as a direct run.
