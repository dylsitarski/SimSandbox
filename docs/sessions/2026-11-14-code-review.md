# 2026-11-14 — Code review

- **Who:** Dylan Sitarski with Claude Code
- **Goal:** review the whole project against its vision, for performance, cleanliness, and usability by people and AI agents; deliver a report

## Done

- [Review report](../research/2026-11-14-code-review.md): four read-only reviews by crate group and a hands-on pass with the release CLI.
- The review's first-priority small items, at the owner's request: W1 (fusion rule and notes, [ADR 0095](../decisions/0095-one-block-with-states-fuses-with-its-formulas.md)), W2 (depth limits), W3 (checked unit powers), W12 (schema keys and a test), the committed wheels removed. 589 workspace tests pass; clippy and fmt clean; the Python tests were not run.
- Reproduced with the CLI: a spring formula block on a mass is not fused and runs 5% off under a `Quantitative` label (W1); 200 000 nested parentheses abort `check` with a stack overflow (W2); unit exponents wrap at ±128 (W3); `t` drifts by accumulation.

## Decided

- Owner: the Phase 2 gate before further typed-equation features (Q57).
- Owner accepted [ADR 0093](../decisions/0093-scenes-from-primitive-layers.md) (scenes; frames added at the owner's question).
- [ADR 0090](../decisions/0090-one-engine-for-nested-models.md), accepted: the merged simulator is the one engine for nested models (the owner left the choice to the agent).
- Owner accepted [ADR 0091](../decisions/0091-equations-first-hosting-for-existing-codes.md) (equations first; vision, roadmap and risks revised) and [ADR 0092](../decisions/0092-what-a-model-file-may-read.md) (what a model may read).
- Owner: views from primitives are designed first, and only what the cart-pole needs is built before the gate.
- Owner: the full audience, none first ([ADR 0094](../decisions/0094-the-full-audience.md), superseding ADR 0009).

## Left open

- The review's W4 and W5 (first priority, a few days each) and its later items; ADRs 0090, 0092 and 0093 to build.

## Notes for the next session

- Findings not marked "reproduced" in the report were read in code, not run. Check before acting, in particular the study that may read a saved file instead of unsaved edits.
- The two wheels (numpy, scipy; 52 MB) are gone from the tree but stay in the history; shrinking the repository would need the history rewritten, which is the owner's call.
- A chain of operators counts towards an expression's depth (`a + b + …` nests to the left), so a flat sum of more than about 1000 terms is refused: raise `MAX_DEPTH` only with a test that everything walking a tree survives it.
- The cloud environment's setup script fails (`install: missing destination file operand`); the Rust toolchain was present anyway.
