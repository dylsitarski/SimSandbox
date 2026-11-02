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
   - **4c:** editing: a probe added when wiring across clocks, the edit question (edit the original, make a local copy, save as a new model file), saving several files, "Add model…".
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
- **Canvas:** a use is drawn with a double outline, like a subsystem use, in a stronger colour. Its menu names its file and clock. Opening it comes with 4b.
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

## Consequences

- `models` (ADR 0065 step 1, never released) goes. A used file is any model file, and any model file can use others.
- Subsystems stay for definitions inside a file. Model files used on a shared clock are subsystems kept in their own files.
- The runtime stays flat: one list of domains and one list of links, whatever the nesting, so ordering, replay and multi-core stepping do not depend on depth.
- ADR 0065's suggestions accepted by the owner stand: links through input blocks, parameter values and clocks per use, the three edit choices, and rewinding one use alone later.
