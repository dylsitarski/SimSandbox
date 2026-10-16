# Interaction, parameters and visualization

_Last updated: 2026-10-16_

Because parameters, ports and state are declared metadata, most of the interface can be generated rather than hand-built. The desktop app implements run, pause, restart, rewind with branching, paced playback, live time-series, phase-portrait and 3D trajectory views, and the parameter panel; everything else here is planned, in the order given in [STATUS.md](../STATUS.md).

## For fun

- Live parameter changes while the simulation runs, with no restart (kernel support exists: `Simulator::set_param`).
- Direct manipulation: drag bodies, paint obstacles into a flow, drop agents, inject a disturbance.
- Pause, rewind and branch from any point (implemented in the app: a rewind slider; snapshots also restore parameter values, and running on from a rewound point starts a new branch).
- Inputs from keyboard, gamepad and mouse; later sensors or MIDI.

## For learning

- Control panels generated from `ParamSpec` (implemented): a typed entry field with units for every parameter, and an opt-in slider with a user-set range (initially the suggested range), aimed at real-time models ([ADR 0011](../decisions/0011-parameter-entry.md)). Initial-condition parameters are marked ↻ and apply on Restart, or at once before the run starts. Slider choices and ranges, default speed, default plots and the opening view are saved with the model ([ADR 0012](../decisions/0012-controls-in-model-files.md)); window layout and plot zoom are app settings.
- Playback paced by wall-clock time with a speed control; slow models run in slow motion with the achieved speed shown ([ADR 0010](../decisions/0010-live-loop-pacing.md)).
- Linked views: change a parameter and see the time series, phase portrait and 3D view update together (implemented as three switchable views of the same run; side-by-side views are planned).
- Equation view beside the running model, highlighting the term being changed.
- Guided explorables: a model plus a short narrative and locked or unlocked parameters, shared as one file that opens in the app or the free player.

## For research

- Notebook integration (Jupyter) through the Python API.
- Export of raw results to CSV, HDF5, Parquet and VTK for fields.
- Batch experiment designer for sweeps and design of experiments.

## Visualization layer

Architecture: [visualization](../architecture/visualization.md) ([ADR 0015](../decisions/0015-pluggable-views.md)).

- 2D plots and phase space with GPU-accelerated plotting; 3D scenes and fields with native GPU rendering; graph views for networks and agents.
- Large CFD fields stream decimated data to the viewer and hand off to ParaView for heavy post-processing.

## Three ways to build, one model file

1. Visual canvas: drag and wire Blocks.
2. Code: a Python (later Julia) API that produces the same graph.
3. Natural language: an AI assistant drafts Blocks and equations; the result is ordinary, editable model code that passes the same unit and validation checks.

## Labels in the UI

Speed and accuracy labels appear on the model, on every plot and on every export, with the Block that sets each limit ([method classification](../architecture/method-classification.md)).
