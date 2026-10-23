# 0046 — Units of declared parameters and their formulas

- **Status:** Accepted
- **Date:** 2026-10-23

## Context

[ADR 0045](0045-units-in-typed-equations.md) checked units in typed equations and at connections, but left the units of declared parameters ([ADR 0043](0043-subsystems.md)), the model's own and a subsystem's, as descriptive text, and their formulas unchecked. The owner asked (2026-10-23) to check those too. The platoon example showed why: its integral gain was labelled N·s⁻¹·m⁻¹ where `F = ki*I` needs N/m, and nothing noticed.

## Options considered

| Question | Options | Chosen |
| --- | --- | --- |
| Rules for formulas | Their own; **the same as typed equations** | The same, from one module (`simsandbox_expr::units`), so a formula and an equation balance by the same rules |
| Older files with unit-less parameters | Check everything; **check a formula once one of its names has a unit** | As for typed equations: without units nothing is checked and nothing changes |
| A formula with units into a block that gives none | Error; **not checked, value as written** | Typed blocks without units are unchecked (ADR 0045), and older files fed them this way |

## Decision

- A declared parameter's `unit` must be a unit (`Unit::parse`, which now also reads `·`, the way units are shown); an error at `params.NAME.unit` or `subsystems.S.params.NAME.unit` otherwise. Leaving it out means dimensionless.
- A formula that uses a declared parameter with a unit is checked by the rules of ADR 0045 (names without a unit are dimensionless) and must give the unit of what it sets: a subsystem use's parameter (the subsystem's declared unit) or a block parameter (the unit in the block's type, or in its typed equations). A mismatch is an error where the formula is written, e.g. at `blocks[0].params.L`: "tank.L is in m but 'size' is in kg". Formulas inside a subsystem are also checked when nothing uses it.
- Checked formulas are computed in SI and given in the target's unit: `size` = 2 km into a parameter in m gives 2000, also when the value changes later.
- Not checked, as before: a formula whose names have no units, and a formula setting a parameter of a block that gives no units (every parameter and port dimensionless, such as typed equations without units); its value is computed from the numbers as written.
- Grouping blocks into a subsystem copies the declared parameters it lifts in with their unit, description and range.

## Consequences

- Units can be written once on the model's parameters and hold through subsystems to the blocks; the platoon example now carries units throughout.
- A built-in block whose parameters and ports are all dimensionless (Lorenz) is not checked either; that only matters for a formula with units feeding it, which is a mistake the block cannot describe.
- Sliders and fields show declared parameters in their declared units; only the block parameters formulas set are converted.
