# Typed equations: the language

_Last updated: 2026-11-17_

The lines of an `eq.equations` block's `equations` ([ADR 0017](../decisions/0017-typed-equations.md)), the primary way to build a model ([ADR 0091](../decisions/0091-equations-first-hosting-for-existing-codes.md)). This page is the whole language as the code reads it; the decisions behind each part are in the ADRs it links. Every block of lines marked `ssbx` here is parsed by a test (`crates/simsandbox-eq/tests/reference.rs`), which also checks that every function is listed, so the page cannot fall behind the code.

A block is one of three kinds, by its equations:

| Kind | Has | Method | Options |
| --- | --- | --- | --- |
| Continuous | a line `dx/dt = …` or `x' = …` | `rk4` (if left out), `euler`, `dopri5`, `backward_euler`, `trbdf2` | `substeps` (1 to 1e6, default 1); for `dopri5` `rtol` (1e-8) and `atol` (1e-10) |
| Map | a line `x[n+1] = …` | `iterate` | none |
| Formulas | only lines `name = …` | `evaluate` | none |

A block cannot be both continuous and a map. Options are written beside the method's name: `"method": {"name": "dopri5", "rtol": 1e-6}`.

```ssbx
# a damped oscillator
dx/dt = v
v' = -k*x - c*v
E = 0.5*v^2 + 0.5*k*x^2
```

This block has states `x` and `v`, parameters `k`, `c`, `x0`, `v0` (each 1 unless set) and outputs `x`, `v`, `E`.

## Lines

- One statement a line. Blank lines are skipped.
- **Comments** ([ADR 0109](../decisions/0109-parameter-roles-key-parameters-port-groups.md)): `#` starts a comment, to the end of the line (not inside `"…"`).
  - Comment lines before the first statement are the block's **summary**.
  - A comment after a statement **describes** what the line declares: a parameter, input or state, or the quantity an equation defines (`dx/dt = v  # position` describes `x`). `describe`, `check` and the Inspector show it.
  - Other comment lines are skipped.
