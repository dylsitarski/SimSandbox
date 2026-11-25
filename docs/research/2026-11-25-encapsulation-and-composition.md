# Encapsulation and composition as the interface's organizing principle

_Written 2026-11-25 at the owner's request; nothing here is decided._

The owner asked:
- whether the interface work under way ([ADR 0108](../decisions/0108-interface-synchronized-projections.md) and its steps) burdens model building with documentation and housekeeping;
- whether models are encapsulated, and should be;
- which other properties would make SimSandbox more powerful;
- whether nesting and ideas from computer science (encapsulation above all) should organize the interface, instead of what the owner called the "somewhat arbitrary naming of groups".

This note answers each and compares the two directions.

## 1. Does the current direction add housekeeping?

| Piece | Who pays, and when | Verdict |
| --- | --- | --- |
| [ADR 0106](../decisions/0106-mechanism-forces-are-parameters.md): forces are parameters | Nobody; it removed a filler line and a wire | Less work |
| [ADR 0109](../decisions/0109-parameter-roles-key-parameters-port-groups.md): roles, descriptions, summaries | Optional for people: roles default (`property`, `start` inferred), comments optional | No overhead unless wanted |
| [ADR 0107](../decisions/0107-model-cards-and-agent-notes.md): notes and agent marks | Agents only, by design (`check --agent`); people never | Overhead placed deliberately on agents |
| The card and the Overview | Generated from the file | None |
| The Inspector by intent (next) | Generated from roles | None |
| Planned: port groups, the Structure lens's layout by inferred roles, bundles by block pair | Block-type authors (groups); nobody else | No overhead, but legibility rests on heuristics and labels |

So the work does not burden people with housekeeping. Its weakness is elsewhere: it makes a *flat* graph legible by decorating it with inferred roles, group labels and bundles. That does not scale. The platoon's card (2026-11-25) lists `car1/car`, `car1/control`, `car2/car`, `car2/control`, `car3/car` and `car3/control` as six parts beside the leader, because `card.rs` reads the flattened model. The same three-car structure is told three times, and the follower's interface, which the model already declares, is not shown at all.

## 2. Are models encapsulated?

