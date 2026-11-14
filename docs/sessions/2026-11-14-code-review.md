# 2026-11-14 — Code review

- **Who:** Dylan Sitarski with Claude Code
- **Goal:** review the whole project against its vision, for performance, cleanliness, and usability by people and AI agents; deliver a report

## Done

- [Review report](../research/2026-11-14-code-review.md): four read-only reviews by crate group and a hands-on pass with the release CLI.
- The review's first-priority small items, at the owner's request: W1 (fusion rule and notes, [ADR 0095](../decisions/0095-one-block-with-states-fuses-with-its-formulas.md)), W2 (depth limits), W3 (checked unit powers), W12 (schema keys and a test), the committed wheels removed. 589 workspace tests pass; clippy and fmt clean; the Python tests were not run.
- W5 ([ADR 0097](../decisions/0097-implicit-steps-relative-tolerances-and-fallback.md)) and W4 ([ADR 0096](../decisions/0096-the-native-cache.md)), at the owner's request. 602 workspace tests pass, among them the native build from end to end; clippy and fmt clean; the Python tests were not run.
- Reproduced with the CLI: a spring formula block on a mass is not fused and runs 5% off under a `Quantitative` label (W1); 200 000 nested parentheses abort `check` with a stack overflow (W2); unit exponents wrap at ±128 (W3); `t` drifts by accumulation.

- Second priority begun: the typed-equations reference, held to the code by a test and given by `describe` (W11); run files and render shared by the three interfaces (W10; the Python tests were built and run for it: 30 pass); STATUS's history archived, two guides brought up to date. 610 workspace tests pass.

## Decided

- Owner: the Phase 2 gate before further typed-equation features (Q57).
- Owner accepted [ADR 0093](../decisions/0093-scenes-from-primitive-layers.md) (scenes; frames added at the owner's question).
- [ADR 0090](../decisions/0090-one-engine-for-nested-models.md), accepted: the merged simulator is the one engine for nested models (the owner left the choice to the agent).
- Owner accepted [ADR 0091](../decisions/0091-equations-first-hosting-for-existing-codes.md) (equations first; vision, roadmap and risks revised) and [ADR 0092](../decisions/0092-what-a-model-file-may-read.md) (what a model may read).
- Owner: views from primitives are designed first, and only what the cart-pole needs is built before the gate.
- Owner: the full audience, none first ([ADR 0094](../decisions/0094-the-full-audience.md), superseding ADR 0009).

## Left open

- The review's W6, W7, W8, W9 and the kernel tidy; its third and fourth priorities; ADRs 0090, 0092 and 0093 to build; tolerances per state for implicit methods (ADR 0097 point 6).

## Notes for the next session

- Robertson's problem showed two faults the review had not: the step's Jacobian at a start with no intermediate species sends Newton's method off to infinity (hence fresh Jacobians at every iteration as the fallback, not one retry), and the structure probed at the start misses every product with a factor of 0.
- A perturbation relative to each state alone is wrong, though it looks more accurate: a state near 0 beside one near 1 changes the rates by less than their rounding, and the Jacobian's entry is noise. Hence a floor shared by the system.
- The permission checks of the native cache read mode bits only; tests ran as root.
- Findings not marked "reproduced" in the report were read in code, not run. Check before acting, in particular the study that may read a saved file instead of unsaved edits.
- The two wheels (numpy, scipy; 52 MB) are gone from the tree but stay in the history; shrinking the repository would need the history rewritten, which is the owner's call.
- A chain of operators counts towards an expression's depth (`a + b + …` nests to the left), so a flat sum of more than about 1000 terms is refused: raise `MAX_DEPTH` only with a test that everything walking a tree survives it.
- The cloud environment's setup script fails (`install: missing destination file operand`); the Rust toolchain was present anyway.
