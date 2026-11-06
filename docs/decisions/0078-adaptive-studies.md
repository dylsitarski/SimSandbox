# 0078 — Adaptive studies: strategies that choose points in rounds

- **Status:** Proposed
- **Date:** 2026-11-06
- **Builds on:** [ADR 0067](0067-experiments-and-time-from-data.md) (step 5: adaptive experiments need their own ADR), [ADR 0070](0070-studies-are-blocks.md) (point 4: adaptive studies run in rounds), [ADR 0071](0071-sweeps-are-studies.md) (study outputs shaped by design)

## Context

A study block runs a fixed set of points: a grid, a list, random, Latin hypercube, Sobol or a file. Many questions need points chosen from earlier results: the parameters that minimise a cost or best fit measured data, where a resonance peak or a threshold lies, or how a state follows a parameter that changes slowly (hysteresis). ADR 0070 fixed the mechanism: a study runs a batch, a strategy (a labelled method, built in or Python) adds points, and this repeats until a budget or a tolerance is reached. Seeds come from point ids, and the strategy sees each round complete and in id order. This ADR fixes the strategies, the file form and the outputs.

Constraints:
- Results must be the same bits on any machine and core count. The points of a round run in parallel, as a study's do today, so a strategy should propose several points per round.
- Concepts should stay few ([concept list](../architecture/overview.md#concepts)). A study's outputs are arrays by run, read by array formulas (`argminof`, `at`).
- Every feature must be usable headless and from Python ([ADR 0039](0039-ai-agent-access.md)).

## Options considered

**How a strategy is written**

| Option | Pros | Cons |
| --- | --- | --- |
| **Built-in strategies as new `design` values, each a labelled method** | Same file section, form, check and run path as other designs; exact and fast | Users cannot add their own |
| Python callbacks | Any strategy | Needs Python at run time, not exact across versions, outside the model file |
| Both: built-ins first, Python later | Covers both needs | Python strategies need their own decision on hashing and replay |

**Optimisation method** (derivative-free, since runs are simulations)

| Option | Pros | Cons |
| --- | --- | --- |
| **Pattern (compass) search:** each round tries ± a step on every parameter around the best point (2·d runs in parallel); move to the best, else halve the step | Parallel within a round; deterministic; respects bounds; simple to verify | Local; slower than model-based methods on smooth problems |
| Nelder–Mead | Common, few runs | One or two runs per round, so no parallelism; can stall |
| Bayesian optimisation (Gaussian process) | Few runs for expensive models | Heavy (a regression model and an acquisition optimiser); hard to make exact |
| CMA-ES | Robust, parallel populations | Random (seeded from ids), more parameters to explain |

**Outputs**

| Option | Pros | Cons |
| --- | --- | --- |
| **Every run as arrays by run, in id order, plus a `round` output per run** | Nothing new: formulas find the best (`at(cost_x, argminof(cost))`); views plot the search | The best point needs a formula |
| Extra scalar outputs (`best_*`) | Direct | A second output shape for studies |

## Proposal

**1. Strategies are designs.** `sample.design` gains adaptive values. Each takes `params` with a `range` (the bounds) and a `budget` (the most runs), and stops at its tolerance or at the budget. Each is a method with speed and accuracy labels, listed by `describe`.
- **`minimize` / `maximize`:** an `objective` naming one `reduce` result; pattern search from `start` (default: the middle of the ranges), first step a quarter of each range, stopping when the step falls below `tolerance` (a fraction of each range, default 1e-3).
- **`refine`:** a starting grid over one or two parameters, then rounds that split the intervals (or cells) where a named result changes most, until the budget. For finding peaks, thresholds and edges; the output is lines by run, unordered, drawn with `plot.xy`.
- **`fit`:** parameters that best fit a data file (CSV of `t` and measured signals by full name) in least squares. The objective is the sum of squared residuals at the data times. Levenberg–Marquardt with a forward-difference Jacobian: each round runs d + 1 points in parallel.
- **`continue`:** natural-parameter continuation. Points in order along one parameter, each run starting from the previous run's final state, which shows hysteresis and follows branches. One run per round, so no parallelism.

**2. Rounds and ids.** Round 0 is the start. Each round's points take the next ids, run in parallel (the outermost study, as now), and are given to the strategy complete and in id order. The strategy is a pure function of the results so far, so a study gives the same points and bits on any core count. Seeds come from ids as now. The run hash covers the study as it does any design.

**3. Outputs.** As for other designs: one value per run, in id order, for each swept parameter and each `reduce` result, plus `round` (the round each run belonged to). Their shape is lines by run. The best point is a formula (`at(kp, argminof(cost))`), and the search is plotted with `plot.xy`.

**4. Running.** As other studies: run again from round 0 when an input changes, given up for newer changes in the app, with progress shown by round and runs. `keep: "series"` stays refused (Q52).

**5. Later, separately:** Python strategies (hashing and replay need their own decision); continuation of equilibria (pseudo-arclength, which needs a steady-state solve); global optimisers.

**Order of work** (each step headless, then Python, then the app's Inspector form):
1. The round mechanism and `minimize` / `maximize`.
2. `refine`.
3. `fit`, with the data file in the run hash.
4. `continue`.

## Consequences

- One study block covers sweeps, optimisation, fitting, refinement and continuation; the file section, Inspector form, check and run path are shared.
- Pattern search is local and needs bounds; it finds a nearby optimum, not the best over the whole range.
- **Concepts added:** none. Strategies are designs; rounds are a property of a study's runs, seen as one more output.
