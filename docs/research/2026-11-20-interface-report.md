# The interface: from concept to a running, legible model

_Written 2026-11-20, at the owner's request after the Phase 2 gate; nothing here is decided except where an ADR is linked._

The question (the owner, 2026-11-20): how does a user take a model, with its visuals, from concept to reality in SimSandbox? The answer should be straightforward without giving up control. Models should show their methods so plainly that a person can understand one an AI agent built entirely. Any refactoring is allowed, including replacing the node canvas. This report:

- reviews the first ideas (the brainstorm of 2026-11-20);
- surveys how other tools answer the same question;
- compares five interface architectures against SimSandbox's vision;
- recommends one, with an order of work.

The tools in the survey are described from their public documentation and common use as known in 2026; their details were not re-checked for this report.

## 1. What the interface must do

From the [vision](../vision.md), [ADR 0094](../decisions/0094-the-full-audience.md) (the full audience) and [ADR 0039](../decisions/0039-ai-agent-access.md) (AI agents), the criteria used below:

| | Criterion | Why |
| --- | --- | --- |
| C1 | **Legible at a glance**: what a model is for, its parts, their equations and methods, how far to trust it, including a model an agent made | Learning pillar; "parameters, units and equations are first-class metadata"; AI-made models |
| C2 | **Short path from idea to a running model with its picture** | "Type an equation and get controls and plots immediately"; hobbyists |
| C3 | **No ceiling on control**: every method, setting and connection reachable and exact | Research pillar; "choose the method, know what you get" |
| C4 | **Play**: drag, poke, steer the running model where it is drawn | Fun pillar |
| C5 | **Composition across kinds** (ODEs, agents, mechanisms, grids, wrapped codes) with units checked | Core value |
| C6 | **Parity for agents**: the file is the truth, and every surface is a view of it that agents can make, check and render headless | ADR 0039 |
| C7 | **Scales** from a pendulum to models of dozens of parts | Research models |
| C8 | **Fits a desktop app in egui**, buildable in steps | Feasibility |

## 2. Where the interface stands (the cart-pole, 2026-11-20)

`examples/cartpole.ssbx.json` is small: three blocks (a pointer, the mechanism, the controller) and two views.

- **Diagram.** It has 12 model wires and 13 dashed data wires. One of the model wires, with its filler line `n = 0`, exists only because an input must be wired; [ADR 0106](../decisions/0106-mechanism-forces-are-parameters.md) removes it. The scene's node has one port for every binding (`layers[3].bind.x1`, `frames.pole.angle`, …). The layout says nothing about which part is the plant, which the controller and which the interaction.
- **Inspector.** Selecting the mechanism lists about 25 parameters with equal weight (`slider_at_y` beside `pole_mass`), then the bodies table. Nothing says what the block does or how it is solved.
- **Where things are edited.** There are six places: the canvas, the Inspector, the equations window, a view's tile settings, the toolbar's dialogs (Data, Memory, Solving, Runs) and the file. Each is exact, but the model's story is spread across them.
- **The picture.** The scene is written by hand, frames and all, and repeats geometry the mechanism already knows (the pole's length and the cart's position).
- **AI parity is good.** Everything is in the file and checkable headless, which is the asset to keep.

The interface was built bottom-up: a node canvas over the model graph, panels for what the canvas cannot show. Every piece is right and the whole is hard to read. The gap is in C1, C2 and C7, not in C3, C5 or C6.

## 3. The first ideas, reviewed

| Idea (2026-11-20) | Verdict | Refinement |
| --- | --- | --- |
| Inputs with defaults | **Dropped** | They are parameters ([ADR 0075](../decisions/0075-parameters-are-inputs-with-defaults.md)). The mechanism's forces become parameters ([ADR 0106](../decisions/0106-mechanism-forces-are-parameters.md)). The general lesson: a block should expose only what must be wired as inputs, and everything else as parameters a wire may drive |
| Bundled wires | **Kept, as port groups** | A block declares groups (a mechanism's joint state; a vector's components); the structure view draws one connection per group and expands on demand. Units stay per port |
| A view as one node | **Kept, and taken further** | Views are observers ([ADR 0032](../decisions/0032-filters-as-a-graph.md)); they leave the structure view and live on the stage (section 6). The data layer becomes a lens, off by default |
| Layout by role | **Kept, as a derived layout** | Roles are inferred from the graph (state-bearing block driven by a stateless one = plant and controller; input blocks = interaction; observers = views) and may be declared; the layout follows roles and loops, and a person may pin positions |
| Grouping by meaning | **Kept, as components** | A group is a subsystem ([ADR 0043](../decisions/0043-subsystems.md)) with a card and a picture: the unit people reuse and plugins ship (section 7) |
| Inspector tiers, parameter roles, "what it does" first | **Kept** | Parameter roles (property, geometry, start, force, numerics) and key parameters become metadata of block types and models, the same metadata the card and plugins use |
| Model card | **Kept, central** | It becomes the Overview, the model's front page ([ADR 0107](../decisions/0107-model-cards-and-agent-notes.md)): generated, with notes required only of agents and agents' parts marked |
| Clarity checks | **Kept** | As `check --agent` (ADR 0107) |
| Change view between versions | **Kept** | Most valuable for agents' edits: "what changed" on the card and the structure view |
| Visuals that come with the block | **Kept, a key enabler** | A component carries its default picture (a mechanism draws its bodies from its rows), so the stage composes itself |
| Starting points | **Kept, as packaged models** | A template is a component or model in the library, made the way an agent would make it |
| Plugin as a package with a manifest | **Kept** | Section 7 |

