# 0019 — Typed models without time: formulas and discrete maps first

- **Status:** Accepted
- **Date:** 2026-10-17

## Context

Typed equation blocks ([ADR 0017](0017-typed-equations.md)) need at least one `dx/dt = …` line. Some models have no time or no states: pure formulas, discrete maps, implicit equations and steady states, and equations in another independent variable ([open question Q12](../open-questions.md#resolved)).

## Decision

- **First:** stateless typed blocks (only `name = …` lines), explored by sweeping a parameter and plotting outputs against it; and discrete maps written `x[n+1] = …`, stepped by iteration.
- **With algebraic loops ([Q5](../open-questions.md)):** implicit equations and steady states (`f(x) = 0`), solved by Newton's method.
- **Later:** an independent variable other than `t` (`dy/dx`).

## Consequences

- Formulas and maps (e.g. the logistic map) become available to hobbyists without new solvers.
- Sweeping a parameter needs a view or tool that reruns or re-evaluates a model over a range; it is designed with these blocks.
