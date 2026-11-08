# 0082 — Exactness is a labelled property; models need no outside compiler

- **Status:** Accepted (owner, 2026-11-08)
- **Date:** 2026-11-07
- **Amends:** [ADR 0059](0059-run-files.md) (run files replay bit for bit), [ADR 0062](0062-probes-recording-and-replay.md) (its per-run exact or fast mode, `run.mode`, is not built: exactness is this ADR's label per method), [ADR 0077](0077-sums-in-fixed-chunks.md) (the same bits on any machine), [ADR 0078](0078-adaptive-studies.md) point 2 (runs remembered by point)
- **Builds on:** the speed and accuracy labels of methods ([verification policy](../guides/verification-policy.md)), [ADR 0017](0017-typed-equations.md) (typed equations), [ADR 0057](0057-phase-2-plan.md) (ML, FMI 3.0 and GPU fluids after the Phase 2 gate)

## Context

Every result in SimSandbox is the same bits on any machine and core count, and a model file runs without a compiler outside the program. The owner asked whether these are worth their cost in speed.

**What depends on exact bits:**
- **Replay:** run files replay recorded inputs bit for bit (ADR 0059; the Phase 2 gate is an interactive cart-pole replayed so). With tolerances, a chaotic or contact-rich model drifts from what was recorded.
- **Rewind:** recomputing from a snapshot must give what was shown.
- **Remembered runs:** a study reuses a run because reusing it equals running it again (ADR 0078).
- **Checking changes:** every refactor is compared byte for byte over the examples and profiling cases; "did this change behaviour?" is answered by equality, not judgement.
- **Sharing:** a run's hash and results mean the same on a collaborator's machine, and an agent verifies a result by equality.

Most of these need the same bits on the same machine and build; only sharing needs them across machines.

**What exactness costs** (measured 2026-11-07, [performance](../design/performance.md#typed-equations-against-fused-code-2026-11-07)): on the CPU, little. Fused multiply-add gave no gain on Gray–Scott; a cheaper formula that is the same on every machine gave 1.4–2×; the largest gap, typed equations 4–6× slower than fused code of the same operations, comes from how the program runs, and fusing keeps the bits. The large costs are elsewhere: GPUs (orders of magnitude for large grids and fluids), vendor BLAS and LAPACK for large dense systems (which choose code by CPU), and parallel reductions in whatever order threads finish. These arrive with the backends ADR 0057 plans after its gate: ML (ONNX Runtime), FMI 3.0, GPU fluids.

**What depends on no outside compiler:** a changed equation runs in milliseconds (a C compile takes seconds, a Modelica build longer); the app and the Python wheel need no toolchain on any system; a model file someone sends cannot run native code; an agent edits a file and runs it at once.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Exact everywhere, always (as now) | Every guarantee for every model | Rules out GPU backends, vendor linear algebra and most outside solvers, or wraps them in slow exact code |
| Drop exactness for speed | Any backend | Loses replay, rewind, reuse of runs and byte-for-byte checking for every model, for little gain on the CPU (measured) |
| Same bits on the same machine only | Allows CPU-specific code | Run files and results no longer shareable bit for bit; GPUs still excluded |
| **Exactness as a labelled level per method, the model taking its least exact block's; exact by default** | Exact models keep every guarantee; fast backends are possible where chosen; the user sees what a model promises | Each level's meaning for replay, rewind and reuse must be defined and shown |

## Decision

**1. Three levels.** Every block and method declares one, beside its speed and accuracy labels:
- **exact**: the same bits on any machine and core count, for a given SimSandbox version (all code today);
- **exact here**: the same bits on the same machine and build (vendor BLAS choosing code by CPU, code compiled for this CPU);
- **approximate**: may differ from run to run (a GPU summing in the order threads finish).

**2. A model's level is its least exact block's**, shown by `check`, `describe`, Python and the app with the other labels.

**3. What each level keeps:**

| | exact | exact here | approximate |
| --- | --- | --- | --- |
| Run file replay | checked bit for bit | bit for bit on the machine that recorded it (the file names it), within a tolerance elsewhere | within a recorded tolerance; the largest difference reported |
| Rewind | as now | as now | recomputed values may differ from those shown; the app says so |
| Remembered runs (ADR 0078) | reused | reused on the same machine | reused as the run's result, marked so |
| The project's own checks | byte for byte | byte for byte on one machine | by tolerance |

**4. Speed within exactness first.** Exact code may go faster by:
- evaluating in fewer passes (fusion, chunks, several cores) with the same operations in the same order (measured to keep the bits);
- choosing a cheaper formula that is the same on every machine, changing the last bits once (recorded in the changelog, as ADR 0077 did).

Fused multiply-add and fast-math reassociation stay out of exact code: measured, they gain nothing here.

**5. No outside compiler for model files.** Typed equations run in the program: interpreted, or compiled in the process (a JIT such as Cranelift) if built, keeping the same operations in the same order, so exact. Code compiled outside (an FMU, a WebAssembly plugin, an ONNX model, an external solver) enters as a block of its own, declaring its level. Opening and running a model file never calls a compiler.

**Order of work:**
1. The level as a label on methods and blocks, the model's level in `check`, `describe`, Python and the app.
2. Run files record the level (and the machine for "exact here"); replay checks bits or a tolerance accordingly; remembered runs follow the table.
3. Built when the first block that is not exact arrives (vendor linear algebra, ONNX, GPU fluids), not before.

## As decided

The owner accepted this ADR on 2026-11-08, choosing it over ADR 0062's step 4 (a per-run `run.mode`, exact or fast). Exact code already uses every core (cell-by-cell work, ADR 0062 step 3; sums in fixed chunks, ADR 0077), and the measurements of 2026-11-07 found little left on the CPU to gain by giving up exactness, so a fast mode would add a run setting that changes almost nothing. Non-exact speed arrives with the blocks that need it (GPU fluids, vendor linear algebra, ONNX), labelled by this ADR's levels; nothing is built until the first such block.

## Consequences

- Exact models, every model today, keep every guarantee; fast backends can be added without weakening them.
- A user sees what a model promises before sharing a run file or trusting a reused run.
- Speed work on typed equations goes to fusion and in-process compilation, which keep exactness ([performance](../design/performance.md#typed-equations-against-fused-code-2026-11-07)).
- **Concepts added:** none. Exactness is a label beside speed and accuracy on methods, as they already are.
