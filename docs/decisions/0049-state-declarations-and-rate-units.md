# 0049 — State declarations; a line's unit is its left side's

- **Status:** Accepted
- **Date:** 2026-10-23

## Context

[ADR 0045](0045-units-in-typed-equations.md) put a unit at the end of an equation for the name the line defines, so `dx/dt = v [m]` meant that `x` is in metres: the bracket sat beside `dx/dt` but described `x`. The owner found this odd (2026-10-23), and also that a state was only introduced by its `dx/dt` line, with its initial value in an `x0` parameter that appeared from nowhere. The syntax was added the same day and is unreleased.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Keep ADR 0045's meaning, add `state` | Nothing changes for files written today | The odd reading stays for anyone who labels the rate |
| **A line's unit is its left side's as written; add `state x [m] = 0`** | One rule for every line; states declared like inputs and parameters, with unit and initial value in one place | Changes the meaning of `dx/dt = … [unit]` written today |
| Replace `dx/dt` by a new keyword (`der(x)`) | Like Modelica | Departs from the notation people write ODEs in |

## Decision

- **A unit at the end of a line is the unit of its left side as written.** `E = m*v^2/2 [J]` and `x[n+1] = … [m]` label the quantity; `dv/dt = F/m [m/s^2]` labels the rate. A state labelled only by its rate is in the rate's unit times s (`[mm/s]` gives mm), or in SI when that product has no name (`[km/h]` gives m).
- **`state x [m] = 0` declares a state**, optionally with its unit and initial value (the default of the parameter `x0`, which sliders, formulas and "Start from here" still set). Its dynamics remain a `dx/dt`, `x'` or `x[n+1]` line. Declaring is optional; without it a state starts at `x0` = 1, as before.
- **Checks.** A declared state needs a line giving its dynamics, and a name defined by `x = …` is not a state. A rate's unit must agree with its state's declared unit ("dx/dt is in m, so x would be in m·s, but x is in m"). `x0` given both by `state x = …` and `param x0 = …` is an error naming both lines. `state`, `param` and `input` followed by `=` are still equations defining those names.
- No conversion of the old meaning: files from 2026-10-23 that wrote a state's unit on its rate now fail to balance, with an error at that line, instead of being read differently. The examples were updated.

## Consequences

- ADR 0045's syntax for units on equations is amended by this decision; its other rules stand.
- The "New model…" starter declares its states (`state x = 1`, `state v = 0`).
- Every name in a typed block can now be declared with its unit before it is used: inputs, parameters and states; only defined quantities take their unit at the end of their line.
