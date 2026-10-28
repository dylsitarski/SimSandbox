# Method classification

_Last updated: 2026-10-15_

Every method carries two labels, one for speed and one for accuracy. They are separate because they are independent: an analytic pendulum is both real-time and verified, while a coarse CFD run can be slow and only roughly right. Rationale: [ADR 0004](../decisions/0004-method-classification.md). Code: `SpeedClass`, `AccuracyClass`, `MethodLabels` in `crates/simsandbox-core/src/block.rs`.

| Speed class | Meaning | Examples |
| --- | --- | --- |
| Real-time | Keeps a 30–60 fps loop on a typical desktop GPU and accepts live input | Fixed-step RK4 on small ODEs, MuJoCo rigid bodies, GPU stable-fluids |
| Responsive | Results in seconds to minutes; change a parameter, then rerun | Adaptive stiff solvers, mid-size agent models, small FEM cases |
| Batch | Hours or more, run as a job, often on a cluster | Full OpenFOAM cases, large Monte Carlo ensembles |

| Accuracy class | Meaning | What backs it |
| --- | --- | --- |
| Qualitative | Right behaviour; numbers not to be trusted | Visual plausibility only |
| Quantitative | Numerically controlled, not yet checked for this case | Declared method, order and tolerances |
| Verified | Error estimated for this case; benchmarks pass | Convergence study, benchmark suite, error bars |

Shorthand for common combinations: **Interactive** = real-time; **Exploratory** = responsive and quantitative; **Research** = verified at any speed.

## Labels of the current methods

| Method | Speed | Accuracy |
| --- | --- | --- |
| Explicit Euler | Real-time | Qualitative |
| RK4, fixed step | Real-time | Quantitative |
| Dormand–Prince 5(4), adaptive | Responsive | Quantitative |
| Backward Euler (implicit, L-stable) | Real-time | Qualitative |
| TR-BDF2 (implicit, L-stable) | Real-time | Quantitative |
| Exact methods: unit conversions, input blocks, formulas, maps | Real-time | Quantitative (no method declares Verified, [ADR 0061](../decisions/0061-verified-is-earned-per-case.md)) |

## Composition rules

- A model runs only as fast as its slowest method and is capped by its least accurate method (`MethodLabels::combine`, used by `Simulator::labels`).
- Both labels show on the model, on every plot and on every export, along with which Block sets each limit (UI planned).
- Verified applies to a specific case, not to a method in general: it is earned when that case's convergence check passes and is lost if the model changes. No method may declare itself Verified.

## How a case earns Verified (planned tooling)

The convergence check is on the roadmap in Phase 3 ([ADR 0061](../decisions/0061-verified-is-earned-per-case.md)).

- **Declared accuracy:** each Block states its method, order and tolerances; run reports record them.
- **Verification suite:** every backend ships tests against analytic or benchmark solutions (see [verification policy](../guides/verification-policy.md)).
- **Convergence tools:** one click to rerun at halved step or refined mesh and show the change.
- **Coupling-error control:** the master controls step size and iterates or rolls back where Blocks support it.
- **Provenance:** model version, parameters, seeds, solver versions and hardware stored per run.
- **Uncertainty:** parameter sweeps, Monte Carlo and sensitivity analysis.
