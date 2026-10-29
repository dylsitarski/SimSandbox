# 0064 — Rewind is opt-in: the recorder node

- **Status:** Accepted
- **Date:** 2026-10-29
- **Amends:** [ADR 0062](0062-probes-recording-and-replay.md) (snapshots on their own budget, an app setting), [ADR 0035](0035-simulation-worker-thread.md) (rewind always available)
- **Settles:** [Q41](../open-questions.md)

## Context

Rewind needs replay state: snapshots of every Block's state (and the outputs that cannot be recomputed, [ADR 0063](0063-probes-on-wires.md)) taken every few steps, plus the input track ([ADR 0058](0058-inputs-live-scheduled-replayed.md)). Until now the session always took them, within a fixed budget (256 MB), whether or not anyone rewound. On 2026-10-29 the owner asked whether recording state could be a node, like probes, with its settings on the node and possibly on subsystems that rewind on their own.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| **A recorder node, rewind off without one (owner's choice)** | Explicit and visible: what is kept and what it costs is on the canvas; no memory spent on rewind nobody uses | A model must add one to rewind; existing models lose rewind until they do |
| An implicit recorder, the node only for settings | Rewind works without doing anything | Memory spent unasked; the rewind bar's cost is hidden |
| Rewind as an app setting | Simple | Not with the model; not per model when a session holds several |

## Decision

- **Rewind is off by default.** A model rewinds only when it has a **recorder**: a node on the data layer holding the replay state's settings. Without one, no snapshots are taken and **the rewind bar is hidden**. Restart, run files and replaying a run from t = 0 work as before, since they need no snapshots.
- **One recorder per model** for now, saved as `controls.recorder`: `every` (steps between snapshots, default 10) and `mb` (its memory budget, default 256 MB; past it, snapshots thin, so the whole run stays reachable, coarser further back), and `canvas`. It never changes results.
- It is not wired: it records the whole model's state. Adding it during a run makes rewind possible from then on; removing it drops the snapshots.
- **Subsystems** that record and replay on their own, against the inputs they recorded, are planned for later (the owner, 2026-10-29); so are recorders per model when a session holds several models (ADR 0063 step 5).

## Consequences

- The examples showing replay after rewind ("Drag the bob", "Steer the car with keys") include a recorder; the others do not.
- The memory budget of ADR 0062 step 5 is split: the recorder's `mb` for snapshots, probes' `keep` for their recordings.
- Tests that rewind add a recorder.

## As built (2026-10-29)

- **File:** `controls.recorder` (`RecorderEntry`: `every`, `mb`, `canvas`; `every()`, `mb()`, `check()`); bad settings drop it with a warning. In the schema.
- **Session:** `recorder`; `step` takes snapshots only with one, every `every` steps, thinned to `mb` (`fit_snapshots`; `MAX_SNAPSHOTS` stays); `set_recorder` (added mid-run: a snapshot now; removed: snapshots dropped); `recorder()`; `rewind_to` does nothing without snapshots; `history_range` is (now, now) without them; saving writes it. "The run has not moved since restart" checks the step count instead of the number of snapshots.
- **Canvas:** `NodeKind::Recorder`, node `recorder` (`observers::RECORDER`) beside the probes, summary "every 10 steps · 256 MB"; "Add recorder" in the canvas menu (once), "Settings…" and double-click (`Request::EditRecorder`), `Edit::{AddRecorder, SetRecorder}`, delete removes it; undo covers it.
- **App:** `recorder_editor.rs` (steps and MB); `Outcome::{Data, Placed}` carry it; the rewind bar is drawn only with a recorder.
- **Examples:** "Drag the bob" and "Steer the car with keys" have `"recorder": {}`.
- Tests: rewinding tests add a recorder; `rewind_needs_a_recorder_and_reaches_back_to_when_it_was_added`; the canvas's recorder edits; the settings window.
