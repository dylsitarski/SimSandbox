# 0078 — Adaptive studies: strategies are models that choose a study's points

- **Status:** Proposed
- **Date:** 2026-11-06
- **Builds on:** [ADR 0067](0067-experiments-and-time-from-data.md) (step 5: adaptive experiments need their own ADR), [ADR 0070](0070-studies-are-blocks.md) (point 4: adaptive studies run in rounds; ports of variable length), [ADR 0075](0075-parameters-are-inputs-with-defaults.md) (starting values are parameters, and may be driven)

## Context

A study block runs a fixed set of points: a grid, a list, random, Latin hypercube, Sobol or a file. Many questions need points chosen from earlier results:
- the parameters that minimise a cost or best fit measured data;
- where a resonance peak, a threshold or an edge lies (refinement);
- how a state follows a slowly changing parameter (continuation, hysteresis).

ADR 0070 fixed the mechanism: a study runs a batch, a strategy adds points, and this repeats until a budget or a tolerance is reached; seeds come from point ids, and the strategy sees each round complete and in id order.

A first draft of this ADR made the strategies built-in designs (`minimize`, `refine`, `fit`, `continue`). The owner asked whether they could be models of blocks and wires instead, for generality and the fewest concepts. They can. A strategy is a process in rounds: it keeps a state (the best point so far, a step size), proposes points, reads their results and updates its state. That is a discrete-time model beside a study block. The owner also asked that refinement may add any number of points per round, not a fixed batch.

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
| Fixed-capacity arrays with a mask the strategy fills in | No new shape | The strategy must compute free slots for its new points; clumsy and easy to get wrong |
| **Lists: arrays whose length changes between steps, up to a declared capacity, built with `select`, `concat` and `sort` (ADR 0070's ports of variable length, first for numbers)** | Any number of new points per round; natural to write (`concat(points, select(mid, change > tol))`); fixed storage (the capacity) keeps the kernel's buffers and exactness | A new shape (a length on array ports) through the expression language, ports, probes and views |
| Agent tables (rows with births and deaths, ADR 0034, Q26) | The general form | The rule language is not built; heavy for lists of numbers |

## Proposal

**1. Points from inputs.** A study design `inputs`: the points arrive through the study's inputs, one array input per swept parameter, a value per point, wired from any model. The study runs one point per cell; its outputs are arrays in the same order, as now. Other inputs stay scalars shared by every point. A starting value is a parameter (`x0`), so a point can set the state a run starts from: continuation feeds the previous run's final state in this way.

**2. Runs remembered by point.** A point is identified by its place in the inputs (its id), which seeds its randomness as now. When the inputs change, a point whose values and shared inputs are unchanged keeps its results; only new or changed points run. A strategy can therefore send every point so far at each round and pay only for the new ones. Results are the same bits as running them all again.

**3. Lists.** An array may be a **list**: its length changes between steps, up to a declared capacity, for example `list x = 0 .. 400` in typed equations: at most 400 values, starting empty.
- **Built** by `concat(a, b)`, `select(a, mask)` (the cells where `mask` holds) and `sort(a)` with `sortindex(a)` (the order of `a`, to read other lists in that order with `at`). The functions that already work on arrays work on lists: element-wise arithmetic, `sum`, `minof`, `argminof`, `at`, `prev` and `next`.
- **Ports** carry a list's current length with its values; storage is the capacity, so the kernel's buffers and exactness are unchanged. Probes and views show the current length.
- **A study over list inputs** runs as many points as the lists are long; with point 2, appending k points runs exactly k new runs.
- **Ids** are places, so a strategy that only appends keeps every point's id; one that reorders its points gets them run again (point 2).
- Grids keep fixed lengths. Lists are the first form of ADR 0070's ports of variable length; rows with stable ids that are removed (agents, meshes) stay with that plan.

**4. A linear solve.** `solve(A, b)`: the solution of a small dense system, `A` a d × d grid (row-major) and `b` a list or array of d values, by LU with partial pivoting in a fixed order (exact bits on any machine). Levenberg–Marquardt and other model-based strategies need it.

**5. Strategies shipped as models.** Example definitions (model files in `examples/strategies/`), each a map wired to a study, with labels and a verification test:
- **pattern search** (`minimize`): ± a step on every parameter around the best point, 2·d runs per round in parallel, the step halved when nothing improves;
- **Levenberg–Marquardt** (`fit`): least squares against a data file read by `input.file`, d + 1 runs per round for a forward-difference Jacobian, `solve` for the step;
- **refinement** (`refine`): start from a grid; each round, split every interval where a result changes by more than a tolerance, as many as there are, until the capacity;
- **natural-parameter continuation** (`continue`): one run per round, each starting from the previous run's final state.

**6. Stopping and budget.** A strategy's run lasts its clock's steps to `t_end` (the round budget). A strategy that has converged stops changing its outputs, so the study does not run again (point 2) and later rounds cost nothing. Ending a run early on an event belongs to Activation (Q53).

**7. Helpers.** Python (`Model.minimize(target, params, objective, budget)`, `fit`, `refine`, `continue_`) and the app (a "Search…" action on a study) write the strategy model from the examples. A strategy model stays an ordinary model the user can open and change.

**Order of work** (each step headless and tested, then Python, then the app):
1. Points from inputs and runs remembered by point; pattern search as an example model.
2. `solve`; Levenberg–Marquardt fitting.
3. Continuation.
4. Lists (`list`, `concat`, `select`, `sort`, `sortindex`, lengths on ports, studies over lists); refinement.
5. Helpers in Python and the app.

## Consequences

- No built-in strategy designs: optimisation, fitting, refinement and continuation are models anyone can read and change. The examples and helpers keep the simple case short.
- A study with points from inputs is fed by any model, not only strategies: an experiment planned by another model, or points from a probe.
- Remembering runs by point also helps fixed designs whose inputs change only in part.
- Lists add a shape (a length on array ports) through the expression language, ports, probes and views. Grids are unchanged.
- **Concepts added:** the list (an array with a varying length, up to a capacity). Strategies, rounds and searches are existing concepts (models, clocks, studies, wires). **Removed:** none.