## 4. How others answer the question

| Paradigm | Examples | What it does well | What goes wrong | Lesson for SimSandbox |
| --- | --- | --- | --- | --- |
| **Signal-flow block diagrams** | Simulink, LabVIEW, Max/MSP, Node-RED, Unreal Blueprints, ComfyUI | Dataflow is explicit; good for control and signal chains; subsystems scale it | Wire clutter at modest size; layout is the user's labour; physical structure (a cart, a pole) disappears into signals | The canvas SimSandbox has; keep it as one lens, not the front page |
| **Physical (acausal) schematics with layers** | Modelica tools (OpenModelica, Dymola), Simscape | The diagram looks like the system; one model has an icon, a diagram and a text layer, each a view of the same definition | Acausal semantics are heavy; diagrams still crowd | One definition, several projections. SimSandbox gets acausal physics via FMI ([ADR 0084](../decisions/0084-where-typed-equations-stop.md)) but should copy the layering |
| **Stock-and-flow** | Vensim, Stella, Insight Maker | A domain diagram non-experts read | One paradigm only | Domain-specific lenses beat one generic notation |
| **Text first, live projection** | Modelica text, ModelingToolkit, Pluto.jl, Mathematica `Manipulate` | Text is exact, diffable, writable by agents; widgets appear from the text | Hard to see structure; play is secondary | The file stays the truth; every surface is generated from it |
| **Notebooks and explorable documents** | Jupyter, Observable, NetLogo's Info tab, explorable explanations | Narrative plus live results: explains a model | Composition and real-time play are awkward; state hidden in cell order | The card as a live, generated document; NetLogo shows a model's own "Info" page is natural |
| **Tabs per concern** | NetLogo (Interface, Info, Code), AnyLogic (agent types, each with logic and presentation) | Each concern has a place; presentation is first class | Switching tabs to follow one thing | Presentation (the picture) belongs to the component, as in AnyLogic's agent presentations |
| **Scene-first world editors** | Godot, Unity, Roblox Studio; Algodoo, Universe Sandbox | Build in the picture; a hierarchy tree and an inspector per object; behaviour attached as components; play in place | Equations and dataflow hidden in scripts; weak for abstract models | For physical models the stage should be an editing surface: select a body in the picture, its definition opens |
| **Procedural graph + viewport + parameter pane** | Houdini, Grasshopper with Rhino, TouchDesigner, Blender geometry nodes | The graph is the exact recipe, the viewport shows the result, and the parameter pane shows the selection; selection syncs across all three; subnetworks with promoted parameters hide detail | Graphs still grow large; learning curve | The closest match: graph for structure, stage for result, inspector for the selection, all synced; "promoted parameters" are key parameters |
| **Equation sheets** | Mathcad, Desmos | Maths reads as maths, units inline, results beside the formula | Not for large composed models | Render typed equations as maths with units and values beside them |
| **Conversation first** | AI coding assistants, generative app builders | Fastest from idea to first version | Opaque results; trust depends on review | Agents build; the card, the agent check and the change view make their work reviewable (ADR 0107) |

What the strongest tools share is **one definition with several synchronized projections**:

- Modelica: icon, diagram and text;
- Houdini: network, viewport and parameters;
- Godot: tree, viewport and inspector.

Each lets a person start from the projection that fits the task and reach the exact definition in one step. None makes the wiring graph the front page for models that have a physical picture.

## 5. Five architectures compared

