# 0078 — Adaptive studies: strategies are models that choose a study's points

- **Status:** Accepted
- **Date:** 2026-11-06
- **Builds on:** [ADR 0067](0067-experiments-and-time-from-data.md) (step 5: adaptive experiments need their own ADR), [ADR 0070](0070-studies-are-blocks.md) (point 4: adaptive studies run in rounds), [ADR 0075](0075-parameters-are-inputs-with-defaults.md) (starting values are parameters, and may be driven), [ADR 0079](0079-tables-of-members.md) (tables of members, for refinement)

## Context

A study block runs a fixed set of points: a grid, a list, random, Latin hypercube, Sobol or a file. Many questions need points chosen from earlier results:
- the parameters that minimise a cost or best fit measured data;
- where a resonance peak, a threshold or an edge lies (refinement);
- how a state follows a slowly changing parameter (continuation, hysteresis).

ADR 0070 fixed the mechanism: a study runs a batch, a strategy adds points, and this repeats until a budget or a tolerance is reached; seeds come from point ids, and the strategy sees each round complete and in id order.

A first draft of this ADR made the strategies built-in designs (`minimize`, `refine`, `fit`, `continue`). The owner asked whether they could be models of blocks and wires instead, for generality and the fewest concepts. They can. A strategy is a process in rounds: it keeps a state (the best point so far, a step size), proposes points, reads their results and updates its state. That is a discrete-time model beside a study block.

The owner also asked that refinement may add any number of points per round, and that the method carry over to adaptive mesh refinement. Refining a study over its parameters is mesh refinement over parameter space, so refinement uses the tables of [ADR 0079](0079-tables-of-members.md) (rows with stable ids, edges, split rules, transfers), the structure AMR and agents will use too. A second draft's lists (arrays of varying length, ids by position) were dropped: their ids are not stable when rows are split or removed.

