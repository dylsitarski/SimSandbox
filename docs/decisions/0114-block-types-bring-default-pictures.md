# 0114 — Block types bring default pictures

- **Status:** Accepted (owner, 2026-11-27)
- **Date:** 2026-11-27
- **Builds on:**
  - [ADR 0113](0113-views-bind-to-interfaces.md) (templates; models offer pictures to their users);
  - [ADR 0108](0108-interface-synchronized-projections.md) (the Stage; "components bring default pictures");
  - [ADR 0093](0093-scenes-from-primitive-layers.md) (scenes) and [ADR 0102](0102-scene-bindings-may-name-parameters.md) (bindings may name parameters);
  - [ADR 0016](0016-model-curated-views.md) (a model with no views opens with one chosen by fit).
- **Block contract:** adds an optional capability trait ([block contract](../architecture/block-contract.md)); the `Block` trait is unchanged.

## Context

With [ADR 0113](0113-views-bind-to-interfaces.md), a *model* can offer pictures to the models that use it. A *block type* cannot. A model with a mechanism therefore shows nothing of its bodies until someone writes a scene with frames bound to its rows. The [interface report](../research/2026-11-20-interface-report.md) (section 6) and ADR 0108 ask that "a model with a mechanism has a scene before anyone writes one; a person restyles it instead of writing frames". The owner asked for this ADR on 2026-11-27.

A block's picture depends on how the block is configured. A mechanism's bodies, joints and geometry come from its rows, and its parameter names (`pole_centre_y`) from its bodies' names. So the picture must come from the configured block, not from the type alone.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| A. A fixed template per type, in the registry's `TypeInfo` | Simple; `describe` lists it | Cannot follow a block's configuration (which bodies, their lengths) |
| **B. A capability of the configured block: it offers templates built from its configuration** | Draws what this block is (each body with its own geometry); one contract for built-ins and plugins | One more optional trait |
| C. A view type per block type (as `eq.pendulum`'s view) | Full freedom | Hard-coded drawing: not editable, not data, against ADR 0113's direction of retiring these |

## Decision

**B.**

1. **A capability trait, `Pictures`.** A block may offer pictures: each a name, a description, and a template in the form of ADR 0113 (a view, usually a scene, with inputs, parameters and offered frames, its own names written with `$`), as JSON text.
   - The kernel stays free of JSON and of drawing: it carries the text, and the file crate reads it as a template.
   - It is an optional capability, like the others ([ADR 0098](0098-the-kernel-holds-the-contract.md)). A block without it offers none, which is most blocks.
2. **Named by the block, like a use's.** A block `cartpole` offering `bodies` offers the template `cartpole/bodies`, with the same rules as a used model's pictures (ADR 0113 point 5).
   - Each input not bound otherwise binds by name to the block's port of that name (`x` to `cartpole.x`).
   - A template may also bind the block's parameters by name (ADR 0102), so a body is drawn at its own geometry and follows a slider.
   - It computes nothing (ADR 0093 point 3): a position it needs is a port or a parameter of the block.
3. **Found where templates are found.** Each place that expands views has the model's templates, those its used models offer, and now those its blocks offer, from the loaded model. A block inside a used model offers its pictures under the use's path (`left/cartpole/bodies`).
4. **Pictures layer into one view.** A picture is a template layer (ADR 0113), so any number of them, a block's or a used model's, share one scene. Each is drawn in the frame its layer names, in the scene's own frame if none, and in the order of the layers.
   - **Opening a model that writes no views.** It draws, besides the view chosen by fit (ADR 0016), one scene holding every picture its own blocks offer, layered in the scene's own frame in the order of the blocks. The blocks of one model draw in the same coordinates (a mechanism's bodies, its particles, a field over the same ground), so overlaid is where they belong.
     - Pictures in another length unit go into a scene of their own, one scene per unit, since a scene has one unit.
     - The scene's extent follows what is drawn.
   - **Adding a picture** (a block's or a used model's) offers two ways:
     - **into a new view**, a scene of the picture's unit holding it alone;
     - **into an existing scene** of the same unit, as a new layer on top, in its own frame.

     The layer can then be moved, placed in another frame (side by side, as "Two balancing poles" places its two cart-poles), restyled through its parameters, or removed, in the scene's form like any layer.
   - **Where pictures are offered.** A model with views draws what it writes, and its blocks' pictures are offered:
     - in "+ view" on the Stage (a "Pictures" section);
     - as **Draw** on the Overview and in the Inspector (ADR 0108 point 4), each with "new view" or "into Scene N";
     - from Python, with `add_view(template=…)` for a new view, and a template layer in `add_scene` or in an existing scene's `layers`.
   - A picture shown by default becomes part of the file only when the model is saved with that view, as any view does (ADR 0016).
   - It changes no result: views and templates are outside the model hash (ADR 0113).
