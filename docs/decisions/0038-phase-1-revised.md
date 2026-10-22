# 0038 — Phase 1 revised: Python-defined blocks decided later

- **Status:** Accepted
- **Date:** 2026-10-22
- **Supersedes:** [ADR 0036](0036-python-blocks.md)

## Context

[ADR 0036](0036-python-blocks.md) brought Python-defined blocks into Phase 1. Discussing why they are wanted (2026-10-22), the need turned out to be **blocks users can define without rebuilding SimSandbox**, and Python is one way among several: our own typed languages (typed equations, typed agent rules, [Q26](../open-questions.md)), Rust block types, Python, Julia and WebAssembly plugins. Python's main advantage is its libraries; its costs are finding or shipping a Python, slow per-agent loops, crashes inside the app, arbitrary code in shared files and the discipline of keeping no hidden state. The decisions that protect the architecture for any foreign-language block are made either way: kernel-owned state ([ADR 0002](0002-layered-block-contract.md), extended to tables by [ADR 0034](0034-agent-domain.md)) and the worker thread ([ADR 0035](0035-simulation-worker-thread.md)). Deferring Python-defined blocks therefore costs no later refactoring. The owner decided to decide them later.

## Decision

- **How users define their own blocks is decided later** ([Q28](../open-questions.md)): Python, WebAssembly plugins, Julia or extended typed languages, once the typed agent rules show what remains missing. Whatever is chosen keeps its state in kernel memory and runs under the worker thread.
- **The headless Python API stays in Phase 1:** a Python package (`simsandbox`, PyO3 and maturin) that loads, builds, edits, runs and sweeps models and returns results as numpy arrays. It is an extension module, so the app embeds no Python and the embedding spike is dropped; [Q27](../open-questions.md) waits for Q28.
- **Phase 1 order:**
  1. Simulation worker thread ([ADR 0035](0035-simulation-worker-thread.md)), with slider timing ([ADR 0037](0037-when-parameter-changes-apply.md)).
  2. Kernel support for populations ([ADR 0034](0034-agent-domain.md)).
  3. Agent domain and the grazing model (the gate, [ADR 0033](0033-phase-1-plan.md)).
  4. Direct feedthrough and algebraic loops ([Q5](../open-questions.md)).
  5. Headless interface for people and AI agents ([ADR 0039](0039-ai-agent-access.md)): the Python API and the CLI's describe, check and render commands.
  6. Visual node canvas.
  7. Subsystems collapsed into reusable Blocks.
  8. The rest of shareable model files: ask before discarding unsaved changes; a stated rule for converting older files.
- The headless interface moves ahead of the canvas: it is smaller, it lets AI agents build and check models from then on, and the canvas's edits can reuse its operations.

## Consequences

- No Python runtime in the app in Phase 1, so no trust prompt for model files yet.
- The [block contract](../architecture/block-contract.md)'s note on Python Blocks returns to "decided later".
