# 0015 — Pluggable views with slots; style options limited to encodings and readability

- **Status:** Accepted
- **Date:** 2026-10-16

## Context

The app has three fixed views (time series, phase portrait, 3D trajectory) over a flat list of scalar outputs, one view at a time. The owner asked how this scales to models with many dimensions or with pictures that are not obvious from the outputs (a pendulum should look like a pendulum), and how to decide which draw-style options to expose ([open questions Q9 and Q10](../open-questions.md#resolved)). Agents (Phase 1) and fields (Phase 2) need views the fixed set cannot provide.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Keep a fixed set of views; add new ones in the app | Simple | Every new kind of model edits the app; domain knowledge of how to draw a model sits in the wrong place; picking 3 of N outputs is useless for large N |
| View registry with slots, mirroring the block registry | Domains contribute their own views; any view works with any outputs that fit its slots; derived signals extend every view at once | A trait, a registry and a file representation to design and keep stable |
| Hand everything to an external viewer (ParaView, Rerun) | Mature rendering | Loses live interaction, parameter linking and the single-app experience; still useful as an export path |

Style options: ad-hoc toggles per view, versus a rule for what to expose.

## Decision

- **Views are pluggable, like Blocks.** A view type has a namespaced name (`plot.time`, `plot.phase`, `plot.3d`, `eq.pendulum`), declares **slots** (named inputs such as x, y, z, colour, size, field) with the signal kinds each accepts, and declares a small set of **style properties**. A registry maps names to view types. The three current views become the first built-in types.
- **Slots bind to signals:** recorded output elements, time `t`, and later **derived signals** (expressions, norms, spectra, projections, Poincaré sections) computed from them. Derived signals extend every view at once and are how high-dimensional models get readable pictures.
- **Richer output shapes** (grids, point sets) and an optional semantic role on ports (position, angle, field) are added to the kernel when the first Block needs them, so views can bind to them and the app can choose a sensible default view.
- **Several views at once**, laid out in tiles; the file stores a list of views with their type, slot bindings and style values (amending the single `view` of [ADR 0014](0014-view-in-controls.md)).
- **What to expose as style (Q10):**
  1. A choice that changes what can be read is an **encoding**, so it is a slot: trail fading is the colour or opacity slot bound to `t`; "colour by speed" binds it to a speed signal.
  2. A setting some models need to stay readable (trail length, point size, line width) is a **declared style property** with a default, a suggested range and a description, edited with the same panel as parameters ([ADR 0011](0011-parameter-entry.md)).
  3. Purely aesthetic choices get a good fixed default and are not exposed.
  An option is added only when an example or a user need shows it; each one is saved with the model.
- **Who a setting is for decides where it lives:** the author's explanatory choices (bindings, encodings, style properties) are saved in the model's `controls` ([ADR 0012](0012-controls-in-model-files.md)); the viewer's needs (colour-blind-safe palette, light or dark theme, global line-width scale) are app settings and never stored in model files.
- **Crates:** the view trait, registry, built-in views and shared palette live in a new UI crate (`simsandbox-views`); a domain crate provides model-specific views behind an optional `views` feature, so headless builds do not pull in egui. The kernel stays free of UI.

Details: [visualization architecture](../architecture/visualization.md).

## Consequences

- New kinds of models get pictures without changing the app; the pendulum, rigid-body and field views live with their domains.
- The existing views and the `controls.view` representation are migrated in steps, listed in the architecture document; the format version stays 1 until a reader is released.
- The view trait becomes a second public contract next to the Block contract and needs the same care to stay small.
- Exporting to ParaView or Rerun stays available for heavy post-processing ([interaction design](../design/interaction.md)).
