# 0008 — All-native desktop UI with egui and wgpu

- **Status:** Accepted
- **Date:** 2026-10-16

## Context

[ADR 0003](0003-desktop-first.md) chose the desktop and leaned towards an all-native UI, leaving a Tauri web UI open ([open question Q3](../open-questions.md#resolved)). The desktop app shell is next, so the choice is needed now. The Phase 0 gate needs live plots and sliders driven by a simulation stepping at 30–60 fps.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| All-native: egui (via eframe) + wgpu | One process and one language; the simulator runs in the same address space with no IPC; direct GPU access for viewports; immediate mode suits fast-changing data; builds the same on Windows, macOS, Linux | Less polished look than web UI; fewer ready-made widgets; node editor needs a crate or custom work |
| Tauri: web UI + native render window | Rich web widget ecosystem; familiar styling | Two languages and an IPC boundary for every frame of data; separate native window for GPU views; more build complexity |

## Decision

Build the desktop app as an all-native Rust application: `eframe` with the wgpu backend for the window, egui for panels and controls, `egui_plot` for time-series plots, and wgpu paint callbacks for 3D viewports. It lives in the new crate `crates/simsandbox-app`; UI dependencies stay out of the kernel and domain crates.

## Consequences

- The app calls `Simulator` directly; parameter panels are built from `ParamSpec` through `Simulator::model()`.
- Versions are pinned when the crate is created; egui releases break APIs often, so upgrades are deliberate.
- The node editor (Phase 1) is chosen later between an egui node-graph crate and a custom widget.
- A web viewer stays possible later (egui also targets WebAssembly), consistent with ADR 0003's mitigation.
