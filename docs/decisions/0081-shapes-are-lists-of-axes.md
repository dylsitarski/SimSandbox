# 0081 — Shapes are lists of named axes

- **Status:** Accepted (owner, 2026-11-07; stages 1 and 2 built 2026-11-08; stage 4's ports built with ADR 0079 stage 1b, 2026-11-08)
- **Date:** 2026-11-07
- **Amends:** [ADR 0080](0080-named-axes.md) point 5 (ports of one or two axes), [ADR 0015](0015-pluggable-views.md) and [ADR 0024](0024-heat-map-axes-and-coordinates.md) (views of fields and their coordinates)
- **Builds on:** [ADR 0080](0080-named-axes.md) (named axes in typed equations), [ADR 0079](0079-tables-of-members.md) (tables: axes whose length changes)
- **Used by:** the generalization of views (next after ADR 0080); models over feature spaces

## Context

A port's shape is one of a closed list: a scalar, a vector of `n`, a 2D grid `nx × ny` (a line when `ny` = 1), `count` points of `dim` coordinates, or a population of `dim` values per member. Recording, probes, run output, Python results, wires and every view are written against that list.

ADR 0080 gave typed equations quantities over any of their block's axes, and kept ports to one or two axes "until a view needs them". Three things make that limit worth removing by design rather than case by case:
- **Feature spaces** (machine learning, design of experiments, data sets): a data set of `N` samples with `D` features is `[sample, feature]`. Its dimensionality is a long axis (`D` cells), not many axes, but models of such data meet three to five axes routinely: images in a batch `[sample, channel, row, column]`, attention `[batch, head, query, key]`, a study over three parameters `[a, b, c]`.
- **Tables** (ADR 0079) are axes whose length changes and whose rows carry ids; populations are such an axis already, kept as a separate shape.
- **Views are to be generalized next.** A view written against `Grid { nx, ny }` cannot show a field over three axes, and each new shape would need each view changed. Views written against axes need not change when shapes grow.

Every current shape is already a list of axes: a scalar has none, a vector one, a grid two, points two (the point and its coordinate), a population two (the member, a table axis, and its values).

Constraints: existing models, files, run files and Python results stay valid; exact bits; recording stays flat and fast; no new user-visible concept.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| **A shape is an ordered list of named axes, each of a fixed length or a table's rows; today's shapes are special cases** | One rule for every shape; any rank; tables fit; views bind to axes, not shapes; feature spaces, studies over three parameters and tensors pass between blocks | Touches the kernel's port shape, wires, recording, output, Python and every view (in stages) |
| Add a `Tensor { dims }` beside today's shapes | Small change | Two ways to say a grid; views handle both; tables still separate |
| Keep ports to two axes; flatten higher ranks into lines | No change | Axis names and lengths lost at the port; views cannot pick or facet; the user unflattens by hand |
| Ragged or nested shapes (lists of lists) | Covers irregular data | Not needed by any planned model; tables with edges (ADR 0079) cover irregular structure with ids |

## Proposal

**1. A shape is a list of axes.** Each axis has:
- a **name**, belonging to the block (`x`, `feature`, `run`);
- a **length**: a fixed number of cells, or the rows of a table (variable between steps, with ids and a structure version, ADR 0079);
- optionally a **coordinate**: an output of the same block giving each cell's position along the axis, with its unit (ADR 0024, as typed equations' axes are now).

Values are stored flat, the first axis fastest, as grids are now. A scalar has no axes. There is no limit on rank in the shape; the typed-equation compiler's own limit (`MAX_AXES`) rises from 6 to 8.

**2. Today's shapes are cases of it**, kept as constructors so blocks need not change at once:

| Today | Axes |
| --- | --- |
| `Scalar` | none |
| `Vector(n)` | `[component: n]` |
| `Grid { nx, ny }` | `[x: nx, y: ny]` (`[x: nx]` when `ny` = 1) |
| `Points { count, dim }` | `[point: count, dim: dim]` |
| `Population { dim }` | `[member: rows, dim: dim]` (moves to tables in ADR 0079 stage 5) |

**3. Wires** connect ports with the same number of axes, the same lengths in order and the same unit; names need not match (each belongs to its block). To meet axes in another order, a block in between reorders them (`on`, ADR 0080). A table axis connects to a table axis, carrying its ids and version (ADR 0079 point 5).

**4. Recording and output.** Probes and the history keep each field's frames flat with its shape stored once, so a field over any number of axes is recorded as a grid is now; the memory budget counts cells. Run output (`--format json`) describes a shape by its axes and order (`"axes": [{"name": "x", "length": 128}, …], "order": "first axis fastest"`), keeping `kind`, `nx` and `ny` for one or two axes during a transition. CSV names a cell by its places (`T[3,2,0]`). Python gives an output of `n1 × … × nk` cells per row as an array of shape `(rows, nk, …, n1)`: the storage order without copying, and today's `(rows, ny, nx)` for grids; `Results.axes(name)` gives the axes' names, lengths and coordinates.

