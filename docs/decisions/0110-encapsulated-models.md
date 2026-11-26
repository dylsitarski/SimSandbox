# 0110 — Encapsulated models: what changes or feeds on a model goes through its interface; observing stays open; surfaces show one level

- **Status:** Accepted (owner, 2026-11-25)
- **Amended by:** [ADR 0112](0112-experiments-may-reach-inside.md) (experiments may set values inside, marked)
- **Date:** 2026-11-25
- **Evidence:** [encapsulation and composition](../research/2026-11-25-encapsulation-and-composition.md)
- **Answers:** [Q65](../open-questions.md) (structure first)
- **Amends:**
  - [ADR 0043](0043-subsystems.md) and [ADR 0066](0066-models-nest-as-blocks.md): inner values set for one use;
  - [ADR 0053](0053-docked-panes-inspector-and-node-values.md): the Inspector's column per use;
  - [ADR 0108](0108-interface-synchronized-projections.md): what its projections show.
- **Builds on:** [ADR 0068](0068-one-kind-of-nested-model.md) (one kind of nested model, local copies), [ADR 0074](0074-model-interfaces.md) and [ADR 0075](0075-parameters-are-inputs-with-defaults.md) (a model's interface)

## Context

On 2026-11-25 the owner asked whether models are encapsulated, and whether nesting and encapsulation should keep the interface organized and scalable in place of labels such as port groups. The [research note](../research/2026-11-25-encapsulation-and-composition.md) found:

- **Wires into a use go through its interface; wires and links out of it do not.**
  - Into a use: a wire reaches only its exposed inputs, input blocks and declared parameters (`resolve` in `simsandbox-file/src/subsystems.rs`; "a wire into a model on a shared clock goes to one of its input blocks or parameters"). The canvas refuses a wire to an inner port that is not exposed (ADR 0074).
  - Out of a use: a wire may start at any inner output by path. A file wiring `a/c.x` to an outer block, with nothing offered, passes `check` (tried 2026-11-25). Probe links reach any depth (`probe:speed.plant/car.v`, ADR 0066 point 4).
- **Values do not.** A use's `params` may set any inner block's value for that use alone, as `"block.param"`, or a parameter declared by a use nested inside, as `"inner_use/k"`. The load error for an unknown parameter even suggests it.
  - The Inspector writes these from its table of a subsystem's uses, a column per use (ADR 0053).
  - None of the examples uses them.
- **Start states by inner block.** A use may give an inner block a start state, as `"starts": {"c": {...}}`.
- **Reading reaches inside by path, by design.** Views (`car1/car.v` in the platoon), probes, `run --signals` and links name inner signals.
- **Surfaces flatten.** The card ([ADR 0107](0107-model-cards-and-agent-notes.md)) and the Overview list `car1/car`, `car1/control`, `car2/car` … as parts. The used model's interface, the thing a reader needs, is not shown.

When a use can change what is inside its model, its model's interface does not say what the use depends on. Editing the model can then silently break the use. A surface that lists every inner block does not scale.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| A. As is: interfaces declared, inner values and surfaces open | No work | A model's interface is not its contract; surfaces repeat themselves as models grow |
| B. Strict: nothing inside is read or changed from outside | Simplest rule | Views, probes and debugging lose sight of the inside; trust needs it |
| **C. What changes or feeds a result goes through the interface; observing stays open and is marked; surfaces by level** | Interfaces become contracts; legibility comes from the structure; inspection keeps full depth | A format conversion; the Inspector's per-use table changes |
| D. Inner values allowed but warned | Least change | Warnings are ignored; nothing is a contract |
| E. C, and observing also only through outputs | One rule for everything | Every signal plotted, recorded or inspected must be offered at each level: interfaces bloat with debugging outputs and building slows; no gain in safety, as observers do not change results |

## Decision

1. **What depends on a model goes through its interface.** A use is changed only through:
   - its declared parameters;
   - its exposed inputs and input blocks.

   Anything whose value enters a computation reads only the model's outputs:
   - wires;
   - probe links;
   - formulas;
   - events.

   - A use's `params` names only its model's declared parameters: `"block.param"` and `"inner_use/k"` are no longer written.
   - A wire or link from an inner output that is not offered is refused, naming the fix: offer it.
   - Study formulas and search objectives are reviewed when this is built, and placed by the same test (does the value enter a result?).
2. **Older files convert as they are read.**
   - **In-file definitions:** each inner value set by a use of a model defined in the file becomes a declared parameter of that model:
     - it is named after the inner parameter, made unique (`car.c` becomes `c`, or `car_c` if taken);
     - its default is the value the model had;
     - the inner block reads it by formula;
     - the use sets it by name.

     Other uses keep the default, so every result is unchanged.

     Likewise, each wire or link from an inner output that is not offered becomes an offered output of that model, named after the port, made unique, through every level between, and the wire or link reads it. The conversion is recorded as a sentence in `conversions` and covered by an older-form fixture ([changing the file format](../guides/changing-the-file-format.md)).
   - **Uses of separate model files:** a file is not rewritten because another file uses it.
     - Such a value, wire or link still loads.
     - `check` reports it as reaching inside, at its path, with the fix: expose or offer it in that file.
     - The app and the Python package no longer write new ones.
     - Saving keeps it until it is exposed.
3. **Start states stay settable by inner block.** A start state is a state reached before (a steady state, a run's end), not configuration: it restores, as a run file does. `starts` stays, and the card shows it as reaching inside.
4. **Observing stays open and is marked.** Observing means reading inside without entering a result, as a debugger reads private fields:
   - views;
   - probes that only record;
   - `run --signals`;
   - scene bindings;
   - the card and the Inspector.

   - These may still name an inner signal by path. Closing them too was considered and rejected (Options, E): every plotted signal would have to be offered, level by level, which is the housekeeping this work avoids. It would buy no safety: an observer cannot change a result, and a path that breaks is an error at the reference.
   - Where they are shown (the card, the Overview, the Inspector, `check`), such a path is marked as reaching inside, with a one-step **Offer as output**. Offering adds the signal to the model's outputs and rewrites the reference to the output (`car1.v`).
   - A path whose inner block is gone after the model changes is an error at the reference, as today.
5. **Surfaces show one level.**
   - The card, the Overview and the Inspector present the level being looked at:
     - its blocks;
     - each use as one part, with its model's name and summary, its parameter values for this use, its inputs and outputs, and its notes and marks.
   - Opening a use (on the Overview, in the Inspector, on the canvas) shows its model's own card and parts, one level down.
   - Headless forms:
     - `simsandbox explain FILE --part car1`, and `--depth N` for more levels at once (default 1);
     - Python's `explain(…, part=, depth=)`;
     - `check`'s parts by level.
   - A level's **key parameters** are its model's declared parameters first, then its pins ([ADR 0109](0109-parameter-roles-key-parameters-port-groups.md) point 2).
6. **The Inspector's table of uses** has a row per declared parameter, a column per use (ADR 0053's table, its rows now the interface).
   - A selected inner block shows its values for the model, read-only in each use's column.
   - It offers two actions:
     - **Expose** makes the value a declared parameter, settable per use (ADR 0074's toggle);
     - **Change in the model** changes it for every use.
   - Changing inside a model from a use is either of those, or a local copy (ADR 0068).
7. **Agents.**
   - `Model.set_param` on a path inside a use is refused, naming the fix.
   - `Model.expose(model, "car.c", name=None)` declares the parameter and returns its name.
   - `check --agent` reports values that reach inside a separate file as problems to fix.

**Order of work:**

1. Conversion of in-file definitions (values and outputs), the loader's refusal of new path-set values and of wires and links from inner outputs not offered, the reports for separate files, and the fixture (points 1–2). Then the Python package's refusal and `expose` (point 7).
2. The card and the Overview by level, `explain --part`/`--depth`, and the reaching-inside marks and **Offer as output** (points 4–5).
3. The Inspector's table and its actions (point 6). The Inspector by intent (ADR 0108 point 5) is built on it.

## Consequences

- A model's interface is its contract: a model may change inside without changing any result that uses it. Only an observer reading by path can break, and `check` names it.
- A model whose uses differed by inner values gains declared parameters. That is the honest form of what those uses already depended on.
- **The other projections of ADR 0108 are built per level.** The card's flat list of parts goes.
- The [research note](../research/2026-11-25-encapsulation-and-composition.md) proposes further follow-ups, each needing its own ADR:
  - port groups become labels on a block's own ports, not an organizing concept;
  - the derived layout for the Structure lens is dropped;
  - extension, replaceable parts, replication, and contracts with tests.
- **No run-time cost.** The merged simulator still flattens and fuses across boundaries ([ADR 0090](0090-one-engine-for-nested-models.md)). Results stay bit for bit those of the same model before conversion.
- Documents to update when built:
  - `docs/architecture/` (nesting);
  - the model file reference;
  - the headless guide (`explain --part`, `expose`);
  - the Inspector's design.

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)):
- **Added:** "reaching inside", a mark on a reference by path. No existing concept says that a reference depends on a model's inside.
- **Removed:** an inner value set for one use (replaced by a declared parameter).

