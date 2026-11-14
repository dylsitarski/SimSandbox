# 0097 — Implicit steps: tolerances relative to the state, fresh Jacobians when a stage fails

- **Status:** Accepted (owner, 2026-11-14: asked for the review's work item W5)
- **Date:** 2026-11-14
- **Builds on:** [ADR 0060](0060-arrays-in-typed-equations.md) (implicit methods), [ADR 0087](0087-implicit-lu-on-the-matrix-pattern.md); [review](../research/2026-11-14-code-review.md) W5

## Context

Backward Euler and TR-BDF2 solve each stage by Newton's method with one Jacobian per communication step. The review found:

- **Sizes measured against 1.** A stage converged when its last correction was below 1e-10 of the largest state *or of 1*, and the Jacobian's finite differences perturbed each state by 1.5e-8 of its size *or of 1*. Values are in SI, so a system of small numbers (currents in nanoamperes, lengths in micrometres) converged at a tenth of its state and was perturbed by many times it. Measured on a stiff nonlinear problem with a known solution: in units of 1e-9 the error was 0.018 where it is 0.000079 in units of 1.
- **No second try.** A stage that did not converge with the step's Jacobian was an error, though the Jacobian may be far from the one the solution needs (a state that falls fast within the step). Robertson's reactions, the standard stiff test, could not be solved at all.
- **A failed step left the state half-moved.**
- **Structure found at one point.** Without a structure given, each state was perturbed alone at the step's start: a product hides what it depends on where its other factor is 0 (a reaction `k·a·b` with no `b` yet), for the whole run.
- **Dormand–Prince** met a rate that is not a number by trying the full step 10 000 times and reporting "step size underflow".

## Decision

1. **Relative to the state.** A stage has converged when its last correction is below 1e-10 of the largest value among its solution and its right side (the state it starts from). Perturbations are relative to each state's size, with a floor of the largest state when all are below 1 (1 as before otherwise, and when all are 0). A system gives the same run in any units.
2. **When a stage fails** with the step's Jacobian, it goes on with the Jacobian taken afresh at every iteration (20 at most): from where it got to, or from its start if it ran off to values that are not numbers. A stage that converges with the step's Jacobian is computed exactly as before.
3. **A step that fails is an error and leaves the state as it was**, for the implicit methods and Dormand–Prince. The error says whether the iteration did not converge or the equations gave values that are not numbers.
4. **Structure is probed beside the state**, every state moved a little and each by a different amount, so products and differences that vanish at the start show what they depend on. Typed equations give their structure exactly and are unchanged.
5. **Dormand–Prince** treats an estimate that is not a number as a rejected step, shortens the step as far as one rejection may, and gives up with that reason when no step is short enough.
6. Not done: tolerances per state (`rtol`, `atol` as method options) for systems whose states differ by many orders of magnitude; the convergence test is still of the largest state.

## Consequences

- Results change, in their last bits or more, for: systems all of whose states are below 1 run with an implicit method (the perturbation's floor); code-written systems and fused groups run with an implicit method whose structure the probe now finds whole; and, rarely, any implicit run where a correction lay between the old and new thresholds. Every example gives the same bytes (none uses an implicit method); every existing test passes unchanged.
- Runs that failed to converge may now succeed, more slowly in the stages that needed it.
- Verification added ([policy](../guides/verification-policy.md)): order kept on a stiff nonlinear problem with a known solution; the same in units of 1e-9, 1e-3 and 1e6; Robertson's problem against the published values at t = 40; Van der Pol's oscillator for Dormand–Prince against RK4 with very short steps.

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none.
