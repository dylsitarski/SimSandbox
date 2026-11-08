# 0084 — Where typed equations stop: a causal language of primitives; blocks wrap anything

- **Status:** Proposed
- **Date:** 2026-11-08
- **Resolves:** [Q54](../open-questions.md) (where typed equations stop and wrapped libraries, FMI or other backends begin)
- **Amends:** [ADR 0057](0057-phase-2-plan.md) (item 4's WebAssembly spike gains a comparison of execution routes; "physical (acausal) connections" after the gate become acausal physics through Modelica and FMI), [ADR 0017](0017-typed-equations.md) (scope restated)
- **Builds on:** [ADR 0082](0082-exactness-is-labelled.md) (exactness levels), [ADR 0078](0078-adaptive-studies.md) (strategies are models), [Q28](../open-questions.md) (user-defined blocks)

## Context

Named axes, `solve`, `rcond`, vectors per run and strategies written as models (Levenberg–Marquardt fitting, pattern search) grew typed equations quickly. ADR 0017 had kept the language deliberately small. The owner asked:
- whether this reinvents what exists;
- how SimSandbox stands to Modelica, its closest relative, without stepping on it or ignoring what it offers;
- why have a language at all, when models could be code JIT-compiled to native speed, especially as AI agents write models.

**What the fitting example showed.** The strategy is about 50 lines of equations, not engine code; what the language gained (axes, `solve`, `rcond`, vectors per run) is small, general, exact and tested against references. Its real cost was correctness at the edges, found by checking against SciPy.

**Why a language, when code could be compiled.** Its value is that SimSandbox reads the model, not that people do. From the equations it derives units (checked at load, with line and column), which outputs depend at once on which inputs and parameters (algebraic loops, driven parameters), cell-by-cell sparsity (implicit solvers on large grids), port shapes and axes, whether a model reads time (evaluated, iterated or run), and the equations themselves (fusion into one system; the editor's parameters). Code gives none of this unless declared by hand. Generated code is what most needs checking. Code in a shared file can also do anything (Julia cannot be sandboxed in practice), its reductions vary with CPU and packages, and a runtime such as Julia's is large and slow to start after an edit.

**What it costs.** The interpreter is about 2× slower than built-in Rust on small scalar models and 4–6× slower than fused code on large grids, with identical bits ([performance](../design/performance.md#typed-equations-against-fused-code-2026-11-07)): the gap is the way the program runs, not exactness.

**Modelica.** Its strengths are acausal component modelling (connectors with no fixed direction), the compiler work this needs (flattening, DAE index reduction, tearing, code generation) and the Modelica Standard Library. SimSandbox's are interactive exploration with rewind, agents and changing members, spatial fields, studies and strategies inside models, coupling different engines, headless access for agents and exact replay. Typed equations are roughly a small causal subset of Modelica; acausal physics, still listed after ADR 0057's gate, is Modelica's core.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| **A small causal language of verified primitives; algorithms as models; blocks wrap anything; acausal physics through Modelica and FMI; no outside compiler for model files, speed from compiling our own program** | SimSandbox keeps understanding its models; Modelica's value comes in instead of being rebuilt; heavy or foreign code has a home | Two kinds of model to explain; FMI import and a compiled back end are work |
| Grow typed equations into a general modelling language (acausal, index reduction) | One language | Rebuilds Modelica, decades behind; contradicts ADR 0017 and the vision |
| Models as code (Julia, Rust) compiled on demand | Native speed; any algorithm | Loses units, loop and sparsity analysis, fusion and the editor; unsafe to share; exact at best "here"; heavy runtime, slow after each edit |
| Decide case by case | Flexible | No boundary: each feature argues it again |

## Decision

**1. Blocks can wrap anything**: a library, a solver, an engine, an FMU, an ONNX model, code. Each declares its interface and is labelled with speed, accuracy and exactness (ADR 0082). However far the language grows, it never has to be complete.

**2. Typed equations are a causal language of primitives.** Operations on values: arithmetic, units, axes, reductions, stencils, linear algebra (`solve`, `rcond`; `lstsq`, `eig`, `svd` when needed). Each is small, exact on any machine and verified against a reference. Every ADR adding to the language has an option row "use library X instead" and says why not.

**3. Algorithms built from primitives are models.** Optimisers, fitting, continuation and refinement ship as example models and helpers (ADR 0078), never as built-in designs. A strategy that outgrows a model (many parameters, an elaborate method) becomes a block wrapping a library (point 1).

**4. Acausal physics comes from Modelica.** Typed equations stay causal: no acausal connectors, index reduction or physical component libraries. ADR 0057's "physical (acausal) connections" become: FMI 3.0 import (Modelica models as blocks; model exchange so SimSandbox's integrators and fusion can step them), then FMI export, so Modelica users can use SimSandbox models; optionally an OpenModelica back end that compiles `.mo` files to FMUs where it is installed, as a block from another tool. SimSandbox is where Modelica models are explored, studied, fitted and combined with what Modelica does not do.

**5. Model files never need an outside compiler.** User code enters through blocks: the plugin interface of ADR 0057 item 4, chosen in Q28 after its spike (WebAssembly recommended there, not yet decided).

**6. Speed comes from compiling our own program.** The checked program typed equations already produce is compiled in the process, keeping each operation and its order (exact). ADR 0057 item 4's WebAssembly spike also compares, on Gray–Scott's right-hand side:
1. the interpreter (now);
2. our program compiled by Cranelift directly;
3. our program compiled to WebAssembly and run by Wasmtime (which compiles it with Cranelift);
4. a hand-written WebAssembly block (the spike's own);
5. native fused Rust (`examples/fused_speed.rs`).

For each: time per evaluation, compile latency after an edit, whether the bits equal the interpreter's, and the size it adds. If route 3 is close to route 2, one runtime serves typed equations and plugins and reaches the browser; otherwise route 2 for typed equations. The interpreter stays as the reference and the fallback.

## Consequences

- Each model has a home: typed equations for models SimSandbox should understand (checked, analysed, fused, exact, editable live, safe to share), blocks for everything else.
- Modelica is a source of models, not a rival; ADR 0057's acausal item becomes FMI import (and export).
- The language stays small; heavy cases are blocks, not new syntax.
- The spike decides the compiled back end with measurements, not estimates.
- **Concepts added:** none. Blocks and typed equations exist; this sets their boundary.
