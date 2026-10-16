# Adding an integrator

_Last updated: 2026-10-15_

A worked example of adding a method to the equation domain. The same pattern (model definition stays, new method with labels and tests) applies to other domains.

## Steps

1. **Add a variant** to `Integrator` in `crates/simsandbox-eq/src/integrators.rs`, carrying its settings (e.g. `Rk45 { rtol: f64, atol: f64 }` or `Leapfrog { substeps: usize }`).
2. **Label it** in `Integrator::labels()` with an honest speed and accuracy class, and give it a readable name in `method_spec()`.
3. **Implement it** in `Integrator::advance()`. The signature is fixed: advance `x` in place from `t` to `t + dt` using `f(t, x, dx)`. Inputs are held constant over the step by `OdeBlock`, so the method does not see them.
4. **Test it** following the [verification policy](verification-policy.md): a reference test, plus an order test (fixed step) or a tolerance test (adaptive). The existing tests in `integrators.rs` are templates.
5. **Expose it** where users choose methods: today the `--method` flag in `crates/simsandbox-cli/src/main.rs`.
6. **Document it:** add a row to "Labels of the current methods" in [method classification](../architecture/method-classification.md), a line under **Unreleased** in `CHANGELOG.md`, and update `docs/STATUS.md`.

## Things to keep in mind

- No allocation inside hot loops if the method is meant to be real-time; preallocate per call at most.
- Symplectic or structure-preserving methods need their own conservation test (energy drift over long runs).
- Implicit or stiff methods will want the `Derivatives` capability for Jacobians; if you add that, update the [block contract](../architecture/block-contract.md) table.
