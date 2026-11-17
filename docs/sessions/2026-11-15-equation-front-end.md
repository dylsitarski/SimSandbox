# 2026-11-15 — The equations' front end (review W6)

- **Who:** Dylan Sitarski with Claude Code
- **Goal:** restructure the typed-equations front end (review W6) and fix the eight faults listed in the reference

## Done

- The eight faults fixed, each with a test that failed before; the reference corrected. 620 workspace tests pass; every example the same bytes.
- Stage 1: `symbolic.rs` split into `symbolic/` (eight files), moves only.
- Stages 2 and 4 (2026-11-16): typed statements; `parse_lines` a pipeline of eight phases with typed results; compared with the old code over 16 568 definitions. Three faults it brought to light fixed (a crash on a starting mesh larger than its table; duplicate parameters per cell).
- 2026-11-17: the three faults left fixed (rules' units, a quantity named like a transfer, the first wrong `start` told); a plain number in a rule now in its column's unit.

## Decided

- Owner (2026-11-14): the single build of a model that uses models is deferred ([ADR 0090](../decisions/0090-one-engine-for-nested-models.md), as built).

## Left open

- W6's stage 3: one table of the language's functions; `axes_of` and `lower` are still one long match each.

## Notes for the next session

- The comparison of old and new front end over a corpus of definitions (harvested from examples, the reference and tests, then mutated) found what the tests did not: build such a corpus before the next change of this size. Some error choices depend on `HashMap` order and differ from run to run; compare those without line and column.
- The safety net for work on the front end: `cargo test --workspace` (bit-for-bit tests against the interpreter and across routes), `crates/simsandbox-eq/tests/reference.rs`, and the hashes of every example's renders, runs and checks before and after.
