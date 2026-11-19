# 2026-11-19 — Implicit methods' structure and exact replay

- **Who:** agent (Claude Code)
- **Goal:** make runs with backward Euler or TR-BDF2 on systems that do not give their Jacobian's structure replay bit for bit when restored into a fresh workspace (found by the rigid-body spike).

## Done

- `crates/simsandbox-eq/src/implicit.rs`: an undeclared structure is dense up to `DENSE_UP_TO` (128) states, else probed at the state and two offset states (union; on the development branch merged with its own probe: beside the state, scaled to the state, rates that are not numbers counting as dependent). Tests: a restored run into a fresh workspace gives the same bits (1 and 64 copies of two oscillators coupled through `v₁·v₂`, started at rest); the probe finds the vanishing entries. The restore test fails with the old single probe (checked).
- ADR 0060's structure note corrected; `OdeSystem::sparsity`'s doc and the heat-rod test updated.

## Decided

- [ADR 0104](../decisions/0104-implicit-structure-independent-of-the-state.md): dense when small, three-point probe when large.

## Left open

- [Q63](../open-questions.md): exact structure for large fused groups.

## Notes for the next session

- The issue's own example, `v' = −x + v²` at `v = 0`, was found by the old probe too: its 10⁻³ perturbation makes `v²` nonzero. Entries are missed for products of two states both 0 (`v₁·v₂`), or a state times an input that is 0.
- The rigid-body spike (`spikes/rigid-body/`) is not on this branch; it passes its structure explicitly and can keep doing so.
- `maturin` was not installed in this container (the environment's setup script failed), so the Python tests were not run; the Python crate's fmt and clippy pass. Nothing in the Python API changed.
- Ported to `claude/great-ramanujan-shfh9u` the same day (from branch `claude/adoring-archimedes-8t6bo2`): ADR 0090 → 0104, Q56 → Q63, the probe merged as above.
