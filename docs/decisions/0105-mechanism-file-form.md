# 0105 — The mechanism block's file form: a bodies table, numbers as parameters, ports by joint kind

- **Status:** Accepted (owner, 2026-11-20)
- **Date:** 2026-11-20
- **Builds on:** [ADR 0103](0103-rigid-body-engine.md) (a native articulated-body solver, one block per mechanism, joints as rows), [ADR 0075](0075-parameters-are-inputs-with-defaults.md) (parameters are inputs with defaults), [ADR 0093](0093-scenes-from-primitive-layers.md) (frames over rows, grabbing), [ADR 0079](0079-tables-of-members.md) (rows with ids), [ADR 0045](0045-units-in-typed-equations.md) (units on every connection), [ADR 0039](0039-ai-agent-access.md) (headless and AI access), [changing the file format](../guides/changing-the-file-format.md)

## Context

ADR 0103 chose a native articulated-body block for jointed mechanisms: one block per tree of bodies, joints as rows of its bodies table, and a file form left to this decision. That form has to settle:

- **Where the tree is written.** A block's `settings` are text pairs (`BTreeMap<String, String>`), which cannot hold a table.
- **Where the numbers live.** A mass or a length must be a slider, swept by a study, set per use of a model, or wired, as any other parameter is (ADR 0075). It must also have a unit.
- **What the ports are.** A joint's coordinate is in metres or radians depending on its kind, and a port has one unit, so joints of mixed kinds cannot share an array port. Names cannot contain `.`, which separates a block from its port.
- **How a scene draws it.** A frame over rows binds `x`, `y` and `angle` to quantities over a table's rows (ADR 0093 point 8), each one quantity in one unit.

