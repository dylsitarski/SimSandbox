# 0020 — Only approximating operations have a method choice

- **Status:** Accepted
- **Date:** 2026-10-17

## Context

The owner asked whether the methods behind operations in equations should be customisable ([open question Q14](../open-questions.md#resolved)). Methods with speed and accuracy labels already exist for integrators ([ADR 0004](0004-method-classification.md)).

## Decision

- Exact operations (`+`, `*`, `sin`, `exp`, …) have no method choice.
- Operations that approximate get a method choice with a sensible default and speed and accuracy labels, chosen the same way as integrators: derivatives for Jacobians (symbolic by default, finite differences as a fallback), integrals inside expressions, root finding for implicit equations, smoothing or differentiating recorded signals, and spectra.
- A model's labels combine those of every method it uses, so an approximation can never make a result look more trustworthy than it is.

## Consequences

- Added when the first approximating operation lands (symbolic Jacobians or implicit equations).
