# Documentation system

_Last updated: 2026-10-16_

This page is the map of every document in the repository and the rules for keeping them current. The repository is the single source of truth for SimSandbox ([ADR 0005](decisions/0005-repository-as-source-of-truth.md)).

## Principles

1. **One home per fact.** Each piece of information lives in exactly one document; others link to it.
2. **Typed documents.** Every document has a type (below) that fixes its purpose and when it must change.
3. **Decisions are immutable, status is living.** ADRs and session notes are append-only history; status, roadmap and architecture docs are kept current.
4. **Docs change with code.** A change that alters behaviour, architecture or plans updates the matching document in the same commit.
5. **Checked by CI.** `scripts/check_docs.py` fails the build if a document is missing from this map, a relative link is broken, or an ADR is missing from the decision index.

## Document types

| Type | Purpose | Changes when | Style |
| --- | --- | --- | --- |
| Entry point | Orientation and session protocol for agents and contributors | The workflow changes | Imperative checklist |
| Status | The current state of the project: done, in progress, next, limits | Every working session | Short lists, dated |
| Vision | Why the project exists, pillars, scope and non-goals | Rarely; a change needs an ADR | Prose |
| Roadmap | Phases, gates and the task list for each phase | A task finishes or the plan changes | Checklists |
| Architecture | How the system is built and why it is shaped that way | The design changes | Spec, with diagrams as needed |
| Design | How users experience the product | UX plans change | Spec |
| Decision record (ADR) | One accepted decision, its context and consequences | Never edited after acceptance, except its Status line; superseded by a new ADR | Fixed template |
| Open questions | Undecided questions, each with options and an owner | A question is raised or resolved (resolution becomes an ADR) | Table |
| Risks | Known risks and mitigations | A risk changes | Table |
| Guide | How to do a recurring task | The task changes | Steps |
| Research | Background findings that informed decisions | New research | Prose with sources |
| Session note | What one working session did, decided and left open | Created once per session; not edited later | Fixed template |
| Changelog | User-visible changes per release | Any user-visible change | Keep a Changelog |

## Map

### Root

| Document | Type | What it covers |
| --- | --- | --- |
| [README.md](../README.md) | Entry point | Public front page: what SimSandbox is, how to build it |
| [AGENTS.md](../AGENTS.md) | Entry point | Reading order, working rules and session-close checklist for agents |
| [CLAUDE.md](../CLAUDE.md) | Entry point | Points Claude Code at AGENTS.md |
| [CONTRIBUTING.md](../CONTRIBUTING.md) | Guide | Branches, commits, pull requests, definition of done |
| [CHANGELOG.md](../CHANGELOG.md) | Changelog | User-visible changes |

### Project

| Document | Type | What it covers |
| --- | --- | --- |
| [STATUS.md](STATUS.md) | Status | Current state and next tasks |
| [vision.md](vision.md) | Vision | Purpose, three pillars, scope, non-goals |
| [roadmap.md](roadmap.md) | Roadmap | Phases 0–3, gates, task checklists |
| [open-questions.md](open-questions.md) | Open questions | Decisions not yet made |
| [risks.md](risks.md) | Risks | Risk register |
| [glossary.md](glossary.md) | Guide | Terms used across the project |

### Architecture and design

| Document | Type | What it covers |
| --- | --- | --- |
| [architecture/overview.md](architecture/overview.md) | Architecture | Layers, crate map, data flow, tech stack |
| [architecture/block-contract.md](architecture/block-contract.md) | Architecture | The Block contract, coupling, connections, composition |
| [architecture/method-classification.md](architecture/method-classification.md) | Architecture | Speed and accuracy labels, composition rules, earning Verified |
| [architecture/solver-backends.md](architecture/solver-backends.md) | Architecture | Planned domains and the engines each wraps |
| [architecture/visualization.md](architecture/visualization.md) | Architecture | Signals, pluggable views, slots, style layers, file representation, migration steps |
| [design/interaction.md](design/interaction.md) | Design | Interaction, parameters, visualization, ways to build |

### Decisions

