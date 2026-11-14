# 0093 — Views from primitive parts: a scene is layers of primitives drawn from signals and tables

- **Status:** Proposed (drafted at the owner's request, 2026-11-14; the owner has decided that only what the cart-pole needs is built before the Phase 2 gate)
- **Date:** 2026-11-14
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

## Decision (proposed)

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