**5. Views bind to axes by role.** A view type declares what it draws in axes, not shapes: a profile one axis and a value; a heat map, contour or surface two; a scatter a point axis and two or three components (or two or three fields over the same axis); `plot.values` none. Binding a field assigns its axes to the view's roles; the coordinate of an axis, if any, places it (ADR 0024). Every axis left over takes one of:
- **pick**: one place along it, chosen by a slider in the tile (the cut of today, generalised; by default the first place);
- **reduce**: the `mean`, `minof`, `maxof` or `sum` along it;
- **facet**: one small panel per place along it, for a short axis.

The choice is saved with the view (`"pick": {"z": 0}`, `"reduce": {"run": "mean"}`, `"facet": "c"`). Views of feature spaces are views over `[point, feature]`: a scatter matrix and parallel coordinates (to build with the views); a projection (principal components, an embedding) is a computation, so it is a block or filter producing fewer axes, checkable headless, not a view.

**6. Compatibility.** Model files do not store port shapes, so they are unchanged. Every block keeps its shapes through the constructors in point 2. Run files and their hashes are unchanged (inputs are scalars). JSON output gains `axes`; Python arrays keep their shapes for zero to two axes.

**Order of work:**
1. The kernel's shape as axes (with today's constructors), wires checked by axes, typed-equation ports of any rank, `MAX_AXES` 8.
2. Recording, probes, history, run output (JSON, CSV) and Python over any rank.
3. Views by axis roles, with pick, reduce and facet; cuts become picks; the generalization of views builds on this.
4. Table axes, with ADR 0079 stage 1; populations move to them with its stage 5.

## As built

**Stages 1 and 2 (2026-11-08).**
- **The shape stays an enum, with a general case.** Point 1 described a shape as a list of axes. `Shape` (a `Copy` enum used in about 170 places) gains `Tensor { rank, dims }` for three or more axes, up to `MAX_PORT_AXES` (8), and every shape gives its axes through `Shape::axes()` (`AxisLen::Cells(n)` or `AxisLen::Rows`) and `dims()`. `Shape::of_axes(&[…])` gives the canonical shape: none a scalar, one or two a `Grid` (as before), more a `Tensor`. So one or two axes keep their old form and every existing consumer and output is unchanged, while new code reads axes. Replacing the enum by a list would have touched every crate at once for no change in behaviour; the enum's variants are point 2's constructors.
- **Points' axes:** point 2's table writes `[point: count, dim: dim]`; stored point by point, the coordinate varies fastest, so `axes()` gives `[dim, count]` (and a population `[dim, rows]`), the first fastest as everywhere.
- **Names:** `PortSpec::axis_names` (`axes_named`), in `axes()` order. Typed equations name each port's axes after its grid lines; a study names its results' axes (`run`, or each swept parameter's over a grid of two, after a vector's own axis, `sample` or `time`). Wires compare lengths in order, not names (`describe_shape` in messages).
- **Typed equations:** declared and defined quantities over up to 8 axes are ports (the "at most two" limit of ADR 0080 point 5 is gone; `MAX_AXES` 6 → 8). A study's vector result over a grid of two parameters, refused before, is a port of three axes.
- **Recording and output (stage 2):** the history records `FieldShape::Tensor`; probes write it to `.npy` (the last axis fastest, as numpy stores it). Run output: every shape in JSON gains `axes` (each axis's `name` and `length`), keeping `kind`, `nx` and `ny` for one or two axes (`kind: "axes"` for more); CSV names a cell of three or more axes by its places (`q[1,2,3]`, first fastest), and keeps flat indices for one or two, so existing CSV is unchanged. Python gives `(rows, nk, …, n1)` and `Results.axes(name)` names them in the array's order. `check` and `describe` name a port's axes (`axes`).
- **Not yet:** views draw tensors in stage 3 (until then they are recorded but not offered to views).

**Stage 4, ports (2026-11-08, with ADR 0079 stage 1b).** `Shape::Rows { dim }` is the rows axis on a port: `axes()` gives `[Rows]` (or `[Cells(dim), Rows]`), and frames carry the rows' ids and the table's version. Recording rows comes with ADR 0079 stage 1c; populations move to tables with its stage 5.
- **Checked:** every example and profiling case gives the same CSV byte for byte and the same JSON apart from the new `axes`, and every example renders to the same image. Tests: a three-axis output through JSON, CSV, the session's history and Python; a typed block reading a study's three-axis result.

## Consequences

- A block's output over any number of axes can be wired, recorded, read from Python and shown, with its axes' names and coordinates kept.
- Views are written once against axis roles; a new shape needs no view changed. Picking, reducing and faceting extra axes replace special cases (cuts across a heat map, a study's results over three parameters shown as lines by run).
- Feature-space models pass `[sample, feature]` data between blocks as data, not as many scalars.
- The work spans the kernel, session, views, CLI and Python; staged so each step keeps every example's results and pictures unchanged.
- **Concepts added:** none. Shapes were already part of ports; this makes them one rule instead of five kinds, and tables' rows one kind of axis.