## Built so far

- **Step 1 (2026-11-25), conversion and enforcement:**
  - `simsandbox-file/src/encapsulate.rs`: `encapsulate(text, registry, folder)` converts a file before the composition loader reads it (`load_composition`, `load_merged_with`). The app, `run`, `check`, `upgrade` and Python therefore all see, run and save the converted form, and `check` and `upgrade` list the `conversions`.
    - It works on the file's JSON, one change a level down at a time, until nothing reaches inside. Every use setting the same path shares one parameter.
    - A default comes from the value written inside, a declared parameter that value names, or the block type's default (the block is built to read it, with its unit and description; a suggested range the value lies outside of is dropped). For a separate model file used inside, it is read from that file.
  - Uses of separate model files are reported at their path, with the fix, and left as written. So is a value set inside by a formula other than a declared parameter's name.
  - "Refused" (point 1) is built as this conversion on every load: a path written by an older tool, or by hand, becomes the interface the next time the file is read.
  - Probe links are converted with their probe's record and every reference to the probe's signal.
  - `expose(text, registry, folder, model, key, name)` and `offer_output(text, model, signal, name)` make the interface by hand. Python's `Model.expose` and `Model.offer` call them. `Model.set`, `add_model(params=)`, `connect` and `link` refuse inner paths, naming the fix.
  - `simsandbox upgrade` now loads a model that uses models (before, it refused one).
  - Tests: `tests/encapsulation.rs` (shared parameters, results bit for bit, formulas kept, separate files reported, `expose` and `offer_output`); the older-form fixture `tests/old/adr-0110-reaching-inside.json` with values one and two levels deep, a wire and a link (its runs are the same bytes from the build before, from this build, and from its current form); Python's composition test rewritten through interfaces. Every example's run gives the same bytes.
