# 0034 — The agent domain: populations, update order and randomness

- **Status:** Accepted
- **Date:** 2026-10-19

## Context

Phase 1's gate is an ODE model and an agent model in one graph, shown by the grazing model: grass regrowing per grid cell by an ODE, sheep that wander, eat, breed and die ([ADR 0033](0033-phase-1-plan.md)). Agents are the first domain that is not equations, and they strain the Block contract ([ADR 0002](0002-layered-block-contract.md)) in three ways:

- **Populations change size.** The kernel owns each Block's state as a fixed-length `f64` slice, so snapshots, rewind and saving work the same for every Block; ports have fixed shapes, and `Shape::Points` has a fixed `count`.
- **Update order changes results.** When agents compete (two sheep, one patch of grass), who acts first matters.
- **Rules are random.** Runs must still be reproducible and rewind must replay exactly.

The owner decided on 2026-10-19: no arbitrary capacity limits, chosen for future flexibility over the simplest build; agent behaviour as built-in Block types first; update order chosen per block, as part of the model.

## Options considered

**Populations of changing size**

| Option | Pros | Cons |
| --- | --- | --- |
| Fixed capacity: slots with an alive flag in the fixed state | No kernel change | An arbitrary limit in every model; births beyond it lost; memory for empty slots; the owner wants no such limit |
| Kernel-owned **tables**: named collections of rows that grow and shrink, beside the fixed state; variable-length point-set ports | No limit; snapshots, rewind and saving stay uniform and inspectable; one structure for agents, particles and any later population | Kernel, ports, connections and history learn variable lengths |
| Block-owned state with its own save and restore | Most freedom inside a Block | The kernel can no longer inspect, save or compare that state uniformly; every Block re-implements snapshots |
| An ECS library (`bevy_ecs`, `hecs`) holding the agents | Mature queries and storage | State outside the kernel (the previous row's cons); one table per agent type is all Phase 1 needs |

**Update order**

| Option | Pros | Cons |
| --- | --- | --- |
| A method setting | Existing mechanism | Methods are reserved for approximations ([ADR 0020](0020-methods-for-approximations.md)); update order is part of what is modelled |
| A **block setting in the model definition**, from the orders the type supports | Visible, saved, changeable per block, honest that it changes results | One more field in block entries |
| Fixed by each type | Simplest | The user cannot choose |

**Randomness**

| Option | Pros | Cons |
| --- | --- | --- |
| A generator whose state is saved with the Block | Usual | Draws depend on storage order and on how many draws came before |
| **Counter-based draws**: each draw is a hash of (seed, block, step, agent id, purpose) | Reproducible regardless of storage order; rewind and replay give the same draws with nothing extra to save; agents keep their draws when others are born or die | Needs stable agent ids and a good mixing hash |

## Decision

- **Tables in kernel-owned state.** A Block may declare **tables** (`BlockSpec::tables`: a name and named `f64` columns, e.g. `sheep: [x, y, energy]`) beside its fixed `states`. The kernel owns the rows, a structure of arrays that grows and shrinks each step; snapshots, rewind and saving include them exactly as they include fixed state. Each row has a stable integer **id** given at birth by the kernel and never reused within a run. There is no capacity; models may set limits of their own as parameters.
- **Variable-length point sets.** `Shape::Points` gains a variable count: a port of `dim` coordinates per point whose length changes from step to step (for example, sheep positions). Connections between variable point sets check `dim` and units; the history records each frame at its own length, and views already draw whatever points a frame holds. Fixed shapes are unchanged.
- **The agent domain** is a crate, `simsandbox-agents`, of built-in Block types in `Domain::Agents`, the first being `agents.grazers` (sheep). A typed rule language like typed equations ([ADR 0017](0017-typed-equations.md)) comes once two or three agent models show what it needs ([Q26](../open-questions.md)). The space is a 2D rectangle that wraps at its edges (a torus), size a parameter; positions are continuous; agents read and write per-cell grids (the grass) through grid ports aligned with that space. Neighbourhood queries use a uniform-grid index inside the domain crate, rebuilt each step.
- **Update order is part of the model.** Agent Blocks take an `update` setting in their block entry, saved in the file and shown with the block's parameters: `random_order` (agents act one after another in a seeded random order, new each step; the default), `synchronous` (every agent decides from the same start-of-step state, then the type's conflict rule applies, e.g. sheep in one cell share its grass in proportion to appetite) and `id_order` (oldest first; deterministic, biased). A type lists the orders it supports.
- **Randomness is counter-based.** Every draw is a function of the model's seed, the Block's name, the step number, the agent's id and what the draw is for (SplitMix64-style mixing, as in `simsandbox_core::sampling`). The seed is `run.seed` (default 1, part of the `run` section, [ADR 0013](0013-run-section.md)). Same file, same seed: the same run, bit for bit, including after rewind.
- **Outputs and inputs.** Agent Blocks output populations as variable point sets (positions) and aligned variable columns (e.g. each sheep's energy), aggregates as scalars (count, mean energy), and effects on the environment as grids (grass eaten per cell per second). They read grids and scalars from other Blocks. Agents and equations meet through the master like any two domains: the coupling is one communication step and is labelled as such.
- **Labels and verification.** Agent rules are computed exactly as stated, so `agents.grazers` is labelled real-time and quantitative; the model is stochastic and is verified statistically ([verification policy](../guides/verification-policy.md)): bit-for-bit reproducibility and rewind replay; a random walk's mean squared displacement against 4Dt within a confidence bound; births and deaths at constant rates against the expected exponential mean over many seeds; grass with no sheep against the logistic solution; grass eaten equal to energy gained.

## Consequences

- Kernel work comes first: `BlockSpec::tables`, row storage with stable ids in snapshots, `StepIo` access to tables, variable point-set ports and connections, and variable-length frames in the history. Fixed-size Blocks are unaffected.
- The file format gains `run.seed` and, for agent blocks, `update`; the schema and [block contract](../architecture/block-contract.md) are updated with them.
- The grass is an equation Block with a grid state and a grazing input grid (logistic growth minus grazing), so the gate's "ODE and agents in one graph" holds literally.
- Direct feedthrough ([Q5](../open-questions.md)) comes next on the roadmap; agent Blocks avoid it by reading inputs at the start of a step and writing outputs at its end.
- The simulation stays on the UI thread until the grazing model is measured ([ADR 0033](0033-phase-1-plan.md)).