- **A. Keep the canvas, fix it.** Port groups, collapsed views, a derived layout, Inspector tiers and roles, a card. Least change.
- **B. Text first.** The canvas is replaced by a structured document of the model: blocks as sections with rendered equations, inline widgets and live plots, notebook-like; the picture is a pane.
- **C. Stage first.** The scene is the main editor, with a component tree and an inspector as in a game engine; the graph is hidden, connections are made in the inspector ("driven by…").
- **D. One model, synchronized projections.** An Overview (the card), a Stage (scene and views, playable and selectable), a Structure lens (a semantic graph of components with port groups and a derived layout; data layer on demand), definitions edited where they live (equations as maths, bodies as a table), and an Inspector by intent. Selection is shared by all.
- **E. Conversation first.** A prompt builds and edits the model; the card and change view explain it; the other surfaces are for review.

| Criterion | A | B | C | D | E |
| --- | --- | --- | --- | --- | --- |
| C1 legible at a glance | ○ | ● | ○ | ● | ◐ (with the card) |
| C2 idea to running model with picture | ○ | ◐ | ● (physical models) / ○ (abstract) | ● | ● |
| C3 no ceiling on control | ● | ● | ◐ (dataflow hidden) | ● | ○ (alone) |
| C4 play in place | ◐ | ○ | ● | ● | ◐ |
| C5 composition across kinds | ● | ◐ | ○ (abstract models fit badly) | ● | ◐ |
| C6 parity for agents | ● | ● | ● | ● | ● |
| C7 scales | ○ | ◐ | ◐ | ● (components, lenses) | ◐ |
| C8 buildable in steps in egui | ● | ◐ (rendered maths needed) | ◐ | ◐ (in steps from A) | ● (needs D or A beneath) |

● strong, ◐ partial, ○ weak.

- **B** explains well but plays badly, and composition of many parts in a document reads as a long scroll.
- **C** is excellent for the cart-pole and wrong for a heat equation, an agent population or a study, where there is no world to click.
- **E** is the fastest entry but needs a legible surface underneath to be trusted; it is a mode of D, not an alternative.
- **A** is the right first steps but not the destination: it keeps the wiring graph as the front page.

**D wins on every criterion but cost, and it is reached through A's steps.**

## 6. The recommended interface (D)

### The surfaces

1. **Overview**, the model's front page ([ADR 0107](../decisions/0107-model-cards-and-agent-notes.md)). It is generated from the file and always current:
   - purpose (an agent's notes, or a person's if they wrote one);
   - components and their roles in a small diagram;
   - each component's equations as maths with units;
   - methods with speed and accuracy labels, fusion, verification;
   - what each view shows;
   - the key parameters as live controls;
   - agents' parts marked.

   Everything links to where it is defined. It opens first for a model someone else made, and on request for one's own.
2. **Stage**, where the model runs: scenes and plots, play, grab, rewind. Selecting a drawn thing selects what drew it. A body selects its mechanism row, a curve its signal's block. "Show definition" goes one step to the Structure lens or the editor. Components bring default pictures, so a model with a mechanism has a scene before anyone writes one; a person restyles it instead of writing frames.
3. **Structure**, the semantic graph:
   - components (blocks and subsystems) laid out by role: interaction left, plant centre, controller below with its loop drawn as a loop, observers off;
   - connections drawn per port group;
   - parameters shown as values, ports only when driven;
   - the data layer and individual ports as lenses.

   Positions are derived and may be pinned. Zoomed out it shows components and roles, zoomed in ports. This is today's canvas, re-based on roles and groups rather than replaced.
4. **Definitions**, edited where they live and supplied by the block type (or plugin):
   - typed equations as an equation sheet (maths, units, current values beside each line, the text one click away);
   - a mechanism as its bodies table with its picture;
   - a study as its sample design.
5. **Inspector**, for the selection:
   - "what it does" (summary, method, labels);
   - its key parameters;
   - the rest grouped by role (property, geometry, start, force, numerics).

Selection is one thing across all five. Undo, drafts and `check`'s errors work as they do now, shown where the selection is.

### From concept to model

- **Start from intent:**
  - a starting point from the library (a component or model, built-in or from a plugin);
  - a prompt to an agent, whose result arrives with its notes and marked (ADR 0107);
  - a blank model with typed equations.
- **Add components from the library.** Each brings its parameters with roles, its port groups, its picture, its card section and its tests.
- **Connect by intent:**
  - from the Inspector ("drive `slider_force` from…" lists compatible outputs by unit and group);
  - on the Stage (drag from a body to a controller);
  - on the Structure lens.

  Each makes the same edit.
