# 0037 — When a dragged slider applies

- **Status:** Accepted
- **Date:** 2026-10-22

## Context

A parameter slider applies its value on every movement while it is dragged, for running models and for sweeps alike ([ADR 0011](0011-parameter-entry.md)). That suits a real-time model, where the effect should follow the hand. It does not suit a model whose step or sweep takes longer than a frame: each movement queues a recomputation, so the drag stutters and the result lags behind the slider. The owner decided on 2026-10-22 that sliders should be reactive while dragged for real-time simulations only.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Apply on every movement, always | Simple; what exists | Stutters and lags for slow models and large sweeps |
| Apply on release, always | Never stutters | Real-time models lose live feedback, the point of the fun pillar |
| **Live for real-time models, on release otherwise** | Live where it can keep up; smooth elsewhere | Which models count as real-time must be decided per model |

## Decision

- **Real-time models apply on every movement.** A model counts as real-time when every method in it is labelled `RealTime` ([ADR 0004](0004-method-classification.md)) and, for a sweep, when its last recomputation finished within two frames (about 33 ms).
- **Everything else applies on release** (or when a typed value is entered). While dragging, the slider and its field show the pending value, marked as not yet applied.
- **With the worker thread** ([ADR 0035](0035-simulation-worker-thread.md)) only the newest pending value is sent, so a live drag never queues work behind it.
- **Initial conditions are unchanged:** they apply at once before the run has started (dragging then restarts the run live) and on Restart afterwards.

## Consequences

- The parameter panel needs to know whether the open model is real-time: the session exposes it from the method labels and the measured sweep time.
- The rule is per model, not per slider; a model mixing a real-time block with a slower one applies on release.