| Document | Type | What it covers |
| --- | --- | --- |
| [decisions/README.md](decisions/README.md) | Decision record | Index of ADRs and how to write one |
| [decisions/0000-template.md](decisions/0000-template.md) | Decision record | Template |
| [decisions/0001-rust-core.md](decisions/0001-rust-core.md) | Decision record | Rust core |
| [decisions/0002-layered-block-contract.md](decisions/0002-layered-block-contract.md) | Decision record | Layered Block contract |
| [decisions/0003-desktop-first.md](decisions/0003-desktop-first.md) | Decision record | Desktop-first |
| [decisions/0004-method-classification.md](decisions/0004-method-classification.md) | Decision record | Method classification |
| [decisions/0005-repository-as-source-of-truth.md](decisions/0005-repository-as-source-of-truth.md) | Decision record | Repository as single source of truth; this documentation system |
| [decisions/0006-apache-2-license.md](decisions/0006-apache-2-license.md) | Decision record | Apache-2.0 license |
| [decisions/0007-json-model-files.md](decisions/0007-json-model-files.md) | Decision record | JSON model files with a block-type registry |
| [decisions/0008-native-egui-ui.md](decisions/0008-native-egui-ui.md) | Decision record | All-native desktop UI with egui and wgpu |
| [decisions/0009-hobbyists-first.md](decisions/0009-hobbyists-first.md) | Decision record | Hobbyists are the first audience |
| [decisions/0010-live-loop-pacing.md](decisions/0010-live-loop-pacing.md) | Decision record | Pace the live loop by wall-clock time |
| [decisions/0011-parameter-entry.md](decisions/0011-parameter-entry.md) | Decision record | Typed parameter entry; ranges are suggestions |
| [decisions/0012-controls-in-model-files.md](decisions/0012-controls-in-model-files.md) | Decision record | Exploration controls are saved in model files |
| [decisions/0013-run-section.md](decisions/0013-run-section.md) | Decision record | Run settings are saved in a `run` section |
| [decisions/0014-view-in-controls.md](decisions/0014-view-in-controls.md) | Decision record | The opening view is saved in `controls` |
| [decisions/0015-pluggable-views.md](decisions/0015-pluggable-views.md) | Decision record | Pluggable views with slots; which style options to expose |
| [decisions/0016-model-curated-views.md](decisions/0016-model-curated-views.md) | Decision record | Models choose the views shown; "More views…" for the rest |
| [decisions/0017-typed-equations.md](decisions/0017-typed-equations.md) | Decision record | Built-in expression parser; typed equation blocks |
| [decisions/0018-one-xy-plot.md](decisions/0018-one-xy-plot.md) | Decision record | One x–y plot replaces time-series and phase views; view presets |
| [decisions/0019-equations-without-time.md](decisions/0019-equations-without-time.md) | Decision record | Typed models without time: formulas and discrete maps first |
| [decisions/0020-methods-for-approximations.md](decisions/0020-methods-for-approximations.md) | Decision record | Only approximating operations have a method choice |
| [decisions/0021-maps-formulas-and-sweeps.md](decisions/0021-maps-formulas-and-sweeps.md) | Decision record | Maps, formula blocks, parameter sweeps, the index axis |
| [decisions/0022-two-parameter-sweeps.md](decisions/0022-two-parameter-sweeps.md) | Decision record | Sweeps over one or two parameters, results as grid fields |
| [decisions/0023-automatic-fusion.md](decisions/0023-automatic-fusion.md) | Decision record | Equation blocks fused automatically; opt-out and explicit groups |
| [decisions/0024-heat-map-axes-and-coordinates.md](decisions/0024-heat-map-axes-and-coordinates.md) | Decision record | Heat map `x`/`y` take the index or declared coordinate fields |
| [decisions/0025-index-aware-view-settings.md](decisions/0025-index-aware-view-settings.md) | Decision record | Slots and settings declare where they apply |
| [decisions/0026-profile-slices.md](decisions/0026-profile-slices.md) | Decision record | Profiles of 2D grids as slices |
| [decisions/0027-explicit-experiments.md](decisions/0027-explicit-experiments.md) | Decision record | Experiments: sampling × run × reduction, at model level |
| [decisions/0028-sweeps-as-samples.md](decisions/0028-sweeps-as-samples.md) | Decision record | Sweeps recorded as samples; grid structure as metadata |
| [decisions/0029-profile-cuts.md](decisions/0029-profile-cuts.md) | Decision record | Profiles as straight cuts in any direction; chosen horizontal axis; interpolation |

### Guides

| Document | Type | What it covers |
| --- | --- | --- |
| [guides/development-setup.md](guides/development-setup.md) | Guide | Toolchain on Windows, macOS, Linux and WSL; commands |
| [guides/verification-policy.md](guides/verification-policy.md) | Guide | Rules and tests every numerical method must meet |
| [guides/adding-an-integrator.md](guides/adding-an-integrator.md) | Guide | Worked steps for adding a method to the equation domain |
| [guides/adding-a-block-type.md](guides/adding-a-block-type.md) | Guide | Making a model definition loadable from model files |

### Research and history

| Document | Type | What it covers |
| --- | --- | --- |
| [research/market-landscape.md](research/market-landscape.md) | Research | Existing tools and the gap SimSandbox targets |
| [sessions/README.md](sessions/README.md) | Session note | Index of session notes and how to write one |
| [sessions/_template.md](sessions/_template.md) | Session note | Template |
| [sessions/2026-10-16-kernel-prep-and-adrs.md](sessions/2026-10-16-kernel-prep-and-adrs.md) | Session note | Kernel prep for files and UI; ADRs 0007–0013; model file format; desktop app shell; parameter panel; phase and 3D views; saved views; ADR 0015 |
| [sessions/2026-10-15-project-inception.md](sessions/2026-10-15-project-inception.md) | Session note | Market research through the first commit |

## Conventions

- Living documents start with `_Last updated: YYYY-MM-DD_` under the title.
- File names are lowercase with hyphens; ADRs are `NNNN-short-title.md`; session notes are `YYYY-MM-DD-short-title.md`.
- Link with relative paths so links work on GitHub and locally.
- Diagrams: Mermaid in fenced code blocks (GitHub renders them), so they stay diffable.
- Code references use paths from the repository root, e.g. `crates/simsandbox-core/src/block.rs`.
