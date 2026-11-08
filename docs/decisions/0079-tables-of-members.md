# 0079 — Tables: members with stable ids whose structure changes between steps

- **Status:** Accepted (stage 1a built 2026-11-08)
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

## As built

**Stage 1 is built in parts** (owner, 2026-11-08): 1a, tables inside one typed-equations block; then [ADR 0081](0081-shapes-are-lists-of-axes.md) stages 1–2 (shapes as axes, with a rows axis); then 1b, ports carrying rows with ids and a version; 1c, probes and history by id; 1d, studies over tables.

**1a (2026-11-08): tables inside one block.**
- **Declarations:** `table pts` or `table pts max 200` (default 100 000 rows), `start pts = 5` (default 0). A table is an axis whose length is the current number of rows. States over it hold a value per row (`state age[pts] = id(pts)`, a starting value that may read `id(pts)`); defined quantities over it follow. A quantity over rows has no other axis; parameters, inputs and edges over rows are refused (inputs come with 1b). Tables are for maps only: continuous states over rows are stage 4.
- **Rules** run after each step's update, in written order, on the new values: `remove pts where cond` (cond over the rows); `add pts [from k] [where cond][: c = value, …]`, one new row for each place of `k` (another table, a grid axis, or none: one row) where the condition holds, in that order, until the table's `max`. A column not given takes its starting value if that is one value, else must be given. New rows get the next ids, never reused (the kernel's `Table`).
- **Functions:** `id(pts)` (each row's id), `rows(pts)` (how many), `of(T, i)` (T in the row with id `i`, NaN if none). Reductions over rows work as over any axis; stencils, `on` and edges do not (rows have no neighbours by place until stage 2).
- **How it runs:** a table is a kernel table (ADR 0034), its columns the states over its rows, so snapshots, rewind and run files keep rows, ids and the id counter. The block compiles its equations for the current numbers of rows (`EquationSet::parse_with_rows`), keeping each compiled set by numbers of rows (up to 64), and compiles again when a rule changes them: every step runs fixed-length loops, exact as before. States over rows come after the fixed ones in the program; row ids are extra inputs; quantities over rows are not ports (1b). Rules compile to a program of their own (`EquationSet::rule_registers`). `ExplicitBlock::init_tables` makes the starting rows. A model of maps with tables iterates in the app (ADR 0083): the hold-still check compares tables too.
- **Tested** (`crates/simsandbox-session/tests/tables.rs`): 30 rounds of a population with ageing, deaths, births from young rows, a row budget and rows added from a grid axis, against the same rules written directly in Rust, bit for bit; ids never reused and `of` NaN once its row is gone; rewinding and running on gives the same rows; mistakes explained; a table that empties and refills; a map with tables holding still. Every example and profiling case is unchanged.

**1b (2026-11-08): ports carrying rows.**
- **Shape:** a quantity over a table's rows is a port of shape `Shape::Rows { dim: 1 }` (ADR 0081's rows axis, named after the table), states and defined quantities alike. Like a population it takes no room in the fixed arrays: each frame is `RowsFrame` (`[version, n, ids…, values…]`) on the variable-length channel. The version is the table's (`Table::version`, raised by each push, removal or reassignment, kept in snapshots). A wire joins rows to rows (`dim` equal); a unit's factor scales the values, not the ids.
- **Following:** `input size[q]` is over the table `q`, which then follows the rows of that input: its rows are the input's rows, in the input's order, with their ids (`Table::assign`). A followed table takes no `start` and no rules (its rows come from upstream). A state over it keeps each row's value by id; a row new to the input takes the state's starting value (which may read `id(q)` and parameters), and a row gone from the input is dropped. Other inputs over the same table are matched to those rows by id, NaN where one lacks a row. A block of formulas may have followed tables (`input u[p]`, `y = 2*u`); tables with rules still need a map.
- **The same instant:** an output that reads a followed table (a quantity over it, `id(q)`, `rows(q)`, a reduction over it) depends on the input it follows at the same instant (`PortSpec::depends_on`), so the kernel computes it after its source, with this instant's rows. The kernel's output plan orders outputs through rows wires as through fixed ones (`loops::RowsLink`), and routes the frames into a block just before computing it. So a block downstream always holds the rows its source holds now. A loop through rows is refused (rows cannot be solved for by Newton's method); a state breaks it.
- **Not yet:** rows are not recorded, written by `run` or shown in views (recording built in 1c; views with ADR 0081 stage 3); rows into a study (1d).
- **Tested** (`crates/simsandbox-session/tests/table_ports.rs`): rows from the 1a population passed to a block that follows them, and on to a third: counts, newest ids and sums agree at every instant to the bit; a row's sum kept by its id while rows come and go, and NaN once the row is gone; wrong shapes, `start` or rules on a followed table, and a loop through rows refused with explanations. Every example and profiling case gives the same CSV and JSON byte for byte, and every example the same image.

**1c (2026-11-08): rows recorded by id.**
- **History:** a rows port is recorded as `FieldShape::Rows { dim }`, each sample a frame with the table's version and the rows' ids, counted against the memory budget as populations are. `Field::by_id(id)` follows one row through the samples (NaN where it is not there), `Field::ids()` lists every id seen, `Field::rows(k)` gives a sample's version, ids and values. Views do not draw rows yet (with ADR 0081 stage 3).
- **Probes:** a probe recording rows writes `<stem>.<field>.csv` beside its CSV, a line per row per sample (`t,version,id,value`): appended as it records (streamed), and cut back on a rewind like the rest. JSON gives each sample as `{version, ids, values}`. A probe does not pass rows on to a block yet (refused with a message saying so).
- **`run`:** JSON has `over_rows`: for each quantity over rows, per row of the results, `versions`, `ids` and `values`. CSV has a column per id any row had (`pop.size#7`; `#7[c]` for several values a row), NaN where that row is not there; populations, whose members have no ids, are still left out. The JSON key `over_rows` is new in every output (empty without tables); nothing else changes.
- **Python:** `Results.over_rows` and `Probe.over_rows`, each a `Rows`: `ids`, `values` (lists over rows), `versions`, `of(id)` (one row through the run, NaN where absent) and `by_id()` (every id and a matrix over them); `results["pop.size"]` finds it.
- **Tested** (`table_ports.rs`, `python/tests/test_api.py`): a row followed by id in `run`'s CSV, the session's history and Python equals `of(size, 2)` as the model computes it, NaN once gone; downstream rows have the source's ids at every round; JSON's first frame; a streamed probe's rows file holds a line per row per round, is cut back by a rewind and written the same on running again. Every example and profiling case gives the same CSV and JSON (apart from the empty `over_rows`) and the same images.
