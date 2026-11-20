# Mechanisms (`mech.planar`)

A tree of rigid bodies in the plane, each hanging from its parent (or the ground) by a joint that turns, slides or is welded. One block holds one tree; its state is the joints' coordinates and rates. The decisions are [ADR 0103](../decisions/0103-rigid-body-engine.md) (the solver) and [ADR 0105](../decisions/0105-mechanism-file-form.md) (this form); `simsandbox describe mech.planar` prints this page.

## The `bodies` rows

A block of type `mech.planar` lists its bodies in `bodies`, one object per body, parents before children:

| Column | Unit | Default | Meaning |
| --- | --- | --- | --- |
| `name` | | required | The body's name: letters, digits, `_`; unique in the block |
| `parent` | | required | `ground`, or a body listed before this one |
| `joint` | | required | The joint's name, unique among bodies and joints |
| `kind` | | required | `revolute` (turns), `prismatic` (slides along the joint frame's x axis) or `fixed` (welded) |
| `at` | m | `[0, 0]` | The joint's point in the parent's frame (the world's for `ground`) |
| `turn` | rad | `0` | The joint frame's rotation from the parent's |
| `mass` | kg | required | 0 or more |
| `centre` | m | `[0, 0]` | The centre of mass in the body's frame |
| `inertia` | kg·m² | `0` | The moment of inertia about the centre of mass |
| `damping` | N·m·s (revolute), N·s/m (prismatic) | `0` | Viscous friction in the joint |
| `stiffness` | N·m (revolute), N/m (prismatic) | `0` | A spring in the joint toward `rest` |
| `rest` | rad or m | `0` | The spring's rest coordinate |
| `start` | rad, rad/s or m, m/s | `[0, 0]` | The coordinate and its rate at the start |

A body's frame has its origin at its joint and turns with it: at angle 0 it is the joint frame, so a pole with `centre: [0, 0.5]` stands up. A fixed joint takes no `damping`, `stiffness`, `rest` or `start`. Gravity is the parameter `g` along −y.

A body that floats freely is three joints through massless bodies: two prismatic (along x, then turned π/2 along y) and a revolute one.

## Parameters

Every number of a row is a parameter of the block, the row's value its default: sliders, studies, `params` and wires work on them as on any parameter.

| Parameter | From |
| --- | --- |
| `g` | gravity, m/s², default 9.81 |
| `<body>_mass`, `<body>_inertia`, `<body>_centre_x`, `<body>_centre_y` | the body's columns |
| `<joint>_at_x`, `<joint>_at_y`, `<joint>_turn` | the joint's place |
| `<joint>_damping`, `<joint>_stiffness`, `<joint>_rest` | moving joints |
| `<joint>_angle0`, `<joint>_rate0` (revolute), `<joint>_position0`, `<joint>_speed0` (prismatic) | `start`; they act when the run starts again |

A number changed while the model runs acts from the next evaluation; the joints keep their coordinates.

## Ports

| Port | Direction | Unit | Present |
| --- | --- | --- | --- |
| `<joint>_angle`, `<joint>_rate` | output | rad, rad/s | each revolute joint |
| `<joint>_position`, `<joint>_speed` | output | m, m/s | each prismatic joint |
| `energy` | output | J | always: kinetic, plus the potential of gravity (0 at the world's origin) and of the joints' springs |
| `x`, `y`, `angle`, `vx`, `vy`, `omega` | output over the rows `bodies` | m, m, rad, m/s, m/s, rad/s | always: each body's frame (its joint's point and angle) in the world and its origin's velocity; ids 0, 1, … in row order |
| `<joint>_torque` (revolute), `<joint>_force` (prismatic) | input | N·m, N | each joint named in the setting `actuated` |
| `<body>_fx`, `<body>_fy`, `<body>_torque` | input | N, N, N·m | each body named in the setting `loaded`: a force at its centre of mass in world axes, and a torque |

`actuated` and `loaded` list names separated by commas or spaces (`"actuated": "slider"`). Only listed joints and bodies have inputs, since every input must be wired.

No output depends on an input at the same instant, so a controller wired from the joints back to `actuated` makes no algebraic loop. Wired to typed equations, the block is solved with them as one ODE (fusion), so a controller acts within the step.

A scene draws the bodies with one frame over the rows and a layer per shape in it:

```json
"frames": {"body": {"x": "cartpole.x", "y": "cartpole.y", "angle": "cartpole.angle"}}
```

## In the app

With the block's node selected, the Inspector shows its bodies as a table: one row per body with its name, its joint's name, the joint's kind, its parent (`ground` or a body above it) and its mass, and buttons to move a row up or down or remove it (its children then hang from its parent). `…` opens a row's other columns and two boxes for its inputs: the joint driven (`actuated`) and the body loaded (`loaded`). `+ body` adds a body on a turning joint hanging from the last one.

Each change is one edit of the model (`Edit::SetMechanism`), undone like any other. Renaming a body or a joint renames the ports and parameters it makes, and the wires, sliders, scheduled inputs and views that name them. An input that goes takes its wire with it. A change that `check` refuses (a body moved above its parent, a welded joint that still has damping) waits as a draft with the error at its row and column; the running model stays. The numbers are also parameters, so a slider on `pole_mass` changes the value while the model runs, without editing the row.

## Methods

`method` as for equation blocks: `rk4` (the default), `dopri5`, `trbdf2`, `backward_euler`. RK4 is fourth order; Dormand–Prince controls its error and carries long chains through their whip; TR-BDF2 is second order and damps. None is labelled verified.

## What `check` refuses

At the row and column (`bodies[1].parent`): a name that is not a name, used twice, or `ground`; a parent not listed before the body; an unknown kind or column; a negative mass, inertia, damping or stiffness; numbers that are not finite; a fixed joint with a start or spring. A joint whose bodies beyond it give it no mass or inertia to move. A setting other than `actuated` and `loaded`, or one naming what the mechanism does not have. Two ports or parameters that the rows' names would make the same.

## Example: the cart-pole

```json
{
  "name": "cartpole",
  "type": "mech.planar",
  "settings": {"actuated": "slider"},
  "bodies": [
    {"name": "cart", "parent": "ground", "joint": "slider", "kind": "prismatic", "mass": 1.0},
    {"name": "pole", "parent": "cart", "joint": "hinge", "kind": "revolute",
     "mass": 0.1, "centre": [0, 0.5], "inertia": 0.0083, "start": [0.1, 0]}
  ]
}
```

Outputs `cartpole.slider_position`, `cartpole.slider_speed`, `cartpole.hinge_angle`, `cartpole.hinge_rate`, `cartpole.energy` and the rows; input `cartpole.slider_force`.
