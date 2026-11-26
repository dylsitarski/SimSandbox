# 0108 — The interface: one model in synchronized projections, legibility first

- **Status:** Accepted (owner, 2026-11-20)
- **Date:** 2026-11-20
- **Amended by:** [ADR 0110](0110-encapsulated-models.md) (the projections show one level of encapsulated models); [ADR 0113](0113-views-bind-to-interfaces.md) (default pictures become templates and component pictures)
- **Evidence:** [the interface report](../research/2026-11-20-interface-report.md)
- **Builds on:** [ADR 0107](0107-model-cards-and-agent-notes.md) (model cards), [ADR 0039](0039-ai-agent-access.md) (AI agents), [ADR 0042](0042-node-canvas.md) (the node canvas)

## Context

The cart-pole showed the app hard to read: a crowded diagram, a flat Inspector, a model's story spread over six places. The owner asked how a model goes from concept to a running, legible model, with any refactoring allowed, and for a report comparing alternatives. The report compared five architectures against the vision's criteria and recommended synchronized projections. The owner answered its three open questions ([Q64](../open-questions.md), 2026-11-20).

## Options considered

As in the report, section 5: keep and fix the canvas (A), text first (B), stage first (C), **one model in synchronized projections (D)**, conversation first (E).

## Decision

1. **The destination is D.** One model file, shown through five projections that share one selection:
   - **Overview**, the generated model card;
   - **Stage**, where the model runs and is played with, and where views are made (point 4);
   - **Structure**, today's canvas re-based on roles and port groups;
   - **Definitions**, each block edited where it lives;
   - **Inspector**, by intent.

   Each has a headless form for agents.
2. **The Overview opens first** for every model.
3. **Positions on the Structure lens stay free**, as on today's canvas. The layout by roles and loops is a suggestion a person applies, not a rule.
4. **Views are made on the Stage, not in the structure graph.** A view is added there or from a signal, block or component wherever it is shown (Overview, Inspector, Structure). Its bindings are edited when it is selected, as scenes are now ([ADR 0093](0093-scenes-from-primitive-layers.md)). Components bring default pictures. What a view reads stays visible on the Structure lens's data layer, which is off by default.
5. **Work starts with legibility**, in the report's order:
   - [ADR 0106](0106-mechanism-forces-are-parameters.md);
   - metadata (parameter roles, key parameters, port groups);
   - the Overview, headless first;
   - the Inspector by intent;
   - then default pictures, one selection, the Structure lens, Definitions, plugin packages and runtimes, 3D scenes.

## Consequences

- The app is refactored in those steps, each usable alone and each with its headless form in the same step.
- The canvas stays and is re-based; free positioning stays.
- Each step's ADR, where it changes a contract (metadata, the plugin manifest), follows its own decision.
- Measured against the cart-pole built from nothing: steps, visible wires, what a newcomer says from the first screen.

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none yet; components and port groups are named in their own ADRs.

## Built so far

- **The Overview (2026-11-21):**
  - `Pane::Overview` in the dock (`simsandbox-app/src/app/overview.rs`), saved in layouts as `"overview"` (an older SimSandbox ignores it). It is in front of the right-hand stack in the standard arrangement, and arrangements saved before it gain it in front of the diagram. It is focused whenever a model opens; `--view overview` too, and `--view time|phase|3d` focuses the views instead.
  - It shows the card ([ADR 0107](0107-model-cards-and-agent-notes.md)): built on the worker from the session when the file changes; values read live from the published parameters; parameters grouped by role (property, drive and tuning open, the rest open when short; drive parameters at 0 that nothing drives counted); a type's summary shortened to its first sentence with the rest on hover; agents' marks with **Mark as reviewed**; **Inspect** on each part selects it for the Inspector and the diagram.
- **The Overview by level (2026-11-25, [ADR 0110](0110-encapsulated-models.md) step 2):** each used model is one part with its interface and **Open**; a breadcrumb back up; what reads inside a use, with **Offer as output**.
- **The Inspector by intent (2026-11-26):** for a block of the model, in the model tree and when selected (`simsandbox-app/src/app/inspector/intent.rs`):
  - what it does first: its purpose or summary (the first sentence, at most about two lines, the rest on hover), its method with labels and what it is solved with (from the model's card, [ADR 0107](0107-model-cards-and-agent-notes.md));
  - then its key parameters: those pinned, else those its type suggests ([ADR 0109](0109-parameter-roles-key-parameters-port-groups.md) point 2);
  - then the rest by role, each a section (property, drive and tuning open, the others open when short), starting values last; drive parameters at 0 that nothing drives are counted, not listed.

  Used models keep their table of uses: the interface first, the values inside read-only ([ADR 0110](0110-encapsulated-models.md) step 3).
- **Default pictures** became [ADR 0113](0113-views-bind-to-interfaces.md) (templates, component pictures, view files), built 2026-11-26.
- **One selection (2026-11-26):** the block picked anywhere is selected everywhere, at the model's own level.
  - **In the picture:** a press on any instance of a scene selects the block its layer shows (`Scene::selects`: the first signal or parameter its attributes read, else its frames', up the chain; constants alone select nothing). A press on a grabbing instance grabs and selects. The scene reports it through `View::picked`, as a plot's clicked curve does, and the app selects the block it comes from (`inspect_signal`). A use is selected through what it offers (`left/x` selects `left`).
  - **Shown:** the diagram and the Inspector follow, as before. The Overview marks the selected part ("(selected)", in the selection's colour). A scene outlines every instance that shows the selected block (`ViewInput::selected`, set by the app from the diagram's selection at the model's level).
  - Tests: a press selecting what its layer shows, grabbed or not, and constants selecting nothing (`scene/tests.rs`); the app selecting a use through its output and a block through its signal, with the picture outlining it (`scene_tests.rs`).
  - Not yet: a plot does not mark the selected block's curves; a selection inside a use (the diagram opened in it) is not shown in views or the Overview.
- Not yet: the Stage, the Structure lens, Definitions; the Inspector's own headless form beyond `explain` (the card carries the same facts). (point 5's fourth step), then default pictures, one selection, the Structure lens, Definitions.

