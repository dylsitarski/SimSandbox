# 0109 — Block metadata: parameter roles, key parameters, port groups, descriptions

- **Status:** Proposed (draft for the owner, 2026-11-21)
- **Date:** 2026-11-21
- **Builds on:** [ADR 0108](0108-interface-synchronized-projections.md) (the interface, legibility first: this is its second step), [ADR 0107](0107-model-cards-and-agent-notes.md) (model cards), [ADR 0053](0053-docked-panes-inspector-and-node-values.md) and [ADR 0054](0054-controls-pane-pins-and-node-toggles.md) (pinned controls), [ADR 0074](0074-model-interfaces.md) (exposed parameters), [ADR 0039](0039-ai-agent-access.md)

## Context

The Overview, the Inspector by intent and the Structure lens (ADR 0108) all need the same few facts about a block, which today exist only in part:

- **Parameter kinds.** Every parameter looks alike. The cart-pole's mechanism lists about 35 (`slider_at_y` beside `pole_mass` beside `pole_fx`), and its controller lists gains beside the grab's spring constants. Only "sets the start" is marked (`ParamSpec::initial_condition`).
- **What matters.** A model already says which values matter to it: its pinned controls ([ADR 0054](0054-controls-pane-pins-and-node-toggles.md)), shown in the Controls panel. Nothing else uses them.
- **Bundles of wires.** The cart-pole's controller takes four wires from the mechanism and returns three. They form a pair of bundles between two blocks, drawn as seven lines.
- **What a block is.** Block types have a summary (`describe`); parameters have a description (`ParamSpec::description`), empty for typed equations, whose lines starting with `#` are skipped and which allow no comment after a statement.

## Options considered

**Parameter kinds**

| Option | Pros | Cons |
| --- | --- | --- |
| No kinds; group by name prefix (`pole_*`) | Nothing new | Works for the mechanism only; says nothing of what a value is |
| **A role per parameter, from a short fixed list, declared by block types and in typed equations** | Groups that mean the same in every block; the card and Inspector say "tuning: k1…k4" | A list to agree on; typed equations need a word for it |
| Free-form tags | Flexible | Agents and people invent synonyms; nothing to group by reliably |

**What matters**

| Option | Pros | Cons |
| --- | --- | --- |
| **A model's key parameters are its pinned controls; a block type may suggest its own** | No new field: what a person pins and what an agent pins are the same thing; the Controls panel already shows them | A model with nothing pinned shows the types' suggestions |
| A new `key` list in the file | Separate from the Controls | Two lists that drift apart |

**Bundles of wires**

| Option | Pros | Cons |
| --- | --- | --- |
| **Draw every wire between the same two blocks as one bundle; port groups name and order a bundle's ports** | Works for every block today, no metadata needed to bundle; groups only label | A bundle mixing unrelated signals is labelled by count, not name |
| Port groups as connectable units (one wire carries a group) | Fewer wires in the file too | Changes the file's connections and their checking; matching groups between blocks needs rules |

**Typed equations**

| Option | Pros | Cons |
| --- | --- | --- |
| **A trailing `# text` is the description of what the line declares; leading `#` lines of the block are its summary; `as ROLE` after a `param`'s default sets its role** | Reads naturally; existing files unchanged (a `#` line stays a comment) | Comments after statements become meaningful; a small grammar change |
| Roles by section headings (`## tuning`) | No change to `param` | Position-dependent; easy to break by reordering |
| Metadata only in the file beside the equations | Language unchanged | Two places for one parameter; agents must keep them in step |

## Decision (proposed)

1. **Parameter roles.** Every parameter has one role, from this list:

   | Role | Means | Examples |
   | --- | --- | --- |
   | `property` (default) | A physical or model property | masses, damping, `g`, rates |
   | `geometry` | Where things are and how big | lengths, joint points, centres, turns |
   | `start` | The state it starts from; acts on restart | `x0`, `hinge_angle0` |
   | `drive` | Pushes the model from outside; usually driven by a wire | the mechanism's forces and loads ([ADR 0106](0106-mechanism-forces-are-parameters.md)), a forcing amplitude |
   | `tuning` | A design choice: gains, limits, set points | `k1`…`k4`, `umax`, the grab's `ks` |
   | `numerics` | How it is computed, not what is modelled | grid spacing, tolerances given as parameters |

   - A block type declares each parameter's role (`ParamSpec::role`).
   - `start` replaces the flag `initial_condition`, which becomes the test `role == start`.
   - Typed equations infer `start` for the initial values of states (`x0`) and `property` for the rest, unless the line says otherwise (point 4).
2. **Key parameters.**
   - A model's key parameters are its **pinned controls**; pinning is how a person or an agent marks one.
   - A block type may mark parameters as suggested keys (`ParamSpec::key`). Those are shown first only for a block whose model pins nothing of it.
   - Nothing is added to the file.
3. **Port groups and bundles.**
   - The Structure lens draws all wires between the same two blocks, in one direction, as one bundle, labelled by the group its ports share or by their count. It expands on demand.
   - A block type may give ports a group (`PortSpec::group`), which names and orders a bundle. The mechanism groups its joint coordinates and rates as `joints` and its poses as `bodies`.
   - Connections in the file stay one per port.
4. **Typed equations.**
   - `# text` after a statement is the description of what it declares (a parameter, input, state or defined quantity).
   - `#` lines before the block's first statement are the block's summary.
   - `param NAME [unit] = number as ROLE` gives a role.
   - A role that is not in the list is an error at its line.
   - A `#` line anywhere else stays a comment, so every existing file reads as before.
5. **Where it shows.**
   - `describe` and Python's `describe` give each parameter's role, suggested key and description, each port's group, and the block's summary.
   - The Inspector groups by role and shows key parameters first.
   - The Overview lists key parameters as live controls and quotes descriptions.
   - `check --agent` ([ADR 0107](0107-model-cards-and-agent-notes.md)) will require descriptions only on what agents made.
6. **Built-in types declare theirs.**
   - `mech.planar`:
     - geometry: centres, joint points, turns;
     - property: masses, inertias, damping, stiffness, rest, `g`;
     - start: starts;
     - drive: forces and loads;
     - port groups `joints` and `bodies`.
   - `eq.pendulum` and the other built-in systems, the input blocks and conversions declare roles likewise.
   - The cart-pole example's controller says `as tuning` for its gains and limit and describes them.

## Consequences

- `ParamSpec` gains `role` and `key` (its `initial_condition` field becomes a method reading the role: about 37 uses to move), `PortSpec` gains `group`, both with defaults, so blocks that say nothing are unchanged. Every block type that marks initial conditions moves to the role.
- The typed-equations reference, front end and its tests change for point 4. Older files read unchanged.
- No model file field is added. Pins gain a meaning they already had in practice.
- The interface's next steps (the Overview, the Inspector by intent, the Structure lens) read these; plugins' manifests will carry the same fields (ADR 0108's later step).

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none; roles, key parameters and groups are properties of parameters and ports.
