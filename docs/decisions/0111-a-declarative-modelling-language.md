# 0111 — SimSandbox is a declarative modelling language, not a library

- **Status:** Accepted (owner, 2026-11-25)
- **Date:** 2026-11-25
- **Builds on:** [ADR 0039](0039-ai-agent-access.md) (AI agents), [ADR 0091](0091-equations-first-hosting-for-existing-codes.md) (typed equations first), [ADR 0110](0110-encapsulated-models.md) (encapsulated models)

## Context

Encapsulation ([ADR 0110](0110-encapsulated-models.md)) made the likeness between models and functions plain. On 2026-11-25 the owner asked: why not build SimSandbox as a programming language or library, with the interface built on top? And what sets it apart from existing languages and libraries?

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Models as code in a host language (a library: Drake's systems, Julia's ModelingToolkit) | A full language for free; familiar to programmers and agents | The system cannot see inside arbitrary code (dependencies, determinism, structure); the app cannot edit code without loss; opening a model runs code; hobbyists must program |
| **A declarative modelling language with several front ends (the app, the file and CLI, Python)** | The system reads models (checks, fusion, labels, replay, cards); the app and agents share one lossless form; files are safe to open | Each language feature (extension, replication …) must be designed as a construct |
| A GUI only, with files as storage | Simple | Agents and scripts lose parity (ADR 0039) |

## Decision

1. **SimSandbox is a declarative modelling language.** The model file is its form. The app, the CLI and the Python package are front ends to it, as SimSandbox already works.
2. **A model is data.** Loading a model evaluates no code from it. Procedures live inside blocks (typed equations, block types, plugins, wrapped codes), never in how blocks are composed.
3. **Features borrowed from programming languages become constructs of the file.** These are encapsulation, extension, replaceable parts, replication, pinned versions, and contracts and tests. None is code that writes the file, so every front end sees them and `check` holds them.
4. **Python writes models; it does not hold them.** A script may use loops and functions to build a model, and the result is a file.
5. **The language gets one reference** that states its semantics in one place:
   - composition;
   - scope;
   - interfaces and encapsulation;
   - units and shapes;
   - clocks and determinism.

   Today they are spread over the ADRs.

## Consequences

- The [vision](../vision.md#a-modelling-language-not-a-library) and the [architecture overview](../architecture/overview.md#the-model-language) state this. The language reference is in Next up.
- A request for "a bit of code" in composition is answered by a construct, or by a block that holds the code.

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none; this names what the concepts together are.
