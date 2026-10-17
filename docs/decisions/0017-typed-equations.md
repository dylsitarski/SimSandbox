# 0017 — A built-in expression parser for typed equations

- **Status:** Proposed
- **Date:** 2026-10-17

## Context

Hobbyists should be able to type `dx/dt = sigma*(y - x)` and get a running model with parameter controls ([vision](../vision.md), [ADR 0009](0009-hobbyists-first.md)). The parser approach was [open question Q4](../open-questions.md#resolved). The same parser will compute derived signals for views ([ADR 0015](0015-pluggable-views.md)), and model files need a way to store a block's equations ([ADR 0007](0007-json-model-files.md)).

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Small Rust parser and evaluator in the project | No dependencies or start-up cost; error messages written for hobbyists, with line and column; evaluates fast enough for real-time; one parser for equations and derived signals | Symbolic manipulation (simplification, derivatives) is ours to write when needed |
| An existing Rust expression crate (e.g. evalexpr, meval) | Less code | Error messages and syntax are someone else's; still no symbolic derivatives; another dependency to track |
| SymPy through Python | Full computer algebra | Needs Python installed and a bridge; slow start; poor fit for a desktop app for hobbyists |
| Julia Symbolics as a service | Strong symbolic and DAE tooling | Large runtime, long start-up; planned only as an optional heavy backend ([solver backends](../architecture/solver-backends.md)) |

## Decision

- **Parser:** a dependency-free crate, `simsandbox-expr`: numbers, names, `+ - * / ^` (right-associative `^`, binding tighter than a leading minus), parentheses, `pi`, and a fixed set of functions. Parsing gives an AST with names; resolving maps names to slots and gives an expression that evaluates by tree walk. Errors carry a column; `2x` is rejected with "write 2*x".
- **Typed equation blocks:** the block type `eq.equations` (`simsandbox_eq::EquationSet`). Lines `dx/dt = …` or `x' = …` define states; `name = …` defines a computed quantity that may use those above it; every other name is a parameter (default 1); each state `x` gets an initial-value parameter `x0` (default 1, marked as an initial condition); `t` is time; `#` starts a comment. Every state and computed quantity is an output port.
- **Model files:** a block's lines are stored in an `equations` array next to `params`. The kernel passes them to factories as `BlockConfig::definition`, so the file format stays domain-neutral; errors are reported at `blocks[i].equations` with the line and column.
- **Later, in the same crate:** symbolic derivatives of the AST (Jacobians for the `Derivatives` capability) and units in equations.

## Consequences

- **Scope:** the language stays deliberately small: expressions, ODEs, computed quantities and, if [Q12](../open-questions.md) is accepted, discrete maps and simple implicit equations. It does not grow into a general modelling language; acausal models, index reduction of DAEs and heavy simplification go to an external backend (Julia ModelingToolkit or a Modelica tool, [solver backends](../architecture/solver-backends.md)), which can read the same `equations` text.
- **Costs we take on:** units in expressions, symbolic derivatives and conditionals are ours to write (small for this grammar); arrays and indexed equations for large models are real language-design work; the tree-walking evaluator is several times slower than compiled code, so large real-time models will need bytecode or JIT compilation.

- No new runtime dependency; typed models start instantly and run at the same speed class as built-in ones (a typed Lorenz system matches the built-in one bit for bit).
- Equations are dimensionless until units are added; typed blocks have no inputs yet, so they cannot be wired to other blocks.
- Derived signals (views step 4) can reuse the parser directly.
