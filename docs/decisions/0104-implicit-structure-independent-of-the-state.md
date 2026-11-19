# 0104 — The implicit methods' structure does not depend on where it was found

- **Status:** Accepted (agent, 2026-11-19, on the owner's request to fix the problem and choose the fix)
- **Date:** 2026-11-19
- **Supersedes:** the structure bullet of [ADR 0060](0060-arrays-in-typed-equations.md)'s "As built (step 4)", in part
- **Builds on:** [ADR 0097](0097-implicit-steps-relative-tolerances-and-fallback.md) (perturbations relative to the state), [ADR 0062](0062-probes-recording-and-replay.md) (exact replay), [ADR 0087](0087-implicit-lu-on-the-matrix-pattern.md) (the LU)

## Context

Backward Euler and TR-BDF2 (`crates/simsandbox-eq/src/implicit.rs`) keep the Jacobian's structure in a `Workspace` between steps; it decides which entries of the finite-difference Jacobian are kept. When a system does not give its structure (a Rust `OdeSystem` whose `sparsity()` is `None`, or a fused group), it was found by perturbing each state alone at the state of the first implicit step. ADR 0060 said a missed entry "slows Newton's method but cannot change what it converges to". That is wrong at the level of bits: Newton's method stops within its tolerance (10⁻¹⁰), and where it stops depends on the matrix.

Entries vanish at particular states. The case found (the rigid-body spike, 2026-11-19): a mechanism's accelerations are quadratic in its velocities (Coriolis terms, `v₁·v₂`), so started at rest, perturbing one velocity alone leaves them unchanged and the entries are missed. A cart-pole run snapshotted at step 2000 and restored into a **new** workspace found the structure again away from rest, with those entries, and differed by up to 5.9×10⁻¹² by step 5000: a run restarted from a saved state (a new session loading a run, `start-from`, a snapshot after the workspace was rebuilt) broke ADR 0062's exact replay. Restoring into the same workspace, or passing the structure, was bit-identical. Typed equations give their structure exactly and were not affected.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Require `sparsity()` for systems used with implicit methods | Exact, no guessing | Fused groups cannot give one (the kernel's `GroupRhs` has no structure); every Rust system must declare it |
| **Dense when not given, for small systems** | Exact whatever the state; nothing to declare | `n` evaluations per Jacobian and a dense LU: too slow for large systems (a fused 128 × 128 grid has 32 768 states) |
| **Probe at the state and at offset states, union** | Cheap once; finds entries that vanish at one state (a product of states at rest) | Not a guarantee: an entry zero at all the probed states (a branch never taken there, a state multiplied by an input that is 0 when probed) is still missed |
| Store the structure in snapshots and run files | Exact replay of what was found | A format change for something the system should know; a structure found at rest is still wrong |

## Decision

When a system does not give its structure:

1. **Up to 128 states (`implicit::DENSE_UP_TO`), it is dense.** Exact whatever the state, so a restored run replays bit for bit. This covers the built-in Rust systems (`eq.heat_rod`: 50) and mechanisms.
2. **Beyond that, it is probed at three states:** just beside the state (every state moved by 1 to 2 thousandths of its size, as the probe before this did), and two states where every state is moved by a quarter to three quarters of `s + |xⱼ|` (a fixed sequence; `s` the state's scale, ADR 0097, so other units probe alike), up at one and down at the other, alternating from state to state; the union of what changes is kept. A rate that is not a number at a probed state counts as depending on the state moved (safe: the row is denser, never wrong).
3. Systems that give their structure (typed equations; an `OdeSystem` with `sparsity()`) are unchanged. Large Rust systems should give it: point 2 can still miss an entry, and the run would then not replay exactly from a fresh workspace.

## Consequences

- Restored runs of small undeclared systems are bit-identical to uninterrupted runs, tested in `implicit.rs` (`a_run_restored_into_a_fresh_workspace_gives_the_same_bits`: two oscillators coupled through `v₁·v₂`, started at rest, TR-BDF2, snapshot, restore into a new workspace; one copy (dense) and 64 copies (256 states, probed)). The test fails with the old single probe.
- Results of undeclared systems with implicit methods change in their last bits once (now dense or with more entries). `eq.heat_rod` with TR-BDF2 takes 50 evaluations per Jacobian instead of 3; negligible at its size.
- The rigid-body spike can drop its explicit dense structure if its system has at most 128 states; it may keep it.
- Fused groups of more than 128 states remain on the probe. Giving a fused group its members' structure would need the kernel's `GroupSolver` to carry it; not done until a model needs it.

- Ported onto the development branch on 2026-11-19 from a branch made from `main`: first written as ADR 0090 there, renumbered because 0090–0103 were taken; point 2 merged with the branch's probe of 2026-11-14 (beside the state, scaled by ADR 0097), and its open question is Q63.

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none.
