# 0079 — Tables: members with stable ids whose structure changes between steps

- **Status:** Proposed
- **Date:** 2026-11-06
- **Builds on:** [ADR 0070](0070-studies-are-blocks.md) (point 4: models over sets of members), [ADR 0034](0034-agent-domain.md) (kernel-owned tables with stable ids), [ADR 0060](0060-arrays-in-typed-equations.md) and [ADR 0069](0069-repetition-in-blocks.md) (fixed grids, neighbours by index), [ADR 0077](0077-sums-in-fixed-chunks.md) (sums in a fixed order)
- **Used by:** [ADR 0078](0078-adaptive-studies.md) (refinement in adaptive studies); adaptive mesh refinement; agents written as models (Q26)

## Context

Several planned features change a model's members while it runs:
- **Adaptive studies** add points in rounds: as many as the results call for, not a fixed batch.
- **Adaptive mesh refinement (AMR)** splits cells where an error estimate is large and merges them where it is small, then fills the new cells from the old.
- **Agents** are born and die.
- **Unstructured meshes** connect cells by explicit faces.

Refining a study over one or two parameters is mesh refinement over parameter space, with a run in place of a PDE update. ADR 0070 chose one form for all of these: members are rows with stable ids in kernel-owned tables, neighbours are an edge table, structure changes only between steps by a rule, a transfer fills new rows, and ports of variable length carry ids and a structure version. This ADR makes that form concrete and orders its building.

A list (an array whose length varies, identified by position) was considered for ADR 0078 and rejected. Positions shift when a parent is replaced by its children or a row is removed, so ids would not be stable, and a strategy that reordered its points would have them all run again.

Constraints: results the same bits on any machine and core count; fixed grids keep their fast code; snapshots, rewind, probes and run files keep working; no new user-visible concept beyond the existing **Members** ([concept list](../architecture/overview.md#concepts)).

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| **Tables of rows with stable ids, edge tables, rules and transfers (ADR 0070's form), built in stages** | One structure for studies, meshes and agents; ids survive splits, merges and removals; probes and views follow members by id | Table declarations and rule syntax in typed equations; integrators, ports, recording and views must handle a changing length |
| Lists (varying length, ids by position) | Small | Not stable under splits and removals: fails for AMR and agents, and for strategies that reorder |
| Fixed-capacity arrays with masks | No new shape | Ids are slots; free-slot bookkeeping in every model; no structure for neighbours |
| A separate structure per feature (study points, mesh, agents) | Each tuned to its case | Three implementations of ids, versions, recording and transfer; contradicts ADR 0070 |

## Proposal

**1. Tables in typed equations.** `table cells` declares a table; `table pts max 200` bounds its rows (a budget, and a guard against runaway growth). Quantities indexed by a table hold a value per row: `state T[cells] [K] = 300`, `param kp[pts] [N/m]`, `input gain[pts]`, `q[cells] = k*T[cells]`. A table starts with the rows its starting rule gives (`start cells = 16`, or rows from a file), with ids 0, 1, 2, ….
- **Ids** come from a per-table counter, given at birth and never reused (as the kernel's tables already do).
- **Order:** rows are kept and processed in id order, so every loop, sum (ADR 0077) and output is the same on any machine. New rows from one rule get ids in the order of their parent ids, then child index.
- **Array functions** work on table quantities over their rows: arithmetic cell by cell, `sum`, `mean`, `minof`, `argminof`; `at(T, i)` reads by place, `of(T, id)` by id; `id(cells)` is each row's id.

**2. Edge tables.** `edges faces of cells` declares rows that each join two members (`from`, `to`) and may carry attributes (`param area[faces] [m^2]`). Operations along edges: `other(T, faces)` is the value at a face's other end; `sumover(expr, faces)` sums over each member's edges, in edge id order. A fixed grid's stencils are the special case where edges are implicit; grids keep their code. For a study, edges join neighbouring points in parameter space (the intervals, or the sides of cells).

**3. Rules change structure between steps.** After a step's outputs are computed, a table's rules run in their written order on the current values; then outputs are computed again and passed on.
- `add pts: kp = …, where …` adds rows (for example from another table or from an edge).
- `remove pts where cond` removes rows; their edges go with them.
- `split cells where cond into 2` (along an axis) or `into 4` (in two dimensions) replaces a row by children; edges between children and to neighbours are rebuilt. `split faces where cond` places a new member in the middle of an edge and replaces the edge by two: 1D refinement.
- `merge cells where cond` puts children back into their parent.

Each change increments the table's **structure version**.

**4. Transfers fill new rows.** For each quantity, a split or merge says how new values come from old ones: `copy` (a child takes its parent's value: intensive quantities), `share` (a parent's value divided among its children by size: conserved quantities), `mean` and `sum` (on merge), `interpolate` (from the parent and its neighbours, second order). Defaults: `copy` on split, `mean` on merge. A transfer is a method with speed and accuracy labels; conservative ones are tested to conserve exactly.

**5. Ports carry ids.** A table quantity's port shape is `rows`: its values, the rows' ids and the structure version. Probes record ids with values and follow members by id across changes. Views draw the current rows. A block downstream joins by id, and may keep work keyed by version.

**6. Time across structure changes.** Continuous states indexed by a table are integrated with the table's length fixed within a step. Between steps, after the rules, integrators resize their working arrays, and implicit methods rebuild their sparsity when the version changes. Snapshots and rewind keep each table's rows, ids, id counter and version; run files replay them exactly.

**7. Exactness.** Rule evaluation, ids and transfers follow id order, and each row's new values depend only on its own and its neighbours' old values, so results are the same bits on any core count. Large tables use the cell-by-cell phases of ADR 0062 step 3 over their rows.

**8. Studies over tables.** A study whose points come from inputs (ADR 0078) may take table quantities: it runs one point per row, its results are table quantities with the same ids, and it remembers each run by id, rerunning only rows that are new or whose values changed.

**Order of work** (each stage verified on a model that needs it):
1. Tables of rows in typed equations (declaration, starting rows, `add` and `remove`, `id`, `of`), `rows` ports with ids and versions, probes by id, studies over tables remembering runs by id.
2. Edge tables, `split` of edges and `merge`, with `copy` and `mean` transfers: refinement of a study over one parameter (ADR 0078).
3. `split` of cells into 4, with `share` and `interpolate`: refinement over two parameters, and the data structure of AMR in two dimensions.
4. Time on changing tables: operations along edges in rates, integrators across structure changes, conservative transfers. AMR on the heat equation and Gray–Scott, verified against a fine fixed grid; unstructured meshes from a file.
5. Agents as models: neighbour queries (within a radius), birth and death as rules; today's population ports and `agents.grazers` move to tables (Q26).

## Consequences

- One structure carries adaptive studies, AMR, unstructured meshes and agents. Each later feature adds rules, transfers or queries, not a new kind of data.
- Fixed grids are unchanged in code and speed; a model chooses tables only where members change.
- Typed equations grow table declarations and rule syntax: the start of Q26's rule language.
- Views of tables beyond values and scatter (meshes drawn by cell, refined heat maps) come with stages 3 and 4.
- **Concepts added:** none. This builds the existing **Members**: rows with ids, collapsed by array functions, with rules and transfers as labelled methods.
