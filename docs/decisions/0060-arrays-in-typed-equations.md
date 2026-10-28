# 0060 — Arrays, grids and an implicit method in typed equations

- **Status:** Accepted
- **Date:** 2026-10-27

## Context

Phase 2 item 2 ([ADR 0057](0057-phase-2-plan.md)) extends typed equations so that heavy custom physics needs no rebuild: arrays and grid stencils for the method of lines, faster evaluation ([Q35](../open-questions.md)), and an implicit method for stiff systems ([Q17](../open-questions.md)). Typed agent rules ([Q26](../open-questions.md)) were to be revisited here.

What exists today:
- Typed equations ([ADR 0017](0017-typed-equations.md)) are scalar: each state, input, parameter and defined quantity is one number. A tree-walking interpreter evaluates them, about 2× slower than built-in blocks (measured 2026-10-24, Q35).
- Grid models exist only as built-in Rust blocks (`eq.heat_rod`: 50 cells, RK4 with 10 substeps because explicit diffusion forces small steps). A new grid model needs a developer and a rebuild.
- Ports already carry grids (`Shape::Grid {nx, ny}`), with coordinate outputs for views (`plot.heatmap`, `plot.profile`).
- The master steps blocks with a fixed communication step; snapshots copy every state for rewind.

On 2026-10-27 the owner asked what arrays in typed equations gain over a wrapper or another route. The answer, recorded here:
- **Built-in blocks** are fastest but need a rebuild for every new problem.
- **Generated scalar equations** (`T1` … `T2500`) need no solver change but give huge, slow, unreadable files.
- **WebAssembly plugins** (item 4) give near built-in speed and are sandboxed. But they need code and a compiler, have no unit checking, and hide which cells depend on which, so an implicit method must probe them cell by cell.
- **Python blocks** are not sandboxed and are too slow per step for live play ([ADR 0038](0038-phase-1-revised.md)).
- **Wrapped PDE engines** (FEniCS, OpenFOAM) are heavy installs, out of process and not rewindable; they fit research-scale problems later.
- **Modelica through FMI import** needs an external compiler, and the result cannot be edited in SimSandbox.

Arrays in typed equations keep what typed blocks have: one shareable file, unit checks, sandboxed by construction, rewind, sliders, inputs and replay unchanged. And the solver sees the structure, which makes the implicit method cheap. The other routes stay for their own jobs: engines and FMUs for established codes, plugins for custom code and views.

## Options considered

**Notation**

| Option | Pros | Cons |
| --- | --- | --- |
| **Whole-array equations on a declared grid, with named stencils and declared edges** | Reads like the physics (`dT/dt = alpha*lap(T)`); edges written once; the solver knows each stencil's neighbours | Stencils outside the built-in set wait for a later indexed form |
| Indexed equations (`dT[i]/dt = …(T[i-1] - 2*T[i] + T[i+1])/h^2`) | Any stencil | Long; edges and spacing are easy to get wrong; dependencies must be inferred from index arithmetic |
| Both now | Most flexible | About twice the parser, unit checking and test work before the gate |

**Grids**

| Option | Pros | Cons |
| --- | --- | --- |
| **1D lines and 2D rectangles, regular spacing** | Covers the heat equation and reaction–diffusion; matches the grid views that exist | No 3D or irregular meshes |
| 1D only | Smallest | Reaction–diffusion patterns wait |
| 1D, 2D and 3D | Most general | No 3D field view exists; 3D grids are slow without compiled code |

**Evaluation (Q35)**

| Option | Pros | Cons |
| --- | --- | --- |
| Tree-walking interpreter (now) | Exists | Overhead per node per cell |
| **Compact instruction list, each instruction looping over whole arrays, constants folded at load** | Interpreter overhead paid once per instruction, not per cell, so array models run near compiled speed; scalar models gain too; no new dependency | Scalar models stay slower than built-in blocks |
| Compile to machine code (Cranelift) | Close to built-in speed everywhere | Large dependency; harder to debug and port |

**Implicit method (Q17)**

| Option | Pros | Cons |
| --- | --- | --- |
| **One-step L-stable methods: backward Euler and TR-BDF2, Newton with a sparse Jacobian** | No history between steps, so snapshots, rewind, restarts and input changes work as now; L-stable, so stiff diffusion takes large steps | Second order at most |
| Variable-order BDF (as in CVODE) | Efficient on large stiff problems | Keeps past steps: every rewind, restart or input change drops it to first order |
| IMEX (stiff part implicit) | Cheaper per step | The user must split the equations; pairs of methods need care ([ADR 0023](0023-automatic-fusion.md)) |