- **Step 2 (2026-11-25), surfaces by level:**
  - `simsandbox-session/src/card.rs`: `card_at(session, summary_of, part, depth)` gives the card of the model at a path of uses. Its own blocks are parts, as before (each with `path`, its full name); each model it uses is one part (`kind: "use"`, role `model`).
    - A use part carries its model or file, summary (the use's purpose, else its model's purpose or description), notes and mark, clock and the least labels of its blocks.
    - It also carries its interface for this use (declared parameters with this use's values and formulas, exposed inputs and input blocks, offered outputs, each with what is wired to it as written), and the names of what it holds. With `depth` above 1, its own card is nested as `card`.
    - `card(session, …)` is the top level.
  - Wires are told as written at the level (`car1/lead`, not the inner port they feed). A block's port that the model exposes says so ("the model's input lead").
  - Key parameters are the level's declared parameters, then its pins, else what the types suggest.
  - `reaching_inside` lists views and probes reading inside a use by path, with whether its model offers that output already (`offered_as`) or can (`can_offer`, a model defined in the file), and uses' start states by inner block.
  - Markdown: a use as a heading with its interface table, `Inside: …` or its nested card with headings two levels down, and a "Reaching inside" section.
  - `Edit::OfferAsOutput { signal }` (`diagram.rs`): the used model defined in the file offers the output (through `encapsulate::offer_output`), and every reference to that signal at the level reads `use/name`. For a separate file it says to offer it there.
  - Views and filters may read a used model's offered output by name (`car1/v`): the merged load checks them against what it stands for (`load_merged_with`), and the session already recorded it under that name.
  - The platoon's and the forced spring's views read `car1/v` and `held/x`, which their models offered already. Their run outputs are the same bytes; recording keeps what views read (ADR 0063), so the session records `car1/v` in place of `car1/car.v`.
  - `simsandbox explain FILE --part PATH --depth N`; Python's `explain(…, part=, depth=)` and `Model.explain`.
  - The app's Overview (`app/overview.rs`) shows one level:
    - a breadcrumb back up;
    - each use with **Open** (its own level, in place), **Inspect** at the top level, its interface with live values, and what is wired to it;
    - "Reaching inside" with **Offer as output** (or **Read car1/v** when it is offered already), applied as `Edit::OfferAsOutput`.
    - A level that is gone falls back to the model.
  - Its arrows and `▸` are words and `>`, which the app's font has; `--view overview` is accepted by the app's argument check (it was listed but refused).
  - Tests: the card by level and the edit (`card.rs`), `explain --part/--depth` (CLI), Python's levels, the app opening a level. A screenshot of the platoon's Overview under Xvfb.
- **Step 3 (2026-11-26), the Inspector's table of uses:**
  - A row is inside a used model when its owner is a path (`car1/car`). Such rows are read-only, after a divider "inside", both in the table of a model's uses (a column per use) and in a single use's nested rows.
  - Right-click offers two actions:
    - **Expose as a parameter of its model**: `Edit::ExposeInside { key }` declares it in the model defined in the file, through `encapsulate::expose`; pins and schedule entries that set it by path in any use of that model then set it by name (`car1/car.c` becomes `car1.c`).
    - **In the model, for every use**: a value field, `Edit::SetInside { key, value }`, through `encapsulate::set_inside`; refused where a formula sets it.
  - Uses of separate files are told to open the file.
  - The model's declared parameters (`mass`) stay editable per use. Pinning stays (ADR 0112).
  - The Overview's experiment marks have **Expose** too, except a study's points (a path in the studied model).
  - Tests: both edits in the session (the run with a schedule unchanged to the bit after exposing; every use changed in the model), and exposing from the app with the Inspector drawn.
- **Left:**
  - Point 1's events, and study formulas, have nothing that reads inside yet.
  - Exposing inside a separate model file from the model using it goes through opening that file (the edit question of ADR 0068), not these edits.