Constraints:
- Results must be the same bits on any machine and core count; a round's points run in parallel.
- Concepts should stay few ([concept list](../architecture/overview.md#concepts)).
- Everything must work headless and from Python ([ADR 0039](0039-ai-agent-access.md)).

## Options considered

**Where strategies live**

| Option | Pros | Cons |
| --- | --- | --- |
| **Models: a strategy is a map (typed equations on a clock) wired to a study whose points come from inputs** | No new concept for strategies; any strategy a user can write; editable, visible in probes, views and rewind; rounds are clock steps | Writing one is harder than naming a design: needs shipped examples and helpers |
| Built-in designs (`minimize`, `refine`, …) | One line in a file | A new design per strategy; hidden code; users cannot change or add them |
| Python callbacks | Any strategy | Needs Python at run time; outside the model file; not exact across versions |

**Rounds that add any number of points (refinement)**

| Option | Pros | Cons |
| --- | --- | --- |
| A fixed number of new points per round | Fixed array shapes | Refinement cannot follow the problem (declined by the owner) |
| Lists: arrays of varying length, ids by position | Few new functions | Ids shift when a point is split or removed: not the structure AMR needs; a strategy that reorders reruns everything |
| **Tables of [ADR 0079](0079-tables-of-members.md): points as rows with stable ids, intervals or cells as edges, `split` rules** | Any number of points per round; the same structure as AMR and agents | Waits for ADR 0079's first two stages (one parameter) and third (two) |

## Proposal

**1. Points from inputs.** A study design `inputs`: the points arrive through the study's inputs, one array input per swept parameter, a value per point, wired from any model. The study runs one point per cell; its outputs are arrays in the same order, as now. Other inputs stay scalars shared by every point. A starting value is a parameter (`x0`), so a point can set the state a run starts from: continuation feeds the previous run's final state in this way.

**2. Runs remembered by point.** A point's id is its place in array inputs, or its row id in table inputs ([ADR 0079](0079-tables-of-members.md)); it seeds the point's randomness as now. When the inputs change, a point whose values and shared inputs are unchanged keeps its results; only new or changed points run. A strategy can therefore send every point so far at each round and pay only for the new ones. Results are the same bits as running them all again.

**3. Refinement on tables.** A study's points may be a table ([ADR 0079](0079-tables-of-members.md)): one run per row, results as table quantities with the same ids. A refining strategy holds the points as a table with edges between neighbours in parameter space (intervals in one parameter, cell sides in two) and a rule such as `split faces where abs(other(gain, faces) - gain) > tol`. Each round adds as many points as the rule finds, up to the table's `max`. Points keep their ids, so only the new ones run.

**4. A linear solve.** `solve(A, b)`: the solution of a small dense system, `A` a d × d grid (row-major) and `b` an array of d values, by LU with partial pivoting in a fixed order (exact bits on any machine). Levenberg–Marquardt and other model-based strategies need it.

**5. Strategies shipped as models.** Example definitions (model files in `examples/strategies/`), each a map wired to a study, with labels and a verification test:
- **pattern search** (`minimize`): ± a step on every parameter around the best point, 2·d runs per round in parallel, the step halved when nothing improves;
- **Levenberg–Marquardt** (`fit`): least squares against a data file read by `input.file`, d + 1 runs per round for a forward-difference Jacobian, `solve` for the step;
- **refinement** (`refine`): start from a grid; each round, split every interval (one parameter) or cell (two) where a result changes by more than a tolerance, as many as there are, up to the table's `max`;
- **natural-parameter continuation** (`continue`): one run per round, each starting from the previous run's final state.

**6. Stopping and budget.** A strategy's run lasts its clock's steps to `t_end` (the round budget). A strategy that has converged stops changing its outputs, so the study does not run again (point 2) and later rounds cost nothing. Ending a run early on an event belongs to Activation (Q53).

**7. Helpers.** Python (`Model.minimize(target, params, objective, budget)`, `fit`, `refine`, `continue_`) and the app (a "Search…" action on a study) write the strategy model from the examples. A strategy model stays an ordinary model the user can open and change.

**Order of work** (each step headless and tested, then Python, then the app):
1. Points from inputs and runs remembered by point; pattern search as an example model.
2. `solve`; Levenberg–Marquardt fitting.
3. Continuation.
4. Refinement over one parameter, once ADR 0079's stages 1 and 2 are built; over two parameters after its stage 3.
5. Helpers in Python and the app.

## As built

**Step 1 (2026-11-06): points from inputs, runs remembered by point, pattern search as a model.**
- **The design:** `"design": "inputs"` with `count` (required) in `simsandbox_file::Design::Inputs`. The study block takes one input per swept parameter, a line of `count` cells named as the parameter's output would be (`cruise_kp`), before the model's own input blocks; the swept parameters are not outputs. Its results are lines by run, as for other designs other than grids. The app's study form offers "points from its inputs" and its count.
- **Remembered by point:** `StudyBlock::evaluate_points` keeps, for each place, the point's values and results (each output's replicates), with the shared inputs and parameters they ran with. When the inputs change, only places whose values changed, or all of them when the shared inputs or parameters changed, run, as a plan of those points with their places as ids, so seeds mixed from ids are unchanged. Tested: a point that moves runs alone, and the results equal a study of the same points from scratch, bit for bit.
- **A study starts late:** before, every block computed its outputs once at the start of a run with inputs of zero, before the first exchange, so a study ran every point once for nothing. The simulator now skips that first computation for a block with no state or tables whose outputs the output plan computes entirely from its inputs, outside loops (`Simulator::late`, `OutputPlan::computes_all_of`). What it held was never read, so results are unchanged (all examples and profiling cases byte-identical).
- **Maps with grids:** typed-equation maps may declare grids (`grid k = 5`), array inputs and array states (`state c[k] = …`, `c[n+1] = …`); a scalar state stepped by an array is refused, asking for the declaration. Which quantities are arrays is now found in any order of the lines.
- **Pattern search** is `examples/cruise-search.ssbx.json` (the ADR planned `examples/strategies/`; the examples are one folder): a map over `kp` and `ki` sends the study its centre and a step either way on each (five points), reads each run's score (reach time plus a penalty past a peak limit), moves to the best or halves the step, and holds still below a tolerance. From kp 400, ki 40 it reaches a score of 0.8 s in about 22 rounds; the run takes 0.24 s. A test runs the same strategy on a bowl with a known minimum, (1.3, −0.4), to within 1e-4, and checks that rounds after it holds still run nothing.
- Not yet: `solve` and fitting (step 2), continuation (step 3), refinement (step 4, on ADR 0079), helpers in Python and the app (step 5). Python runs these models as any other.

## Consequences

- No built-in strategy designs: optimisation, fitting, refinement and continuation are models anyone can read and change. The examples and helpers keep the simple case short.
- A study with points from inputs is fed by any model, not only strategies: an experiment planned by another model, or points from a probe.
- Remembering runs by point also helps fixed designs whose inputs change only in part.
- Refinement in studies and adaptive mesh refinement share one structure ([ADR 0079](0079-tables-of-members.md)), so neither builds its own.
- **Concepts added:** none. Strategies, rounds and searches are existing concepts (models, clocks, studies, wires); refinement uses Members as ADR 0079 builds them.