5. **The mechanism's first picture, `bodies`** (`mech.planar`), built from its rows:
   - Each body is drawn in its pose frame, as a stick from its joint's point to its centre and on, with a disc at its centre and its axes (as the cart-pole's `body_axes`). Its joint is drawn as a small ring.
   - Sizes come from the body's own geometry parameters, bound by name, so they follow changes to them.
   - It offers each body's frame by the body's name (`pole`), so a parent can draw on a body.
   - Its pointer input `hand` lets a press grab a body, if the model wires a pointer into the mechanism.
   - Other built-in types bring theirs when a model needs them: the pendulum's (retiring `eq.pendulum`'s hard-coded view, [Q62](../open-questions.md)), point sets and populations.
6. **Discoverable** ([ADR 0039](0039-ai-agent-access.md)):
   - `check` lists the pictures each block offers (with inputs, parameters and offered frames) beside the file's templates.
   - The card lists them with each block, as it lists a use's.
   - `describe TYPE` says whether the type offers pictures and names them.
   - A plugin's block offers pictures through the same trait.
7. **Typed equations offer none yet.** A way for typed equations to declare a picture is for its own decision when a model needs one.

**Settled by the owner (2026-11-27):**
- A block's pictures are named as a use's are (`cartpole/bodies`).
- Pictures may be layered into one view, both when a model with no views opens and when a picture is added (point 4).

**Order of work:**

1. The trait and its reading:
   - pictures from loaded blocks, named `block/name`, bound by name to ports and parameters;
   - offered under a use's path;
   - found wherever views are expanded;
   - `check`, the card and `describe`.
2. The mechanism's `bodies`. The cart-pole then draws `cartpole/bodies` beside its own picture in a test, and a model with a mechanism and no views opens with it.
3. Pictures in "+ view", on the Overview and in the Inspector, each into a new view or into an existing scene. The Stage's first increment's **Draw** (2026-11-27) gains the second way.

## Consequences

- **A model built from block types gets pictures for free,** and its author restyles them through the template's parameters, or writes its own.
- **Block type authors gain one more thing to provide:** optional, and expected of types whose outputs have an obvious picture (bodies, particles, fields).
- **A picture's look can change between versions of a type** without changing any result. A view that names it draws the newer look.
- **Documents to update when built:**
  - the [block contract](../architecture/block-contract.md);
  - the [visualization architecture](../architecture/visualization.md);
  - the mechanism reference;
  - the headless guide.

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none. A block's pictures are templates (ADR 0113), offered by a part as a used model's are; the trait is a capability of the existing Block concept.

## Built so far

- **Step 1 (2026-11-27), the trait, its reading and its listing:**
  - `simsandbox_core::block::{Picture, Pictures}` and `Block::as_pictures` (default `None`), as every capability is reached; the `Block` trait's required methods are unchanged.
  - `templates::block_template` marks a picture's template as offered by a block (`offered_by`), so its inputs bind by name to `BLOCK.input` (a port, or a parameter, ADR 0102), where a used model's bind to `USE/input`.
  - `session::block_pictures(model, ids)` reads them from the loaded model by flat name, so a block in a used model offers `u/d/dot`. `Session::templates()` is every template the model can draw: the file's, its used models' and its blocks'. Every place that expands views uses it: the session's recording, the views' tiles, `check`, `render`, the card; the app's published model carries it.
  - `check` lists `pictures` beside `templates`, the card lists a block's `pictures` (and in Markdown "Pictures it offers"), and `describe TYPE` names its example's.
  - Python's quick scene check defers a block's picture to `check`, since it is known only once the model is built.
  - Tests (`simsandbox-session/tests/pictures.rs`), with a test block offering a picture:
    - bound to its port;
    - drawn;
    - listed by `check`, the card and `describe`;
    - offered under a use's path.

