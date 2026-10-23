# 0047 — The unit vocabulary

- **Status:** Accepted
- **Date:** 2026-10-23

## Context

With units checked in equations, formulas and connections ([ADR 0045](0045-units-in-typed-equations.md), [ADR 0046](0046-units-of-declared-parameters.md)), the parser's fixed list of about 25 symbols became the limit: `kN`, `MPa`, `µs`, litres, degrees and every US/imperial unit were refused, and some units the program displayed (`1e-5·kg·m`) could not be read back. The owner asked (2026-10-23) for prefixes and common units, with imperial sources and external data in mind.

## Options considered

| Question | Options | Chosen |
| --- | --- | --- |
| Vocabulary | Hand-listed scaled units; **SI prefixes on SI-style symbols plus curated non-SI units**; a full units database (UCUM, Pint's registry) | Prefixes plus a curated list: covers engineering use, stays small enough to test exhaustively; extend by adding rows |
| `a/b*c` | Everything after `/` divides (as before); left to right (the usual reading); **refuse it** | Refuse: the two readings differ, and a unit misread silently is worse than one rejected |
| °C and °F | Offset units; **refuse with the reason** | Refuse for now: a unit here is a scale, and offset scales make differences and absolute values ambiguous; temperatures are in K |

## Decision

- **Symbols** (`simsandbox_core::units::symbols`, listed by `simsandbox describe units`):
  - SI and derived, taking prefixes: m, g, s, A, K, mol, cd, Hz, N, Pa, J, W, C, V, F, Ohm (Ω), S, Wb, T, H, rad; also L (l), t, bar, eV, Wh, cal.
  - Without prefixes: min, h, d, ha, atm, deg (°), rev, rpm, %, ppm.
  - US and imperial, at their exact legal definitions: in, ft, yd, mi, nmi, kn, mph, lb, oz, lbf, psi, gal (US), hp (mechanical), BTU (IT), slug.
- **Prefixes:** Q, P, T, G, M, k, h, da, d, c, m, µ (also μ and u), n, p, f, a. A whole symbol wins over a prefix reading: `ft` is the foot, not a femtotonne, the only such case (pinned by a test).
- **Grammar:** factors joined by `*`, `·` or `/`, read left to right; a factor is a symbol, a positive number (`0.3048*m`) or a parenthesised group, each with an optional whole power. `a/b*c` is an error suggesting `a*c/b` or `a/(b*c)`. Every displayed unit reads back: named when a symbol or prefixed symbol fits, otherwise as a number and SI base units.
- **Temperatures with an offset** (°C, °F, degC, degF) are refused, saying to use K. A difference of 1 °C is 1 K, so rates and coefficients are unaffected; absolute readings in °C are converted where they enter the model (for now with typed equations, `T = Tc + 273.15 [K]`).
- **Errors** come from the parser and say which symbol is unknown and what exists, wherever a unit is read (equations, settings, declared parameters).
- Angles are dimensionless (rad, deg, rev), as in SI; a value in degrees is converted to radians when used, so `sin(a)` with `a [deg]` is right.

## Consequences

- The meaning of `a/b*c` changes from "all after `/` divides" to an error; only files written on 2026-10-23 could contain it, and none of the examples or fixtures do.
- Quantities of the same dimension (torque and energy, Hz and rad/s) are still not told apart; that needs quantity kinds, not more symbols.
- How data from external sources (dimensionless CFD output, imperial files) enters a model is [Q31](../open-questions.md).
