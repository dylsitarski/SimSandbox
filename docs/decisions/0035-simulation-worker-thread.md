# 0035 — The simulation runs on a worker thread

- **Status:** Accepted
- **Date:** 2026-10-19
- **Supersedes:** [ADR 0010](0010-live-loop-pacing.md)

## Context

[ADR 0010](0010-live-loop-pacing.md) paces the live loop by wall-clock time and, for Phase 0, steps the simulation on the UI thread within an 8 ms share of each frame. That keeps the window responsive only while every step is short. Phase 1 adds steps that can be long or unbounded: agent populations without a capacity limit ([ADR 0034](0034-agent-domain.md)) and Python-defined blocks ([ADR 0036](0036-python-blocks.md)), whose code can be slow or hang. On the UI thread one long step freezes the window, including Pause. Sweeps of up to 250 000 samples already recompute on the UI thread on every slider change. [ADR 0033](0033-phase-1-plan.md) deferred the move until the grazing model was measured; the owner decided on 2026-10-19 to settle it now, so that later work builds on the final structure.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Stay on the UI thread, move later if needed | Nothing to change now | Every app feature built meanwhile (agent views, canvas, Python blocks) is built on a structure that then changes; a hung block freezes the window |
| **A worker thread that owns the simulation**; the UI sends commands and draws from a shared history | The window never waits on a step; long steps and sweeps run in the background; one place for pacing | Commands become messages; the history is shared under a lock |
| A worker per Block or a thread pool | Parallel stepping | The co-simulation master steps Blocks in a fixed order; parallelism inside a step is a later, separate change |

## Decision

- **One worker thread per open model owns the simulation:** the `Simulator`, snapshots and rewind, filters and sweeps (today's `Session`). The UI thread never steps a Block.
- **Commands go to the worker over a channel:** run, pause, speed, step, restart, rewind to t, set a parameter, the step size, filters, sampling, and replace the model (equation edits, fusion changes, opening a file). The worker applies them between steps, in order. Saving asks the worker for the current file. A newer command of the same kind replaces a pending one (a dragged slider sends many).
- **The recorded history is shared.** The worker appends samples in batches under a write lock and does not block on it: while the UI holds the lock it keeps stepping and appends the batch later. The UI takes a read lock to draw a frame. Status (time, achieved speed, the step in progress and for how long, messages for the log) is published beside it. A new sample asks the UI to repaint.
- **Pacing stays as ADR 0010 decided, on the worker:** each tick adds elapsed wall time × speed to an accumulator and steps while it holds at least `dt`; pause is speed 0; a backlog left after a tick's budget is dropped, so slow models run in slow motion; the achieved speed is shown beside the requested one. The per-frame budget becomes a per-tick budget, and the worker sleeps until the next tick when it is ahead.
- **Sweeps run on the worker** and report progress. A command that changes the sweep (a slider, the sampling) cancels the running sweep and starts it again, so the latest values win.
- **A step cannot be interrupted** in general: commands wait for the step in progress. The status shows which Block has been stepping and for how long, and Pause takes effect after it. Python blocks can additionally be interrupted ([ADR 0036](0036-python-blocks.md)).
- **Results do not change.** The worker calls the kernel exactly as the UI thread does now; tests check that runs, rewinds and sweeps give the same numbers bit for bit as stepping directly. The CLI and the Python API run without a worker.

## Consequences

- `Session` keeps its role but lives on the worker; the app's frame code reads the shared history and status and sends commands instead of calling `Session` directly. Views are unchanged: they draw from a `History`.
- This is the first Phase 1 build item ([ADR 0036](0036-python-blocks.md)), before agent and app features that would otherwise be built twice.
- Headless screenshots (`--screenshot`) wait until the worker has produced the samples asked for, so they stay deterministic.
- Parallel stepping of independent Blocks, and several models open at once, become possible later without changing this structure.
