# Verification policy

_Last updated: 2026-10-28_

AI-written numerical code can look right and be subtly wrong: lost conservation, instability at edge cases, a sign error in a Jacobian. These rules make every method trustworthy before anyone relies on it.

## Rules for every method (integrator, solver, coupling algorithm)

1. **Reference test.** At least one test against an analytic solution or a published benchmark, with an explicit error bound. Put it in the crate's `tests/verification.rs` (or a sibling file named for the method).
2. **Convergence-order test** for fixed-order methods: halve the step, check that the error ratio matches the theoretical order (e.g. about 16 for RK4).
3. **Tolerance test** for adaptive methods: the error stays below the requested tolerance on a reference problem.
4. **Honest labels.** Declare `MethodLabels` that match reality. Never declare `Verified`, even for exact methods (conversions, inputs, formulas): it is earned per case by the planned convergence check ([method classification](../architecture/method-classification.md), [ADR 0061](../decisions/0061-verified-is-earned-per-case.md)).
5. **Units.** Every port and parameter carries a real `Unit`. Never pass dimensioned values through dimensionless ports to get past a unit check.
6. **Determinism.** Same inputs, same outputs. Seed anything random and expose the seed as a parameter.

## Rules for coupling and fusion

- A new coupling scheme comes with a test that compares it against the fused (exact-coupling) solution of the same equations, like `cosimulation_adds_coupling_error`.
- Snapshot and restore must reproduce a run bit for bit (`snapshot_and_rewind_reproduce_the_run`).
- Results pinned per platform ([ADR 0103](../decisions/0103-rigid-body-engine.md) point 7): `crates/simsandbox-cli/tests/platform_hashes.rs` runs the mechanism cases (the cart-pole, the cart-pole grabbed from a run file, a 10-link chain under Dormand–Prince and under TR-BDF2) through the binary and compares a hash of their printed results with `tests/platform-hashes.json`, keyed `os-arch`. CI runs it on Linux, Windows and macOS with the other tests. A platform with no values fails, printing the line to add. A change meant to alter results is pinned again on each platform: `SIMSANDBOX_PIN_HASHES=1 cargo test -p simsandbox-cli --test platform_hashes` writes this platform's line, and CI's failure gives the others'. Say why in the commit.

## Reference problems in use

| Problem | Exact solution | Used by |
| --- | --- | --- |
| Exponential decay x' = −x | x = e^(−t) | Order tests for Euler and RK4; Dopri5 tolerance (`integrators.rs`) |
| Two coupled oscillators, ω0² = 1, κ = 0.5 | x1 = (cos t + cos √2 t) / 2 | Fusion and co-simulation tests (`tests/verification.rs`) |
| Heat equation ∂T/∂t = α ∂²T/∂x², one mode (sine held at 0, cosine insulated, sine periodic) | T = mode · e^(−α k² t); on cells of width h exactly mode · e^(−α (4/h²) sin²(kh/2) t) | Second order in space for each edge kind, the stencil's discrete rate to rounding, the order in time of RK4 (4), backward Euler (1) and TR-BDF2 (2) on the grid, implicit steps far beyond the explicit limit (`simsandbox-eq/tests/heat.rs`, [ADR 0060](../decisions/0060-arrays-in-typed-equations.md)) |
| Heat equation, a Gaussian on a rod held at 0 at both ends | The spreading Gaussian by images: Σₙ g(x − c − 2nL) − g(x + c − 2nL), g of variance w² + 4αt | The typed rod and `eq.heat_rod` (`tests/heat.rs`), the typed heat-rod example (`simsandbox-file/tests/arrays.rs`) |
| Pendulum period, a point mass on a massless rod from rest at a | T = 4 K(sin(a/2)) / ω₀, K the complete elliptic integral | `mech.planar` with RK4 at 1 ms, within 1e−10 at 0.1 and 2 rad (`simsandbox-mech/tests/verification.rs`) |
| Double pendulum of two uniform rods | Its Lagrange equations written by hand; RK4 on them at 1e−5 s | The articulated-body algorithm's accelerations to 1e−13; RK4 order 4 (`simsandbox-mech/tests/verification.rs`) |
| Cart-pole with a cart force, a hinge torque and a load on the pole | Its Lagrange equations written by hand | Accelerations to 1e−13 (`simsandbox-mech/tests/verification.rs`) |
| Random trees of revolute, prismatic and fixed joints, with springs, damping and loads | The composite rigid-body algorithm with recursive Newton–Euler (O(n³), written separately) | The articulated-body algorithm to 1e−12 (`simsandbox-mech/tests/verification.rs`) |
| A damped mass on a spring, m ẍ + c ẋ + k x = 0 | The underdamped solution | A prismatic joint with damping and stiffness, RK4 at 1 ms within 1e−12 (`simsandbox-mech/tests/verification.rs`) |

Add new reference problems to this table when you introduce them. Candidates for later backends: Lotka–Volterra invariant (ODE), lid-driven cavity (CFD).

## Review checklist for numerical pull requests

- [ ] Reference test with an error bound
- [ ] Order or tolerance test
- [ ] Labels justified in the PR description
- [ ] Units on all ports and parameters
- [ ] Table above updated if a new reference problem was added
