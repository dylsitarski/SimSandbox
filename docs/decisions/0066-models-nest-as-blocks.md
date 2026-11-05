# 0066 — Models nest: a model's blocks may be uses of model files, each sharing its parent's clock or running on its own

- **Status:** Accepted
- **Date:** 2026-11-02
- **Supersedes:** [ADR 0065](0065-several-models-with-own-clocks.md) (its file format, its recorder rule and its order of work; its clock, link and evaluation rules carry over as stated below)

## Context

ADR 0065 step 1 (built 2026-11-02) added compositions: a kind of file with `models` and no blocks of its own, using model files on their own clocks, linked by probes. The owner asked why compositions are defined apart from models, and whether defining them alike, so models nest, would be practical (Q45). Two further gaps showed in clocks whose steps barely overlap. Tick counts were 64-bit: with steps of 0.1 s and 0.3333333333333333 s the tick is 10⁻¹⁶ s, so the counter overflows after about 1,800 simulated seconds. And each model took `round(t_end / dt)` steps, which overshoots (`dt` 0.4 to `t_end` 1.0 ends at 1.2). ADR 0065's recorder also snapshotted only at instants every clock shares: about every 333 s for those two clocks.

The owner agreed (2026-11-02) to: one recursive kind of model; a used model file running on its own clock by default, sharing its parent's clock one setting away; links into any depth.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| **One kind of model: a model's `blocks` hold built-in blocks, subsystem uses and uses of model files** | Any model can be used in any other, to any depth. Model files used as subsystems (on the parent's clock) and as models on their own clocks are the same thing with one setting. No special kinds of file. | The loader becomes recursive; a file may mix blocks and uses |
| Compositions as a separate kind of file (ADR 0065 step 1) | Simple loader | No nesting; a composition cannot itself be reused; two kinds of file to learn |

| Rewind with clocks that rarely meet | Pros | Cons |
| --- | --- | --- |
| **Each clock snapshots at its own steps; links record what they delivered; rewind restores each to its own latest snapshot and replays, taking recorded link values until the producer catches up** | Fine rewind whatever the clocks; exact, since a run is deterministic | Link values are recorded too (scalars at sample times, small) |
| Snapshots only where every clock has a step (ADR 0065) | Simple | Unusable when the clocks share instants rarely |

## Decision

**1. One kind of model, nesting to any depth.** An entry in a model's `blocks` is one of three things:
- a built-in block (`"type": "eq.equations"`, …);
- a use of a subsystem defined in the file (`"type"` naming it, ADR 0043);
- a **use of a model file**: `{"name": "plant", "file": "car.ssbx.json", "params": {"car.m": 1200}, "clock": {"dt": 0.05}}`. The `file` is found from the using file's folder, or is a full path. `params` set values for this use only: a block parameter `block.param` or a declared parameter of the used file. A used file may itself use files, to any depth. A file that uses itself, directly or through others, is an error naming the chain.

The file opened is the top level. Its own blocks keep plain names (`car.v`), so every existing file is unchanged. Everything inside a use is prefixed with the path of uses leading to it: `plant/car.v`, `vehicle/plant/car.v`, as subsystem uses already are.

**2. Clocks.** A use of a model file has `clock`:
- absent: **its own clock** at the used file's `run.dt` (the default);
- `{"dt": 0.5}`: its own clock at that step;
- `"shared"`: **its parent's clock**. Its blocks are flattened into the parent's model as a subsystem use's are, so direct wires reach its input blocks and fusion works across the boundary.

Of a used file, only `run.dt` is read; the top level's `run` (`t_end`, `seed`, `inputs`) governs. A clock domain is a model on its own clock together with every shared-clock use flattened into it. Each domain is one `Simulator`. A model with no blocks of its own on its own clock (only uses) is no domain. Models without states run "on new data" (ADR 0065 point 4) from a later step.

**3. Inputs are input blocks, at every level.** A model's inputs are its `input.value` blocks, which have no input port of their own. Run alone, its slider, schedule or recorded input drives one. In a use on its own clock, a probe link drives it. In a shared-clock use, a parent's wire into `plant/force` drives it in place of its value. Outputs are any block output, by path.

**4. Links** go only through probes between domains (ADR 0065 point 2). A connection in any model whose ends lie in different domains must come from a probe of that model and end at an input block: `{"from": "probe:speed.plant/car.v", "to": "controller/speed"}`. Paths reach any depth (`vehicle/plant/car.v`). A connection within one domain is a wire, or a probe feeding its own model (ADR 0063 step 3). A probe records from one domain. **When a model is used, the probes that feed its links or its own blocks are kept**, since they are part of the model (ADR 0063). Its other controls (views, recorder, other probes) are not used.

**5. Clocks interleave in exact ticks** (ADR 0065 point 3):
- Each `dt` is read as the decimal it is written as, and all domains count a common tick. **Tick counts are 128-bit**, which covers more than 10⁴ years at the smallest tick.
- Between its steps a domain advances with its link inputs held. At an instant shared by several domains, they evaluate in link order: feeding domains first, loops broken in written order (depth-first, the top level first). In a loop, the domain that evaluates first takes the other's sample right after its own outputs, so only an output that depends on it at that same instant is late, and that is reported.
- A domain runs to **its last step at or before `t_end`**, counted exactly in ticks.
- Randomness: the top level's domain uses `run.seed` as before. A domain inside uses a seed mixed from it and the use's path, so two uses of one file draw differently.

**6. Rewind with clocks that rarely meet.** The recorder (one, at the top level, ADR 0064) snapshots each domain at its own steps on its own budget. Each link keeps what it delivered, as (tick, value) pairs, back to the oldest snapshot. Rewinding to t restores each domain to its latest snapshot at or before t. Replay goes forward in tick order, with each domain held until the replay reaches its restored tick. Until then, a link from a domain restored later than its reader delivers its recorded values. Replay gives the same bits as the run, since the run is deterministic. ADR 0065's common-instant snapshots and its `check` warning are dropped.

**7. Hash, check, headless, canvas.**
- **Hash:** a run's model hash covers the top-level file, each used file's contents and the data files each reads, keyed by use path.
- **`check`:** lists the domains (path, file, `dt`, ticks per step, outputs, input blocks), links, tick, order and loops.
- **`run`:** gives each domain's results at its own times. The top level's own domain is named `.`, so `--model .` selects it.
- **Python:** `Model.add_model(name, file, params=, dt=, shared=)` adds a use, and `Model.link`.
- **Canvas (later step):** uses are nodes, as subsystem uses are, with a clock badge ("every 0.05 s", "shared", "on new data"). Double-click opens them. ADR 0065's edit question applies at every level, and a "local copy" becomes an inline subsystem with that use's clock.

**Order of work:**
1. Rework ADR 0065 step 1 to this form: uses in `blocks` with their own clocks, nesting to any depth, links at any depth, top-level blocks beside uses, 128-bit ticks, exact end counts, seeds per use, the recursive hash. Tests:
   - the cruise control split into two files gives the same bits as one file with a hold;
   - clocks of 0.1 s and 0.3 s match values by hand;
   - a nested use gives the same bits as the same models used side by side;
   - a run long enough to overflow 64-bit ticks;
   - the end-time rule.
2. Shared-clock uses (`"clock": "shared"`): a used file flattened as a subsystem, its input blocks driven by wires.
   - **2b (owner, 2026-11-02): any model file can be used on a shared clock, and wires and parameter values reach any depth.**
     - **What a file used shared may contain:** models on their own clocks, probes feeding its own blocks, and `groups`. These move up into the using model under the use's path: its models on their own clocks become clock domains at `plant/engine`, its probes and links are renamed into the using model, and its groups' blocks are prefixed.
     - **Wires at any depth:** a wire may reach an input block or an output of any model on a shared clock below it (`vehicle/plant/force`, `vehicle/plant/car.v`). The levels in between pass it on through ports of their own. Pass-through blocks are no longer needed. A wire into a model on its own clock is still refused; it takes a probe link.
     - **Parameter values at any depth:** a use's `params` keys may be paths: `block.param`, a declared parameter `k`, `plant/car.m` or `plant/k`, at any depth. A value may be a number, or a formula of the declared parameters of the file it is written in. On a shared clock a formula is live: it follows those parameters when they change, as subsystem formulas do. A model on its own clock is a separate simulator, so there a formula is evaluated once, when the model is built, from the using file's declared values; values that change during a run go through a probe link into an input block.
     - **Kept as a rule:** wires go to input blocks and from outputs. Ordinary subsystems are still reached through their ports.
3. Models without states on new data.
4. The canvas and the app (ADR 0065 point 6 and the edit question), in three parts:
   - **4a:** uses as nodes on the diagram, with their input blocks as inputs, their outputs by path, and their clocks; wires, links and probe records drawn; headless `render --diagram`.
   - **4b:** the app's session runs models that use files: a multi-clock engine taken out of the headless runner (same results), behind the session, so such files open and run with views, sliders and inputs by full name, and uses open to show their insides.
     - **The interface of a used model (owner, 2026-11-02, Q46):**
       - **Declared outputs:** a model file may declare the outputs it offers, `"outputs": [{"name": "speed", "from": "car.v"}]`, as a subsystem does. `from` is one of its blocks' outputs, or a path into a model it uses. Users wire and link to `plant/speed`; the names stay valid when the file's insides change.
       - **What its node shows:** its input blocks as inputs, its declared outputs, and whatever is wired, even deep paths. Input blocks' own `y` is not listed. Inputs of models nested inside it show only when wired.
       - **The rest:** every output and input stays reachable by path from a searchable list on the node ("＋"), grouped by nested model.
       - **Parameters:** the node lists the values set for that use. The Inspector shows every parameter as a tree by path (`vehicle ▸ plant ▸ car.m`), with search.
       - **Headless:** `check` lists a model's declared outputs apart from the full list by path.
   - **4c:** editing: a probe added when wiring across clocks, the edit question, saving several files, "Add model…".
     - **The edit question (owner, 2026-11-02):** the first edit inside a used model asks, once per use until the model is closed:
       - **Edit the original:** the edit goes to the used file, which every model using it sees. Save writes both files; undo covers both.
       - **Edit as a new copy with its own file:** the used file's contents are written to a new file next to the using file (named after the original, e.g. `car-2.ssbx.json`, changeable), this use's `file` points to it, then the edit applies. Other uses of the original, here or elsewhere, are untouched. The copy is an ordinary model file, reusable by other models.
       - **Make a local copy:** the contents are written inline as a subsystem of this file, with the use's clock and names unchanged, detached from any file.
     - **A copy deep inside another used model (owner, 2026-11-02, Q48):** for `vehicle/plant`, where `vehicle.ssbx.json` uses `car.ssbx.json`, the file being edited replaces the deep use's file by path, as `params` reach any depth: `{"name": "vehicle", "file": "vehicle.ssbx.json", "files": {"plant": "car-2.ssbx.json"}}`. `vehicle.ssbx.json` is left alone, so only this use sees the copy.
       - Keys are paths of uses below this one (`plant`, `plant/engine`); a path that is not a use of a model file, or a file that cannot be read, is an error at `blocks[i].files`.
       - A replacement holds for everything under it: a deeper replacement in the replaced file still applies, and the nearer one wins when two name the same path.
       - The question also offers the other two ways: edit the file in between to point at the copy (it reaches every model using that file, and says so), or copy every file on the way down (`vehicle-2` using `car-2`).
       - `check` lists the replacements; the hash already covers each used file by path. Python: `add_model(..., files={"plant": "car-2.ssbx.json"})`.
   - **4d (owner, 2026-11-02, Q47): expanding in place.** "Expand in place" on a use or subsystem node draws an outline at its place showing its inner diagram one level down; nested uses and subsystems inside it stay nodes until expanded in turn, one level at a time.
     - **The outline:** labelled with the file (or subsystem) and clock, coloured by clock, so models apart and models sharing a clock show at a glance. Inner nodes keep their own file's positions; neighbours are pushed aside on screen only, never written to the file.
     - **Wires:** a wire to a port of the expanded node is drawn to the inner node it reaches (an input block, or the output a declared output stands for). Live values inside show as when opened.
     - **Editing inside:** the same rules as when opened: a subsystem's edits reach every use of its definition, and the outline says how many; a used model's edits ask the edit question.
     - **Kept:** expanded nodes are listed in `controls` (as `controls.open` lists open nodes, ADR 0055), by path.
     - **Headless:** `render --diagram --expand NAME` (repeatable, by path); Python `render(diagram=True, expand=[...])`.
5. The recorder and rewind above; run files.
6. Domains stepping on several cores between instants (ADR 0062 step 3).

## As built

**Step 1 (2026-11-02): ADR 0065 step 1 reworked to this form.**

- **File:** `BlockEntry` gains `file` and `clock` (`UseClock::Step(dt)` or `Shared`, written `{"dt": …}` or `"shared"`), and `type` may be absent for a use. `models` and `ModelUse` are gone. `compose::check_uses` checks each entry when the file is read:
  - an entry has a type or a file;
  - a use carries only `name`, `file`, `params`, `clock` and `canvas`, and its name is letters, digits, `_` and `-`;
  - `clock` appears only on a use, and its `dt` is positive;
  - uses are not allowed inside subsystem definitions.

  `load` and `load_in` refuse a model that uses files, saying it runs headless for now. The schema lists `file` and `clock`, and `type` is no longer required.
- **Loader:** `simsandbox_file::compose::load_composition` walks the tree depth first. For each model it:
  - builds its own blocks, and the connections among them with the probes feeding them, as one domain, from that file's folder. Errors there are mapped back to their place in the file (`written_at`).
  - loads each use from the using file's folder, applying `params`, catching a file that uses itself (the chain is named) and hashing the file's bytes (`PATH.file`).
  - checks its links, the connections reaching into a use. Names resolve through the levels (`Level::resolve`) to a domain and a name inside it.

  It returns the domains (`path`, `name()`, `.` for the top level; `full()`), the probes feeding links (with full names such as `vehicle/speed`), the links, the ticks, the order, the loops, the digests and `domain_of` for full names. Inner domains get `seed_for(top seed, path)`. A model with no states is refused until that step.
- **Clock:** `Ticks::steps` is `Vec<u128>`. `Ticks::steps_within(k, t)` counts exactly in ticks the steps of clock k that fit in t.
- **Runner:** `simsandbox_session::compose` runs over domains: u128 counters, `steps_within` for each domain's last step, and scheduled input routed by `domain_of`. Results are named by domain, with `Composed::values(full name)`.
- **`check`:** reports each domain's `name`, `file`, `dt`, `ticks_per_step` (a string, since it is 128-bit), `outputs` and `inputs`, the links with full names, the tick, the order, the loops and `model_hash`.
- **CLI and Python:** `run --model NAME` (`.` for the top level). Python `Model.add_model(name, file, params=, dt=)` adds a use to `blocks`. `Composed` finds any full name and is named by domain. `shared=` waits for step 2.
- **Tests:**
  - the split cruise control against the one-file model with a hold, bit for bit;
  - a vehicle file using both, used by a top level with blocks of its own, against the same uses side by side, bit for bit, with the top level reading `vehicle/plant/car.v` every 0.1 s;
  - clocks of 0.1 s and 0.3 s, each reading the other, against values by hand;
  - 3,000 s at a 10⁻¹⁶ s tick (30,001 and 9,001 steps), where b's step at "1" takes a's sample at 0.9;
  - the end rule (`dt` 0.4 to 1.0 ends at 0.8);
  - different seeds for two uses of one file, and different again for another top-level seed;
  - `check`, the hash following files, parameter values and clocks, every refusal, a file using itself through another, and Python, including a file of uses used by another.

**Step 2 (2026-11-02): uses on a shared clock.** `Loader::inline_shared`, run at the start of every model's level, puts each `"clock": "shared"` use in place as a subsystem of the using file, made from the used file, so the existing flattening does the rest. The used file's uses on a shared clock are put in place first, recursively.

- **Contents:** its blocks, connections and declared parameters become the definition, named after the use (made unique). Its own subsystems join the using file's under `USE_NAME` names, with references renamed. Data-file settings (`TypeInfo::file`) are made absolute from the used file's folder. Its `run`, `sample`, `fusion` and `controls` are not used; the using model's fusion setting applies.
- **Wires in:** a wire into `use/input` must reach an `input.value` block of the used file. It becomes an input port of that name feeding what the block fed, and the block is dropped.
- **Wires out:** a wire from `use/block.port` (one of its own blocks) becomes a generated output port. Deeper models are wired in their own files.
- **Probes and links:** connections from probes are left as they are. They name flat signals (`plant/car.v`), and a link from another clock drives `plant/force`, which stays an input block when no wire drives it.
- **Parameter values:** a use's `params` are written into the used file as for other uses.
- **Refused for now:** a used file on a shared clock may not itself use models on their own clocks, have probes feeding its own blocks, or have `groups`.
- **Python:** `Model.add_model(…, shared=True)`.
- **Tests:**
  - the car and controller files used shared, wired directly, give the same bits as the cruise control in one file, fused into one group (`plant/car`, `controller/cruise`) with the input blocks gone;
  - the same one level deeper through a vehicle file;
  - a shared plant beside a controller on its own clock gives the same bits as both on their own clocks;
  - a shared use in `parts/` reads `parts/push.csv`;
  - refusals: a wire into a non-input, an input wired twice, a used file with uses on their own clocks, a deep wire, a file using itself;
  - Python.

**Step 2b (2026-11-02): any file used on a shared clock; wires and values at any depth.**

`inline_shared` returns the uses it put in place as a tree (`Inlined`) and the uses on their own clocks it moved up (`OwnClock`).

- **Uses on their own clocks inside a shared use:** they load at the using model's level under their paths (`vehicle/plant`). `Level::resolve` and `in_use` take multi-segment use names, the longest that fits.
- **Probes:** connections from a shared use's probes, and those reaching its own-clock models, move up, with the probes renamed `USE_PROBE` and their records prefixed. A hold into its own blocks goes through a generated input port (`hold_N`). A probe connection touching one of its own-clock models stays a link, written by path.
- **Groups:** a shared use's `groups` move up with their blocks prefixed.
- **Wires at any depth:** `expose_input` and `expose_output` pass a wire through every level in between with generated ports. A wire into an input block drops the block. A wire reaching a model on its own clock is refused with "takes a probe link".
- **Values on a shared use:** they go to its subsystem instance. Subsystem flattening now takes formulas for inner `block.param` keys at any depth, compiled in the instance's scope and live, and `use/k` keys setting a parameter declared by an instance inside, as a number or a live formula, found by its flat key.
- **Values for a use on its own clock** (`set_params` with the scope of the file the value is written in):
  - a declared parameter of the used file takes a formula's value once (`subsystems::evaluate_formula`, units fitted);
  - for `block.param` and paths into uses inside, the declared parameters a formula names are copied into the used file (`carry`; a clash of names is refused), so the formula keeps its meaning one level down;
  - values reaching a model on its own clock inside a shared use go to that model's moved-up entry.
- **Use names:** letters, digits and `_`, as for blocks, since a shared use becomes a subsystem instance.
- **Python:** `add_model` passes formula values through.
- **Tests:**
  - a file of models on their own clocks used shared gives the same bits as the same uses at the top;
  - a file with a hold every 1 s used shared gives the same bits as the file alone, and likewise a file with `groups` (its group named `g/car`, `g/cruise`);
  - wires two levels down, with the car's mass set by `plant/car.m` as a number and as a live formula of the top level's `mass`, give the same bits as the one-file model with that mass;
  - into a model on its own clock, `car.m = "2*half"`, a declared `m = "2*half"`, and the path `plant/car.m = "2*half"` through a file of uses each give the same bits as `car.m = 1500`;
  - refusals: a formula naming an undeclared parameter, a wire into a model on its own clock inside a shared use, a wire from a shared use with no such output;
  - Python, with a deep formula.

**Step 3 (2026-11-02): models without states on new data.** A domain whose model has no states (`Loaded::plan`) and whose use gives no clock runs on new data (`Domain::on_data`); so does the top level's own blocks without states.

- **Ticks:** they count only clocked domains (`Composition::clocks` maps each domain to its clock, or none). With no clocked domain at all the model is refused, since nothing would deliver.
- **When it is evaluated:** at the start, and at every instant where a link into it delivered a sample during the pass. Domains are taken in link order, so its feeders go first. It is evaluated at the time of the domain that delivered (`Simulator::restart(t)` at that time, so `t` in its formulas is right). Its probes then sample, counting evaluations as steps.
- **Loops:** in a loop it is evaluated again when a later domain delivers in the same instant, and its record for that instant is replaced.
- **Scheduled input** for it applies at its evaluations.
- **With a clock:** a model without states whose use gives `clock: {"dt": …}` is evaluated at each step instead.
- **Swept models:** a model with a `sample` section runs its whole sweep at each delivery (`Running::evaluate`). Its results are the latest sweep, indexed by `sample` and the swept parameters by full name, with a note giving when it ran. It cannot feed links, and its use may not have a clock.
- **Difference from ADR 0065 point 4:** until a link's first sample, the input block keeps its own value rather than NaN, as for clocked models since step 1.
- **`check`:** each model gets `on_new_data` and `swept`, and `dt` and `ticks_per_step` are null on new data.
- **Tests:**
  - a doubling model fed every 0.3 s by a counter is evaluated at 0, 0.3, 0.6, 0.9 and 1.2 s with the values by hand; given a 0.2 s clock, it is evaluated every 0.2 s instead;
  - the top level's own blocks without states are evaluated on new data;
  - in a loop, the counter shows the doubled value at the instant it was delivered and holds it, with no delay;
  - a swept model's latest sweep at 1.2 s;
  - refusals: a sweep feeding a link, a sweep with a clock, a model with nothing in time.
- **Changed by [ADR 0067](0067-experiments-and-time-from-data.md) the same day:** a model with nothing in time is no longer refused. It is one evaluation of every model at t = 0, in link order (tested: a pipeline of two doubling models after an input block gives 3 and 6).

**Step 4a (2026-11-02): uses on the diagram.**
- **Nodes:** `NodeKind::Use` nodes (`diagram::use_node`) read the used file from the using file's folder, or the thread's. Their inputs are its `input.value` blocks, with units from their settings. Their outputs are its blocks' outputs (`car.v`), including subsystem uses, and those of the uses inside it by path (`engine/x.y`), to depth 8. A file that cannot be read shows its error on the node.
- **Clocks:** the node's line shows the clock first: "every 0.05 s", "shared clock", or "on new data" for a model without states, then the file.
- **Wires:** wire ends inside a use (`plant/car.v`, `controller/speed`) find the use's port by the rest of the path. On the data layer, `sources` give a use's outputs by full name, so probes record from them, and links from probes into a use's input blocks are drawn solid like other probes that feed the model.
- **Canvas:** a use is drawn with a double outline, like a subsystem use, in a stronger colour. Its menu names its file and clock. Opening it came with 4b.
- **Tested:** nodes, ports and units, clocks (own, set, shared, on new data), probe records and links, direct wires between shared uses with nothing dangling, and a missing file. A headless render of the cruise control from two files was looked at.

**Step 4b, first part (2026-11-02): a used model's interface.**
- **The file:** `ModelFile::outputs` (`OutputPort`: `name`, `from`) is checked when the file is read (`compose::check_outputs`): names plain, unique and apart from input blocks' names, and `from` starting at one of its blocks. When a model is loaded alone, `from` must be a real output (a block's, or a subsystem use's port).
- **Links:** they resolve declared outputs as aliases on each level of the loader (`Level::aliases`, its own and those of shared uses in it, by path), so `plant/speed` links like `plant/car.v`.
- **Shared uses:** a shared use's declared outputs become ports of its subsystem, passed through nested shared uses (`expose_output` knows them by name), so wires reach `plant/speed` and `vehicle/v`.
- **Nodes:** `diagram::use_node` shows a used model's input blocks (never their own `y`) and its declared outputs, with units taken from what they stand for. Every other port, nested ones included, goes to `Node::reachable`. `reveal_wired` moves onto the node whatever connections, probe records, views or filters reach by path.
- **`check`:** reports `declared_outputs`, for a model alone and for each model in a composition.
- **Tests:**
  - linked by declared names, the same bits as by inner names;
  - wired by declared names on a shared clock, the same bits as the model in one file;
  - a vehicle file re-declaring a nested output, read through a probe link;
  - nodes showing only their interface until something wires an inner port;
  - refusals.

**Step 4b, second part (2026-11-02): the app runs models that use files.** Rather than give the session several simulators, the kernel steps several clocks in one model.
- **The kernel:**
  - `Model::set_clocks` takes clocks (`Clock`: `dt` with `every` ticks, or `on_data`) and each block's clock; `Model::set_stream` sets a block's random stream.
  - `Simulator::step` then advances to the next instant of any clock. The clocks reaching it step their blocks (and fused groups) over their own `dt`, at their own times and step counts. Those blocks, and those on new data, resolve in dependency order, each at its own clock's time (`OutputPlan::solve_active`).
  - A block takes its inputs only at its own instants: `exchange` and the loop solver skip the others, so it holds them over its step, as models apart do.
  - Snapshots keep each clock's time and steps and every input. `next_instant` and `clock_steps` report the clocks.
- **The merged model:** `compose::load_merged` puts every use in place, its own clock kept by the kernel.
  - A link is a probe feeding the model; its hold samples on the source's clock.
  - An input block a link drives becomes a pass-through of the same name (`input u`, `y = u`), reached through ports at any depth (`expose_link`).
  - Probe signals named by declared outputs are made flat.
  - Blocks of used models draw from `seed_for(path)` with their names inside it, as alone.
  - Its `Loaded` keeps the file as written for saving, and `aliases` (declared names → flat outputs).
- **The session:** `Session::load_in` loads such a file merged. It records declared outputs under their names too (`fast/x`), from the same outputs. `Session::step_dt` gives the time to the next instant, which the app's worker paces by.
- **The app** opens and runs these files; sliders reach parameters by full name.
- **Difference from the headless runner:** a model on new data is evaluated at every instant of the others in the app, not only at deliveries. Its values agree at deliveries, unless its formulas read `t`. A loop of links that is algebraic is solved (Newton) in the app, rather than broken a step late.
- **Tests:**
  - the merged simulator gives the runner's bits, at each model's own instants, for the split cruise control, two clocks reading each other, a shared plant beside a controller on its own clock, a nested use, and a model on new data at its deliveries;
  - the session gives the runner's bits;
  - rewinding two clocks between their instants replays exactly;
  - the app opens such a file from disk, runs it, and records a declared output by name.

**Step 4b, third part (2026-11-02): looking inside, the port list, the parameter tree.**
- **Opening a use:** "Open" in a use node's menu, or a double-click, shows the used model's diagram as its file is written, with the run's values (its names inside, without the path of uses). A breadcrumb leads back. Uses inside open the same way, to any depth. The file is read from the folder of the file using it (`diagram::used_file` in the app). The view is read-only: an edit says that editing a used model comes with step 4c.
- **The port list:** a use node's menu has "Show a port", a search over `Node::reachable` by path (at most 200 shown). A port chosen stays on the node while the canvas lives (`Canvas::revealed`, applied by `Canvas::reveal`); undo history and reloading forget it.
- **The Inspector:** a group's parameters nest by the path of uses in their names (`inspector::nest`): `plant/car.m` shows as `car.m` under a collapsed `plant`, to any depth. The Inspector's search covers every level. A use's group is labelled "uses FILE".
- **Tests:** rows nesting by path; a port chosen moving from the list onto the node; opening a use reads its file, and an unknown name is an error.

## Consequences

- `models` (ADR 0065 step 1, never released) goes. A used file is any model file, and any model file can use others.
- Subsystems stay for definitions inside a file. Model files used on a shared clock are subsystems kept in their own files.
- The runtime stays flat: one list of domains and one list of links, whatever the nesting, so ordering, replay and multi-core stepping do not depend on depth.
- ADR 0065's suggestions accepted by the owner stand: links through input blocks, parameter values and clocks per use, the three edit choices, and rewinding one use alone later.

**Step 4c, first part (2026-11-02): replacing the files of deep uses.**
- **The file:** `BlockEntry::files` (written `files`), on a use of a model file only, maps paths of uses inside it to files. Keys are names joined by `/`, checked when the file is read; on a block with a `type` it is refused.
- **The loader:** `compose::replace_files` puts a use's replacements into the model it reads, before that model loads: a key naming one of its own uses becomes that use's `file`; a deeper key goes into that use's own `files`, overwriting one of the same path written there (the nearer to the top wins). Paths are made full from the folder of the file they are written in. Shallow keys go first, so `plant` and `plant/engine` together replace both. A key that is not a use of a model file is an error at the use (`blocks[i]`).
- **Everything reading a used file applies them:** the loader for own and shared clocks (so runs, `check`, the hash and the app's merged model), the use node on the diagram, and opening a use in the app.
- **`check`:** `replaced_files` lists the top file's replacements by full path (`vehicle/plant`); each model's `file` is the one read.
- **Python:** `Model.add_model(..., files={"plant": "car-2.ssbx.json"})`.
- **Tests:** a replaced car gives the bits of the original car given the copy's mass by `params`, at one and two levels down and on a shared clock; the nearer replacement wins; the used file is left unchanged; the hash changes; refusals; Python.

**Step 4c, second part (2026-11-02): editing a used model from the model using it.**
- **A local copy is written in the use** (replaced the same day by a definition of the file, [ADR 0068](0068-one-kind-of-nested-model.md)), as `"model": {…}` (the used model file's contents) in place of `"file"`, with the same name, clock, `params` and canvas place. A subsystem cannot hold one: it shares its model's clock and cannot hold models on their own clocks, probes or declared outputs. Its files are found from the folder of the file holding it. `BlockEntry::is_use` covers both kinds of use; `check`, runs, the hash (its contents under the use's path), the diagram ("a local copy") and Python treat them alike.
- **Unsaved edits to used files** are kept by path and read in place of the disk on the thread that loads models (`compose::set_edited`, `read_model`), so the run, `check` and the diagram see them before Save. `compose::used_model` reads a use as the run does (edited files, replacements), for the diagram and the app.
- **The answers** (`edit_use::prepare`, headless and tested):
  - *Edit the original:* edits go to the file the use reads (after replacements), or to the local copy it is.
  - *Edit as a new copy:* the model as read goes to a new file next to the model open (`car-2.ssbx.json` offered, refused if it exists); this use points to it: `file` for a use at the top, a `files` replacement for a deeper one (Q48). Paths inside it are rebased to its folder (`edit_use::rebase`: uses, replacements, `input.file` data files, nested local copies).
  - *Make a local copy:* every model along the path becomes a local copy, each held in the one above it, rebased to the model open.
  - Edits then go where `edit_use::Target` says (a file, then local copies inside it, by name): `edit_use::read`, apply, `edit_use::write`.
- **The app:** the first edit inside a used model (a value, a block, a wire or a move) opens the question, with Cancel. The answer holds for that use until the model is closed or an edit is undone. The breadcrumb says where edits go. Edits apply as at the top level: refused with the reason if the model no longer loads. Undo and redo cover the used files and the model open together. Save writes the model and every edited file, and the log names each. Unsaved edits to used files count as unsaved changes; opening another model drops them, after asking.
- **Tests:** each answer gives the bits of the same change made by `params`, leaves the disk alone until saved, and saves what it should (headless); in the app, each answer applies, the next edit does not ask, undo and redo restore the files, and a saved model reopens with the edit.

**Step 4c, third part (2026-11-02): wiring between used models, and Add model….**
- **Wires drawn to a used model's ports** are written by path (`plant/car.v`, `controller/speed`): the canvas names them `node.port`, which the model level turns into paths (`diagram::use_end`). Disconnecting works the same way.
- **On one clock** (both ends on the model's clock, or in the same use): a wire.
- **Across clocks** (`diagram::connect_uses`): the wire becomes a link. A probe recording the output (one already recording it, or a new `link_…` probe) feeds the input block. The log says so. Into a block that is not an input block it is refused, with the reason. Removing the link removes a `link_…` probe that then feeds nothing.
- **Add model…** in the canvas's menu chooses a model file (`Request::AddModel`) and adds a use named after it (`car`, `car_2`), on its own clock, at that place, its path relative to the model's folder when inside it (`Edit::AddUse`). Inside a subsystem it is refused.
- **Tests:** two models wired on their own clocks from the canvas give the bits of the same links written by hand; on a shared clock they are plain wires; disconnecting removes the probe; refusals; Add model… names and paths.
- **Not yet tried in the window:** the question, Add model… and wiring between used models, beyond tests.

**Step 4d (2026-11-02): expanding in place.**
- **The diagram** (`simsandbox_session::expand`, `Diagram::expand(file, registry, paths)`): each use at a listed path is replaced by its model's nodes, named by path (`car1/car`), placed at the use's spot with their own file's layout, inside an `Outline` (path, model, clock, rect, shift). A path inside one (`p/one`) expands only when its outer use is expanded too: one level each.
  - Wires to the use's ports are drawn to the inner nodes they reach. An input goes to its input block, which gets an input port on the canvas. An output goes from the block its declared output shows, or from a port of a use inside.
  - Nodes to the right and below make room on screen (`Diagram::shifted`); the file's places stay. The inner nodes carry the run's values by full key.
- **The canvas:** outlines are drawn under the wires, coloured by clock (a shared clock in the selection colour, others by a colour from their clock's text), titled "path · model · clock". Nodes inside show their own names. "Expand in place" is in a use node's menu; "Collapse PATH" is in the menu on the outline's background, and takes the uses expanded inside it along.
- **Edits** (`expand::route`):
  - An edit on nodes inside an outline becomes an edit of the model it uses, in its own names and places, and goes through the edit question as when the use is opened.
  - A block added inside an outline goes into that model.
  - An edit reaching across an outline is refused with the reason.
  - A move of a node pushed aside is saved without the push.
  - The question can now be asked from the model's level (`Asking::path`).
- **Saved:** `controls.expand` lists the expanded uses by path. The app reads it on opening and writes it when one is expanded or collapsed. "Edit…" on a block inside an outline opens its equations (`path::block`); "Open" on a use inside an outline opens it by its full path.
- **Headless:** `render --diagram --expand car1,vehicle/plant` (comma-separated, besides `controls.expand`); Python `render(diagram=True, expand=[...])`.
- **Tests:**
  - the Platoon with `car1` expanded: inner nodes in the outline, wires through the input block and the declared output, `car2` pushed aside by a known amount, nothing changed when collapsed;
  - edits routed inside, adds inside, moves unshifted, refusals across;
  - nested expansion one level at a time;
  - in the app: expanded and saved, an edit inside asks the question for that use and changes every use when answered, a pushed node's move saved without the push, reopened expanded;
  - Python.

**Step 5 (2026-11-02): rewind, inputs, probes and run files.**
- **Rewind per clock** came with step 4b: the app runs a model that uses models as one kernel model on several clocks. Its snapshots hold every clock's time and steps and every input, so rewinding between instants replays exactly (tested then). Point 6's design was for models run apart and stays unbuilt: per-model recorders, links keeping what they delivered, and replay with models held at their restored ticks.
- **One loader** (`simsandbox_file::load_any`): a plain model, or a model using models put in place as one ([`compose::load_merged`]). Plain runs with inputs, probe runs, model hashes and run files' checks use it, in the CLI, the session and the app.
- **Runs:** `run_with` steps a model with clocks from instant to instant (`Simulator::next_instant`), each up to the end. It does not count steps of `dt`, so the rows are every instant of any clock, with each model's values at its own instants equal to the headless runner's.
- **Headless:**
  - `run` keeps giving each model at its own times.
  - With `--inputs`, `--probe`, `--save-probes` or a run file, the models run as one. Keys and recorded signals are full names (`slow/u.y`, `car3/car.v`), and `--model` does not apply.
  - `--dt` is refused: each model keeps its own step.
  - Python: `run(inputs=...)` returns `Results` for such a model.
- **Run files** of such models record the model hash with each used file's digest by path, so changing a used file shows the run as stale.
- **Tests:**
  - a schedule and a recorded track reach an input block inside a use;
  - the merged run gives each model's own-instant values bit for bit;
  - a probe inside a use records and writes its file;
  - the hash changes with a used file;
  - Python inputs.

**Step 6 (2026-11-05): models on several cores between instants.**

- Between instants each model on its own clock steps with its link inputs held, touching nothing another model touches, so the models due at an instant step at once on several cores; the results are the same bits as one after another, in any split.
- `simsandbox_core::parallel`: `each` runs independent units on the available cores (results in order; the calling thread takes a share), and `Costs` keeps each unit's recent step time. Threads start only when what the units besides the largest cost exceeds 0.2 ms (starting threads costs some tens of microseconds), so small models stay on one thread. `Parallel::{Auto, Never, Always}` chooses; `Auto` is the default.
- **The app's engine (one simulator on several clocks):** `Simulator::advance` lends each clock stepping now its blocks and fused groups as a unit (`Unit`, `Slot`) and runs the units with `each`; `GroupPlan::rhs` takes its members' blocks, parameters and held inputs in member order, so a group runs with its own members only. Outputs in feedthrough order, links and exchange stay one after another. `Simulator::set_parallel`.
- **The headless runner:** the models due at an instant step with `each`, and record their outputs with it too. `compose::run_composition_with` takes the `Parallel` choice.
- **Tests:** a kernel model on two clocks with fused groups and wires across clocks, and two linked models in the headless runner, give the same bits on one core and on several; the examples are unchanged, bit for bit.
- **Measured** (4 cores, a virtual machine whose timings vary by about 10% between runs; release build; two unlinked Gray–Scott models, 128 × 128, on their own clocks, 200 steps of about 1.3 ms each):
  - the kernel's stepping alone: one model 0.27 s; two models 0.51 to 0.55 s on one core, 0.35 to 0.37 s on several;
  - whole runs: headless 0.88 to 0.92 s on one core, 0.57 to 0.63 s on several; the app's engine 0.53 to 0.65 s. Headless and the app's engine are the same within the noise: headless keeps every output at every step (131 MB per model here, with the page faults that brings), the app only what its views and probes read, but does more per step.
  - Why two models on several cores take more than one alone, although four separate processes each run as fast as one: the models meet at every instant (here about 0.4 ms of each 1.3 ms step goes to waking the other thread, which sleeps between instants, and waiting for it), and each model's own work costs about 14% more CPU time side by side in one process. The latter is likely the caches: each instant starts new threads, which may run on another core than the model's data is cached on; this was not confirmed with a profiler. The allocator was ruled out (keeping large allocations changed nothing). Long-lived threads instead of new ones per instant gained about 0.03 s.
  - Not done, and what would close most of the gap: models with no link between them need not meet at every instant, only where a link delivers a sample; and one long-lived thread per model.
- Not split: one model's own work (its blocks on one clock, a large grid). Splitting cell loops and reductions into fixed chunks is ADR 0062 step 3.
