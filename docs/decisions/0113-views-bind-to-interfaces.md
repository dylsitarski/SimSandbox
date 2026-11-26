# 0113 — Views bind to interfaces: templates, component pictures and view files

- **Status:** Proposed
- **Date:** 2026-11-26
- **Replaces:** the "default pictures" step of [ADR 0108](0108-interface-synchronized-projections.md) point 5
- **Builds on:**
  - [ADR 0015](0015-pluggable-views.md) (pluggable views, declared style properties);
  - [ADR 0093](0093-scenes-from-primitive-layers.md) (scenes of layers and frames);
  - [ADR 0101](0101-scene-primitives-are-parametric-meshes.md) (primitives);
  - [ADR 0110](0110-encapsulated-models.md) (encapsulated models);
  - [ADR 0111](0111-a-declarative-modelling-language.md) (a declarative modelling language).

## Context

The owner asked, on 2026-11-26:
- whether views are tied to models;
- whether views can be built and swapped independently of them;
- whether the plan for views shows the same properties as models (nesting, encapsulation and the others).

The owner also required that no model has to supply a view, since many models are simple building blocks for others.

As built:
- **View types** are independent of models: a registry, pluggable like blocks (ADR 0015).
- **A configured view** is tied to one model by the names it binds (`cart.x`), and lives in that model's `controls.views`.
- **A view reads recorded data and computes nothing** (ADR 0093 point 3). Views are outside the model hash, and run files hash a model without its `controls`, so a recorded run does not depend on its views.
- **A used model's views are dropped** ([ADR 0066](0066-models-nest-as-blocks.md) point 4). A cart-pole used inside a larger model loses its picture, which is then rebuilt by hand under new names.

ADR 0110 made a model's interface its contract. Views can follow the same rule: a view binds to an interface, not to names inside a model. The scene work already has what this needs:
- frames that chain;
- bindings to signals, constants and rows;
- geometry that arrives through ports (ADR 0101 point 4);
- a pointer that means the same in every frame.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| A. As now: views bind names inside one model | Nothing to build | Pictures are rebuilt for every model that uses a component; a view breaks when a model's insides change |
| **B. Views may declare an interface (templates), used by binding it** | The same structure as models: interface, nesting, encapsulation, swapping, replication. Pictures come with components; views can be swapped on a run | A file form and a binding step to design |
| C. Views as blocks that output pictures | One kind of thing | Pictures would enter the model hash and its bit-for-bit promise; views would compute, against ADR 0093 point 3 |
| D. Reusable pictures only as plugin code | Full freedom | Not data: not editable in the app, not visible to `check`, against ADR 0111 |

## Decision

**B.**

1. **A template is a view with an interface.**
   - It is any view, a scene or a plot, that declares:
     - **inputs**: signals it needs, each with a unit and a kind (scalar, or rows);
     - **parameters**: constants with a default and a unit, such as a colour, a drawn size or a scale. These are ADR 0015's declared style properties, settable per use;
     - for a scene, **offered frames**: anchors a parent may draw in.
   - Inside the template, its bindings name its inputs and parameters with a `$` (`"$x"`). Everything else it binds is a constant. A template binds no signal by name, so it fits any model that can feed its inputs.
2. **A template computes nothing.** As for every view (ADR 0093 point 3), it holds no expressions, no state and no clock. A position that must be computed is an output of the model or of an expression filter, and the template binds it. Templates are data ([ADR 0111](0111-a-declarative-modelling-language.md)), not a second language.
3. **Where templates are written:**
   - **In a model file, under `templates`, by name.** A model's own views may use them, and the models that use it are offered them (point 5).
   - **In a view file** (point 8).
4. **Using a template:**
   - **As a whole view:** a view entry `{"template": "phase", "bind": {"x": "osc.x", "v": "osc.v"}, "params": {…}}` in `controls.views`.
   - **Inside a scene:** a layer `{"draw": "template", "name": "rig", "template": "cartpole", "in": FRAME, "bind": {…}, "params": {…}}` draws it in a frame of the parent scene.
   - Every input is bound, with its unit checked as a wire's is. A parameter left out keeps its default.
   - **Nesting** is a template layer inside a template. Frames chain through it, as they chain now.
5. **Component pictures.** A model's templates are offered to every model that uses it, as `USE/NAME` (`cart1/cartpole`). Each input binds by default, by name, to the use's output of the same name (`x` to `cart1/x`).
   - This keeps the binding inside the interface (ADR 0110): a component's picture reads what it offers.
   - An input with no output of its name must be bound explicitly.
   - A model's own shown views stay its own, and are still not carried into its users (ADR 0066 point 4). Templates are the part of its pictures it offers.
6. **Encapsulation.**
   - A user of a template sets only its inputs and parameters. It never reaches a layer or frame inside, except the frames it offers.
   - A parent draws in an offered frame by its path: `"in": "rig/tip"` for the frame `tip` that the template layer `rig` offers. This is the view counterpart of an output a model offers.
   - A template may change inside without breaking its users, as long as its interface holds.
7. **Swapping and replication:**
   - **Swapping.** Any template whose inputs the same bindings can feed can replace another: the car drawn as a box, or as a mesh. In the app this is "Replace with…", which lists the templates that fit. In the file it means changing the `template` name.
   - **Replication.** A template layer bound to quantities over a table's rows is drawn once per row, each in its row's frame. This comes with frames over rows as parents, already planned after the gate (ADR 0093). It is the view counterpart of N uses of one model.
