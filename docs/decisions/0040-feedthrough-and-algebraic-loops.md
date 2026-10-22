# 0040 — Direct feedthrough and algebraic loops

- **Status:** Accepted
- **Date:** 2026-10-22

## Context

Block outputs may depend on state and parameters only ([ADR 0002](0002-layered-block-contract.md)'s v0 restriction, [Q5](../open-questions.md)). A gain, a controller or any `y = k*u` needs its output to depend on its input at the same instant: **direct feedthrough**. Today such a block sees its input one communication step late, and inside a fused group it cannot be written at all. When feedthrough outputs form a cycle (an output depends on an input fed, through other feedthrough outputs, by itself) there is an **algebraic loop**: values that must be solved together. Typed equations have no inputs yet, so typed blocks cannot be wired. The owner decided on 2026-10-22: detect loops and say so clearly; Newton's method by default, changeable; declare feedthrough per output and per input; typed equations declare inputs with an `input` line.

## Options considered

| Question | Options | Chosen |
| --- | --- | --- |
| Declaration | Per block; per output (any input); **per output, per input** (as FMI) | Per output, per input: no false loops |
| Loops | **Newton's method** with a finite-difference Jacobian; fixed-point iteration; refuse | Newton by default; each model may choose fixed-point or refuse |
| Typed inputs | **An `input` line**; parameters become inputs when connected | An `input` line: what is an input does not depend on the wiring |

## Decision

- **Declaration.** Each output port lists the inputs of its Block it depends on directly (`PortSpec::feedthrough`, set with `depends_on`). `Equations::outputs` receives the inputs. Population ports (ADR 0034) take no part in feedthrough for now.
- **Consistent outputs at every communication point.** After each step (and at the start), the master evaluates feedthrough outputs in dependency order, each from inputs routed from outputs already final, so a chain such as sensor → controller → actuator agrees at the same instant instead of lagging a step per link. Inside a fused group the same is done at every right-hand-side evaluation, so feedthrough Blocks fuse and their connections carry no lag.
- **Loops are detected when the model is built**, from the declarations (strongly connected components of feedthrough outputs), and reported by name (`a.y → b.u …`) as a notice when a model opens, and by `simsandbox run`.
- **How loops are solved is part of the model:** `run.loops` = `newton` (default: Newton's method on the loop's output values, Jacobian by finite differences, converged to a relative 10⁻¹⁰), `fixed_point` (repeat the loop until it settles; cheap, fails when the loop gain exceeds one) or `refuse` (a model with a loop does not run; the error names the loop). Failing to converge stops the run with an error naming the loop.
- **Typed equations:** a line `input u, F` makes those names scalar input ports instead of parameters. Defined quantities that use an input, directly or through earlier quantities, depend on it; states do not.

## Consequences

- `Equations::outputs` gains the inputs; every equation system is updated. Code-built `EquationGroup`s still require members without feedthrough.
- The kernel gains loop analysis and the solver (`crates/simsandbox-core/src/loops.rs`); the master and fused groups use it.
- Implicit equations and steady states ([ADR 0019](0019-equations-without-time.md)) can build on the same Newton solver later.
- Resolves Q5.