- A statement begins with its word and one space (`state x`). The words are not reserved: `param = 4` is a formula defining `param`.
- **Names** start with a letter or `_` and continue with letters, digits or `_`; any alphabet (`dθ/dt = ω`). Not allowed as names: `t` (time), `pi`, the names of the [functions](#functions), and `n` in a map.
- **Numbers**: `3`, `0.5`, `.5`, `1e-3`.
- Errors give `line L, column C`.

## Expressions

Loosest first:

```text
compare := sum [ ('<' | '<=' | '>' | '>=') sum ]
sum     := term { ('+' | '-') term }
term    := unary { ('*' | '/') unary }
unary   := ('-' | '+') unary | power
power   := atom [ '^' unary ]
atom    := number | name | '(' compare ')' | name '(' compare { ',' compare } ')'
```

- `-x^2` is `-(x^2)`; `2^3^2` is `2^(3^2)`; `2^-k` is allowed.
- A comparison gives 1 or 0, one at most without parentheses: `(a < b)*(b < c)` for both. There is no `==`, `and`, `or`, `not`: multiply for "and", `1 - c` for "not".
- `2x` is an error: write `2*x`.
- There is no indexing: `T[2]` is an error; write `at(T, 2)`.
- An expression nests at most 1000 levels; a chain of operators counts (`a + b + c` nests to the left), so a sum of more than about 1000 terms must be split into named parts.

## Units

A unit is written in square brackets: after a declared name (`param m [kg] = 2`), or at the end of an equation for its left side (`E = m*v^2/2 [J]`; for a rate, the rate's unit: `dv/dt = F/m [m/s^2]`). [ADR 0045](../decisions/0045-units-in-typed-equations.md).

- Unit text: symbols with SI prefixes multiplied, divided and raised to whole powers (`N*s/m`, `kg/m^3`, `1/s`); `simsandbox describe units` lists the symbols. `a/b*c` is refused as ambiguous: `a*c/b`. Temperatures with an offset (`°C`) are refused: use `K`.
- A block with no unit anywhere is not checked. With one, every line must balance, `t` is in seconds, and a name without a unit is dimensionless.
- `+`, `-` and comparisons need the same dimension on both sides; a number is dimensionless (`x + 1` with `x` in metres fails); a right side that is exactly `0` balances anything. `^` takes a constant whole power (or a half, of even powers). `sin`, `exp`, `ln` and the like take and give dimensionless values.
- A plain number as a default, an initial value, an end of a grid, an edge value or a rule's value is in the unit of what it sets.
- A rule balances as an equation does: its condition compares like with like and is a comparison or a number without a unit; each value it gives is in its column's unit.
- Values are computed in SI and shown in the unit declared.

**Brackets after a name** are a list of axes when they follow it without a space and every item is a declared axis or table; otherwise a unit. With `grid m = 3`, `state x[m]` is an array over axis `m` and `state x [m]` a scalar in metres. Both may be given: `state T[x] [K]`.

## Statements

Axes (`grid`, `table`, `edges`, `cells`, `sides`) may be declared anywhere in the block; `edges`, `cells` and `sides` need what they are `of` on a line above. A block has at most 8 axes of all kinds.

### `input`

```text
input NAME [axes] [unit] {, NAME [axes] [unit]}
```

An input port. A formula that uses an input depends on it at the same instant (direct feedthrough, [ADR 0040](../decisions/0040-feedthrough-and-algebraic-loops.md)); a state does not.

```ssbx
input F [N], drag [N]
param m [kg] = 1000
state v [m/s] = 0
dv/dt = (F - drag)/m
```

### `param`

```text
param NAME [unit] [= number] [as ROLE]    one value; 1 if not given
param NAME[axes] [unit] [= number]        a value per cell: parameters NAME[0], NAME[1], …
param NAME[axes] [unit] = file "m.csv"    a value per cell from a file's column
param NAME[table] [unit] [= expression]   a value per row, which does not change
```

- A default is a plain number, not an expression.
- `as ROLE` says what kind of value it is ([ADR 0109](../decisions/0109-parameter-roles-key-parameters-port-groups.md)): `property` (the default), `geometry`, `start` (acts when the run starts again), `drive` (pushes the model from outside), `tuning` (gains, limits, set points) or `numerics`. A state's initial value (`x0`) is `start`. Another word is an error at its line. Interfaces group parameters by role: `param k1 [N/m] = -10 as tuning  # gain on position`.
- Every other name used in an equation, a grid's ends, an array's initial value, an edge value or a [rule](#rules-between-steps) is a parameter too, with default 1, in order of first use (those used only in rules after the others).
- A value per row is kept with the rows and filled from the table's `start` file; it is an output of the block and has no entry among the parameters. It needs a block with states (`dx/dt = …`, `x' = …` or `x[n+1] = …`).

### `state`

```text
state NAME [unit] [= number]                one state; its initial value is parameter NAME0
state NAME[axes] [unit] [= expression]      a state per cell, or per row of a table
```

- A scalar state need not be declared: `dx/dt = …` makes `x` a state with initial value `x0` (default 1). An array state must be.
- An array's initial value is an expression of parameters, the axes and a starting mesh's coordinates (0 if not given); it has no `x0`.
- Every declared state needs its line `dx/dt = …` or `x[n+1] = …`.

### Equations

```text
dNAME/dt = expression [unit]      a continuous state's rate (also NAME' = …)
NAME[n+1] = expression [unit]     a map state's next value; on the right x and x[n] are the present one
NAME = expression [unit]          a formula: an output computed at every instant
```

- A formula may use formulas defined above it; a state's equation may use any formula.
- In a map, `x[n]` is `x`, the present value, in every equation and in the [rules](#rules-between-steps); an initial value uses no state.
- A formula is over the axes its right side uses; a state's right side over fewer axes than the state is spread over them.

```ssbx
x[n+1] = r*x[n]*(1 - x[n])
```

### `grid`: an axis of cells

```text
grid NAME [unit] = low .. high, COUNT     cells of equal width between the ends
grid NAME = COUNT                         cells numbered 0 to COUNT − 1 (no unit)
```

[ADR 0060](../decisions/0060-arrays-in-typed-equations.md), [ADR 0080](../decisions/0080-named-axes.md). The ends are expressions of parameters. The axis's name in an expression is the centre of each cell, and is an output of the block. Quantities are over the axes they name or use, and combine by axis name.

```ssbx
grid x [m] = 0 .. length, 50
param length [m] = 1
param alpha [m^2/s] = 1e-4
state T[x] [K] = 300
edge T: x- = 400, x+ gradient = 0
dT/dt = alpha*lap(T)
```

### `edge`: the ends of an array

```text
edge ARRAY: item {, item}
item := AXIS- = value | AXIS+ = value | AXIS- gradient = value | AXIS+ gradient = value | AXIS periodic
```

`x-` is the low end and `x+` the high. A value is held at the end's face; a gradient is the outward derivative; `periodic` joins both ends. An end with no condition is insulated (gradient 0). Values are expressions of parameters, scalar inputs and `t`. Stencils and `prev`/`next` read these.

### `table`, `start`: rows that come and go

```text
table NAME [max N]                 at most N rows (100000 if not given)
start NAME = N                     N rows to begin with
start NAME = file "rows.csv"       rows from a file
```

[ADR 0079](../decisions/0079-tables-of-members.md). A table has one `start` at most. A table is an axis whose rows have ids and change only between steps, by [rules](#rules-between-steps). A quantity over a table is over it alone. A CSV file has a header of names (a unit may follow in brackets), an optional `id` column, and a column for each state over the table it sets. Files are found from the model's folder.

```ssbx
table pts max 12
start pts = 3
state age[pts] = id(pts)
age[n+1] = age + 1
remove pts where age > 6
```

A table with no `start`, rules or edges that an input is over follows the input's rows by id.

### `edges`, `cells`, `sides`: rows that join rows

```text
edges NAME of TABLE [max N]            each row joins two rows of TABLE
cells NAME of TABLE [max N]: SHAPE     each row has the corners of a cell among TABLE's rows
sides NAME of CELLS [max N]            the sides between cells, kept from the cells
start EDGES = chain                    an edge from each row to the next
start EDGES = file "wires.csv"         columns from and to: the ids of the rows joined
start CELLS = grid x 0 .. 1 by 8, y 0 .. 1 by 8
start CELLS = file "part.msh" at x, y, z
```

[ADR 0085](../decisions/0085-meshes-are-tables-of-cells.md). Shapes: `segment`, `triangle`, `quad`, `tetra`, `hexa`, `prism`, `pyramid`, `simplex d`, `box d` (d from 1 to 6). A starting grid makes the points and the cells (simplices, or boxes for `quad`, `hexa`, `box d`); a mesh file is Gmsh ASCII. A grid or a mesh file starts `cells` (`cells c of pts: segment` for a line of them), not `edges`. Each coordinate a start names must be a state over the points table. Prisms and pyramids have no `sides` yet.

```ssbx
table pts
start pts = 4
state p[pts] = (id(pts) < 0.5)*4
edges segs of pts
start segs = chain
param g[segs] = 1 + id(segs)
Q = g*(from_end(p, segs) - to_end(p, segs))
dp/dt = -outflow(Q, segs)
```

```ssbx
table pts
state x[pts] [m]
state y[pts] [m]
x[n+1] = x
y[n+1] = y
cells c of pts: quad
start c = grid x 0 .. 1 by 2, y 0 .. 1 by 2
sides f of c
A = measure(c, x, y) [m^2]
L = measure(f, x, y) [m]
```

### Rules between steps

```text
remove TABLE where condition
add TABLE [from AXIS] [where condition] [: state = expression, …]
split EDGES_OR_CELLS where condition [: state = transfer, …]
merge EDGES_OR_CELLS where condition [: state = transfer, …]
```

Rules run in the order written, after each step.

- `remove` takes out the rows where the condition holds, with the edges at them.
- `add` puts in one row when its condition holds, or with `from` one for each place of an axis or table where it holds, its values computed there. A state not given takes its starting value.
- `split` and `merge` refine and coarsen: an edge split has a new row at its middle; a cell split is bisected (simplices) or cut in 2^d (boxes), kept balanced. Transfers say what the new or remaining rows hold:

| | On split | On merge |
| --- | --- | --- |
| A state over an edge table's points | `copy` (if not given), `mean`, or an expression | `mean` (if not given), `copy`, or an expression |
| A state over cells | `copy` (if not given), `share`, or an expression | `mean` (if not given), `sum`, `copy`, or an expression |
| A state over cells' points | `mean` (if not given), or an expression | left as they are |

A transfer is its word alone; a quantity of the same name is written in parentheses, as any expression may be: with `param share = 0.25`, `u = share` is the transfer and `u = (share)` the parameter.

```ssbx
table pts max 12
start pts = 3
grid k = 3
state age[pts] = id(pts)
state round = 0
round[n+1] = round + 1
age[n+1] = age + 1
remove pts where age > 6
add pts from pts where age < 1.5: age = 0
add pts from k where round > 10*(k + 1) - 0.5: age = 0
```

## Functions

### Of numbers

Applied cell by cell to arrays; arguments over different axes spread over all of them.

| Functions | Arguments | Units |
| --- | --- | --- |
| `sin` `cos` `tan` `asin` `acos` `atan` `sinh` `cosh` `tanh` `exp` `ln` `log` (the same as `ln`) `log10` | 1 | Dimensionless in and out |
| `sqrt` | 1 | Halves each power, which must be even |
| `abs` `floor` `ceil` | 1 | The argument's |
| `sign` | 1 | Gives −1, 0 or 1 |
| `min` `max` | 2 | Both the same; kept |
| `atan2` | 2 | Both the same; dimensionless out |
| `pow` | 2 | As `^` |

`pi` is the number.

### Of arrays on grids

"An array's name" means the name of a declared or defined array, not an expression: define one above for an expression (`D = k*T`, then `lap(D)`).

| Call | Gives | Unit |
| --- | --- | --- |
| `lap(T)` | The sum of second differences along each of `T`'s axes, which need one unit | `T`'s over the axis's squared |
| `ddx(T)`, `dd(T, x)` | The central difference along axis `x` (`dd` followed by the axis's name, or the axis second) | `T`'s over the axis's |
| `prev(T)`, `next(T)`, `prev(T, x)` | The neighbouring cell's value along the axis (named when `T` has several). Beyond an end: the held value, the end cell's plus the gradient times the spacing, or the far end | `T`'s |
| `spacing(x)` | The width of grid axis `x`'s cells, one value (a table's rows have none) | The axis's |
| `sum(e)` `mean(e)` `minof(e)` `maxof(e)` | One value over all cells; over more than 4096 cells sums add in fixed chunks, the same bits on any machine ([ADR 0077](../decisions/0077-sums-in-fixed-chunks.md)) | `e`'s |
| `sum(e, x)` and the others | The same along one axis, over those left | `e`'s |
| `argminof(e)` `argmaxof(e)` | The cell's number, counted from 0 (the first of equals); with an axis, the place along it | None |
| `at(e, i)`, `at(e, i, j)` | One cell: a whole number for each axis, or one computed number (`at(T, argmaxof(T))`) | `e`'s |
| `on(e, a, b)` | `e` with its axis `a` read as axis `b` of the same length | `e`'s |
| `solve(A, b)` | `x` with `A·x = b`: `A` over two axes of one length, `b` over one of them; NaN when singular | `b`'s over `A`'s |
| `rcond(A)` | How well conditioned `A` is, from 0 (singular) to 1 | None |

### Of tables, edges and meshes

| Call | Gives | Unit |
| --- | --- | --- |
| `id(tbl)` | Each row's id | None |
| `rows(tbl)` | The number of rows now | None |
| `of(T, i)` | `T` in the row whose id is `i`; NaN if there is none | `T`'s; `i` has none |
| `from_end(T, e)`, `to_end(T, e)` | `T` (over the points) at each edge's first and second end | `T`'s |
| `corner(T, c, k)` | `T` at corner `k` of each cell | `T`'s |
| `outflow(F, e)` | Over an edge table: for each point, the flows `F` leaving along its edges less those arriving. Over sides: for each cell, the sum of `F` over its sides, outward | `F`'s |
| `centre(T, c)` | The mean of `T` over each row's corners | `T`'s |
| `measure(c, x, y)` | Each row's length, area or volume from its corners' coordinates | The coordinates', which need one unit, to the power of the shape's dimension: a segment's length in it, an area in its square, a volume in its cube, a point (a segment's side) none |
| `normal(f, k, x, y)` | Component `k` (a whole number from 0) of each side's unit normal, outward from its inner cell | None; the coordinates need one unit |
| `inner(u, f)`, `outer(u, f)` | `u` (over the cells) in the cell on each side of a side | `u`'s |
| `across(u, f)` | `u` in the outer cell, or at the boundary the inner | `u`'s |
| `boundary(f)` | 1 for a side on the boundary, else 0 | None |

`lap`, `dd`, `prev`, `next`, `spacing` and `on` work along grids, not tables: rows have no neighbours by place, only by the edges and sides that join them. Where a call's later arguments name a table or an axis, or are a plain number (a corner, a component), they have no unit to check.

A study's `reduce` formulas have functions of their own over a run in time (`final`, `mean`, `min`, `max`, `amplitude`, `period`, `first_time`, `at_times`, `at_samples`; [headless guide](../guides/headless.md)); they are not part of this language.

## What a block offers

- **Ports**: the inputs; then each state and each formula as an output of its name; then each grid axis as an output of its cells' centres; then each table of cells started from a grid or mesh as an output of its corners.
- **Shapes**: one value; a line of cells (one axis); a grid (two); a tensor (more, up to 8); rows (over a table).
- **Parameters**: those declared, in order (with `x0` for `state x = …` and `m[0]`, `m[1]`, … for a parameter per cell); then every other name in order of first use (in the equations, the grids' ends, initial values and edge values, then in the rules); then `x0` for each scalar state without one.

`simsandbox check FILE` reports all of these for a model as written.