**Typed agent rules (Q26)**

| Option | Pros | Cons |
| --- | --- | --- |
| **Defer past the gate** | Neither arrays nor the cart-pole gate need them | Agent models stay built-in a while longer |
| Design them in this item | Sooner | Lengthens item 2; delays the gate |

## Decision

The owner chose arrays in typed equations in the whole-array form, 1D and 2D grids, one-step L-stable implicit methods and deferring typed agent rules, on 2026-10-27.

### Grids

A block declares at most one grid, of one or two axes, each with its own line:

```text
grid x [m] = 0 .. length, 50
grid y [m] = 0 .. width, 40
```

- An axis has a name, a unit, an interval and a number of cells. The ends are expressions of parameters; the number of cells is a whole number written in the equations, so changing it is an equation edit (the block is rebuilt and the run restarts).
- Values sit at cell centres: `x = a + (i + 1/2)·h`, `h = (b − a)/n`. The axis name is itself an array of those positions, usable in expressions (for initial values) and output as the grid's coordinate for views.
- `spacing(x)` is `h` for the axis `x`.

### Arrays

- `state T[x] [K] = expression` (or `T[x, y]`) declares a state on the grid, one value per cell, with one unit. Its initial value is an expression over the axes and parameters, evaluated at every cell; it has no `T0` parameter.
- A defined quantity is an array when its expression uses an array, and a scalar otherwise. Scalars broadcast: `alpha*lap(T)` multiplies every cell by `alpha`.
- `input q[x] [W/m^3]` declares an array input port of the grid's shape. Scalar inputs broadcast as values.
- Parameters stay scalars. Array parameters and tables come later, when a model needs them.
- Every array state and array quantity is an output port of shape `Grid {nx, ny}` (`ny` = 1 on a line), with its axes as coordinate outputs, so `plot.heatmap` and `plot.profile` bind them as they bind `eq.heat_rod`.
- Element-wise arithmetic and every function of [ADR 0017](0017-typed-equations.md) apply cell by cell.
- **Stencils**, second-order central differences on the grid:
  - `lap(T)`: the Laplacian, in T's unit per axis unit squared;
  - `ddx(T)`, `ddy(T)`: first derivatives along an axis (named by the axis: `ddx` for an axis `x`);
  - upwind and higher-order stencils come with the first model that needs them.
- **Reductions** give scalars: `sum(T)`, `mean(T)`, `minof(T)`, `maxof(T)`, and `at(T, i)` or `at(T, i, j)` for one cell (0-based).
- Units are checked as for scalars: an array carries one unit, and a stencil divides it by the axis unit (or its square).
- Arrays are for continuous blocks and formulas. Arrays in maps (cellular automata) come later, which keeps `x[n]` unambiguous.

### Edges

Each array state has an edge condition at each end of each axis, written after its declaration:

```text
edge T: x- = left, x+ = right
edge T: x- gradient = 0
edge u: y periodic
```

- `x-` and `x+` name the low and high ends of axis `x`.
- `= value` holds the edge at that value (Dirichlet); `gradient = value` sets the outward derivative (Neumann); `periodic` joins both ends.
- Values are scalar expressions of parameters, inputs and `t`, in the state's unit (per axis unit for a gradient).
- An end without a condition has zero gradient (insulated). Stencils apply the conditions through ghost cells, so equations never mention the edges.

The typed heat rod is then:

```text
grid x [m] = 0 .. length, 50
state T[x] [K] = peak*exp(-((x - length/2)/width)^2)
edge T: x- = left, x+ = right
dT/dt = alpha*lap(T)
```

### Evaluation (Q35)

- Equations are compiled at load to a compact instruction list: a register machine whose instructions loop over whole arrays (or one value for scalars), with constant parts folded.
- Stencils, reductions and edges are single instructions written in Rust, so the per-cell work runs at compiled speed.
- Scalar typed blocks use the same instruction list. The Lorenz benchmark of Q35 is measured again. As built (2026-10-27, `simsandbox_expr::Program`, measured by `cargo run --release -p simsandbox-eq --example typed_speed`): the typed Lorenz system went from 3.1× to 1.8× the built-in block's time per step, the pendulum from 3.0× to 2.0×.
- Compiling to machine code (Q35 option (c)) is reconsidered only if measurements show the instruction list is too slow for a planned demo.

