# Session notes

One note per non-trivial working session (human or agent): what was done, what was decided, what is left open. Notes are history: write once, do not edit later. The current state of the project lives in [STATUS.md](../STATUS.md), not here.

## How

1. Copy [`_template.md`](_template.md) to `YYYY-MM-DD-short-title.md`.
2. Keep it short: a reader should understand the session in under two minutes.
3. Add a row below and an entry in the [documentation map](../README.md).

## Index (newest first)

| Date | Note | Summary |
| --- | --- | --- |
| 2026-11-25 | [Encapsulation and composition](2026-11-25-encapsulation.md) | The interface direction revisited: research note, Q65, ADR 0110 (encapsulated models) proposed |
| 2026-11-20 | [The mechanism block](2026-11-20-mechanism-block.md) | ADR 0103 and 0105 built: `simsandbox-mech` (`mech.planar`), the `bodies` field, verification tests, the cart-pole example, Python's `bodies` |
| 2026-11-19 | [The vector registers after faer](2026-11-19-vector-registers-after-faer.md) | A 25× slowdown after implicit methods traced to faer's AVX kernels leaving the vector registers in use; cleared after factorising |
| 2026-11-19 | [Implicit structure and replay](2026-11-19-implicit-structure-replay.md) | Undeclared Jacobian structures dense when small, probed at three states when large (ADR 0104); restored implicit runs replay bit for bit |
| 2026-11-15 | [The equations' front end](2026-11-15-equation-front-end.md) | Review W6: the eight faults of typed equations fixed; the front end's restructuring |
| 2026-11-14 | [Code review](2026-11-14-code-review.md) | Whole-project review: report, work items W1–W13, questions Q56–Q59; no code changed |
| 2026-10-16 | [Kernel prep and ADRs](2026-10-16-kernel-prep-and-adrs.md) | Parameter access, snapshots include parameters, ADRs 0007–0013, model file format, desktop app shell, parameter panel, 3D view, saved views, ADR 0015, file dialogs, views steps 1–3, ADR 0016, typed equations, maps and sweeps, populations and agents, feedthrough, the headless interface and Python package |
| 2026-10-15 | [Project inception](2026-10-15-project-inception.md) | Market research, plan, core decisions, first commit, documentation system |