The cart-pole of the Phase 2 gate is the first user. A 50- or 200-link chain, a double pendulum and a free-floating body (the spike's cases) must fit too.

## Options considered

**Where the tree is written**

| Option | Pros | Cons |
| --- | --- | --- |
| **A new optional block field `bodies`: a list of JSON objects, one per body** | Checked by the JSON Schema; read and written as dicts from Python and by agents; edited as a table in the Inspector; errors at a path (`blocks[0].bodies[1].parent`) | A format addition (optional, so older files are unchanged) |
| Text lines, like `equations` (`body pole on cart by hinge: revolute`) | Compact; one more small language | A second grammar to parse, document and keep the Inspector in step with; agents write JSON more reliably than a new syntax |
| Only a CSV file beside the model (as `nodes.csv`) | Compact for long chains | The model is not one file; names and kinds in CSV cells; the cart-pole would need two files |
| Encoded in `settings` strings | No format change | JSON inside strings: unreadable, unchecked |

**Where the numbers live**

| Option | Pros | Cons |
| --- | --- | --- |
| **Every number of a row is a parameter of the block, named `<row>_<column>`; the row holds its default** | Sliders, studies, `params` per use and wiring all work unchanged; units declared per column; `describe` lists them | Many parameters for long chains (about ten per body), listed but not shown unless asked |
| Numbers only in the rows | Simplest | Nothing can be varied live or swept; a second kind of number beside parameters |
| Rows name parameters declared elsewhere (`"mass": "m_pole"`) | Only the chosen numbers become parameters | Two places for one number; a reference to check; Python and agents must keep both in step |

**Ports**

| Option | Pros | Cons |
| --- | --- | --- |
| **Scalars per joint, named by its kind (`hinge_angle`, `slider_position`); bodies' poses and velocities over rows** | Units exact per port; a typed controller wires what it reads; a scene binds the rows as ADR 0093 already shows | Many ports for long chains |
| One array over all joints (`q`, `v`) | Few ports | Mixed units (m and rad) in one port: refused by ADR 0045 |
| One array per joint kind (`angles[revolute]`, `positions[prismatic]`) | Few ports, exact units, suits long chains | A controller of one joint must pick it out of an array; not needed by the gate |

## Decision

The first option of each table.

1. **Block type `mech.planar`** (domain `mech`, 2D; `mech.spatial` later for 3D), in a domain crate `simsandbox-mech` (ADR 0103 point 1).

2. **The `bodies` field.** A new optional field of a block, a list of objects, one per body, in tree order: a body's parent is `ground` or a body listed before it. Every row has a joint to its parent; the tree needs no other table. Only `mech.*` blocks accept the field; `check` refuses it on others.

   | Column | Kind | Unit | Default | Meaning |
   | --- | --- | --- | --- | --- |
   | `name` | name | | required | The body's name, unique in the block |
   | `parent` | name | | required | `ground` or an earlier body |
   | `joint` | name | | required | The joint's name, unique in the block and distinct from body names |
   | `kind` | `revolute`, `prismatic`, `fixed` | | required | Turning, sliding, or welded to the parent |
   | `at` | pair | m | `[0, 0]` | Where the joint sits, in the parent's frame (the world for `ground`) |
   | `turn` | number | rad | `0` | The joint frame's fixed rotation from the parent's; a prismatic joint slides along the joint frame's x |
   | `mass` | number | kg | required | The body's mass, ≥ 0 |
   | `centre` | pair | m | `[0, 0]` | The centre of mass in the body's frame (origin at its joint) |
   | `inertia` | number | kg·m² | `0` | Moment of inertia about the centre of mass, ≥ 0 |
   | `damping` | number | N·m·s/rad or N·s/m | `0` | Viscous friction in the joint |
   | `stiffness` | number | N·m/rad or N/m | `0` | A spring in the joint toward `rest` |
   | `rest` | number | rad or m | `0` | The spring's rest coordinate |
   | `start` | pair | rad, rad/s or m, m/s | `[0, 0]` | The joint's coordinate and rate at the start |

   Units follow the joint's kind where two are listed. Numbers are written in the column's unit, as parameters are. A body's frame has its origin at its joint and turns with it, so a pole with `centre: [0, 0.5]` stands up at angle 0. Gravity is the block's parameter `g` (m/s², default 9.81) along −y.

   For 2D only these three kinds. Free-floating bodies are two prismatic joints and a revolute one through massless bodies, as the spike measured; a `free` kind as shorthand comes when a model needs it. Shapes for contact are not columns here: the contact engine's ADR adds them, keeping these columns, so a mechanism moves to it without being rewritten (ADR 0103 point 3).

3. **Every number is a parameter.** Each numeric column of a row is a parameter of the block named `<owner>_<column>`, with `_x` and `_y` for pairs: body columns under the body's name (`pole_mass`, `pole_centre_y`, `pole_inertia`), joint columns under the joint's name (`hinge_at_x`, `hinge_damping`, `hinge_rest`); the start is `<joint>_<coordinate>0` and `<joint>_<rate>0` (`hinge_angle0`, `hinge_rate0`), as typed equations name their starting values. The row's value is the default; `params` on the block overrides it, as for any block (a used model, a study). The parameters are sliders, swept, set per use and wired as ADR 0075 allows; a geometry or mass changed during a run acts from the next evaluation, the joint coordinates staying as they are.

4. **Ports**, with names from the rows (`check` refuses a name that two ports would share):

   | Port | Direction | Unit | Present |
   | --- | --- | --- | --- |
   | `<joint>_angle`, `<joint>_rate` | output | rad, rad/s | each revolute joint |
   | `<joint>_position`, `<joint>_speed` | output | m, m/s | each prismatic joint |
   | `<joint>_torque` / `<joint>_force` | input | N·m / N | each joint listed in `actuated` |
   | `<body>_fx`, `<body>_fy`, `<body>_torque` | input | N, N, N·m | each body listed in `loaded`: a force at its centre of mass in world axes, and a torque |
   | `x`, `y`, `angle`, `vx`, `vy`, `omega` | output, over the table `bodies` | m, m, rad, m/s, m/s, rad/s | always: each body's joint origin and its velocity, ids by row order from 0 |
   | `energy` | output | J | always: kinetic plus potential, for checks |

   `actuated` and `loaded` are settings listing names (`"actuated": "slider"`), so a mechanism has only the inputs it uses: an input is a signal with no default and must be wired ([ADR 0074](0074-model-interfaces.md)), so a port for every joint and body would force wires to nothing.

   A scene draws a mechanism with one frame over the `bodies` rows and a layer per shape (ADR 0093 point 8). A grabbed body's `picked` is its row's id. Arrays per joint kind (the third option) are added when a long chain's controller needs them.

5. **Solving.** `method` as for equation blocks: `rk4` by default, `dopri5`, `trbdf2`, `backward_euler`, each with ADR 0103's labels. The block is an `OdeSystem` that gives its Jacobian's structure (ADR 0104) and fuses with typed equations wired to it (ADR 0095).

6. **`check`** refuses at the field's path: an unknown kind; a parent not listed before the body or a cycle; a name used twice, or `ground` as a body; a negative mass or inertia; a body whose subtree has no mass or inertia along its joint (the articulated inertia would be 0); a name in `actuated` or `loaded` that is not a joint or body. `describe mech.planar` lists the columns with their units and defaults, the joint kinds, the parameters and ports a row creates, and the labels.

7. **Python and the CLI.** `Model.add_block("cartpole", "mech.planar", bodies=[...], settings={"actuated": "slider"})` (one new keyword argument, `bodies`) takes the rows as dicts, checked at the call (ADR 0100 point 3); a list comprehension builds a chain. The CLI needs nothing new: `check`, `describe`, `run`, `render` work as for any block.

8. **The cart-pole** (the gate's model, controller and scene abridged):

   ```json
   {
     "name": "cartpole",
     "type": "mech.planar",
     "settings": {"actuated": "slider", "loaded": "pole"},
     "bodies": [
       {"name": "cart", "parent": "ground", "joint": "slider", "kind": "prismatic", "mass": 1.0},
       {"name": "pole", "parent": "cart", "joint": "hinge", "kind": "revolute",
        "mass": 0.1, "centre": [0, 0.5], "inertia": 0.0083, "damping": 0.001, "start": [0.1, 0]}
     ]
   }
   ```

   It gives `cartpole.slider_position`, `cartpole.slider_speed`, `cartpole.hinge_angle`, `cartpole.hinge_rate`, the rows `x`, `y`, `angle`, …, and `energy`; it takes `cartpole.slider_force` (the controller's output) and `cartpole.pole_fx`, `cartpole.pole_fy` (the grab's spring); its parameters include `cartpole.pole_mass` and `cartpole.hinge_angle0`.

## Consequences

- The model format gains an optional block field (`bodies`), described in `model.schema.json`, round-trip tested, with a changelog line; older files are unchanged ([changing the file format](../guides/changing-the-file-format.md)).
- `simsandbox-mech` holds the spike's 2D core grown into the block, with ADR 0103 point 7's verification tests and tests of every `check` refusal.
- The Inspector edits `bodies` as a table, one row per body; the canvas shows one node with the ports above.
- A long chain lists a row per body and has about ten parameters per body; Python builds the rows. A CSV source for `bodies` (ADR 0092) waits for a model that needs it.
- Not here: closed loops (a second table of loop joints, by its own decision), joint limits, `free` joints, 3D (`mech.spatial`), contact shapes (the contact engine's ADR).

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none beyond ADR 0103's mechanism block. Bodies and joints are rows of its table (ADR 0079); their numbers are parameters (ADR 0075); their poses are rows a scene draws (ADR 0093).

## Built (2026-11-20)

- `crates/simsandbox-mech`: `tree.rs` (the dynamics), `rows.rs` (the rows), `block.rs` (`Mechanism`, `MechBlock`, `mech.planar`); the reference is [docs/reference/mechanisms.md](../reference/mechanisms.md), printed by `describe mech.planar`.
- The `bodies` field reaches the block as its definition lines, one JSON object each, as a study's `sample` does; the file refuses it on any type but `mech.*` and beside `equations` (`blocks[k].bodies`). Errors name the row and column (`bodies[1].parent`).
- The block is in the equation domain (`Domain::Equations`, not a domain of its own as ADR 0103 point 1 suggested), so the equation domain's group solver fuses it with typed equations. Its rows outputs are written whenever its outputs are, after each step and when the master resolves outputs.
- `x`, `y`, `angle` are each body's frame (its joint's point), as point 4 says; a load acts at the body's centre of mass.
- `euler` is accepted as for every ODE block but not listed; `rk4` is the default.
- Python checks the rows when `add_block` is called through a new native `block_problem`, which builds the one block alone.
- Two ports or parameters meeting by name is checked, though the names' suffixes make it hard to reach.
- Not built yet: the Inspector's table for `bodies` (the canvas shows the block and its ports; the file, the CLI and Python edit the rows).
- Verification ([policy](../guides/verification-policy.md#reference-problems-in-use)): ABA against CRBA with RNEA on 250 random trees with every joint kind, springs, damping and loads (1e−12); the double pendulum and the cart-pole against their Lagrange equations (1e−13), RK4 order 4; the pendulum period against the elliptic integral (1e−10); a damped slider against its linear solution (1e−12); momentum and energy kept (1e−9); restored into a fresh block bit for bit with all four methods; every refusal of point 6. The cart-pole example's grab replays bit for bit from a run file (`simsandbox-cli/tests/cli.rs`).

