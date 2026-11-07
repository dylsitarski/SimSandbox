# 0080 — Named axes: arrays of different sizes in one block

- **Status:** Proposed
- **Date:** 2026-11-07
- **Amends:** [ADR 0060](0060-arrays-in-typed-equations.md) (one grid per block; a second `grid` line makes it 2D), [ADR 0070](0070-studies-are-blocks.md) (study results: one value per run)
- **Used by:** [ADR 0078](0078-adaptive-studies.md) step 2 (`solve`, Levenberg–Marquardt fitting); [ADR 0079](0079-tables-of-members.md) (quantities over tables beside grids)

## Context

In typed equations a block has one grid. Each `grid` line adds a dimension to it, and every array has all its cells: with `grid i = 2` and `grid k = 3`, every array has 6 cells. This was enough for fields on a grid (ADR 0060) and a study's results over one index (ADR 0070).

Building ADR 0078 step 2 showed what it cannot express:
- **Levenberg–Marquardt** needs several sizes in one strategy: d + 1 trial points, m residuals per point, a Jacobian of m × d, the d × d matrix JᵀJ, and d-value vectors. `solve(A, b)` itself needs a d × d matrix beside a d-value vector.
- **A run's residuals:** a study gives one value per run for each result, and its reductions are over the whole run (`final`, `mean`, `max`, …). Fitting needs each run's model-minus-data at the data's times: a vector per run.
- **Tables** (ADR 0079) put quantities over rows (`T[cells]`, `kp[pts]`) beside grids and index axes in one block: the same need.

Named axes as in array languages (dimensions with names, as in xarray or Julia's named arrays) answer all three.

Constraints: the existing models give the same results (one example, Gray–Scott, uses two axes); exact bits on any machine and core count; fixed grids keep their fast code; no new user-visible concept.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| **Named, independent axes: each quantity over the axes it names or uses; broadcasting by name; reductions along a named axis** | One rule for fields, index axes, matrices and tables; equations read as written (`sum(J*J', r)`); shapes checked by name, not position | The array compiler gains broadcasting and reductions along an axis; ports and views must say which axis is which |
| Positional shapes (NumPy-style: an array is n1 × n2 × …) | Familiar to some | Broadcasting by position is error-prone; axes lose their meaning (x or y, run or residual) |
| Separate blocks per shape, wired together | No language change | A strategy is split across many blocks; JᵀJ still needs an array over two axes in one block |
| Linear algebra as built-in block types (`linalg.solve`) | Small | A special block per operation; strategies no longer read as equations |

## Proposal

**1. Axes are independent.** Each `grid` line declares an axis: spatial (`grid x [m] = 0 .. L, 50`, with coordinates and a unit) or an index (`grid k = 5`, numbered 0 to 4, no unit). Declaring an axis no longer adds a dimension to the others.
- **Declared quantities** name their axes: `state T[x, y]`, `param m[k]`, `input r[obs, run]`.
- **Defined quantities** take the axes their right side uses, in the order the `grid` lines are written (the first varies fastest in storage, as `x` does today). `a = x` is over `x` alone; `T*u[k]` is over `T`'s axes and `k`.
- **An axis name** used as a value is its coordinates (spatial) or its numbers (index), over that axis alone.

**2. Broadcasting by name.** An operation between quantities over different axes is over all of their axes: `a[i] + b[k]` is over `[i, k]`, cell (i, k) taking `a[i] + b[k]`. Scalars go with every cell, as now. Quantities over the same axes combine cell by cell, as now (the existing loops, unchanged).

**3. Along an axis.**
- **Reductions:** `sum(T)`, `mean`, `minof`, `maxof` over all of a quantity's cells, as now; `sum(T, k)` along axis `k` alone, leaving the others. `argminof(T, k)` gives the place along `k`. Sums along an axis follow the fixed rule of [ADR 0077](0077-sums-in-fixed-chunks.md) for each line.
- **Places:** `at(T, i, j)` reads by place in axis order, as now.
- **Stencils** (`lap`, `ddx`, `ddy`, `prev`, `next`) act along the spatial axes a quantity has.
- **Relabelling:** `on(T, a, b)` is `T` with its axis `a` named `b` (both the same length), so a quantity can meet itself across two axes: `JTJ = sum(J*on(J, p, q), r)` is over `[p, q]`.

**4. Linear solve.** `solve(A, b)`: `A` over two axes of the same length `[i, j]`, `b` over `[i]`; the result `x` over `[j]`, with `sum(A*x, j) = b`. LU with partial pivoting in a fixed order, so exact bits on any machine; a singular `A` gives NaN, not an error mid-run.

**5. Ports and views.** A quantity's port has its axes' lengths and names: one or two axes as today (lines and 2D fields, with their coordinates), more than two allowed inside a block but not on its ports until a view needs them. Wires connect ports with the same lengths and unit, as now; names belong to each block and need not match.

**6. Vectors per run in studies.** A `reduce` result may be an array, over an axis of the run's own:
- `at_samples(expr, data)`: `expr` at each sample time of the recorded-data block `data` (an `input.file`) within the run, for model-minus-data residuals;
- `at_times(expr, t1, t2, …)`: `expr` at the given times (the step at or after each).

The study's output for such a result is over two axes: the result's own (fastest) and the runs. A strategy declares it so: `input res[obs, run]`.

**7. Compatibility.**
- Every quantity in today's examples is over all of its block's axes or none, so results are unchanged; the examples are compared byte for byte when built.
- One visible change: an axis's own output (`gs.x` in Gray–Scott) is over that axis alone, not repeated over the whole grid. Views that place a field by its coordinates read them from the axes, so they show the same picture.
- `state T[x]` in a block with axes `x` and `y` was refused ("write T[x, y]"); it now declares a quantity over `x` alone.

**8. Exactness and speed.** Broadcasting and reductions along an axis are new instructions of the array compiler, each cell computed from the same values in the same order whatever the split; quantities over the same axes keep the existing loops and phases (ADR 0062 step 3).

**Order of work:**
1. Named axes in typed equations: independent axes, declared and inferred axes, broadcasting by name, reductions along an axis, `on`; ports of one or two axes with names; every example unchanged.
2. `solve(A, b)`.
3. Array results per run (`at_samples`, `at_times`), study outputs over the result's axis and the runs.
4. Then ADR 0078 step 2: Levenberg–Marquardt fitting as a model.

Later: tables as axes whose length changes (ADR 0079 stage 1); ports of more than two axes.

## Consequences

- Strategies, matrices and per-run vectors are written as equations, in one block.
- ADR 0079's quantities over tables are axes of a third kind: their length changes between steps.
- The array compiler grows broadcasting, reductions along an axis and `solve`; error messages name axes ("`a[i] + b[i]`: `b` is over `k`, not `i`").
- **Concepts added:** none. Axes are part of Members (arrays and their cells); naming them makes several sizes possible in one block.
