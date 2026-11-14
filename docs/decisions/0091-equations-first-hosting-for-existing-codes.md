# 0091 — Typed equations are the primary way to build models; hosting is for codes that exist

- **Status:** Proposed (the owner's leaning, 2026-11-14: "I am thinking that perhaps"; to accept or change)
- **Date:** 2026-11-14
- **Builds on:** [ADR 0084](0084-where-typed-equations-stop.md); would revise the [vision](../vision.md); answers [Q56](../open-questions.md)

## Context

The vision describes a host: a small kernel, every kind of physics a block type from a domain, mature solvers wrapped. The [review of 2026-11-14](../research/2026-11-14-code-review.md) found the code is an equation sandbox: no solver is wrapped, no plugin interface exists, and typed equations (arrays, tables, meshes, implicit methods, compiled code) are how every recent capability arrived. The owner (2026-11-14): the first idea was host first, so that others wire in codes they developed; equations turned out the easiest way to expand and test capability and are now capable on their own.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| A. Equations first; hosting as the way in for existing codes | Matches what is built; one language carries units, labels, exact replay and AI access to every new model; hosting keeps its real use (codes nobody wants to rewrite) | SimSandbox owns more numerics, each to verify; the non-goal "not replacing mature solvers" must be restated; the language must be specified and its front end restructured |
| B. Host first, as the vision says | Least numerics to own | Contradicts thirty days of work; new models would wait for a plugin interface |
| C. Leave the documents as they are | No work | Agents are told to follow a vision the code does not follow |

## Decision (proposed)

A.

1. New models are built from typed equations where the language can express them. The language grows when a model needs a primitive, each primitive verified ([ADR 0084](0084-where-typed-equations-stop.md) still bounds it: causal, exact, no outside compiler).
2. Hosting (the plugin interface, FMI, wrapped solvers) is how codes and models that already exist join a graph without being rewritten. The Block contract stays the boundary for both, and the kernel still contains no physics.
3. The non-goal becomes: SimSandbox does not rebuild what a mature solver does well when that solver can be wrapped; it does own the numerics its language needs.
4. Because the language is now the product, it gets a reference with a grammar and a function table (review W11) and a front end in typed stages (W6) before it grows further, after the Phase 2 gate ([ADR 0057](0057-phase-2-plan.md); owner, 2026-11-14: the gate comes before further typed-equation features).

## Consequences

- On acceptance: revise the vision (first paragraph, "Where the value is", non-goals), the roadmap and the [risks](../risks.md) (owning numerics is a larger risk; being a thin wrapper a smaller one); say whether [ADR 0009](0009-hobbyists-first.md)'s audience still holds.
- The plugin interface (Phase 2 item 4) is designed for existing codes first: a block written elsewhere, not a second way to write equations.

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none.