- **Run on the Stage at once** with the composed picture; tune key parameters on the Overview or Inspector.
- **Go deeper only where needed:** definitions, methods, the data layer, probes, studies. Nothing is hidden, only placed where it belongs.

### For agents

The same surfaces headless:

- the Overview as the card (a command and a Python call);
- the Structure lens as `render --diagram` with lenses;
- the Stage as `render`.

`check --agent` holds agents to the notes, and the change view shows a person what an agent did. Every surface is a projection of the file, so nothing an agent cannot write or read is introduced.

## 7. Plugins in this picture (Q28, Q37)

A plugin is a **package**, and the interface depends on it carrying more than code:

- **component types** (blocks and views), each with a **manifest**: ports and port groups, parameters with units, roles and key flags, methods with their labels, a summary;
- **default pictures** and **definition editors** where a type needs one (a mechanism's table);
- **card sections**, docs and starting points;
- **verification tests**, run by `check` or a `verify` command, so a plugin's labels are earned ([ADR 0061](../decisions/0061-verified-is-earned-per-case.md)).

The same manifest feeds the Inspector, the Structure lens, the card and `describe`. A plugin's blocks are as legible as built-ins, and agents discover them as they discover built-ins.

**Runtimes are a separate choice under one contract:**

- WebAssembly for shareable, sandboxed blocks and views (the standing recommendation of 2026-10-26);
- Python in process for prototyping;
- out of process for existing codes ([ADR 0091](../decisions/0091-equations-first-hosting-for-existing-codes.md)).

**The cheapest plugin is a packaged model:** a subsystem with its card and picture, no code. Most sharing (a PID controller, a cart-pole plant) may need nothing more, so packaging comes before runtimes.

A view plugin returns a draw list as scenes do ([ADR 0101](../decisions/0101-scene-primitives-are-parametric-meshes.md)). The scene's primitives, frames and picking are the built-in vocabulary such plugins extend. 3D scenes (STATUS Next up) are the test of whether that vocabulary is enough.

## 8. Costs and risks

- **Size.** D is a large refactor of the app: Overview and Definitions are new, Stage selection is new, and the canvas is re-based. Each step below is usable alone, so the risk is in sequencing, not in a big switch.
- **Rendered maths in egui.** egui draws no maths. An equation sheet needs a small renderer of typed equations (fractions, powers, subscripts, units), which is enough since typed equations are a known grammar, or a typesetting crate. To be weighed in its own step.
- **Inferred roles can be wrong.** Declared roles override inferred ones, and the layout must stay pinnable.
- **Two ways to connect** (Inspector and graph) must be one edit underneath (they are: `Edit::Connect`).
- **Metadata burden.** Roles, key parameters and port groups are declared once per block type, not per model; models inherit them. Only agents are asked for notes.
- **AI parity.** Each new surface needs its headless form in the same step (card, lenses, stage renders).

## 9. Recommended order of work

1. **[ADR 0106](../decisions/0106-mechanism-forces-are-parameters.md):** a mechanism's forces as parameters. Small; removes the cart-pole's filler wire.
2. **Metadata:** parameter roles, key parameters and port groups on block types (an ADR), declared for the built-in types.
3. **The Overview, headless first:** the card command and Python call, notes and agents' marks, `check --agent` ([ADR 0107](../decisions/0107-model-cards-and-agent-notes.md)); then in the app as the first tab.
4. **The Inspector by intent:** "what it does", key parameters, groups by role.
5. **Default pictures:** components bring scene layers. The mechanism's first: the cart-pole's scene written by none.
6. **One selection across the Stage, Inspector and canvas:** select in the picture.
7. **The Structure lens:** derived layout by role and loop, port groups drawn as one connection, observers and the data layer as lenses; semantic zoom.
8. **Definitions:** the equation sheet (maths with values) and the mechanism's table moved there.
9. **Plugins:** the package and manifest (an ADR), packaged models first; then the WebAssembly spike and runtimes (Q28, Q37).
10. **3D scenes** on the same vocabulary.

A measure for each step: the cart-pole built from nothing, counted in steps, visible wires, and what a newcomer can say about it from the first screen. That gives a before and after for each change.

## 10. Questions for the owner

- Should the Overview open first for every model, or only for models made by someone else (or an agent)?
- Should the Structure lens keep free positions as today, with the derived layout as a suggestion, or derive positions with pins as exceptions?
- Should the order above start at steps 1–3 (legibility first) or at step 5 (default pictures: the concept-to-model path first)?
