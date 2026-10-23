# 0043 — Subsystems, and parameters with formulas

- **Status:** Accepted
- **Date:** 2026-10-23

## Context

Phase 1 item 7 is subsystems: groups of blocks collapsed into reusable blocks ([ADR 0038](0038-phase-1-revised.md)). [Q30](../open-questions.md#resolved) asked how one parameter can set several blocks (the grazing field size, written twice). The owner decided on 2026-10-23: subsystems are defined in the model file and used many times; declared parameters feed block parameters through formulas; the canvas opens a subsystem to edit it and groups a selection into one.

## Options considered

| Question | Options | Chosen |
| --- | --- | --- |
| Where definitions live | **In the model file, used by name**; separate library files; inline copies | In the file: one definition for every use, and the file stays self-contained; libraries can come later |
| Shared parameters (Q30) | Declared parameters fanning out as plain values; **declared parameters with formulas**; inner parameters exposed as they are | Formulas: `K = 2*capacity` as well as `size = size` |
| How a subsystem runs | A composite Block (like `FusedSystem`, equations only); **flattened into its blocks when the file loads** | Flattening: any domain can be inside, and fusion, loops, start states, views and filters see ordinary blocks |
| Canvas, first version | **Open to edit inside**, **group a selection**; import from another file | Both chosen; importing waits for a library |

## Decision

- **File format.** A top-level `params` declares the model's own parameters (`{"size": {"value": 40, "unit": "m", "description": ..., "range": [10, 200]}}`). `subsystems` defines groups by name, each with `params` (defaults), ordered `inputs` (`{"name", "to": ["block.port", ...]}`) and `outputs` (`{"name", "from": "block.port"}`), `blocks` and `connections`. A block whose `type` names a subsystem uses it: it has no method; its `params` set the subsystem's parameters, plus values for this use only as `"inner.param"`; `starts` holds start states by inner block. Any block parameter may be a number or a formula string of the declared parameters in its scope (the model's, or inside a subsystem its own), using the typed-equation expression language. Names are letters, digits and `_`.
- **Loading flattens.** Each use expands into its blocks named `instance/inner` (nested `a/b/c`), wired through its ports; formulas are evaluated. The kernel model, signals (`car1/car.v`), sliders, sweeps and start states use these flat names; the file as written is kept for saving and the canvas. Errors keep the path where the problem is written (`subsystems.follower.blocks[0].equations`, `blocks[2].params.tau`). Every subsystem is also built on its own, so one nobody uses yet is checked, and a subsystem that contains itself is refused.
- **Parameters at run time.** Declared parameters form a plan evaluated in order; changing one recomputes every formula and sets the block parameters they feed. The panel lists the model's parameters, then each use's, then every block's; values a formula sets are shown with the formula, not edited. Parameters of uses inside another subsystem are fixed by that subsystem. Saving writes back the model's values, each use's values (formulas stay formulas) and only the inner values that differ from the subsystem's.
- **Canvas.** Uses are nodes with the subsystem's ports (double outline); double-click or "Open" shows the inside, with a breadcrumb back, the list of uses and a rename field; its ports are nodes (`in:u`, `out:y`) that are wired, renamed, added and deleted like blocks, and wires to a renamed or removed port follow in every use. Editing inside changes every use. Shift-click and Shift-drag select several blocks; "Make subsystem" moves the selection into a new subsystem: wires that cross become ports, model parameters its formulas use come in as its parameters, and flat names in sliders and views follow (`car.v` → `group1/car.v`).
- **Headless** ([ADR 0039](0039-ai-agent-access.md)): `check` lists `subsystems` (parameters, ports, uses), flat `blocks` and each parameter's `formula`; `render --diagram --subsystem NAME` and `Model.render(diagram=True, subsystem=...)` draw an inside; Python `Model.declare`, `Model.set(key, "formula")` and `Model.group(blocks)`.
- **Examples.** The grazing model's field size is one model parameter used by both blocks; the platoon example uses one `follower` subsystem three times.

## Consequences

- Q30 is resolved; Phase 1 item 7 is built.
- Units of declared parameters are descriptive: formulas are not checked for dimensions.
- Sweeps cannot vary declared parameters yet, only block parameters; filters cannot use declared parameters.
- A rename inside a subsystem updates flat names in controls by pattern (`/old.` → `/new.`), which also matches a block of that name inside another subsystem.
- Not built: subsystem libraries or importing from another file, ungrouping, undo, and agent update orders for blocks inside subsystems in the side panel.
