# 0113 — Views bind to interfaces: templates, component pictures and view files

- **Status:** Accepted (owner, 2026-11-26)
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

**Names** (owner, 2026-11-26):
- a template's own names are written with a `$` (`"$x"`);
- the word is "template";
- a view file's extension is `.ssbv.json`.

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

## Built so far

- **Step 1 (2026-11-26), templates in the model file:**
  - `ModelFile::templates` (written like `models`, read by `templates::Template::read`), `ViewEntry::template` and `ViewEntry::params`, and template layers in scenes.
  - `templates::expand` makes the view a template stands for. Every part that draws, records or checks views reads the expanded view, and the file keeps the written one (see the [visualization architecture](../architecture/visualization.md#scenes)).
  - Offered frames are enforced already: a parent reaching a frame that is not offered is refused.
  - `check` lists templates, as `describe` does for types. Python has `define_template`, `add_view(template=…)` and template layers.
  - The cart-pole's picture is the template `cartpole`, rendered to the same pixels as before at 0.5, 2 and 3 s.
  - Tests: expansion and its refusals (`templates.rs`); the views and the history (session); the app opening, drawing and saving the cart-pole; Python.
  - Kept for later steps:
    - an input's declared unit is not compared with its signal directly, only through the attributes it reaches, which are checked as before;
    - `rows` inputs are accepted but not yet drawn per row (step 3);
    - a scene's Inspector form edits the view as written, but has no editor for a template layer yet.
- **Step 2 (2026-11-26), component pictures:**
  - `compose::templates_in(file, folder)` gives every template a model can draw: its own by name, and each used model's (defined in the file or a separate file) by path (`left/cartpole`, deeper uses `a/b/name`). Every place that expands views uses it: tiles, what the history records (`Recording::new` and the reads), `check`, `render`, the card and Python's scene check.
  - A template found through a use binds each input not bound otherwise to the use's output of that name (`left/x`). A pointer input left unbound draws without grabbing. Templates a used model's template draws are found under its path.
  - The cart-pole offers `x` and `theta` as outputs. The new example "Two balancing poles" (`examples/two-cartpoles.ssbx.json`) uses it twice from its file, each started with its pole tipped (start states by inner block, ADR 0112), and draws both from `left/cartpole` and `right/cartpole` in frames of its own, the right one's pole recoloured by a parameter.
  - The card lists a model's `templates`, and each use's `pictures` (`left/cartpole`). Markdown has a "Templates" section and "Pictures it offers". The Overview has both.
  - Tests: default binding, explicit binding, a pointer left out and templates within a used model's (`templates.rs`); the card of the example.
- **Grabbing a component's picture (2026-11-26, owner: "fix the two balancing poles grab"):**
  - A model's `input.pointer` blocks are part of its interface, as input blocks are ([ADR 0074](0074-model-interfaces.md)). A template's `pointer` input binds by name like the others, to the use's pointer block (`left/hand`). The card lists a use's pointers among its inputs.
  - A grab is live input to that block by path, as from the model's own scene; its recorded track replays it.
  - The pointer is sent in the frame the template is drawn in, so its model reads the pointer as in its own scene. This refines [ADR 0093](0093-scenes-from-primitive-layers.md) point 4 ("always the scene's own frame") for templates.
    - A scene layer takes `grab_in`: the frame the pointer is sent in, the scene's own if absent. It must be a frame of one pose, on a layer that grabs.
    - Expansion sets it on a template's grabbing layers to the frame the template layer is placed `in`, or to a frame of its own it named, under its path.
    - The scene sends `grab_in`'s inverse pose at the newest sample applied to the pointer (`SceneView::sent_in`).
  - The cart-pole's picture names its pointer input `hand`, as its block is named, and draws the same pixels.
  - In "Two balancing poles", each pole grabs its own cart-pole: a grab driven on `left/hand` moved the left pole by up to 0.39 rad and left the right one equal to the bit.
  - Tests:
    - `grab_in` in a turned frame and in a moved frame, and its refusals (`scene/tests.rs`);
    - binding the pointer by name and `grab_in` set by expansion (`templates.rs`);
    - the app grabbing `left/hand` in `left_track`, with only the left use's pointer changing (`scene_tests.rs`).
- **Step 3 (2026-11-26), offered frames in use and replication over rows:**
  - **Offered frames.** The cart-pole's picture offers `tip`, at the pole's tip. "Two balancing poles" marks each tip with a red disc drawn `in` `left_rig/tip` and `right_rig/tip`. A frame not offered is refused, as built in step 1.
  - **Frames over rows as parents** (ADR 0093's after-the-gate item; `scene/spec.rs`, `scene/eval.rs`):
    - A frame in a frame over a table's rows is over those rows too, one pose a row. Along a chain, rows are of one table, paired by id; two tables are refused at the frame.
    - A layer in such a frame is drawn once a row, its pose composed for that row up the chain.
    - `view_from` and `grab_in` still take only a frame of one pose.
  - **Replication.** A template whose frame binds a quantity over rows is drawn once a row. The cart-pole has the template `body_axes` (inputs `x`, `y`, `angle` of kind `rows`, parameter `length`), which its scene draws over the mechanism's pose rows, one pair of axes a body. The cart-pole's picture changes by those axes, and its run results do not.
  - Tests: a frame in a frame over rows placed once a row (`scene/tests.rs`); the refusal of two tables along a chain; the card listing both templates of each use.

