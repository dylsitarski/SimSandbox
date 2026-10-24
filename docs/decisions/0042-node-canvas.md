# 0042 — The node canvas, and starting from a reached state

- **Status:** Accepted
- **Date:** 2026-10-22

## Context

Phase 1 item 6 is a visual node canvas ([ADR 0038](0038-phase-1-revised.md)): see and edit a model's blocks and wires. The owner decided (2026-10-22) where it sits, how layout is saved, what the first version covers and what an edit does to a running model. On that last point the owner asked whether rebuilding and continuing from the current state would serve real-time models, or continuing a transient from a steady state. Most live interaction is parameters, which already apply while running; carrying state across a structural edit is not reproducible (the start depends on when the edit was made, and the file does not record it) and breaks rewind. The reproducible way to start from a reached state is to save that state as the model's start.

## Options considered

| Question | Options | Chosen |
| --- | --- | --- |
| Placement | **A tab beside Views**; a split with the views; a floating window | Tab: each gets the whole area, as Simulink's model and scopes or LabVIEW's diagram and front panel |
| Layout in the file | **`at: [x, y]` per block**; a `controls.layout` map | Per block: travels with the block when copied, renamed or put in a subsystem |
| Edits to a running model | **Rebuild and restart at t = 0**; rebuild and continue; wait for Apply | Restart, plus "Start from here" to make a reached state the start; live continue can be an opt-in later |
| First version | **Edit the graph**, **headless diagram image**; observer layer (filters and views, [ADR 0032](0032-filters-as-a-graph.md)) | The observer layer follows as its own step |
| Drawing | A node-editor library; **our own canvas on egui's painter** | Third-party node editors lag egui releases, and egui is pinned ([ADR 0008](0008-native-egui-ui.md)); the canvas is small and also renders headless |

## Decision

- **Diagram tab.** The central area switches between Views and Diagram. Blocks show their name, type and method, inputs on the left and outputs on the right (round dots for numbers, squares for grids, point sets and populations, with unit and shape on hover); wires are curves, and wires that run backwards go round underneath. Drag to move or pan, scroll to zoom, drag from output to input (or back) to connect, drag a wire off an input to move or remove it, Delete removes the selection, right-click for a block's menu (edit, rename, delete) or the canvas's (add a block by domain and type), double-click to edit a block's equations.
- **Layout.** A block's position is saved as `"at": [x, y]` in canvas units. Blocks without one are laid out in columns by the longest chain of wires leading to them (wires closing a loop are ignored), each column ordered by where its sources sit; new blocks without a position go to the right of placed ones. The first model edit saves every position, so the layout does not jump.
- **Edits.** Moving a block changes only the layout; the run goes on. Other edits (add, delete, rename, connect, disconnect) change the model file and rebuild it, restarting at t = 0 with the current parameter values, as soon as it loads. A file that does not load yet (e.g. a new block whose input is still open) is kept as a draft, shown with the reason, while the run continues with the last model that worked; Revert drops it. Renaming a block renames every reference to it (connections, groups, sweeps, sliders, views and filters). Edits live in `simsandbox-session` (`diagram.rs`, `canvas.rs`), so the app and headless rendering share them.
- **Start from here.** One command makes the current state where the run starts and restarts from it at t = 0. A block whose states each have an initial-condition parameter `s0` that gives them exactly (typed equations, the built-in systems) gets those parameter values, so its sliders keep working; any other block gets a `start` entry: its state values and table rows (`"start": {"state": [...], "tables": {"animals": [[x, y, energy], ...]}}`). The kernel applies a start after `init` and `init_tables` on every start and restart (`Model::set_start`); Blocks may say which parameters reproduce a state (`Block::initial_params`, default none). "Forget start state" returns to the parameters. Numbers in model files are read back exactly as written (`serde_json` `float_roundtrip`), so a saved start reproduces the run bit for bit.
- **Headless** ([ADR 0039](0039-ai-agent-access.md)): `simsandbox render FILE --diagram --out d.png` and `Model.render(diagram=True)` draw the canvas, even for a file that cannot run; `simsandbox start-from FILE --t T` and `Model.start_from(t)` write the model that starts from the state at `t`; `Model.add_block(..., at=(x, y))` places a block.
- **Scheduled changes** (a parameter step at a set time, so one run covers settling and the transient) go on the roadmap.

## Consequences

- Models can be built and rewired without editing JSON; agents and people see the same diagram.
- A block with a `start` entry ignores its initial-condition parameters until the start is forgotten; the side panel does not mark this yet.
- While a draft is pending, parameter changes made in the side panel apply to the running model, not to the draft.
- There is no undo, multi-selection, copy and paste or subsystem grouping yet; subsystems are Phase 1 item 7. The observer layer of [ADR 0032](0032-filters-as-a-graph.md) is the canvas's next step.
