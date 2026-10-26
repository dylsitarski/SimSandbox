# 0057 — Phase 2 order, scope and gate

- **Status:** Accepted
- **Date:** 2026-10-26

## Context

Phase 1 is done. Its gate, an ODE model and an agent model in one graph, was confirmed by the owner on Linux and Windows on 2026-10-23, and the work was merged into main on 2026-10-24. The roadmap's Phase 2, "Breadth", lists seven items:

- rigid bodies (MuJoCo);
- ML blocks (ONNX Runtime);
- FMI 3.0 import and export;
- fast GPU fluids;
- an adaptive co-simulation master;
- scheduled changes;
- physical (acausal) connections.

Its gate is "the verification suite passes for every backend".

On 2026-10-26 the owner agreed a direction for Phase 2 ([roadmap](../roadmap.md#phase-2--breadth)):

- make real-time models interactive, so a model you compose can be played with like an open Algodoo, without becoming a game engine;
- put heavy custom physics into typed equations first, then wrapped engines, then plugins.

That added four items, in an order still to be planned:

- input from views and devices ([Q36](../open-questions.md));
- arrays and faster typed equations ([Q35](../open-questions.md));
- one plugin interface for user-defined blocks and views ([Q28](../open-questions.md), [Q37](../open-questions.md));
- rigid bodies with a scene view whose bodies can be grabbed.

Eleven items are too many for one phase with one gate. The roadmap's rule is that "each gate is a working demo, not a feature count", and "the verification suite passes" is closer to a count. Some items also depend on others:

- A scene view whose bodies can be grabbed needs input from views (Q36).
- A plugin interface for views is easier to design once the scene view shows what a view needs: draw lists, picking, input.
- Arrays in typed equations make stiff grids common, which raises implicit methods ([Q17](../open-questions.md)).

## Options considered

**Gate**

| Option | Pros | Cons |
| --- | --- | --- |
| **A live demo plus verification:** a cart-pole (MuJoCo or Rapier) balanced by a typed-equation controller, which the user disturbs by dragging the cart or pole and pushing with keys. The run is rewound and replayed with the same input, bit for bit. Every backend added in Phase 2 passes its verification tests | One demo exercises input blocks, recorded input, rigid bodies, the scene view, typed equations and coupling. It is the Fun pillar made concrete, and research-grade replay is checked too | Needs the rigid-body item before the gate |
| Keep "the verification suite passes for every backend" | Already written | A count, not a demo; it says nothing about the agreed direction |

**Order**

| Option | Pros | Cons |
| --- | --- | --- |
| **Inputs → typed arrays and speed → rigid bodies and scene view (gate) → plugin interface → breadth** | Each step uses the last: the scene view uses inputs, and the plugin interface is designed after one rich built-in view and the extended typed equations show what remains. The gate comes as early as it can | Plugins come fourth, though the direction listed them third |
| Inputs → typed arrays → plugins → rigid bodies (the direction's order as listed) | Plugins sooner | The view-plugin interface is designed before the first scene view; the gate comes later |
| Rigid bodies first | The most visible change first | Grabbing bodies needs inputs, so it would be built twice |

**Scope**

| Option | Pros | Cons |
| --- | --- | --- |
| **Phase 2 = the four direction items, plus scheduled changes (folded into inputs) and implicit methods (with typed arrays). ML blocks, FMI 3.0, GPU fluids, the adaptive master and acausal connections follow the gate, in an order chosen then** | A phase of a size Phase 1 had (eight items); everything left is still listed | The phase "Breadth" adds one new external backend before its gate |
| Keep all eleven items in Phase 2 | Nothing moves | Long phase; the gate arrives late |
| Move the adaptive master and acausal connections to Phase 3 (research) | They serve accuracy more than play | Acausal networks are also useful for play (circuits, mechanisms) |

## Decision

The owner chose the gate, the order, the scope and how the rigid-body engine is chosen on 2026-10-26.

**Gate:** the interactive cart-pole demo. It runs live in the app at real-time speed on Linux and Windows:
- a typed-equation controller balances the pole;
- dragging and keys disturb it;
- rewinding and running on with the recorded input reproduces the run bit for bit;
- the demo is also checkable headless (input from a file, a rendered scene).

The verification policy holds for everything added: reference, order or tolerance tests and honest labels for each new method or backend.

**Order and scope:**

1. **Inputs: live, scheduled and replayed** (Q36, recommended option (a)).
   - Input blocks, whose outputs come from the UI: pointer, drag force, keys, gamepad.
   - Views that send input.
   - Every input recorded as a signal, so rewind and replay are exact.
   - Scheduled changes (parameter steps and disturbances at set times in the `run` section, [ADR 0042](0042-node-canvas.md)) become input written in the file instead of typed live.
   - Headless runs read input from a file ([ADR 0039](0039-ai-agent-access.md)).
   - First demos: dragging the pendulum, steering the cruise-control car.
2. **Typed equations with arrays, faster** (Q35).
   - Array states, parameters and ports in typed equations, with grid stencils (Laplacian, gradient) for the method of lines.
   - Faster evaluation: a compact instruction list first; compiling to machine code if measurements call for it.
   - An implicit method for stiff grids ([Q17](../open-questions.md)), since explicit diffusion forces tiny steps.
   - Reference problems: the heat equation against its analytic solution, and a reaction–diffusion pattern.
   - Typed agent rules ([Q26](../open-questions.md)) are revisited here, as arrays may cover part of them.
3. **Rigid bodies and a scene view** (the gate).
   - The first rigid-body backend is chosen by a spike between MuJoCo (the roadmap's choice; accurate and robotics-grade, a C library) and Rapier (pure Rust; 2D and 3D; cross-platform determinism).
   - The spike checks:
     - the build on Windows, macOS and Linux;
     - snapshot and restore for rewind;
     - determinism;
     - real-time speed on the cart-pole;
     - verification against the simple pendulum period.
   - The result gets its own ADR.
   - A 2D/3D scene view draws bodies from model outputs, can be picked and dragged through item 1, and renders headless.
4. **One plugin interface for blocks and views** (Q28, Q37).
   - A WebAssembly spike first: one block and one view, both in a shared model file, sandboxed, with state in kernel memory and running under the worker thread.
   - Then the ADR that settles Q28 and Q37; Python blocks are considered after it.
5. **After the gate, in an order chosen then:** ML blocks (ONNX Runtime), FMI 3.0 import and export, fast GPU fluids, the adaptive co-simulation master, physical (acausal) connections.

**Each item:**
- gets its own ADR before code where it changes a contract;
- meets [ADR 0039](0039-ai-agent-access.md) (usable from the file, the CLI and Python, renderable headless);
- ends with tests and updated documents.

## Consequences

- The roadmap's Phase 2 lists the items in this order with the new gate; STATUS takes **Next up** from it.
- Q36 is settled by the inputs ADR (item 1), Q35 and Q17 by item 2, Q28 and Q37 by item 4. Q28's "after Q26" condition is replaced by items 1–2.
- Scheduled changes leave the list as a separate item: they are input written in the file.
- The adaptive master, acausal connections, ML, FMI and GPU fluids stay listed but are not planned until the gate is met.
