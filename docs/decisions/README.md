# Decision records

Architecture decision records (ADRs) capture one significant decision each: the context, the decision and its consequences. They are the project's memory of *why*.

## Index

| ADR | Title | Status | Date |
| --- | --- | --- | --- |
| [0001](0001-rust-core.md) | Rust core | Accepted | 2026-10-15 |
| [0002](0002-layered-block-contract.md) | Layered Block contract | Accepted | 2026-10-15 |
| [0003](0003-desktop-first.md) | Desktop-first | Accepted | 2026-10-15 |
| [0004](0004-method-classification.md) | Method classification instead of "one model, three modes" | Accepted | 2026-10-15 |
| [0005](0005-repository-as-source-of-truth.md) | Repository as single source of truth | Accepted | 2026-10-15 |
| [0006](0006-apache-2-license.md) | Apache-2.0 license | Accepted | 2026-10-15 |
| [0007](0007-json-model-files.md) | JSON model files with a block-type registry | Accepted | 2026-10-16 |
| [0008](0008-native-egui-ui.md) | All-native desktop UI with egui and wgpu | Accepted | 2026-10-16 |
| [0009](0009-hobbyists-first.md) | Hobbyists are the first audience | Accepted | 2026-10-16 |
| [0010](0010-live-loop-pacing.md) | Pace the live loop by wall-clock time | Accepted | 2026-10-16 |
| [0011](0011-parameter-entry.md) | Typed parameter entry; ranges are suggestions | Accepted | 2026-10-16 |
| [0012](0012-controls-in-model-files.md) | Exploration controls are saved in model files | Accepted | 2026-10-16 |
| [0013](0013-run-section.md) | Run settings are saved in a `run` section | Accepted | 2026-10-16 |
| [0014](0014-view-in-controls.md) | The opening view is saved in `controls` | Accepted | 2026-10-16 |
| [0015](0015-pluggable-views.md) | Pluggable views with slots; style options limited to encodings and readability | Accepted | 2026-10-16 |
| [0016](0016-model-curated-views.md) | Models choose the views shown; other fitting views stay one click away | Accepted | 2026-10-17 |
| [0017](0017-typed-equations.md) | A built-in expression parser for typed equations | Accepted | 2026-10-17 |
| [0018](0018-one-xy-plot.md) | One x–y plot replaces the time-series and phase-portrait views | Accepted | 2026-10-17 |
| [0019](0019-equations-without-time.md) | Typed models without time: formulas and discrete maps first | Accepted | 2026-10-17 |
| [0020](0020-methods-for-approximations.md) | Only approximating operations have a method choice | Accepted | 2026-10-17 |
| [0021](0021-maps-formulas-and-sweeps.md) | Maps, formula blocks and parameter sweeps | Accepted | 2026-10-17 |
| [0022](0022-two-parameter-sweeps.md) | Sweeps over two parameters | Superseded by 0028 | 2026-10-17 |
| [0023](0023-automatic-fusion.md) | Automatic fusion of equation blocks, with opt-out and explicit groups | Accepted | 2026-10-17 |
| [0024](0024-heat-map-axes-and-coordinates.md) | Heat map axes and coordinate fields | Accepted | 2026-10-18 |
| [0025](0025-index-aware-view-settings.md) | View settings that depend on the index | Accepted | 2026-10-18 |
| [0026](0026-profile-slices.md) | Profiles of 2D grids as slices | Superseded by 0029 | 2026-10-18 |
| [0027](0027-explicit-experiments.md) | Explicit experiments: sampling, run and reduction | Accepted | 2026-10-18 |
| [0028](0028-sweeps-as-samples.md) | Sweeps recorded as samples | Accepted | 2026-10-18 |
| [0029](0029-profile-cuts.md) | Profiles as cuts in any direction | Superseded by 0031 | 2026-10-18 |
| [0030](0030-filters-and-native-visualization.md) | Filters feed views; native visualization grows with need | Accepted | 2026-10-18 |
| [0031](0031-cut-filters.md) | Cuts as filters with simple modes | Accepted | 2026-10-18 |

## When to write one

Write an ADR when a choice is hard to reverse, affects more than one crate or document, or settles an item in [open questions](../open-questions.md). Small, local choices belong in code comments or the pull request.

## How

1. Copy [`0000-template.md`](0000-template.md) to the next number: `NNNN-short-title.md`.
2. Fill it in; keep it to one page. Status starts as `Proposed` if the owner has not agreed yet, otherwise `Accepted`.
3. Add a row to the index above (the docs check fails otherwise) and to the map in [`docs/README.md`](../README.md).
4. Update the documents the decision affects, and remove the item from open questions if it resolves one.

## Rules

- Accepted ADRs are not rewritten. To change a decision, write a new ADR and set the old one's status to `Superseded by NNNN`.
- Statuses: `Proposed`, `Accepted`, `Superseded by NNNN`, `Rejected`.
