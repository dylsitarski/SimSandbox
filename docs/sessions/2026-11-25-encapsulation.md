# 2026-11-25 — Encapsulation and composition

- **Who:** owner and agent (Claude Code)
- **Goal:** revisit the interface direction ([ADR 0108](../decisions/0108-interface-synchronized-projections.md)): its overhead, whether models are encapsulated, and whether nesting should organize the interface.

## Done

- [Research note](../research/2026-11-25-encapsulation-and-composition.md): the planned work adds no housekeeping for people (notes are agents' only; roles optional; the card generated), but it makes a flat graph legible by labels, which does not scale (the platoon's card lists six inner blocks). Models nest and declare interfaces, but a use may set inner values by path, and the surfaces flatten. It recommends structure first: encapsulation, surfaces by level, then extension, replaceable parts, replication, contracts.
- [Q65](../open-questions.md) raised; the Inspector by intent paused on it.
- [ADR 0110](../decisions/0110-encapsulated-models.md) drafted (proposed): a use changes its model only through its interface; inner values set by path convert to declared parameters (in-file definitions) or are reported (separate files); start states by inner block stay; reading inside stays open and is marked, with "Offer as output"; the card, Overview and Inspector show one level (`explain --part`, `--depth`); Python's `expose`.

- ADR 0110 step 1 built (owner: "start building step 1"): `simsandbox-file/src/encapsulate.rs` converts values set by path, wires and probe links from inner outputs, level by level, before the composition loader reads a file; separate files reported; `expose` and `offer_output`, and Python's `Model.expose` and `Model.offer`, with refusals of inner paths; `upgrade` loads models that use models; a fixture, `tests/encapsulation.rs`, the Python composition test through interfaces. Every example's run gives the same bytes as before.

- ADR 0110 step 2 built (owner: "start step 2"): `card_at` by level with uses as parts, `reaching_inside`, `Edit::OfferAsOutput`; views read offered outputs by name; `explain --part/--depth`, Python's `part=`/`depth=`; the Overview opens uses in place; the platoon and forced-spring views read offered outputs. Screenshot of the platoon's Overview checked under Xvfb, not yet by the owner in the window.

## Decided

- Owner (2026-11-27): [ADR 0114](../decisions/0114-block-types-bring-default-pictures.md) accepted: a block's pictures named as a use's; pictures layer into one view.

- Owner (2026-11-26): [ADR 0113](../decisions/0113-views-bind-to-interfaces.md) accepted, with `$` names, the word "template" and `.ssbv.json` for view files.

- Owner (2026-11-25): [ADR 0110](../decisions/0110-encapsulated-models.md) accepted (Q65 answered: structure first). Asked whether to close reading too: observing stays open, reading that feeds a result goes through outputs.
- Owner (2026-11-25): [ADR 0111](../decisions/0111-a-declarative-modelling-language.md), SimSandbox is a declarative modelling language, not a library; written into the vision and the architecture overview; the language reference in Next up.

