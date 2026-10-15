# Working on SimSandbox

Guidance for AI-assisted development (and humans) in this repository.

## Before you change anything

- Read `README.md` and the decision records in `docs/decisions/`. Do not contradict an accepted decision without writing a new ADR that supersedes it.
- The kernel (`simsandbox-core`) contains **no physics**. Solvers, integrators and domain logic belong in domain crates (`simsandbox-eq`, later `simsandbox-rigid`, `simsandbox-agents`, ...).
- Keep the `Block` trait small. New behaviour goes into optional capability traits exposed through `as_*` accessors.

## Numerical code rules

AI-written numerics can look right and be subtly wrong. Therefore:

1. Every new integrator, solver or coupling algorithm ships with a **verification test** against an analytic or published benchmark solution, in the crate's `tests/verification.rs` (or an equivalent file).
2. Fixed-order methods also get a **convergence-order test** (halve the step, check the error ratio).
3. Every method declares honest `MethodLabels`. Never label a method `Verified`; that label is earned per case.
4. Units: every port and parameter carries a `Unit`. Never pass dimensioned values through dimensionless ports to get around a unit check.

## Commands

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI runs all three on Windows, macOS and Linux; keep them green.

## Style

- Stable Rust only, no `unsafe` (forbidden at the workspace level).
- Prefer the standard library; add a dependency only when it is well maintained and clearly worth it, and note why in the PR.
- Public items get a doc comment that says what, not how.
