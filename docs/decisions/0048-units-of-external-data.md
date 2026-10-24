# 0048 — Units of data from outside sources

- **Status:** Accepted
- **Date:** 2026-10-23

## Context

SimSandbox will read files and couple to outside solvers, whose data come in their own units: SI, US/imperial, or none at all, as in a non-dimensionalised CFD case. Units are checked everywhere inside a model ([ADR 0045](0045-units-in-typed-equations.md)–[0047](0047-unit-vocabulary.md)); the question ([Q31](../open-questions.md)) was how outside data gets its units and how it is converted or scaled. Decided by the owner on 2026-10-23, adopting the recommendation recorded in Q31.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| **Units declared where data enters; `units.convert` for fixed factors; typed equations for scaling by a reference quantity** | Each step is visible, checked and editable; a reference length or speed is a parameter that can be swept | A dimensionless source needs one more block |
| A dedicated scaling block with the reference as a setting | One block for every case | Hides a physical quantity in a setting, where formulas, sliders and sweeps cannot reach it |
| Readers that always produce SI | Nothing to convert inside the model | Loses the source's own units in plots and outputs; still cannot scale dimensionless data |

## Decision

1. **Units are declared once, where data enters.** A reader or coupling block gives its ports units: from the source's own metadata when it has any (OpenFOAM's `dimensions [kg m s K mol A cd]` entries, mapped to SimSandbox's order), otherwise from the block's settings. Data with no stated units is dimensionless, never guessed.
2. **A change of unit for one quantity is `units.convert`** (ft to m, psi to Pa): a fixed, exact factor, shown on the diagram as `from → to`.
3. **Giving dimensionless data physical units is a typed-equation block** with the reference quantity as a parameter with a unit, e.g. `input xs`, `param L [m] = 0.1`, `x = xs*L [m]`. Units check it, and the reference can be set by a slider, a formula or a sweep.
4. **Conversion factors are not written by hand.** A typed block with units already converts (`input u [ft]`, `y = u [m]` gives metres), so `y = 0.3048*u` would convert twice; a bare factor in a block without units is unchecked.

## Consequences

- File readers and solver couplings, when built, declare port units from metadata or settings, and are tested against a source in non-SI units.
- No new block types are needed now.