- 2026-11-26: Q66 answered ([ADR 0112](../decisions/0112-experiments-may-reach-inside.md)); the card marks experiments that set values inside.
- 2026-11-26: ADR 0110 step 3 built: the Inspector's values inside a use read-only, with Expose (`Edit::ExposeInside`) and Change in the model (`Edit::SetInside`); Expose from the Overview's experiment marks. Screenshot of the platoon's Inspector under Xvfb.
- 2026-11-26: the Inspector by intent (ADR 0108's fourth step): `app/inspector/intent.rs`, in the model tree and the selection; an app test on the cart-pole; screenshots of the mechanism and the controller under Xvfb.
- 2026-11-26: views revisited (owner): view types are independent, configured views tied to one model by names, used models' views dropped. [ADR 0113](../decisions/0113-views-bind-to-interfaces.md) drafted (proposed): views declare interfaces (templates) with the same properties as models; no model has to supply a view.
- 2026-11-26: ADR 0113 step 1 built: `templates.rs` (expansion), the views' written and shown forms, `check`'s templates, Python's `define_template` and `add_view`, the cart-pole's picture as a template (same pixels).
- 2026-11-26: ADR 0113 step 2 built: `compose::templates_in`, defaults by name through outputs, the cart-pole's outputs, the example "Two balancing poles", templates and pictures on the card and Overview.
- 2026-11-26: the two balancing poles' grab fixed (owner): a template's pointer binds by name to the use's pointer block; `grab_in` sends the pointer in the frame the template is drawn in.
- 2026-11-26: ADR 0113 step 3 built: frames over rows as parents, `body_axes` drawn once a body, the cart-pole's offered `tip` marked in "Two balancing poles".
- 2026-11-26: ADR 0113 step 4 built: view files (`render --views`, `check`, "Add views from…", Python), templates outside the model hash, "Replace with…" for template layers; the scene form of a templated scene now edits it as written (it edited the expanded view, unsaved).
- 2026-11-26: one selection (ADR 0108): a scene's press selects the block its layer shows (`Scene::selects`, `View::picked`); uses through their outputs; the Overview marks and scenes outline the selection.
- 2026-11-27: the Stage's first increment: Stage tabs and bar, Plot and Draw from the Overview and Inspector, `--view stage`; screenshot under Xvfb.
- 2026-11-27: [ADR 0114](../decisions/0114-block-types-bring-default-pictures.md) drafted (proposed): block types bring default pictures.
- 2026-11-27: ADR 0114 step 1 built: the `Pictures` capability, `block_pictures` and `Session::templates()`, listings in `check`, the card and `describe`; tested with a test block.
- 2026-11-27: ADR 0114 step 2 built: the mechanism's `bodies` (`simsandbox_mech::picture`), checked against its rows; a model that writes no views opens with its blocks' pictures. Found by a headless render: the session recorded only what written views bind, so a default picture drew nothing outside the app; `shown_views` fixes it, with a test.
- 2026-11-27: ADR 0114 step 3 built: **Draw** for blocks' and uses' pictures on the Overview, in the Inspector and in "+ view", into a new view or an existing scene (`PictureTargets`, `SceneEdit::AddPicture`); ADR 0114 is done.
- 2026-11-27: the rest of the Stage (ADR 0108): Show definition (`app/definition.rs`), a picked body opening its row, plots marking the selection, binding by dragging an output onto a view (`Views::bind_dropped`).
- 2026-11-27: the Structure lens, step 1: `diagram::bundles`, `diagram_paint::Detail`, the canvas by zoom (0.5) or by choice, the app's data layer off by default (`DiagramTab::show_data`), `render --diagram --components`. Headless defaults kept so scripts do not change.
- 2026-11-27: the Structure lens, step 2: `diagram::{role, roles, arrange, arrange_file, place_arranged}`, **Arrange** in the app (one undo), `render --diagram --arrange`, Python `Model.arrange()`.

## Left open

- The Structure lens's steps 3–4 (ADR 0108: compact nodes when zoomed in, connect by dragging), then Definitions. Show definition of a plain block goes to the diagram.
- The setup-script warning at this session's start was a stale file from 2026-11-14 (the old script); the stored script and installed packages are right.

## Notes for the next session

- **Encapsulation's conversion** runs inside `load_composition` and `load_merged_with`, so it covers only files with uses. A file without uses has nothing to convert. It needs the registry: a default the model does not write is the block type's, read by building the block.
- **Definitions used inside definitions** run on their own clocks unless shared (`o/p` in the fixture is a model of its own), so a wire from `o/p/c.x` was refused even before ADR 0110; values reach them by path and convert.

- Wires into a use go only through its interface (`resolve` in `simsandbox-file/src/subsystems.rs`). The leaks are `params` keys with `.` or `/` (`declare`, the loop after `level`), `starts`, wires from inner outputs by path (`a/c.x` passes `check`) and probe links at any depth.
- Owner asked (2026-11-25) whether reading should be closed too; the draft keeps observing open (option E rejected) but closes reading that feeds a result (wires, links, formulas, events).
- A block's picture's pointer input (`hand`) is never bound by name: ports are not pointers. A model that wants to grab its mechanism's bodies binds it in its scene.
- No example sets inner values by path; the Inspector's per-use table (`simsandbox-app/src/inspector.rs`, `app/param_rows.rs`) is what writes them.
