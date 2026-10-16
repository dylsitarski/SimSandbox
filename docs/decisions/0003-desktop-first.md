# 0003 — Desktop-first

- **Status:** Accepted
- **Date:** 2026-10-15

## Context

Performance matters for every kind of method. Browsers cap GPU access (WebGPU only), threads and memory, and cannot run local solver binaries such as OpenFOAM.

## Decision

Target the desktop (Windows, macOS, Linux) plus a headless CLI and containers for cluster or cloud runs. The UI is all-native (egui for panels and the node editor, wgpu for viewports) unless a later spike favours a Tauri web UI with a native render window.

## Consequences

- Full native GPU, threads and memory; heavy solvers run locally.
- Sharing requires an install. Mitigations: portable model files, a free standalone player, recorded runs, and a core kept free of desktop-only assumptions so a web viewer remains possible later.
