# 0110 — Encapsulated models: a use changes its model only through its interface, and surfaces show one level

- **Status:** Proposed
- **Date:** 2026-11-25
- **Evidence:** [encapsulation and composition](../research/2026-11-25-encapsulation-and-composition.md)
- **Answers:** [Q65](../open-questions.md) (structure first), when accepted
- **Amends:**
  - [ADR 0043](0043-subsystems.md) and [ADR 0066](0066-models-nest-as-blocks.md): inner values set for one use;
  - [ADR 0053](0053-docked-panes-inspector-and-node-values.md): the Inspector's column per use;
  - [ADR 0108](0108-interface-synchronized-projections.md): what its projections show.
- **Builds on:** [ADR 0068](0068-one-kind-of-nested-model.md) (one kind of nested model, local copies), [ADR 0074](0074-model-interfaces.md) and [ADR 0075](0075-parameters-are-inputs-with-defaults.md) (a model's interface)

## Context

On 2026-11-25 the owner asked whether models are encapsulated, and whether nesting and encapsulation should keep the interface organized and scalable in place of labels such as port groups. The [research note](../research/2026-11-25-encapsulation-and-composition.md) found:

- **Wires already go through the interface.** At load, a wire to a use resolves only to its exposed inputs, outputs and declared parameters (`resolve` in `simsandbox-file/src/subsystems.rs`), and the canvas refuses a wire to an inner port that is not exposed (ADR 0074).
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
| **C. Changes through the interface; reading stays open and is marked; surfaces by level** | Interfaces become contracts; legibility comes from the structure; inspection keeps full depth | A format conversion; the Inspector's per-use table changes |
| D. Inner values allowed but warned | Least change | Warnings are ignored; nothing is a contract |

## Decision

1. **A use changes its model only through its interface:**
   - its declared parameters;
   - its exposed inputs;
   - its outputs.

   Wires already work this way, and this extends the rule to values. A use's `params` names only its model's declared parameters. `"block.param"` and `"inner_use/k"` are no longer written.
2. **Older files convert as they are read.**
   - **In-file definitions:** each inner value set by a use of a model defined in the file becomes a declared parameter of that model:
     - it is named after the inner parameter, made unique (`car.c` becomes `c`, or `car_c` if taken);
     - its default is the value the model had;
     - the inner block reads it by formula;
     - the use sets it by name.

     Other uses keep the default, so every result is unchanged. The conversion is recorded as a sentence in `conversions` and covered by an older-form fixture ([changing the file format](../guides/changing-the-file-format.md)).
   - **Uses of separate model files:** a file is not rewritten because another file uses it.
     - Such a value still loads.
     - `check` reports it as reaching inside, at its path, with the fix: expose it in that file.
     - The app and the Python package no longer write new ones.
     - Saving keeps it until it is exposed.
3. **Start states stay settable by inner block.** A start state is a state reached before (a steady state, a run's end), not configuration: it restores, as a run file does. `starts` stays, and the card shows it as reaching inside.
4. **Reading stays open and is marked.**
   - Views, probes, `run --signals`, links and scene bindings may still name an inner signal by path.
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

1. Conversion of in-file definitions, the loader's refusal of new path-set values for them, the reports for separate files, and the fixture (points 1–2). Then the Python package's refusal and `expose` (point 7).
2. The card and the Overview by level, `explain --part`/`--depth`, and the reaching-inside marks and **Offer as output** (points 4–5).
3. The Inspector's table and its actions (point 6). The Inspector by intent (ADR 0108 point 5) is built on it.

## Consequences

- A model's interface is its contract: a model may change inside without breaking a use, unless the change removes something the use reads by path, which `check` names.
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