### Implicit methods (Q17)

- Two new methods for continuous equation blocks and fused groups:
  - `backward_euler`, first order, L-stable;
  - `trbdf2` (TR-BDF2), second order, L-stable, one step with no history.
  - Both take `substeps` like `rk4`.
- Each step solves its nonlinear equations by Newton's method:
  - The Jacobian is found by finite differences. Its sparsity is known from the equations: a cell depends on its stencil neighbours, the cells of reductions and its own cell. Columns that share no row are perturbed together (graph colouring), so a 50 × 50 grid costs a handful of evaluations instead of 2,500.
  - Linear systems are solved by a sparse LU factorisation from a pure-Rust library (`faer` unless a spike shows a problem), reused across Newton iterations while it converges.
- A step whose Newton iteration does not converge is an error naming the block, as other step failures are.
- Labels follow the [verification policy](../guides/verification-policy.md); each method ships with order and stability tests.
- Explicit methods stay the default for continuous blocks. The step-size warning suggests an implicit method for stiff grids.

### Verification and demos

- **The heat equation against its analytic solution**: a sine mode decaying as `exp(−α π² t / L²)`, with second order in space and the method's order in time measured.
- **Stability**: `backward_euler` and `trbdf2` stay bounded at steps far beyond the explicit limit; RK4 does not.
- **The typed heat rod matches `eq.heat_rod`** to rounding on the same grid and method. Amended when built (2026-10-27): the two cannot share a grid. `eq.heat_rod` places its points at nodes with the end temperatures on the ends; typed grids are cell-centred with the edge value at the face. Both are checked against the analytic solution instead, and compared for speed.
- **A reaction–diffusion example** (Gray–Scott on a 2D grid) forms its patterns, with a qualitative check (the pattern's spatial scale) and a speed measurement. Target: a 128 × 128 grid live at real-time speed on a desktop.
- **Speed**: typed array blocks within 1.5× of an equivalent built-in block, measured and recorded.

### Unchanged

- The model file format: equations are text, as today. `simsandbox describe` lists the stencils, reductions and methods; `check` reports array shapes and units.
- Rewind, snapshots, sliders, formulas, inputs and run files: array states are kernel state like any other.
- Fused groups take array blocks; their states concatenate as now.

### As built (step 2, 2026-10-27)

Grids, arrays, stencils, edges and units in 1D are built, with these details settled while building:
- **Plain numbers** written as a grid's end, an array's initial value or an edge value are in the unit of what they set: `grid x [cm] = 0 .. 4, 2` spans 0 to 4 cm, `state T[x] [K] = 0` and `edge T: x- = 0` are 0 K. Other expressions there must balance, as equations do.
- **Stencils** (`lap`, `ddx`) take the name of an array, since the edges belong to it: `lap(2*T)` is refused with a hint to define `D = 2*T` first. Reductions and `at` take any array expression; `at`'s cell number is a whole number written in the equations.
- **What each extra expression may use:** a grid's ends only parameters; an initial value parameters and the axis; an edge value parameters, scalar inputs and `t`.
- **Parameters used only in initial values** are marked as initial conditions (they apply on restart), like the `x0` of scalar states.
- **The axis** is an output port and the coordinate of every array output (`PortSpec::coordinate` became a list for this; `describe` writes `coordinate_of` as a list).
- **Array state names** in the block's spec are `T[0]` … `T[n-1]`. "Start from here" saves array states as a `start` entry.
- **Evaluation:** array instructions loop over cells inside one instruction, with a specialised loop per operand case. The stencil reads the interior straight from the cells and only the two end cells through the ghosts. The rates leave out steps they do not need, such as the cell centres.
- **Speed** (`cargo run --release -p simsandbox-eq --example typed_speed`, RK4, dt = 0.001): the typed rod of 50 cells takes 0.48 µs a step against 0.48 for `eq.heat_rod` (1.0×), and 1.1× for ten rods. That is inside the 1.5× target.

### As built (step 3, 2026-10-28)

The heat equation is the reference problem for arrays (`crates/simsandbox-eq/tests/heat.rs`), measured on a 1 m rod with α = 0.01 m²/s:
- **Second order in space** for each edge kind: a sine held at 0, a cosine insulated, a whole-length sine periodic. RK4 to t = 5 s with a step small enough to leave only the space error; from 10 to 80 cells, the error falls by 3.96–4.11 per halving, to 3.9×10⁻⁵ of the amplitude (1.4×10⁻⁴ periodic, k twice as large).
- **The stencil is exact for its discrete modes:** on 16 cells, dT/dt is −α (4/h²) sin²(kh/2) T in every cell to 10⁻¹², under each edge kind. So the ghost cells reflect the modes as they should.
- **Fourth order in time:** RK4 against the exact solution of the discretised system (20 cells, to t = 2 s); the error falls by 16.1 per halving of the step (2.1×10⁻¹⁰ at 0.2 s).
- **The typed rod and `eq.heat_rod`** both follow the exact Gaussian solution (by images): at t = 2 s, with a 33.3 K peak, the largest error is 0.033 K typed and 0.031 K built in (50 points each, RK4 with 10 substeps of 0.05 s).
- **The typed heat-rod example** (`examples/heat-rod-typed.ssbx.json`, "Typed heat equation (arrays on a grid)") is checked against the same exact solution from its file.

### As built (step 4, 2026-10-28)

`backward_euler` and `trbdf2` (option `substeps`) are methods of every continuous equation block and fused group (`crates/simsandbox-eq/src/implicit.rs`):
- **One Jacobian and factorisation per communication step,** at its start, shared by its substeps, Newton's iterations and both TR-BDF2 stages (γ = 2 − √2, so both use `I − (γ/2)·h·J`). Nothing numeric is carried between steps, so a step's result depends only on its start and runs replay bit for bit after a rewind (tested).
- **The structure:** typed equations give it exactly. `Program::dependencies` runs the rates' instructions on sets of state indices instead of values: a stencil cell depends on its neighbours (the far end when periodic), a reduction on every cell. Other systems (built-in blocks, fused groups) are probed once, at the first implicit step, by perturbing each state alone. An entry that happens to be zero there is missed; that slows Newton's method but cannot change what it converges to. Columns sharing no row are coloured together (3 colours for a 1D stencil).
- **Newton's method** stops when the last correction is below 10⁻¹⁰ of the state's largest value (or 10⁻¹⁰ absolutely), within 50 iterations; otherwise the step fails with a message to take shorter steps or more substeps.
- **`faer` 0.24 without `rayon`:** one thread, so the arithmetic is the same every run. This was chosen without a decision or a measurement; how to use several cores without losing exact replay is [Q39](../open-questions.md). Its fill-reducing ordering is computed once with the structure.
- **Labels:** both real-time (a fixed amount of work per step); backward Euler qualitative (first order, like explicit Euler), TR-BDF2 quantitative.
- **Measured** (`tests/heat.rs`, `examples/typed_speed.rs`):
  - against the discretised system's exact solution (20 cells, to t = 2 s), the error falls by 1.98–1.99 per halving of the step for backward Euler and by 4.00–4.01 for TR-BDF2;
  - with steps of 1 s on 50 cells (36 times RK4's limit), a slow mode plus a fast one decaying at 237 /s: the fast mode is gone and the slow one is within 0.014 (backward Euler) and 2.1×10⁻⁵ (TR-BDF2) of exact; RK4 blows up;
  - `eq.heat_rod`, probed, at 0.5 s steps follows the exact Gaussian to 0.08 K of a 33.3 K peak;
  - per 0.05 s step of the typed rod: 50 cells, 0.02 ms (RK4 with the 3 substeps it needs: 0.003 ms); 200 cells, 0.064 ms (RK4, 40 substeps: 0.087 ms); 800 cells, 0.25 ms (RK4, 640 substeps: 5.0 ms, 20 times slower).

## Consequences

- Q35 is resolved as option (b), with (c) kept for later measurement. Q17 is resolved as option (a) with one-step methods. Q26 stays open, revisited after the gate.
- Order of work:
  1. the instruction list for scalar equations, with the Q35 benchmark;
  2. grids, arrays, stencils, edges and units, in 1D;
  3. the heat-equation reference and the typed heat-rod example;
  4. the implicit methods with the sparse Jacobian;
  5. 2D grids and the Gray–Scott example.
- `eq.heat_rod` stays as a built-in reference; the heat-rod example may switch to the typed form once it matches.
- A new dependency for sparse linear algebra, checked on Windows, macOS and Linux in CI.
- Large grids record a lot of history: a 128 × 128 grid at 100 steps a second is about 13 MB a second. How often grids are recorded is [Q38](../open-questions.md).
- Not in this item: indexed equations, upwind and higher-order stencils, array parameters, arrays in maps, 3D grids, irregular meshes, adaptive steps.