8. **View files.**
   - A view file holds templates, and views bound by name. Its form is the model file's `templates` and `controls.views` alone, with `"format"`, and no blocks.
   - It can be applied to a model, or to a recorded run (whose hash leaves views out):
     - in the app: "Add views from…";
     - on the command line: `simsandbox render RUN_OR_MODEL --views FILE`;
     - in Python: `Model.add_views(file)` and `Run.render(views=file)`.
   - Views bound by name are checked against the model they are applied to, as views are now.
   - The same run shown through different view files gives the same results. Only the pictures change.
9. **No model has to supply a view.**
   - A model without templates or views is complete. `check`, `check --agent` and the card ask for none.
   - Its users draw what they choose. A model with no views opens with one chosen by fit, with "More views…" ([ADR 0016](0016-model-curated-views.md)).
   - A component's templates are offered, never imposed: a use shows them only where the model using it places them.
10. **Discoverable and headless** ([ADR 0039](0039-ai-agent-access.md)):
    - `describe` lists a model's or a view file's templates with their inputs, parameters and offered frames.
    - `check` reports an unbound input, a unit that does not fit, or an unknown `$` name, at its path.
    - The card lists a model's templates beside its interface.
    - `render` draws templates headless.
    - Python: `Model.define_template(name, view, inputs=, params=, offers=)`, and `template=` wherever a view or a layer is added.
11. **Later, each by its own decision, beside its model counterpart:**
    - **extension**: a template built on another, recording the difference, such as the cart-pole picture plus a force arrow;
    - **reference renders**: a headless image a template ships, compared within a tolerance, like a verification case;
    - **3D**: templates carry over unchanged in form, with 3D poses and a camera as a frame (ADR 0093's after-the-gate list).

**Not decided here** (for the owner, before step 1 is built):
- the `$` prefix for a template's own names;
- the word "template";
- the view file's extension (`.ssbv.json` is suggested).

**Example.** The cart-pole's picture as a template of its model, and a model using two cart-poles that adds an arrow at each pole's tip:

```json
"templates": {
  "cartpole": {
    "type": "plot.scene", "unit": "m",
    "inputs": {"x": {"unit": "m"}, "theta": {"unit": "rad"}},
    "params": {"pole_length": {"value": 1.0, "unit": "m"}},
    "frames": {
      "cart": {"x": "$x", "y": 0},
      "pole": {"in": "cart", "angle": "$theta"},
      "tip": {"in": "pole", "y": "$pole_length"}
    },
    "offers": ["tip"],
    "layers": [
      {"draw": "boxes", "in": "cart", "bind": {"x": 0, "y": 0, "width": 0.4, "height": 0.2}},
      {"draw": "boxes", "in": "pole", "bind": {"x": 0, "y": 0.5, "width": 0.04, "height": "$pole_length"}}
    ]
  }
}
```

```json
{"type": "plot.scene", "unit": "m",
 "layers": [
   {"draw": "template", "name": "left", "template": "rig1/cartpole"},
   {"draw": "template", "name": "right", "template": "rig2/cartpole", "params": {"pole_length": 1.5}},
   {"draw": "segments", "in": "right/tip", "bind": {"x1": 0, "y1": 0, "x2": 0.2, "y2": 0}}
 ]}
```

Here `rig1/cartpole` binds `x` and `theta` to `rig1/x` and `rig1/theta`, the outputs the cart-pole model offers.

**Order of work:**

1. Templates in the model file:
   - inputs, parameters, `$` names, used as a whole view or as a `template` layer;
   - `check`, `describe`, `render` and Python;
   - the cart-pole's picture rewritten as a template its own view uses, drawing the same pixels.
2. Component pictures:
   - templates offered to users, bound by name through outputs;
   - the cart-pole used twice side by side;
   - the card and the Overview listing a model's templates.
3. Offered frames, then replication over rows (with ADR 0093's frames over rows as parents).
4. View files:
   - applied to a model or a run, in the app, `render --views` and Python;
   - "Replace with…" among the templates that fit.

## Consequences

- **Models and views share one structure.** Each has an interface (inputs, parameters, and outputs or offered frames). Each nests, is encapsulated, can be swapped with another of the same interface, and is replicated over rows. A component is a model plus the pictures it offers.
- **Pictures are built once.** A component's picture follows it into every model that uses it, re-bound through its interface, so it survives changes inside either side.
- **A run's pictures can change** without running it again, since views are outside the hash.
- **The pendulum's domain view can go.** `eq.pendulum`'s hard-coded view, and other domain views, can become templates of their models once scenes draw them (ADR 0093's "existing views as presets"; [Q62](../open-questions.md)).
- **The Stage** (ADR 0108) composes component pictures with template layers. **One selection** extends to a template's instances: selecting an instance selects the use it is bound to.
- **Documents to update when built:**
  - the [visualization architecture](../architecture/visualization.md);
  - the scene reference;
  - the headless guide;
  - the model file schema.

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)):
- **Added:** *a view's interface* (inputs, parameters, offered frames), which makes a view a template.
  - It mirrors a model's interface, and is the same concept applied to an observer. Without it, a picture can only name one model's insides.
  - A view file is not a new concept: it holds templates and views on their own.
- **Removed:** none. Domain view types may later become templates.
