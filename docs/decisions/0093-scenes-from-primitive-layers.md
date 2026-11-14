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
| **B. One scene view made of layers; each layer one primitive whose attributes are bound to signals, constants or rows of a table** | Geometry is ordinary model output, so units, replay, headless renders and AI access come with it; grows by adding primitives and attributes, not view types; the plugin interface for views becomes "a new primitive" | Everything drawn must be computed as signals: more outputs recorded; no hierarchy of transforms (models compute world positions) |
| C. A scene graph of the view's own (nodes, transforms, materials) or an embedded engine's | Familiar from game engines; transforms compose | A second model beside the model, with its own state and calculations outside units, replay and the hash; the "game engine" [ADR 0057](0057-phase-2-plan.md) keeps out of scope |
| D. Views as code (a plugin draws what it likes) | Unlimited | Not buildable by wiring; nothing an agent can check before running; belongs to Phase 2 item 4, and is easier to design after B shows what a view needs |

## Decision (proposed)

B.

1. **A scene.** A view type `plot.scene` holding an ordered list of **layers**, drawn first to last in one coordinate frame with a unit (2D first: x across, y up, equal scales). It is a view like any other: saved in `controls.views`, outside the model hash, rendered by `render`.
2. **A layer** is one **primitive** and a binding for each of its **attributes**. An attribute is bound to a number (a constant), a scalar signal (one instance), or a quantity over a table's rows (one instance a row, attributes paired by id as views pair rows now). A layer whose attributes are all scalars draws one instance.
3. **Calculations stay in models.** A layer does no arithmetic: no expressions, no transforms of its own. A position, a colour value or a size that must be computed is an output of a block or of an `expression` filter. This is what keeps one language, units checked, and scenes reproducible from recorded data.
4. **Primitives and attributes are declared**, like block types and view slots: each primitive lists its attributes with kind, unit and default, in `describe`; `check` reports a layer whose bindings do not fit, at its path. The first set, all 2D:

   | Primitive | Attributes |
   | --- | --- |
   | `points` | `x`, `y`; `size`, `colour` |
   | `segments` | `x1`, `y1`, `x2`, `y2`; `width`, `colour` |
   | `boxes` | `x`, `y` (centre), `angle`, `width`, `height`; `colour` |
   | `discs` | `x`, `y`, `radius`; `angle` (a mark showing rotation), `colour` |

   `colour` is a number through the view's colour scale, or a named colour of the fixed palette; lengths are in the scene's unit, `size` and line `width` in screen points.
5. **Grabbing.** A layer may name an `input.pointer` block (`"grab": "hand"`). Pressing on one of its instances sends the pointer in scene coordinates, as views send it now, and the instance pressed: `input.pointer` gains an output `picked`, the row's id (0 for a layer of one instance), −1 while nothing is held. What is recorded is the input (`x`, `y`, `down`, `picked`) at step boundaries, so a replay never repeats the hit test and is exact whatever draws it. Headless, the same events come from a run file.
6. **The frame.** The scene's extent is given (`extent`: x and y ranges) or follows what is drawn; saved with the view. Pan and zoom as plots do, except while grabbing.
7. **File form** (the cart-pole's picture; names illustrative):

   ```json
   {"type": "plot.scene", "unit": "m", "extent": {"x": [-3, 3], "y": [-0.5, 2]},
    "layers": [
      {"draw": "segments", "bind": {"x1": -3, "y1": 0, "x2": 3, "y2": 0}},
      {"draw": "boxes", "bind": {"x": "cart.x", "y": 0, "angle": 0, "width": 0.4, "height": 0.2}, "grab": "hand"},
      {"draw": "segments", "bind": {"x1": "cart.x", "y1": 0, "x2": "pole.tip_x", "y2": "pole.tip_y", "width": 4}},
      {"draw": "discs", "bind": {"x": "pole.tip_x", "y": "pole.tip_y", "radius": 0.05}, "grab": "hand"}
    ]}
   ```

   A rigid-body block that gives its bodies as a table (`x`, `y`, `angle`, `width`, `height` over rows) is drawn by one `boxes` layer, whatever the number of bodies.

### Before the gate (the owner, 2026-11-14: only what the cart-pole needs)

Points 1–7 as written: 2D, the four primitives, constants, scalars and rows, grabbing with `picked`, drawn by the egui painter in the window and headless alike, shown on the canvas's data layer as a view node, editable in the Inspector as a list of layers. An example scene for a model that exists (the pendulum drawn as a scene, which `eq.pendulum`'s view can then be compared with) before rigid bodies arrive.

### After the gate, each when a model needs it and by its own decision

- More primitives: polylines along rows, polygons, text, arrows, a `field` layer (a grid as an image, which is the heat map), a `mesh` layer (ADR 0086's triangles).
- Existing views as presets of layers where that loses nothing (`plot.scatter`, `plot.3d`, the heat map's points), reducing the number of view types; `eq.pendulum`'s view retired.
- 3D: `z` and orientations, a camera, depth, drawn by wgpu; then lights as a layer (rows of lights with position, colour and strength from signals) and a camera bound to signals.
- New primitives from plugins (Phase 2 item 4).
- Pictures a model computes whole (a ray tracer written as a model gives a grid; a `field` layer shows it). Interactive rates for that need typed equations on the GPU, which nothing here decides.

## Consequences

- The rigid-body spike has its output contract before it starts: bodies as rows with poses and sizes.
- Signals a scene binds are recorded like any a view reads; scenes of many instances rely on rows recording (by id, thinned by the memory budget).
- No hierarchy of transforms: a pole's tip is computed by the model (`tip_x = x + L*sin(theta)`). If models repeat such lines often, a later decision may add a filter that composes poses; not a view feature.
- `input.pointer` changes: a fourth output, `picked`; run files gain its events; older files and runs read as never picking (−1).
- A view's saved form gains `layers`; the JSON Schema, `describe`, `check`, the Python `Model` (a method adding a scene), `render`, the [headless guide](../guides/headless.md) and [visualization](../architecture/visualization.md) change with it ([changing the file format](../guides/changing-the-file-format.md)).
- Built in `simsandbox-views` without widening the review's W9: hit testing and drawing in the view, no interaction code in `simsandbox-session`.
- Verification: a scene's render checked against a known picture; a grab recorded and replayed bit for bit, in the window's worker and headless.

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none at the level of the list: a scene is a view (an observer). Within views it adds the layer and the primitive, because slots bind one fixed picture and cannot say "draw this, then that, one per row"; over time it removes view types that become presets of layers.
