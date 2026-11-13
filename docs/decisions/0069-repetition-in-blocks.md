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
- `describe eq.equations` and the headless guide list both. A data file as an array parameter's values, and network grids, wait for a model that needs them (built 2026-11-13, step 3).

**Step 3 (2026-11-13): networks from files and array parameters from a file.** Asked for by the owner on 2026-11-13, with an example checked against Kirchhoff's laws. Built on tables and edges ([ADR 0079](0079-tables-of-members.md)) rather than a grid of their own; the forms below are mine.
- **Tables from CSV files:** `start nodes = file "nodes.csv"` starts any table from a file read from the model's folder (cached by its bytes; read when the block is first compiled, as a mesh file is): a row per line; an `id` column gives the rows' ids (whole numbers from 0, distinct), else 0, 1, …; every other column the starting values of the quantity over the table it names, converted from a unit in its header (`g [mS]`). An edge table's file (`start wires = file "wires.csv"`) has `from` and `to`, the ids of the rows each edge joins. A column naming no quantity, an edge to an id not among the rows, a unit of another kind, a short line or a repeated id is refused with the file's name and the reason.
- **Parameters over rows:** `param g[wires] [S] = 0.001` (a number or an expression, as a state's start) is a value per row that does not change: compiled as a state over the rows that holds (`dg/dt = 0`, or `g[n+1] = g` in a block of maps), so it is stored, rewound, carried through rules (copied on split) and output as states are, and a file sets it. It is not a parameter of the Inspector (one per row, and rows come and go). A right side that is exactly `0` now balances in any unit (zero is zero in every unit).
- **Flows along edges:** `outflow(I, wires)` is, for each row, the sum of `I` over the edges leaving it less the sum over those arriving (each edge directed from `from` to `to`), as `outflow` over a mesh's sides is out of each cell (ADR 0085 step 3). ADR 0079 point 2's `sumover` and `other` are not built: no model has needed a sum over a row's edges that is not a flow.
- **Array parameters from a file:** `param m[k] [kg] = file "masses.csv"` gives `m[0]`, `m[1]`, … from the column named `m` (or the file's only column), one value per cell, converted from its header's unit; a different number of values is refused. Each remains a parameter that files and the Inspector may set.
- **Part of the model:** both kinds of file are in the run hash (`block.mesh:path` for starts, as mesh files are, `block.param:path` for parameters).
- **Example:** `examples/networks/bridge.ssbx.json` with `nodes.csv` and `wires.csv`: an unbalanced Wheatstone bridge, each free node charging a small capacitance until the currents into it balance. It is in the app's Examples menu (see below).
- **Tested** (`crates/simsandbox-session/tests/networks.rs`): the bridge's free nodes reach Kirchhoff's voltages (230/74 V and (10 + 5·230/74)/9 V) to 1e-9, the file's ids are kept, the held nodes hold exactly, mS are read as S, the power supplied equals the power dissipated to 1e-12, and the currents balance at each free node; changing `wires.csv` changes the model hash and the results; six kinds of bad file are refused with their reasons, and a relative file without a folder; the Platoon with its masses from a file in tonnes gives the same bits as with them written out in kg; a parameter per row in a block of maps; diffusion along a chain's edges keeps its total to 1e-12. Every other example unchanged byte for byte.

**Examples with files (2026-11-13, owner's request).** A built-in example may carry the files it reads (`Example::files`, names and contents built into the app with the model). Opening one in the app writes the model and its files into a fresh folder of its own under the system's temporary folder (`simsandbox-examples/<name>/`, written again each time it is opened) and loads it from there; the model stays unsaved (no path), reading its files from that folder. **Save as** writes the example's files beside the saved model, unless files of those names are already there, so the saved copy opens on its own. Headless, `Example::folder()` unpacks an example once per process for tools and tests (`registry::in_folder(e.folder().as_deref(), …)`); every test over all examples loads each from its folder. Python's `Model.example(name)` takes that folder as the model's (`Model.folder`); `Model.save` elsewhere writes the model alone, so copy the example's files beside it (they are in `Model.folder`). Tested: the bridge opens from the menu and runs, Save as writes its two CSV files beside it, the saved model opens from its path, and an example opened after it reads no folder of the bridge's.
