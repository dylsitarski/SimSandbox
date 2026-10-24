# 0016 — Models choose the views shown; other fitting views stay one click away

- **Status:** Accepted
- **Date:** 2026-10-17

## Context

[ADR 0015](0015-pluggable-views.md) made views pluggable and offered every view type whose slots a model's signals can fill. The owner pointed out that a view that fits is not necessarily useful: a 3D plot of `a.x`, `a.v`, `b.x` for the coupled oscillators is valid but unhelpful as a first view ([open question Q11](../open-questions.md#resolved)). At the same time, plotting outputs against each other in ways the author did not plan is how hobbyists explore ([ADR 0009](0009-hobbyists-first.md)). This ADR extends ADR 0015; it does not replace it.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| (a) Offer every view whose slots fit (as built) | Nothing to curate | Fit is not usefulness; the selector grows with every new view type |
| (b) Offer only the views the model file lists | The author's intent is clear | Blocks exploration the author did not foresee |
| (c) Show the file's views; other fitting views behind "More views…" | Curated first impression, exploration one click away | One more menu |

## Decision

- **The model's views are the views shown.** The app opens with the views in `controls.views` ([ADR 0015](0015-pluggable-views.md)), several at once in tiles. A file with no views opens with one view chosen by fit (the time series).
- **Other views stay available.** A "More views…" menu adds any view type whose slots the model's signals can fill, as another tile; a tile can be closed. Save writes the tiles as they are, so a user's additions become part of the model's views.
- **Domain views fit only their own models.** A view type registered by a domain (e.g. a pendulum drawing) declares the block types it applies to and never fits other models. Generic views (`plot.*`) fit any signals their slots accept.
- **Fit rules may tighten.** Units and port roles (position, angle) can later keep meaningless combinations out of default bindings; they never hide a view the file lists.

## Consequences

- The view selector of one-view-at-a-time is replaced by tiles plus "More views…" (views migration step 3, [visualization architecture](../architecture/visualization.md)).
- The view type contract gains a way to state which block types a domain view applies to; it is added with the first domain view (migration step 6).
- Example files should list the views that make each model understandable.
