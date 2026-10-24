# 0036 — Python-defined blocks in Phase 1

- **Status:** Superseded by 0038
- **Date:** 2026-10-19
- **Supersedes:** [ADR 0033](0033-phase-1-plan.md) (its order and Python scope; the rest is restated below)

## Context

[ADR 0033](0033-phase-1-plan.md) limited Phase 1's Python API to headless scripting and left blocks written in Python for later. The owner asked whether Python blocks should come into Phase 1, so that any change they force on live runs happens before more is built on the current structure. Three decisions would be costly to change late: where a Python block keeps its state (rewind depends on it), whether Python runs in the app's process or its own (it shapes the master and packaging; [block contract](../architecture/block-contract.md) planned a gRPC adapter for out-of-process Python), and whether the simulation leaves the UI thread (decided in [ADR 0035](0035-simulation-worker-thread.md)). The Block contract itself is already language-neutral: flat `f64` arrays for parameters, inputs, state and outputs, stepped as a black box.

## Options considered

**Where Python runs**

| Option | Pros | Cons |
| --- | --- | --- |
| **In process, through PyO3** (the interpreter embedded in the app and CLI; the headless API as a Python extension module) | Calls cost microseconds; numpy arrays can view kernel memory without copying; one binding layer for blocks and the headless API | A crash in native Python code takes the app down; the app must find or ship a Python |
| Out of process, through gRPC | Isolation; the app does not depend on the user's Python | Every step is a round trip with copies; a second adapter to maintain; overkill for small blocks |

**Where a Python block keeps its state**

| Option | Pros | Cons |
| --- | --- | --- |
| **In kernel-owned state, seen from Python as numpy arrays** (fixed state and tables, [ADR 0034](0034-agent-domain.md)) | Snapshots, rewind, saving and reproducibility work as for every Block | Authors must not keep state in Python objects |
| In Python objects, with save and restore hooks | Natural Python | Every block re-implements snapshots; forgotten state breaks rewind silently |

## Decision

- **Python blocks are in Phase 1.** A Python block is a class that declares a spec (ports, parameters, states, tables, time semantics, labels) and implements `step(ctx, params, inputs, state, outputs)` and `outputs(...)`. The arrays are numpy views of kernel memory; tables appear as column arrays with ids. A Rust type, `PyBlock`, implements `Block` by calling it. An optional `rhs` lets a Python ODE be fused with equation blocks later.
- **State lives in the kernel.** A Python block keeps nothing between steps outside the arrays it is given; this is the rule that makes rewind and reproducibility hold, and the documentation and examples follow it. Randomness uses the counter-based draws of [ADR 0034](0034-agent-domain.md), exposed to Python.
- **In process, through PyO3.** The headless API is a Python package (`simsandbox`, built with maturin); the app and CLI embed the interpreter. Out-of-process Blocks (gRPC) stay planned for Linux-only tools and untrusted or crashing code, behind the same traits.
- **Model files refer to Python code** by a path relative to the file and a class name (`"type": "py.block"`, with `"module"` and `"class"`), so a model and its scripts are shared together. **Opening a model that contains Python asks before running it**, naming the files, because running it executes code.
- **Python blocks carry the labels their author declares**, defaulting to responsive and qualitative; the pacer's measured speed is shown as for any block. A step can be interrupted: Pause during a Python step raises `KeyboardInterrupt` in it after a grace period, and the error is reported as the block's failure.
- **Finding Python is settled by a spike** before the Python work starts: embedding works on Windows and Linux with either the user's Python or a Python shipped with the app; which one, and the minimum version, are [Q27](../open-questions.md). Without Python, the app runs every other model and reports Python blocks as unavailable.

**Phase 1 order** (replacing ADR 0033's; the gate, the grazing model and the scope of the other items are unchanged):

1. Simulation worker thread ([ADR 0035](0035-simulation-worker-thread.md)).
2. Kernel support for populations ([ADR 0034](0034-agent-domain.md)).
3. Agent domain and the grazing model (the gate).
4. Python embedding spike on Windows and Linux (resolves Q27).
5. Direct feedthrough and algebraic loops ([Q5](../open-questions.md)).
6. Visual node canvas.
7. Subsystems collapsed into reusable Blocks.
8. Python: the headless API and Python-defined blocks, together.
9. The rest of shareable model files: ask before discarding unsaved changes; a stated rule for converting older files.

## Consequences

- The binding layer (numpy views of state, inputs, outputs and tables) is written once for both Python uses.
- The kernel tables of ADR 0034 are designed to be exposed to Python as column arrays without copying.
- The block-contract document's plan of a gRPC adapter for Python becomes one for out-of-process Blocks only.
- Opening untrusted models gains a confirmation step; models without Python are unaffected.
