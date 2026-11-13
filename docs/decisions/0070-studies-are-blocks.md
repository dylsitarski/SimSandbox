# 0070 — A study is a block; models over sets of members share one interface

- **Status:** Accepted
- **Date:** 2026-11-03
- **Amends:** [ADR 0067](0067-experiments-and-time-from-data.md) (decision 1: where an experiment lives and what `sample` in a used file means), [ADR 0066](0066-models-nest-as-blocks.md) (used models that sweep at each delivery), [ADR 0027](0027-explicit-experiments.md) (the experiment as a section of the model file)
- **Extends:** [ADR 0069](0069-repetition-in-blocks.md) (network grids), [ADR 0034](0034-agent-domain.md) (tables, population ports, the rule language of Q26)

## Context

On 2026-11-03 the owner asked whether experiments should be separate files. A thin model that uses the model under study already acts as one, since the opened file is the experiment (ADR 0067). Wrapping `resonance.ssbx.json`, which has its own `sample`, showed the limit of that rule: the used model became a sweep, the wrapper's experiment was dropped without a warning, and the run failed. The same `sample` section meant "experiment" in the opened file and "this model is a sweep" in a used one.

The owner then asked four wider questions:
- Are blocks general enough to be arbitrary machines?
- Should experiments be blocks or models rather than a section of the file?
- Should agents be models that other models spawn and remove?
- How do structures without a fixed number of members fit: adaptive mesh refinement, unstructured meshes connected by explicit edges, studies whose points adapt?

The owner asked for the fewest concepts that can express everything, and for the README's description to be checked against the code.

## Options considered

| Experiments | Pros | Cons |
| --- | --- | --- |
| **A study block: a block whose model and sample are its definition, inputs from the model's input blocks, outputs as arrays indexed by point** | One concept for experiments in a file, in a separate file, inline, and nested to any depth; a use always means the plain model; studies feed links | A study's outputs are arrays, so reading them needs array functions |
| `sample` on a use (absent: the file's own; `false`: plain; an object: inline) | Small change | Keeps two meanings of `sample` and adds a third place; nested studies still do not feed links |
| A separate experiment file format | Model and study apart | A second schema, loader and hash; uses already give separate files for nothing |

| Agents | Pros | Cons |
| --- | --- | --- |
| **An agent type is a model definition, run as rows of a table** | Users write agents like any model; one interface for arrays, agents and studies; the kernel keeps tables | Needs the rule language of Q26 |
| Each agent a model spawned and removed at run time | One concept | Per-instance overhead rules out 10⁴–10⁶ agents; the graph changes during a run, which breaks unit, loop and fusion checks, snapshots, probes and the model hash; interactions between agents are queries, not wires |

## Decision

**1. Blocks are the machines; a model is a block made of blocks.** The `Block` contract stays the one kind of machine: typed ports, parameters, kernel-owned state and tables, `init`, `do_step`, `compute_outputs`, optional capability traits. A model used in another is a block of its user (ADR 0066). The contract's two known gaps stay open until a model needs them: events (a block's next event time) and values that are not numbers (categories, labels, tokens).

