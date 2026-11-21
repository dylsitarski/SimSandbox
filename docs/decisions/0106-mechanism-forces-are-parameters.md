# 0106 — A mechanism's joint forces and body loads are parameters, not inputs

- **Status:** Accepted (owner, 2026-11-20)
- **Date:** 2026-11-20
- **Supersedes:** [ADR 0105](0105-mechanism-file-form.md) point 4's inputs and its settings `actuated` and `loaded`
- **Builds on:** [ADR 0075](0075-parameters-are-inputs-with-defaults.md) (parameters are inputs with defaults: a wire may drive any parameter, read at every evaluation)

## Context

ADR 0105 gave a mechanism inputs for its joint forces and body loads, but only for the joints listed in `actuated` and the bodies listed in `loaded`, because an input is a signal with no default and must be wired. The cart-pole showed the cost. The pole's torque input forced a filler line (`n = 0 [N*m]`) and a wire. The two lists are settings a user must know about, and the node carries ports nothing uses. Asked whether an input with a default is anything but a parameter (2026-11-20), the answer was no: ADR 0075 already lets a wire drive a parameter. A driven parameter is read at every evaluation, and inside a fused group at every stage, so a controller still acts within the step.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Inputs for listed joints and bodies (ADR 0105) | Only used ports exist | Lists to keep; a filler wire for each unused part of a load; the node grows with what is listed |
| Inputs with a default for every joint and body | No lists | Duplicates parameters (ADR 0075); a second kind of default |
| **Parameters, default 0, for every moving joint and every body** | No lists, no filler; a wire drives what it needs; undriven ones are values, not ports; sliders and studies work on them too | Five more parameters per body (one per joint, three per body), to be grouped in the Inspector |

## Decision

1. Every moving joint has a parameter `<joint>_torque` (revolute, N·m) or `<joint>_force` (prismatic, N), default 0. Every body has `<body>_fx`, `<body>_fy` (N, world axes, at its centre of mass) and `<body>_torque` (N·m), default 0. The names are those of ADR 0105's inputs, so a wire that fed one drives the parameter unchanged.
2. A mechanism has no inputs and no settings `actuated` or `loaded`. A file that has them is read with the settings dropped and a conversion note naming this ADR ([changing the file format](../guides/changing-the-file-format.md)); its wires stay valid.
3. No output depends on these parameters at the same instant, so driving them from outputs that depend on the joints makes no algebraic loop.

## Consequences

- The cart-pole loses `n = 0` and its wire; `loaded` and `actuated` leave the example, the reference, the Inspector's table (its two checkboxes), Python's checks and `Edit::SetMechanism`'s handling of the settings lists.
- The parameter list grows; the Inspector groups these as forces once parameters carry roles (see the [interface report](../research/2026-11-20-interface-report.md)).
- The pinned results of the cart-pole cases are unchanged if the arithmetic is (a load of 0 adds 0); they are re-checked when this is built.
- To build: next in STATUS.

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none; two settings removed.

## Built (2026-11-21)

- `simsandbox-mech/src/block.rs`: the forces are parameters after each row's numbers (a moving joint's after its start, then the body's three); the energy and pose outputs declare every parameter but the forces as what they depend on, so a wire driving a force from them is no loop. A setting is refused, naming this ADR.
- The file reader drops `actuated` and `loaded` from `mech.*` blocks with a conversion note (`tests/old/adr-0106-mechanism-settings.json` and its current form).
- The Inspector's table lost its two input boxes and the toggle edit; `rows::listed` is gone.
- The cart-pole's results are the same bits in every column but the removed `control.n` (compared over 6 s, 3001 rows); its picture renders the same. Its pinned hashes changed only by that column and were pinned again for Linux; Windows and macOS are pinned again from CI.

