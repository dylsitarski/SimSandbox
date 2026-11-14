# 2026-11-14 — Code review

- **Who:** Dylan Sitarski with Claude Code
- **Goal:** review the whole project against its vision, for performance, cleanliness, and usability by people and AI agents; deliver a report

## Done

- [Review report](../research/2026-11-14-code-review.md): four read-only reviews by crate group and a hands-on pass with the release CLI. No code changed.
- Reproduced with the CLI: a spring formula block on a mass is not fused and runs 5% off under a `Quantitative` label (W1); 200 000 nested parentheses abort `check` with a stack overflow (W2); unit exponents wrap at ±128 (W3); `t` drifts by accumulation.

## Decided

- Nothing. The report's work items wait for the owner.

## Left open

- Q56–Q59 in [open questions](../open-questions.md); work items W1–W13 in the report, not yet placed in STATUS's Next up.

## Notes for the next session

- Findings not marked "reproduced" in the report were read in code, not run. Check before acting, in particular the study that may read a saved file instead of unsaved edits.
- Two wheels (numpy, scipy; 52 MB) are tracked at the repository root, apparently by accident. Left in place for the owner to remove.
- The cloud environment's setup script fails (`install: missing destination file operand`); the Rust toolchain was present anyway.