**2. A study is a block** (type `study`):
- **Its definition:** the model studied, `settings.model` (a definition in scope, ADR 0068) or `settings.file` (a model file, found from this file's folder and part of the run hash like any data file), and a `sample` section with the designs, `keep`, `seeds`, `replicates` and `reduce` of ADR 0067.
- **Inputs:** the studied model's own input blocks (`input.value`), one scalar input each. Every point of a delivery takes their values.
- **Outputs:** arrays with one value per run (point times replicates): one per swept parameter, and one per result. Results are the `reduce` formulas, or without them the final value of each scalar output of the model.
- **Running:** it has no state. Each time its inputs change, every point runs from the model's start to its `t_end` (one evaluation for a model with nothing in time). Results depend only on the inputs, so they are kept until the inputs change, and rewind and run files need nothing new.
- **Cores:** only the outermost study runs its points on several cores; a study inside another runs on its caller's thread. Results never depend on the count.
- **Labels:** those of the studied model.

**3. A use always runs its model as a plain model.** To study a model, put a study block around it. A study inside the model of another study gives studies of studies: the outer study's points reach the inner one through ordinary wires and input blocks, and a formula in between reduces the inner arrays.
- The top-level `sample` section of a model file is read as a definition of that model plus a study block over it, so `sample` has one meaning wherever it is (order of work, step 2).
- ADR 0066's used models that sweep at each delivery and cannot feed links are replaced by study blocks.

**4. Models over sets of members.** Arrays, agent populations and studies are one model applied over a set of members. They differ in three independent choices:

| | Members | How members interact | Time |
| --- | --- | --- | --- |
| Arrays and grids (ADR 0060, 0069) | fixed, by index | implicit: stencils, `prev`/`next` | in step together |
| Agent populations (ADR 0034) | rows that change, stable ids | queries: neighbours within a radius | in step together |
| Unstructured meshes | rows, fixed or changing | an explicit edge table | in step together |
| Adaptive mesh refinement | rows that change | edges that change with them | finer levels take smaller steps inside the method |
| Studies | points, fixed or added in rounds | none | each point runs on its own |

The rules that follow:
- **Members are rows with stable ids** in kernel-owned tables (ADR 0034). Fixed arrays are the special case whose ids are their indices; they keep their own fast code. From outside, all of them are arrays indexed by member, collapsed by array functions (`sum`, `mean`, `minof`, `maxof`, `at`); reductions collapse time.
- **Edges are a table too:** rows with a from-id, a to-id and attributes (area, normal, length, conductance). The expression language gains operations along edges: the value at the other end, and sums over a member's edges. ADR 0069's network grids are built this way, with an edge table that may change.
- **Structure changes only between steps.** A rule decides what changes (refine or coarsen, births and deaths, new points), and a **transfer** maps old rows to new ones: conservative restriction and prolongation, a parent's values for a birth. A transfer is a method with speed and accuracy labels.
- **Ports of variable length carry ids and a structure version** that changes whenever members or edges do, so probes and views follow members across changes and a block downstream joins by id or caches work until the structure changes. Today's `Shape::Population` ports carry values only.
- **Different structures meet only through explicit mapping blocks** (interpolation or projection, labelled; conservative or not), as the block contract already plans for field coupling.
- **Different step sizes inside one structure** (subcycling, local time stepping) belong to the method, with its labels; the kernel sees communication steps.
- **Adaptive studies run in rounds:** run a batch, let a strategy (a labelled method, built in or Python) add or retire points, repeat until a budget or tolerance. Seeds come from point ids; the strategy sees each round complete and in id order. Their own ADR fixes the strategies (ADR 0067 step 5).
- Large production AMR stays a wrapped external solver (vision non-goals); SimSandbox couples to it through mapping blocks.

**5. Agents.** An agent type will be a model definition whose states and parameters become a table's columns, with rules for birth and death and neighbourhood functions, run as a table. This is the rule language of Q26, without a new kind of thing. Agents that are whole models of several domains or clocks would be `count` on uses with a count that changes (ADR 0069 point 4), which waits for a model that needs it.

**6. Keeping SimSandbox general.**
- **A concept list:** the user-visible concepts are listed in one place, the [architecture overview](../architecture/overview.md#concepts). Every ADR states the concepts it adds or removes ([template](0000-template.md)); a new one says why the existing ones cannot express it.
- **Built-ins go through the public path:** the study block, input blocks and conversions are registered types like any domain's. A special case in the loader is a sign that something should be a block.
- **The kernel holds no methods:** it connects, schedules and steps blocks, owns state, units and randomness. Algebraic-loop solving (`loops.rs`) and the sample designs (`sampling.rs`) move behind the registry, with labels, when they are next changed.
- **One expression language** for equations, formulas, filters, reductions and study outputs; `reduce` formulas become functions of it.
- **Computing and observing stay apart:** observers (filters, probes, views) read results and stay out of the model hash (ADR 0012).

**7. The README's description** says what the code does: the kernel connects, schedules and steps blocks; domains supply block types through a registry (plugins later, Q28); a model is a graph of blocks and is itself a block.

**Order of work:**
1. The study block, headless: `settings.model` or `settings.file`, `sample`, inputs from the model's input blocks, arrays by run, kept until inputs change, the outermost study on several cores (2026-11-03).
2. A top-level `sample` read as a definition plus a study; `run` writes a study's arrays as rows; used models that sweep removed.
3. Study outputs read by array formulas, with `argmaxof` and `argminof`; reductions as functions of the expression language.
4. Point ids, seeds mixed from ids (equal to today's for fixed designs).
5. The app: study blocks on the canvas and a view of their results.

Later, when a model needs them: ids and structure versions on variable ports; edge tables and operations along edges; transfers; mapping blocks; loop solving and sample designs moved out of the kernel; adaptive strategies (own ADR).

## Consequences

- One concept covers experiments in a file, in a separate file, inline and nested; `sample` keeps one meaning. A thin file using a model remains the way to keep studies apart from the model.
- **Concepts removed:** the two meanings of `sample` (an experiment in the opened file, a sweep in a used one) and the used model that sweeps at each delivery. **Added:** the study block type, and member families as a description shared by arrays, agents and studies (no new file section).
- A study's results are arrays; formulas that reduce them need array inputs indexed by point (step 3).
- A study that is re-run at every step is costly; it is kept until its inputs change, and `check` reports its runs.
- Updated: [architecture overview](../architecture/overview.md) (concept list), [block contract](../architecture/block-contract.md), the ADR template, the README and the [headless guide](../guides/headless.md).

## As built

**Step 1 (2026-11-03): the study block, headless** (`crates/simsandbox-session/src/study.rs`, registered with the built-in types).
- **Definition:** the loader gives the block its `sample` as JSON and, for `settings.model`, the definition's text with the definitions in scope but itself (`BlockEntry::study_definition`, `compose::resolve_models`); for `settings.file` the block reads the file (as edited, if open) and the registry hashes it like a data file. A `sample` on any other block is refused.
- **Running:** `run::run_study`, which `run_experiment` now also uses: every row sets the input values, then the point; a model with nothing in time (no states or tables) is evaluated once per point. Points run on threads only outside a study (`IN_STUDY`).
- **Loops:** studies nest to any depth. Building a study loads its model, which builds that model's studies, on one thread; the same model text from the same folder twice in that chain is a loop and is refused, naming the chain (`a.ssbx.json → b.ssbx.json → a.ssbx.json`). A repeat always loops, since the same text builds the same studies, and every loop repeats a file. Definitions cannot loop: a study of one sees every definition in scope but itself.
- **Ports:** inputs from the model's input blocks not swept; outputs `Vector(runs)`, named by key with `.` and `/` as `_`, every output depending on every input. Labels: the model's accuracy, at least Responsive.
- **The default sweep:** a model holding a study counts as having states, so it runs in time rather than being swept over its first parameter (the rule ADR 0070 step 2 removes).
- **Tests:** a study of a definition and of a file gives the experiment's values bit for bit, once per change of inputs; a study re-runs when a scheduled input changes (halving times ln(2·x0)/k); a study of a study (ln 4/k at three rates); refusals; files studying themselves or each other refused; ten definitions deep, each studying the one before; the file in the run hash; Python.

**Step 2 (2026-11-03): one meaning of `sample`.**
- **Uses run as written:** a used file's `sample` is dropped as the composition is loaded, with a warning (`r: its model's sample is not run …`). The runner that swept used models at each delivery, their `swept` report and the errors about their links and clocks are removed.
- **A file's `sample` is the study of its model**, read as such rather than rewritten: the file keeps its `sample` (the app's live sweep and its views of swept results are the study's view until step 5), and every headless run of it goes through `run::run_study`, sweeps included. Tests: the resonance's swept rows equal a study block's over it as a definition, bit for bit; the app's sweep of every swept example equals the headless rows bit for bit; a wrapper with its own `sample` around the resonance (which has one) runs the wrapper's, with the warning.
- **Rows:** a model whose only runs are its studies (no states or tables, no clocks, no scheduled input, no `--t-end`/`--dt`) is evaluated once, and `run` writes a row per run, indexed by `run`, a column per study output; studies of different sizes are refused; other outputs become notes.
- **`check`** reports a sweep under `experiment` too, with `each_run`: "in time" or "one evaluation".
- **Found on the way:** the stateless test behind the default sweep looked blocks up by name and panicked on a use; it now looks at the blocks a use was merged into.

**Step 3 (2026-11-04): study arrays in formulas; reductions in the expression language.**
- **Comparisons** `<`, `<=`, `>`, `>=` are operators of the expression language everywhere (typed equations, formulas, filters, reductions): 1 or 0, NaN never holding; below sums in precedence, one per pair of parentheses; both sides in one unit, the result dimensionless. Compiled programs run them for scalars and arrays (`Ins::Cmp`).
- **Index axes:** `grid k = 5` in typed equations declares cells numbered 0 to 4, no unit (spacing 1).
- **Array functions:** `argmaxof(T)`, `argminof(T)`: the cell (storage order) of the greatest or least value, the first of equals, NaN if all are NaN; dimensionless. `at(T, i)` with a computed `i` reads that cell (rounded; NaN outside), one value and one cell number in storage order on any grid.
- **Study outputs are lines of cells** (`Grid { nx: runs, ny: 1 }`), so array inputs of the same number of cells read them; a different number is refused at the wire. A grid design over one parameter with one replicate declares that parameter's output the coordinate of every result (ADR 0024), for profile views later.
- **Reductions are calls in expressions** (`simsandbox_expr::parse_reductions`): a `reduce` result is any expression of reductions, `max(car.v) - final(car.v)`; signals only inside them; reductions do not nest; `min`/`max` with two arguments stay functions. `first_time(e)` is the first time `e` is not 0, so the comparison is the language's own. The reducer's own small parser is gone.
- **Tests:** comparisons (values, precedence, NaN, chains refused, programs, units); index axes, argmax/argmin, computed cells and refusals; the resonance's peak frequency from a study by `at(w, argmaxof(amp))`, a wrong cell count refused, the coordinate declared; composite reductions against their parts; reductions of a sine.

**Step 4 (2026-11-04): point ids and seeds by id.**
- **Ids:** `Plan::ids` holds the ids a points file gives in a column named `id` (whole numbers from 0 below 2^53, each once); otherwise a point's id is its place (`Plan::id`). Rows show given ids in an `id` index column, and a study gives them as its `id` output. Fixed designs (grid, list, random, Latin hypercube, Sobol) number their points by place, so nothing about them changes.
- **Seeds:** `"per point"` mixes in id × replicates + replicate instead of the row. With ids from places this is the row, so every result from before is unchanged; with ids from a file, a point draws the same however the file is ordered or cut. Common seeds and replicates are as before. Adaptive designs will give each new point the next id.
- **The points file is part of the run hash** (`Loaded::data["sample.file"]`), closing a gap noted in ADR 0067 step 2.
- **Found on the way:** a study of a definition loaded without a folder ran its model with no folder at all, so relative files (a points file) were not found; it now keeps the thread's folder.
- **Tests:** three equal grazing points with ids give the same counts in either order of the file and differ from each other; ids refused when repeated, fractional or negative; the hash changes with the points file; a study's `id` output; the earlier seed tests unchanged.

**Step 5 (2026-11-04): studies in the app; their data in the run hash.**
- **Diagram:** loader and diagram build blocks from one `BlockEntry::config`, so a study node builds (it could not before: the diagram gave it no definition) and shows its ports; its subtitle names what it studies and its runs (`of resonance · 40 runs`).
- **"Study this model"** on a used model's node (`Edit::Study`; Python `Model.study`): a study block named `USE_study` beside it, over the same definition or file, with a grid of 20 points over the model's first declared parameter (its range, else half to twice its value) or else the first block parameter that is not an initial condition (its suggested range); its inputs are wired from what feeds the use's input blocks. Edits that can change what a study names resolve definitions again.
- **Inspector:** for a study, the model or file it studies and its `sample` as JSON (`Edit::SetSample`), applied when the field loses focus; bad JSON is refused with why. Renaming a definition renames it in studies of it.
- **Views:** a model holding a study opens on a profile of the study's results (`plot.profile` first among the presets), against the swept parameter where the study declares it a coordinate.
- **Run hash:** a study reports its model's data digests, its points file included, to the load building it (`simsandbox_file::report_data`); they join the outer model's data as `study:key` (kept by merged loading), so the hashes of models without studies are unchanged.
- **Checked:** tests of the edits (a Platoon follower studied over `ki`, its input fed from the leader, its sample set to masses, a rename followed), of the default view and of the hash; screenshots of the app under Xvfb with a study model: the profile of the resonance and the diagram. The Inspector's sample editor appears only for a node selected on the canvas, which the screenshots do not show.

**After step 5 (2026-11-04, owner): a study names its model as a use does, and opens like one.**
- A study's model is `"model"` (a definition in scope) or `"file"` on its entry, as a use's is, no longer `settings.model`/`settings.file`. A use is an entry with no type that names a model (`BlockEntry::is_use`); an entry naming a model, use or study, is `BlockEntry::names_model`; any other typed entry naming a model is refused at its `type`. The loader resolves a study's definition with the uses' (`resolve_models`), so a study counts among a definition's uses and follows renames; its file is read and hashed as before.
- **Open:** "Open" on a study's node (or opening it) goes inside the model it studies, with the edit question of a used model (edit the definition or file, copy it locally or to a new file); paths into models step through studies (`edit_use`, the app's `used_file`). Edits there change what the study runs. Expand in place is for uses only.
- Tests: the tuning example's study opens to the cruise model; an edit there changes the study's results; a local copy points the study at a new definition; an `input.value` naming a file is refused at its type. `render --diagram --inside tuning` draws the cruise model.

