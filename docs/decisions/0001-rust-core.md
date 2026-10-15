# 0001 — Rust core

- **Status:** accepted
- **Date:** 2026-10-15

## Context

The kernel (model graph, scheduler, units, state) must run real-time loops with steady frame times, ship as a single desktop binary on Windows, macOS and Linux, and be productive to develop with heavy AI assistance. The main alternative was a Julia-first core built on ModelingToolkit and the SciML ecosystem, which offers best-in-class ODE/DAE solvers and symbolic modeling but has garbage-collection pauses, noticeable startup latency, a heavier distribution and a smaller ecosystem.

## Decision

Write the kernel and desktop UI in Rust. Expose a Python API (PyO3) for scripting and research. Keep Julia available as an optional out-of-process backend plugin for heavy equation and DAE work, added only when a model needs it.

## Consequences

- Real-time methods run in Rust with no GC pauses.
- Symbolic work (equation entry, Jacobians) needs a library or a Python/Julia bridge rather than coming for free.
- Two runtimes may coexist later (Rust + Julia), behind the same Block traits.
