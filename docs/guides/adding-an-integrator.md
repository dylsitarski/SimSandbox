# Adding an integrator

_Last updated: 2026-11-14_

A worked example of adding a method to the equation domain. The same pattern (model definition stays, new method with labels and tests) applies to other domains.

## Steps

1. **Add a variant** to `Integrator` in `crates/simsandbox-eq/src/integrators.rs`, carrying its settings (e.g. `Rk45 { rtol: f64, atol: f64 }` or `Leapfrog { substeps: usize }`).
2. **Label it** in `Integrator::labels()` with an honest speed and accuracy class, and give it a readable name in `method_spec()`.
3. **Implement it** in `Integrator::advance_in()` (which `advance()` calls with an empty workspace). Working arrays come from the `Workspace` (`ws.buffers(count, n)`), kept between steps, not allocated at each. The signature is fixed: advance `x` in place from `t` to `t + dt` using `f(t, x, dx)`. A step that cannot be taken returns an error saying why and leaves `x` as it was given ([ADR 0097](../decisions/0097-implicit-steps-relative-tolerances-and-fallback.md)); sizes (tolerances, perturbations) are measured against the state, never against 1, so a system gives the same run in any units. Inputs are held constant over the step by `OdeBlock`, so the method does not see them.
4. **Test it** following the [verification policy](verification-policy.md): a reference test, plus an order test (fixed step) or a tolerance test (adaptive). The existing tests in `integrators.rs` are templates.
5. **Expose it** where users choose methods: give it a file name and options in `Integrator::from_settings()` and `Integrator::settings()` (they must round-trip; see the test in `crates/simsandbox-eq/src/registry.rs`), and list it with its options where block types say which methods they take, which `describe` shows: the two `.method(…)` lists in `crates/simsandbox-eq/src/registry.rs` (the built-in systems' and typed equations'). The list is written three times for now ([review](../research/2026-11-14-code-review.md), section 3).
6. **Document it:** add a row to "Labels of the current methods" in [method classification](../architecture/method-classification.md), a line under **Unreleased** in `CHANGELOG.md`, and update `docs/STATUS.md`.

## Things to keep in mind

- No allocation inside hot loops if the method is meant to be real-time; preallocate per call at most.
- Symplectic or structure-preserving methods need their own conservation test (energy drift over long runs).
- Implicit methods live in `crates/simsandbox-eq/src/implicit.rs`: Newton's method on a Jacobian found by finite differences over a colouring of its structure, which a system gives exactly (`OdeSystem::sparsity`; typed equations do) or which is probed. A new implicit scheme adds its stages to `substep` there and reuses `stage` ([ADR 0097](../decisions/0097-implicit-steps-relative-tolerances-and-fallback.md)).
- A method must give the same bits on every machine and core count, and keep nothing numeric from one step to the next, or a rewound run would not replay ([ADR 0062](../decisions/0062-probes-recording-and-replay.md)).