| Property | Today | Evidence |
| --- | --- | --- |
| Models nest as blocks | Yes | [ADR 0066](../decisions/0066-models-nest-as-blocks.md), [ADR 0068](../decisions/0068-one-kind-of-nested-model.md): uses of model files and of definitions in the file |
| An interface is declared | Yes | [ADR 0074](../decisions/0074-model-interfaces.md): parameters, inputs, outputs, exposed by a toggle |
| Wiring goes through the interface | Into a use, yes; out of it, no | Wires in reach only exposed inputs, input blocks and parameters. A wire may start at any inner output by path (`a/c.x` to an outer block passes `check`, tried 2026-11-25), and probe links reach any depth (ADR 0066) |
| Setting values goes through the interface | No | A use may set any inner block's value for itself as `"block.param"` (`simsandbox-file/src/subsystems.rs`) |
| Reading goes through the interface | No, by design | Views, probes, `run --signals` and links read inner signals by path (`car1/car.v` in the platoon's views) |
| Presentation respects levels | Partly | The canvas shows a use as one node (expandable); the card and parts of the Inspector list the flattened blocks |
| Cost of nesting at run time | None | [ADR 0090](../decisions/0090-one-engine-for-nested-models.md): one merged simulator; fusion works across boundaries |

Nesting is structural and cheap, and the interface exists. What is missing is **information hiding**: the interface is not the only way to change a model from outside, and the surfaces do not present one level at a time.

## 3. Properties that would make models more powerful

From programming languages and modelling languages (Modelica, Simulink's subsystems and libraries, Houdini's subnetworks with promoted parameters), in order of how much each gives SimSandbox:

1. **Encapsulation with transparency.**
   - The interface is the only way to *depend on* or *change* a part from outside. A use sets only exposed parameters and wires only exposed ports.
   - *Reading* inside stays open (views, probes, the card's drill-in), because trust and debugging need it, but it is shown as reaching in.
   - This is how a model stays correct when its inside changes.
2. **Presentation by level.** Every surface shows one level: a model's parts, each used model as one part with its interface and its own card a click deeper.
   - Most of what ADR 0109 and the report reached for with labels follows from the structure: a use *is* the group, its interface *is* the bundle, its exposed parameters *are* its key parameters.
   - Each level stays small, so the Structure lens needs no automatic layout to stay readable.
3. **Extension.** A model built *on* another, recording only the difference: `extends cartpole`, plus friction, with the controller replaced.
   - Copy-and-edit (today's local copies) loses the link to the original. Extension keeps it, so a fix to the base reaches every model built on it.
   - It is the shortest path from idea to model: start from something that runs and change one thing.
4. **Replaceable parts behind an interface.** A part declared by what it takes and gives ("a controller: joint state in, force out") can be any model with that interface.
   - Examples: LQR or PID or a learned policy; a mechanism on the native solver or on Rapier.
   - This is the vision's "choose the method, know what you get" at the level of parts, checked by units and shapes as wires already are.
5. **Replication.** N uses of one model, parameterized by row: a platoon of 100 cars, a chain of links as a model per link.
   - This is the scalable form of what the platoon writes out by hand three times, and the "agents as models" stage of [ADR 0079](../decisions/0079-tables-of-members.md) (stage 5) asks for the same thing.
6. **Contracts and tests that travel with a model.** Invariants (ranges, conservation, units, already) and verification cases attached to the model and run by `check`.
   - They document intent more precisely than prose notes, and they earn the "Verified" label per case ([ADR 0061](../decisions/0061-verified-is-earned-per-case.md)).
7. **Reproducible references.** A use pinned to a version (by hash) of the model it uses, so a shared model runs as it did. Run files already record a model's hash ([ADR 0059](../decisions/0059-run-files.md)).

Already present: deterministic, replayable runs; unit- and shape-typed interfaces; one engine for any nesting.

## 4. The two directions compared

- **A. As planned** (ADR 0108's steps): a flat graph made legible by roles inferred from it, group labels, bundles by block pair, key parameters from pins, a card of every block.
- **B. Structure first:** the same five projections (Overview, Stage, Structure, Definitions, Inspector), but each projects *one level of an encapsulated hierarchy*. Components are used models with interfaces; extension, replaceable parts and replication build on them.

| Criterion ([interface report](2026-11-20-interface-report.md) section 1) | A | B |
| --- | --- | --- |
| Legible at a glance (C1) | Good for small flat models; repeats itself as they grow | Good at every size: each level small, each part summarized by its interface |
| Short path from idea to model (C2) | Templates are copies | Start from a model and extend it; swap a part for another with the same interface |
| No ceiling on control (C3) | Everything reachable | Everything reachable by opening a part; changes go through interfaces, or into the part itself |
| Composition across kinds (C5) | Wires between any blocks | Components of any kind behind typed interfaces |
| Parity for agents (C6) | Good | Better: agents compose and extend named models instead of editing flat files; interfaces are contracts `check` holds them to |
| Scales (C7) | Poorly (the platoon's card) | Well: replication and levels |
| Cost | Lower now | An encapsulation ADR and a format change for values set by path; extension, replaceable parts and replication each need an ADR and work |

B does not replace ADR 0108; it changes what its projections show. What is already built keeps its value:

- **Keep:**
  - ADR 0106;
  - ADR 0107's notes and marks (a component's card is where notes belong);
  - ADR 0109's roles and descriptions (they describe a leaf block's parameters);
  - the Overview (it becomes per level).
- **Revise:**
  - the card and the Overview: show uses as single parts with their interface, drill in for their own card;
  - key parameters: a level's exposed parameters, then its pins;
  - the Inspector by intent: a use shows its interface.
- **Drop or demote:**
  - port groups as their own concept (a use's interface does it); the mechanism's `joints` and `bodies` groups can stay as labels;
  - inferred roles as the organizing principle (a fallback label for leaf blocks only);
  - a derived layout for the Structure lens.

## 5. Risks of B

- **Friction to make a part.** Making a part must be one step from any selection. Grouping into a model defined in the file exists (`Edit::GroupModel`); it needs to be obvious and to choose the interface well (crossing wires become its ports, ADR 0068).
- **Ceremony in tiny models.** Nothing forces a boundary: a three-block model may stay flat.
- **Files that set inner values by path.** These convert on reading: each such value becomes a parameter the used model exposes, so behaviour is unchanged ([changing the file format](../guides/changing-the-file-format.md)).
- **Losing access.** Reading inside stays open. Changing inside means opening the part, which makes a local copy or edits the model it uses, as today (ADR 0068).

## 6. Recommendation

Adopt B as the organizing principle of ADR 0108's projections. Pause the Inspector-by-intent step until the card is per level. The order of work would be:

1. **An ADR on encapsulation.**
   - A use changes its model only through exposed parameters and ports.
   - Values set by path convert to exposed parameters.
   - Reading inside stays open and is shown as reaching in.
   - Surfaces present one level.
2. **The card and Overview per level** (the platoon told once, its three uses with their interface).
3. **The Inspector by intent** on the same basis: a use shows its interface; a leaf block its roles.
4. **An ADR on extension and replaceable parts.**
5. **Replication:** arrays of uses, joined with [ADR 0079](../decisions/0079-tables-of-members.md) stage 5.
6. **Contracts and tests attached to models.**
7. **The plugin interface** as packaged components (the report's section 7), now simply libraries of encapsulated models with their cards, pictures and tests.
