# 2026-11-15 — The equations' front end (review W6)

- **Who:** Dylan Sitarski with Claude Code
- **Goal:** restructure the typed-equations front end (review W6) and fix the eight faults listed in the reference

## Done

- The eight faults fixed, each with a test that failed before; the reference corrected. 620 workspace tests pass; every example the same bytes.

## Decided

- Owner (2026-11-14): the single build of a model that uses models is deferred ([ADR 0090](../decisions/0090-one-engine-for-nested-models.md), as built).

## Left open

- W6's stages: the file split into modules, typed statements, one function table, `parse_with_rows` in steps.
- Three smaller faults found on the way, in [STATUS](../STATUS.md)'s known limitations.

## Notes for the next session

- The safety net for work on the front end: `cargo test --workspace` (bit-for-bit tests against the interpreter and across routes), `crates/simsandbox-eq/tests/reference.rs`, and the hashes of every example's renders, runs and checks before and after.
