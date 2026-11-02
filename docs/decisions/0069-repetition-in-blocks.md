# 0069 — Repetition and recursion of structure belong in blocks

- **Status:** Accepted
- **Date:** 2026-11-02
- **Extends:** [ADR 0060](0060-arrays-in-typed-equations.md) (arrays in typed equations), [ADR 0068](0068-one-kind-of-nested-model.md) (one kind of nested model)

## Context

ADR 0068 refuses a model that uses itself. The owner asked on 2026-11-02 whether self-use would give useful recursion, then, doubting it, whether repetition and recursion should rather live within blocks ([Q50](../open-questions.md)). Typed equations already have 1D and 2D arrays and grids (ADR 0060). The typed heat rod runs at 1.0–1.1× the built-in rod's time per step. ADR 0060 left out array parameters, indexed equations and irregular meshes.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| **In blocks: arrays and grids, extended where they fall short** | One block, near built-in speed, unit-checked, the implicit solver sees its structure, one node and one field in views; N is a parameter | Each element has the same equations; a part made of several block types or clocks cannot be an array |
| In nesting: `"count": n` on a use, then recursion bounded by a load-time condition | Repeats any part, mixed types and clocks included; each copy can be opened | A block, a node and a name per copy; recursion grows as 2^depth and builds trees of fixed shape only; needs a condition language, limits and folding in the diagram |
| Both now | Covers everything | Pays nesting's costs before any model needs them |

## Decision

1. **Models keep refusing to use themselves** (ADR 0068 point 3). There is no recursion in nesting.
2. **Repetition of one element lives in blocks**, as arrays and grids. They are extended when a model needs it, in this order:
   1. **Array parameters:** a value per element (a mass per car), written as a list or read from a data file.
   2. **Neighbours by index:** a shift or indexed form so element *i* reads element *i−1* (a car reading the one ahead), with declared edges as for stencils.
   3. **Network grids:** nodes and edges read from a file, with operators along edges. They cover trees and networks (rivers, vessels, power and pipe networks) without recursion.
3. **A few copies of a part are made by Duplicate** on the canvas (a use or a block, with its values), and are then edited apart.
4. **`count` on uses waits** for a model that needs many copies of a part arrays cannot hold (different block types, clocks or domains). Bounded recursion waits until asked for.
5. **Structure that changes during a run** belongs to the agent domain ([ADR 0034](0034-agent-domain.md)).

**Order of work**, after ADR 0068's merge: Duplicate on the canvas; then array parameters and neighbours by index, checked by the Platoon example as one array block giving the same results as its three uses; network grids when a model needs them.

## Consequences

- Large repeated structures stay fast, compact and visible as fields.
- Nesting stays for parts that differ, as ADR 0066 and ADR 0068 have it.
- A part combining block types repeated many times takes copies by hand (Duplicate) until `count` is built.

## As built

**Step 1 (2026-11-02): Duplicate.**
- `Edit::Duplicate { blocks }` copies blocks or uses beside themselves (40, 60 canvas units off), named by the next free number (`car3` → `car4`), with their values, equations, settings and start states. The wires among the copied blocks are copied too; wires to the rest are not.
- A copy of a use keeps its original's clock: a use sharing its user's clock because it is wired, copied unwired, is written `"clock": "shared"`.
- On the canvas: "Duplicate" (or "Duplicate N blocks") in a block's or use's menu, and Ctrl+D on the selection. Inside an expanded use the edit goes to the model it uses. Python: `Model.duplicate(blocks)`.
- Tests: a use and its values copied; two copied with the wire between them; equations copied; Python.

**Step 2 (2026-11-02): array parameters and neighbours by index.**
- **Array parameters:** `param mass[k] [kg] = 1000` declares one parameter per cell, `mass[0]`, `mass[1]`, …, each with the default and the unit. Files, sliders, sweeps and the Inspector set them like any parameter (`"params": {"mass[2]": 2500}`, a formula too). In the equations `mass` is the array of them, read in place from their registers.
- **Neighbours:** `prev(v)` and `next(v)` are the cell before and after along the first axis, or along a named one (`prev(v, y)`), as `ArrayOp::Shift` and one array instruction. Beyond an end they take the array's edge: a held value as it is (`edge v: k- = lead`), not mirrored as a stencil does; a gradient extends the end cell (insulated: the end cell itself); periodic wraps. Units follow the array; edge values may be inputs, so a scalar input feeds the first cell.
- **The check:** the Platoon as one block (`grid k = 0 .. 3, 3`, `e = prev(v) - v`, the leader's speed through `v`'s edge, `mass[1]` and `mass[2]` set) gives the same bits at every step as its three uses of `follower`.
- `describe eq.equations` and the headless guide list both. A data file as an array parameter's values, and network grids, wait for a model that needs them.

