# 0093 — Views from primitive parts: a scene is layers of primitives drawn from signals and tables

- **Status:** Accepted (owner, 2026-11-14; only what the cart-pole needs is built before the Phase 2 gate); stage 1 built 2026-11-19 (the view, frames, primitives, checks and headless renders; see the end); stage 2 built 2026-11-19 (grabbing); stage 3 built 2026-11-19 (the canvas node, renames, editing in the Inspector, `Model.add_scene`)
- **Date:** 2026-11-14
- **See also:** [ADR 0101](0101-scene-primitives-are-parametric-meshes.md) (how primitives are drawn and tested; shape meshes come from models), 2026-11-18
- **See also:** [ADR 0102](0102-scene-bindings-may-name-parameters.md) (bindings may name parameters; a signal of the same name wins), 2026-11-19
- **Builds on:** [ADR 0015](0015-pluggable-views.md) (views, slots), [ADR 0058](0058-inputs-live-scheduled-replayed.md) (views that send input), [ADR 0079](0079-tables-of-members.md) (tables, rows by id), [ADR 0086](0086-views-by-axis-roles-and-mesh-views.md) (rows and meshes in views), [ADR 0057](0057-phase-2-plan.md) (the gate's scene view); answers [Q60](../open-questions.md)

## Context

The Phase 2 gate needs a scene: a cart and a pole drawn as bodies, grabbed with the pointer, the run replayed bit for bit. The ten view types so far each draw one fixed picture from fixed slots; `eq.pendulum` is a view written for one block type. A scene view written the same way for rigid bodies would be the eleventh, and the next model with its own picture would need a twelfth.

The owner's aim (2026-11-14): views highly modular, so that a person or an agent builds any visualization, simple or complex, from primitive parts that stack, driven by what models compute; in the end whole interactive 3D environments, with even lighting driven by models.

What exists already carries most of it: models compute anything, including values per row of a table with stable ids; views bind signals to slots; a view can feed an `input.pointer`, and all input is recorded and replayed; every view renders headless.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| A. A scene view for rigid bodies only, like `eq.pendulum` | Least to build for the gate | Thrown away or special for ever; nothing for the plugin interface to learn from |
| **B. One scene view made of layers; each layer one primitive whose attributes are bound to signals, constants or rows of a table** | Geometry is ordinary model output, so units, replay, headless renders and AI access come with it; grows by adding primitives and attributes, not view types; the plugin interface for views becomes "a new primitive" | Everything that moves must be a signal: more outputs recorded; relative coordinates need an answer of their own (frames, below) |
| C. A scene graph of the view's own (nodes with state, transforms, materials) or an embedded engine's | Familiar from game engines; transforms compose | A second model beside the model, with its own state and calculations outside units, replay and the hash; the "game engine" [ADR 0057](0057-phase-2-plan.md) keeps out of scope |
| D. Views as code (a plugin draws what it likes) | Unlimited | Not buildable by wiring; nothing an agent can check before running; belongs to Phase 2 item 4, and is easier to design after B shows what a view needs |

## Decision

B.

1. **A scene.** A view type `plot.scene` holding an ordered list of **layers**, drawn first to last in one coordinate frame with a unit (2D first: x across, y up, equal scales). It is a view like any other: saved in `controls.views`, outside the model hash, rendered by `render`.
2. **A layer** is one **primitive** and a binding for each of its **attributes**. An attribute is bound to a number (a constant), a scalar signal (one instance), or a quantity over a table's rows (one instance a row, attributes paired by id as views pair rows now). A layer whose attributes are all scalars draws one instance.
3. **Calculations stay in models.** A layer evaluates no expressions. A position, a colour value or a size that must be computed is an output of a block or of an `expression` filter. The one thing a scene computes is where a frame puts what is drawn in it (point 4), a fixed rule with nothing to write. This keeps one language, units checked, and scenes reproducible from recorded data.
4. **Frames.** A scene has named **frames**: coordinate systems, each a pose in another frame (`in`, the scene's own frame if left out). In 2D a pose is `x`, `y` (the frame's origin, in the scene's unit) and `angle` (radians, counter-clockwise, turning before moving: a point p drawn in the frame lies at origin + R(angle)·p in its parent). Each is bound like an attribute: a constant, a scalar signal, or a quantity over a table's rows, which makes one frame a row.
   - A layer names the frame its positions and angles are in (`"in": "pole"`). Its sizes are lengths and unchanged: frames move and turn, they do not scale.
   - Frames chain (`pole` in `cart` in the scene). A layer over rows in a frame over the same table pairs them by id (each body's shape in that body's frame); a frame over rows cannot be a parent of another yet.
   - A frame holds no state and no drawing: it is a pose read from signals at the drawn instant, so a scene is still a function of recorded data. The alternative, every position computed in the model, stays possible and is right when the model needs the position anyway (a contact point); frames are for what only the picture needs (the corners of a body, a wheel's spokes, an arrow on a moving part).
   - **Seen from a frame.** The scene may be viewed from any frame (`"view_from": "cart"`): the picture is drawn in that frame's coordinates, so the cart stands still and the track moves. It changes the picture only.
   - **Grabbing** (point 6) always sends the pointer in the scene's own frame, whatever the layer's frame or `view_from`, so a model's input means one thing; a model wanting it relative subtracts.
   - Units: every frame shares the scene's length unit; a binding in another length unit is refused as wires are (convert in the model), and `angle` takes radians or a dimensionless number.
   - Not decided here: 3D poses (how an orientation is given: three angles, a quaternion or a matrix over a row's values), handedness and the up axis in 3D, frames that scale, and coordinates that are not Cartesian (polar, geographic), which a model converts for now.
5. **Primitives and attributes are declared**, like block types and view slots: each primitive lists its attributes with kind, unit and default, in `describe`; `check` reports a layer whose bindings do not fit, at its path. The first set, all 2D:

   | Primitive | Attributes |
   | --- | --- |
   | `points` | `x`, `y`; `size`, `colour` |
   | `segments` | `x1`, `y1`, `x2`, `y2`; `width`, `colour` |
   | `boxes` | `x`, `y` (centre), `angle`, `width`, `height`; `colour` |
   | `discs` | `x`, `y`, `radius`; `angle` (a mark showing rotation), `colour` |

   `colour` is a number through the view's colour scale, or a named colour of the fixed palette; lengths are in the scene's unit, `size` and line `width` in screen points.
6. **Grabbing.** A layer may name an `input.pointer` block (`"grab": "hand"`). Pressing on one of its instances sends the pointer in scene coordinates, as views send it now, and the instance pressed: `input.pointer` gains an output `picked`, the row's id (0 for a layer of one instance), −1 while nothing is held. What is recorded is the input (`x`, `y`, `down`, `picked`) at step boundaries, so a replay never repeats the hit test and is exact whatever draws it. Headless, the same events come from a run file.
7. **The extent.** The scene's extent is given (`extent`: x and y ranges) or follows what is drawn; saved with the view. Pan and zoom as plots do, except while grabbing.
8. **File form** (the cart-pole's picture; names illustrative):

   ```json
   {"type": "plot.scene", "unit": "m", "extent": {"x": [-3, 3], "y": [-0.5, 2]},
    "frames": {
      "cart": {"x": "cart.x", "y": 0},
      "pole": {"in": "cart", "angle": "pole.theta"}
    },
    "layers": [
      {"draw": "segments", "bind": {"x1": -3, "y1": 0, "x2": 3, "y2": 0}},
      {"draw": "boxes", "in": "cart", "bind": {"x": 0, "y": 0, "width": 0.4, "height": 0.2}, "grab": "hand"},
      {"draw": "boxes", "in": "pole", "bind": {"x": 0, "y": 0.5, "width": 0.04, "height": 1.0}},
      {"draw": "discs", "in": "pole", "bind": {"x": 0, "y": 1.0, "radius": 0.05}, "grab": "hand"}
    ]}
   ```

   The model gives `cart.x` and `pole.theta` and nothing made only for the picture.

   A rigid-body block that gives its bodies as a table (`x`, `y`, `angle` over rows) is drawn by one frame over those rows and a layer for each kind of shape in it, whatever the number of bodies.

### Before the gate (the owner, 2026-11-14: only what the cart-pole needs)

Points 1–8 as written: 2D, the four primitives, constants, scalars and rows, frames (chained; over rows without children) and `view_from`, grabbing with `picked`, drawn by the egui painter in the window and headless alike, shown on the canvas's data layer as a view node, editable in the Inspector as a list of layers. An example scene for a model that exists (the pendulum drawn as a scene, which `eq.pendulum`'s view can then be compared with) before rigid bodies arrive.

### After the gate, each when a model needs it and by its own decision

- More primitives: polylines along rows, polygons, text, arrows, a `field` layer (a grid as an image, which is the heat map), a `mesh` layer (ADR 0086's triangles).
- Existing views as presets of layers where that loses nothing (`plot.scatter`, `plot.3d`, the heat map's points), reducing the number of view types; `eq.pendulum`'s view retired.
- Frames over rows as parents (a wheel table whose rows name their car), by the edges tables already have.
- 3D: `z` and poses in 3D (point 4's open choices), a camera as a frame to view from, depth, drawn by wgpu; then lights as a layer (rows of lights with position, colour and strength from signals) and a camera bound to signals.
- New primitives from plugins (Phase 2 item 4).
- Pictures a model computes whole (a ray tracer written as a model gives a grid; a `field` layer shows it). Interactive rates for that need typed equations on the GPU, which nothing here decides.

## Consequences

- The rigid-body spike has its output contract before it starts: bodies as rows with poses and sizes.
- Signals a scene binds are recorded like any a view reads; scenes of many instances rely on rows recording (by id, thinned by the memory budget).
- Frames are a second place where positions are computed, beside models. It is kept honest by being one fixed rule, stated in point 4 and tested against the same pose composed in typed equations; a position the model also needs is still computed there, and the two must agree to rounding, not to the bit (pictures are outside the hash and the bit-for-bit promise).
- A value in a frame is not a signal: it cannot be plotted or probed. To record where a frame's point is, compute it in the model or a filter.
- `input.pointer` changes: a fourth output, `picked`; run files gain its events; older files and runs read as never picking (−1).
- A view's saved form gains `layers`; the JSON Schema, `describe`, `check`, the Python `Model` (a method adding a scene), `render`, the [headless guide](../guides/headless.md) and [visualization](../architecture/visualization.md) change with it ([changing the file format](../guides/changing-the-file-format.md)).
- Built in `simsandbox-views` without widening the review's W9: hit testing and drawing in the view, no interaction code in `simsandbox-session`.
- Verification: a scene's render checked against a known picture; a grab recorded and replayed bit for bit, in the window's worker and headless.

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none at the level of the list: a scene is a view (an observer). Within views it adds the layer, the primitive and the frame: slots bind one fixed picture and cannot say "draw this, then that, one per row", and without frames every corner of every moving shape would be a model output; over time it removes view types that become presets of layers.

## Stage 1 built (2026-11-19)

Points 1–5, 7 and 8, without grabbing (point 6) or the window's editing. Details in [visualization](../architecture/visualization.md#scenes); code in `crates/simsandbox-views/src/scene/`.

- **Saved form:** as point 8, in `ViewEntry::scene` (`SceneSettings`, typed fields flattened into the entry, so files keep this order and are written back unchanged); the JSON Schema describes it. `grab` is kept and does nothing yet.
- **Choices this ADR left open:** an attribute bound to a signal with no unit (dimensionless, or a filter's) is accepted as a length; `size` and `width` take numbers or dimensionless signals; `colour` as a number goes through viridis over 0 to 1, clamped; unbound it is `sky` from a fixed palette of ten names (Okabe–Ito and greys), and a palette name wins over a signal of the same name; a layer with no rows binding in a frame over rows draws once per row of that frame; `view_from` refuses a frame over rows; a disc's mark is drawn only when its `angle` is bound; a negative size is drawn as its magnitude; an instance with any value missing or not finite (a row absent from one binding) is left out; tables are told apart by block and table name.
- **Example:** `examples/pendulum.ssbx.json` draws its pendulum as a scene beside the `eq.pendulum` view: a support box, and a rod and bob in a frame turned by `bob.angle` inside a fixed frame turned by −π/2 (the block measures its angle from straight down). Its rod is a constant 1 m, as `L` is by default: a scene binds no parameters, so a changed `L` needs a model output to follow.
- **Tested:** each primitive's triangles, bounds and containment at its edges; poses (apply, chain, inverse); frames composed against the same pose in typed equations (to 10⁻¹²); rows paired by id with and without a frame over rows; `view_from`; colours; every `check` error at its path, in the views crate and through `check`; the cache (a still scene is not rebuilt; a new sample or size rebuilds it); a headless render compared with a known picture by exact colours and sizes; the topmost grabbed instance found by containment (`eval::picked`), ready for stage 2.
- **Left for stage 2:** grabbing: `input.pointer`'s `picked` output, pressing on an instance through `eval::picked` (the plot already reports the pointer in view coordinates, which `view_from` must turn back into the scene's), recording and replaying it, pan and zoom held while grabbing. **Stage 3:** a scene as a node on the canvas's data layer (its signals as edges; block renames reaching layer bindings), editing layers in the Inspector, a Python `Model` method that adds a scene.

## Stage 2 built (2026-11-19)

Point 6, grabbing, and point 7's pause of pan and zoom. Details in [visualization](../architecture/visualization.md#scenes); code in `crates/simsandbox-views/src/scene/mod.rs` (the press and hold), `crates/simsandbox-eq/src/input.rs` (`picked`) and `crates/simsandbox-app/src/app/views.rs` (sending it as input).

- **`input.pointer`'s `picked`:** a fourth output and parameter, dimensionless, default −1, suggested range −1 to 2⁵³ (ids are integers held in an `f64`, exact to 2⁵³). Files and runs made before it have no value or event for it and read −1 throughout; the files are written back unchanged. `describe` lists it; the schemas' descriptions of `grab` and a run's `track` say what is recorded.
- **Check:** a layer's `grab` must name an `input.pointer` block of the model (`Available::pointers`, filled by the session and the app); any other name is refused at `layers[k].grab`, and the view is left out with a warning, as every scene problem is.
- **The press:** tested before the plot is drawn, against the picture on screen (the instances last placed, through the plot's transform last shown and the `view_from` pose they were drawn with), by `eval::picked`: the topmost layer with a `grab` whose instance contains the point, exactly. A press under a window or menu, off the plot or before the scene was first drawn grabs nothing and pans as plots do.
- **Held:** each frame the view sends the layer's block `[x, y, 1, picked]` (`View::grabs`), the pointer in the scene's own frame, also when it leaves the plot; release sends `[x, y, 0, −1]` once, at the last position. Pan, zoom, boxed zoom, axis drags and double-click reset are off while held. New settings release what is held. Only the primary button grabs.
- **Recorded and replayed:** the app sends the four values as live input (`SimApp::send_pointer`); the session records a value only when it changes, as for every pointer (ADR 0058), so a press is four events at one time, a drag two per new position and a release two. A replay, in the worker after a rewind, from a saved run or headless (`run MODEL --inputs RUN`, a run file's `track`), applies those events and never repeats the hit test. Nothing about pointers or hit tests is in `simsandbox-session` (review W9).
- **Choices this ADR left open:** a scene needs no `controls.inputs` entry (its layers name their blocks); views that send the pointer through `controls.inputs` (`plot.xy`, the pendulum view) leave `picked` at −1, since they press on no instance; `x` and `y` are sent as they are, not clamped to the block's suggested range; the press is tested against the last drawn picture rather than the newest sample, so it matches what the user sees.
- **Example:** `examples/pendulum-grab.ssbx.json` ("Grab the bob"): the drag example's typed-equations pendulum with an input `picked`, a spring pull while `held = down*(picked >= 0)`, and a scene of a support, the rod, a line from the bob to the hand (its width is `hand.down`, so it shows only while held) and the bob, a disc that grabs with `hand`. `pendulum.ssbx.json` is unchanged: its `eq.pendulum` has no force input.
- **Tested:** `picked`'s default and range, and a drag example file and run without it reading −1 (`simsandbox-eq`, `simsandbox-session`); through the view with simulated egui input: press, drag off the plot, hold still, scroll and release giving the right events and no pan or zoom while held, a press off every grabbing instance sending nothing and panning, the topmost grabbing layer winning, rows sending their ids, the scene's own frame sent under `view_from` and from a turned frame, new settings releasing (`simsandbox-views`); a grab made through the scene tile in the app, saved as a run, replayed by the worker and headless bit for bit (`simsandbox-app`); a grab in a run file replayed by the CLI, the same as the run file itself, an older run with no `picked` leaving the bob alone, and `check` refusing a `grab` that names no `input.pointer` (`simsandbox-cli`).
- **Left for stage 3:** a scene as a node on the canvas's data layer (its signals as edges, and its `grab` as an edge to the pointer block); block renames reaching layer bindings and `grab` (renaming the pointer block now leaves the scene out with a warning); editing layers, `grab` among them, in the Inspector; a Python `Model` method that adds a scene. Not decided: a cursor showing what can be grabbed; grabbing with several pointers (touch) at once.

## Stage 3 built (2026-11-19)

The last of "Before the gate": the scene on the canvas's data layer, editable in the Inspector as a list of layers, and added from Python. Details in [visualization](../architecture/visualization.md#scenes); code in `crates/simsandbox-session/src/observers.rs` (the node, its wires and canvas edits), `crates/simsandbox-session/src/diagram.rs` (renames), `crates/simsandbox-app/src/app/scene.rs` and `src/scene_edit.rs` (the Inspector) and `python/` (`Model.add_scene`).

- **The node:** a scene stays the view node `view:K` it was since stage 1, now with an input per place it binds a signal, named by the path `check` uses (`frames.cart.x`, `layers[1].bind.x2`; `scene::bound`), wired from that signal as a slot is, and an input per layer that grabs (`layers[3].grab`; `scene::grabs`), wired from its `input.pointer` block's `picked` output. A wire dragged into one of its inputs rebinds it (into a `grab`, only from an `input.pointer` block); one taken away sets a pose or an optional attribute back to its default and lets a grab go, and is refused for a required attribute. Both are canvas edits, so canvas undo covers them.
- **Renames:** the generic walk over `controls` renames dotted names (`bob.x`), which already reached a scene's bindings and poses; names standing alone were missed. A layer's `grab`, the keys of `controls.inputs` (the same gap: renaming `hand` in the drag example left its view driving nothing) and groups' members are now renamed, with tests. Deleting a block lets go of the grabs naming it and drops its `controls.inputs` entry. Renaming a filter or a probe reaches a scene's signals from it.
- **The Inspector:** a scene's node picked alone on the canvas, or its tile's title clicked, shows the scene's unit, extent, `view_from`, frames and layers as lists (add, remove, raise and lower layers; each layer's primitive, frame and `grab`; each attribute a number, its default, a palette colour, a signal or a table's quantity, from pickers like the slot pickers; frames added by name and removed, each with its `in` and pose). Edits apply to the tile at once and are saved with the view.
- **Python:** `Model.add_scene(layers, frames=, unit=, extent=, view_from=, canvas=)` takes the file form and refuses, when called, a wrong form, a binding naming no block, filter or probe of the model, and a `grab` naming no `input.pointer` block (`_native.scene_problem`, without building the model); units and whether a block has that output are `check`'s. Tested with point 8's cart-pole on a model giving `cart.x` and `pole.theta`, checked, saved and rendered.
- **Choices this ADR left open:**
  - Inputs are named by `check`'s paths, one per binding (not one per layer), so an error and its wire point to the same place; layers are numbered from 0 in the Inspector to match.
  - An edit that does not check is kept as a draft in the app, beside the scene still drawn from its last form, with the error next to the field at its path; it is not saved, and "discard" drops it. A view left out with a warning on open was the alternative, and loses the scene on save.
  - Inspector edits are not canvas edits: undo does not reach them, as it does not reach a view's slots and style; canvas undo restores views whole, so it can undo an Inspector edit made after the canvas edit it goes back over.
  - A new layer binds its required attributes to numbers (a 0.2 box, a 0.1 disc, a unit segment at the origin) so it draws at once; changing a layer's primitive keeps bindings of the same name and fills the rest.
  - A removed frame's frames and layers move to its parent; a `view_from` naming it is cleared.
  - Frames cannot be renamed yet (remove and add); frame names are letters, digits and `_`.
  - `simsandbox-app FILE --select NODE` picks a canvas node from the command line (for screenshots and checks).
- **Not built:** binding a model parameter (`bob.L`) in a scene, waiting on the owner's decision; a frame's rename; a scene's check errors on its canvas node (the node knows its paths but not the model's units without the session).
- **Tested:** the node's inputs and wires for the grab example and the pendulum's frame, and canvas edits on them (`simsandbox-session`); renames reaching bindings, poses, grabs, `controls.inputs` and groups, and deletions letting go (`simsandbox-session`); the edits on a form (`scene_edit`) and, through the app's methods, adding a layer, binding an attribute, setting `grab` (and the node's new wire), a binding that does not check kept as a draft with its error, and removing the layer, each changing the saved form and the drawn scene as expected, and a canvas edit of the scene undone and redone (`simsandbox-app`); `Model.add_scene` accepting the cart-pole and refusing each wrong form at its path (`python/tests`).

## Parameters, frame renames and errors (2026-11-19)

What stage 3 left: binding a parameter, renaming a frame, and showing a scene's check error where the scene is. Details in [visualization](../architecture/visualization.md#scenes).

- **Parameters** ([ADR 0102](0102-scene-bindings-may-name-parameters.md)): an attribute or a pose may name a block parameter (`"x2": "bob.L"`) where no signal has the name; a signal wins. The session records the parameters views bind in the history every step, so a scene draws one at its value at the drawn sample, rewound and replayed alike. Not a wire on the canvas: the input names it. `examples/pendulum.ssbx.json` draws its rod and bob at `bob.L`; `pendulum-grab.ssbx.json` already draws its rod to the `bob.x`, `bob.y` outputs and is unchanged.
- **Renaming a frame** (`SceneEdit::RenameFrame`): typed over its name in the Inspector; every frame's `in` and every layer's `in` naming it, and `view_from`, follow. A name already a frame's, or not letters, digits and `_`, is refused with nothing changed. Like the other Inspector edits it is not a canvas edit.
