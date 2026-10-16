# Verification policy

_Last updated: 2026-10-15_

AI-written numerical code can look right and be subtly wrong: lost conservation, instability at edge cases, a sign error in a Jacobian. These rules make every method trustworthy before anyone relies on it.

## Rules for every method (integrator, solver, coupling algorithm)

1. **Reference test.** At least one test against an analytic solution or a published benchmark, with an explicit error bound. Put it in the crate's `tests/verification.rs` (or a sibling file named for the method).
2. **Convergence-order test** for fixed-order methods: halve the step, check that the error ratio matches the theoretical order (e.g. about 16 for RK4).
3. **Tolerance test** for adaptive methods: the error stays below the requested tolerance on a reference problem.
4. **Honest labels.** Declare `MethodLabels` that match reality. Never declare `Verified`; it is earned per case by the planned verification tooling ([method classification](../architecture/method-classification.md)).
5. **Units.** Every port and parameter carries a real `Unit`. Never pass dimensioned values through dimensionless ports to get past a unit check.
6. **Determinism.** Same inputs, same outputs. Seed anything random and expose the seed as a parameter.

## Rules for coupling and fusion

- A new coupling scheme comes with a test that compares it against the fused (exact-coupling) solution of the same equations, like `cosimulation_adds_coupling_error`.
- Snapshot and restore must reproduce a run bit for bit (`snapshot_and_rewind_reproduce_the_run`).

## Reference problems in use

| Problem | Exact solution | Used by |
| --- | --- | --- |
| Exponential decay x' = −x | x = e^(−t) | Order tests for Euler and RK4; Dopri5 tolerance (`integrators.rs`) |
| Two coupled oscillators, ω0² = 1, κ = 0.5 | x1 = (cos t + cos √2 t) / 2 | Fusion and co-simulation tests (`tests/verification.rs`) |

Add new reference problems to this table when you introduce them. Candidates for later backends: Lotka–Volterra invariant (ODE), lid-driven cavity (CFD), simple pendulum period (rigid body).

## Review checklist for numerical pull requests

- [ ] Reference test with an error bound
- [ ] Order or tolerance test
- [ ] Labels justified in the PR description
- [ ] Units on all ports and parameters
- [ ] Table above updated if a new reference problem was added
