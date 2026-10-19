# Changelog

All notable user-visible changes are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/) once it reaches 0.1.0.

## [Unreleased]

### Added

- Surface and contour views: `plot.surface` shows a grid field as a rotatable, shaded 3D surface over its axes and `plot.contour` as lines of equal value (`levels`, `log_scale`); both work on two-parameter sweeps and on lines of cells over time (e.g. the heat rod). The resonance-map example adds a surface.
- Sweeps as samples (ADRs 0027–0028): a top-level `sample` section with designs `grid`, `list`, `random`, `latin_hypercube` and `sobol` over any number of parameters (seeded, at most 250 000 samples); every sample is recorded with the swept parameters as signals, a two-parameter grid also as `grid.<signal>` grids for heat maps and cuts, and unordered samples are drawn as markers coloured by value; the app's sweep bar chooses the design, count, seed and parameters; `simsandbox run` writes one row per sample; resonance-samples example (a Sobol sweep).
- Documentation system: `AGENTS.md` entry point and session protocol, documentation map, status, vision, roadmap, architecture, design, guides, research, open questions, risks, glossary, session notes, ADRs 0005–0006.
- Docs check (`scripts/check_docs.py`) in CI; pull request and issue templates.
- `Model::param`, `Model::params`, `Model::block_ids`, `Simulator::model` and `Simulator::param` for reading parameters and Block specs.
- ADRs 0007 (JSON model files with a block-type registry), 0008 (all-native egui + wgpu desktop UI), 0009 (hobbyists are the first audience), 0010 (wall-clock pacing of the live loop), 0011 (typed parameter entry; ranges are suggestions), 0012 (exploration controls saved in model files), 0013 (run settings in a `run` section), 0014 (opening view saved in `controls`), 0015 (pluggable views; which style options to expose), 0016 (models choose the views shown) 0017 (built-in expression parser for typed equations), 0018 (one x–y plot), 0019 (typed models without time), 0020 (method choice only for approximations) and 0021 (maps, formulas and sweeps).

- Model files (format 1): JSON with blocks, connections and optional `controls`; `simsandbox-file` crate with `load`, `ModelFile::to_json` and `ModelFile::capture_params`; JSON Schema in `crates/simsandbox-file/model.schema.json`.
- Block-type registry (`simsandbox_core::registry`) and `simsandbox_eq::register` with `eq.lorenz` and `eq.coupled_oscillator`; `Integrator::from_settings` and `Integrator::settings`.
- `simsandbox run FILE [--t-end] [--dt]` writes every output of a model file as CSV.
- Example files `examples/lorenz.ssbx.json` and `examples/coupled-oscillators.ssbx.json`.
- Desktop app `simsandbox-app`: open model files or built-in examples, run/pause/reset, rewind with branching, speed control with wall-clock pacing, editable step, speed and accuracy labels, live time-series plot.
- App: parameter panel (typed entry with units, initial-condition markers, opt-in sliders with editable ranges), Restart from current parameters, Save, pause at the file's `t_end`.
- Model files: optional `run` section with `dt` (default 0.01) and `t_end` (ADR 0013); `simsandbox run` uses it unless `--dt`/`--t-end` are given.
- `Simulator::restart` and `ParamSpec::initial_condition`.
- App: phase-portrait view (any output against another) and a rotatable, zoomable 3D trajectory view; `--view time|phase|3d` start-up flag.
- Model files: optional `controls.view` (kind, axes, 3D camera) saved by the app and used when a model opens (ADR 0014); the Lorenz example opens in 3D.
- App: native Open, Save and Save as dialogs (Ctrl+O, Ctrl+S, Ctrl+Shift+S); Save writes back to the opened file, built-in examples ask where to save.
- Pluggable views (ADR 0015): new `simsandbox-views` crate with recorded-signal history, the view contract and registry, and the built-in views `plot.time`, `plot.phase`, `plot.3d`; slot pickers in the app; a colour slot (fade by time or colour by a signal) and a `trail_length` style property (seconds of trail; 0 shows none, -1 the whole run, the default).
- Model files: `controls.views`, a list of `{type, bind, style, camera}`; the older `controls.view` is still read and converted.
- App: several views at once as tiles; "More views…" adds any view that fits; tiles close individually and Save writes them all (ADR 0016). The examples open with curated views.
- Typed equations (ADR 0017): `simsandbox-expr` crate; `eq.equations` block type turning lines like `dx/dt = sigma*(y - x)` into a model with parameters, initial values and computed outputs; `equations` in model files; "New model…" and "Edit equations…" in the app; predator–prey example.
- Typed models without time (ADR 0021): discrete maps `x[n+1] = …` (method `iterate`), formula-only blocks (method `evaluate`), and parameter sweeps for models with no states (`controls.sweep`); logistic-map and resonance examples.
- Derived signals (`controls.signals`): expressions over plotted signals, parameters and `t`, edited in a "Signals…" window and plotted like any signal; the coupled-oscillator example plots its total energy.
- Grid and point-set port shapes (`Shape::Grid`, `Shape::Points`), recorded whole; `plot.heatmap` (a line of cells over time, or a 2D grid) and `plot.scatter` (a point set) views.
- Block types `eq.heat_rod` (heat along a rod, a grid), `eq.orbits` (test particles around a central mass, a point set) and `eq.pendulum` (a damped pendulum) with heat-rod, orbits and pendulum examples.
- Automatic fusion (ADR 0023): connected equation blocks with identical methods are solved together as one system, so their connections add no coupling error; `"fusion": "off"` and explicit `"groups"` (blocks plus a method) in model files; `Model::fuse`, `GroupSolver` and `Registry::register_group_solver` in the kernel; a "Solving" section in the app shows the groups and switches fusion on or off. The coupled-oscillator example now keeps its energy.
- `plot.profile`: line fields as they are now, against a coordinate field (e.g. temperature against position along the rod).
- `eq.heat_rod` has a `length` parameter and a `position` output; `plot.heatmap` has a `position` slot (cells placed along the line) and a `time_axis` style property (time across or upwards).
- App: `--window signals|equations` opens that window at start (for headless checks).
- The first domain view, `eq.pendulum`, drawing the rod and bob; it is offered only for models with a pendulum. Domains register views with `simsandbox_eq::register_views` behind the `views` feature.

### Fixed

- App: a sweep that reaches an infinite value (the resonance example swept over `spring.c` from 0) no longer crashes; plots leave a gap at infinite or NaN values. Choosing a sweep parameter uses its slider range when it has one.

### Changed

- Sweeps moved from `controls.sweep` to the top-level `sample` section (ADR 0027); old files are converted on load: views bound to `t` are bound to the first swept parameter, two-parameter heat maps and cut filters to the `grid.` signals. A one-parameter sweep is indexed by sample number, with the parameter as a signal.
- `plot.profile` draws line fields only; cuts through 2D grids come from cut filters (ADR 0031, superseding ADR 0029's cuts inside the profile).
- Filters (ADRs 0030–0032): `controls.filters` holds named filters that name their inputs; `expression` filters are derived signals (they may now use each other) and `cut` filters cut 2D grids along x, along y or free lines into line fields with a coordinate and readable labels. The "Signals…" window becomes "Data…", listing and editing all filters; a new cut filter opens a profile of its cuts. Older `controls.signals` and profile cut selections are converted on load.
- `plot.xy` (plots against time, phase portraits) has a "swap axes" button: the same signals drawn transposed, saved with the view.
- Profile cuts (ADR 0029): `plot.profile` draws 2D grids as straight cuts in any direction, against fraction along the cut, x, y or distance, with bilinear or nearest interpolation; cuts are saved in the view's new `selection` and edited in the side panel. The resonance-map example adds resonance curves at four dampings.
- Heat map axes (ADR 0024): `x` and `y` each take `t` or a coordinate of the field (declared by its block, e.g. `rod.position`, or a swept parameter); pickers list only those; a "swap axes" button; unevenly spaced coordinates are drawn correctly. Files with the old `position` and `time_axis` are converted on load.
- View settings and slots are shown only where they apply (ADR 0025): e.g. `trail_length` is hidden and ignored in sweeps.
- `PortSpec::coordinate_of` declares an output a coordinate of another.
- Two-parameter sweeps (ADR 0022): `controls.sweep` may list two parameters; every output and derived signal becomes a grid over both, shown by a heat map placed by the parameter values; "+ parameter" and × in the app's sweep bar; heat maps gain `x`/`y` coordinate slots for 2D grids and a `log_colour` style property; resonance-map example.
- Plots span at least a millionth of the largest value shown, so a conserved quantity draws flat instead of magnifying rounding noise into apparent drift (zooming in still works).
- App: messages go to a log: a tile left of the rewind bar shows the newest one (dimmed after 8 s) and opens the full list, so messages never resize the window; the model description is one line with the full text on hover (one tooltip); the Derived signals text boxes are wider.
- App: "More views…" lists only views that apply to the model; saved 3D cameras are written in short form (0.35, not 0.3499999940395355).
- Views: `plot.time` and `plot.phase` are merged into `plot.xy` (any signals against time or each other, several curves, trail length as a scrolling window against time); "More views…" lists presets ("Plot against time", "Phase portrait", "3D trajectory"); files with the old types are converted on load (ADR 0018).
- Block factories receive a `BlockConfig` (method plus definition lines) instead of method settings alone.
- App: the view selector buttons are replaced by tiles and "More views…"; `--view` opens only that view.
- App: the phase portrait fades its trail by time, like the 3D view; the axis pickers became slot pickers.
- App: the File path field is replaced by the file dialogs; the current file's path is shown in the toolbar.
- App: the scroll wheel zooms 2D plots around the pointer instead of panning.
- `ParamSpec::min`/`max` renamed to `suggested_min`/`suggested_max`; the range is a hint for sliders, not a limit. `set_param` rejects only NaN and infinite values (`SimError::InvalidParam`); unknown names return `SimError::UnknownParam` instead of `SimError::BadConnection`.
- Snapshots store parameter values; `Simulator::restore` rewinds them along with state.

## [0.0.1] — 2026-10-15

### Added

- Rust workspace with `simsandbox-core`, `simsandbox-eq` and `simsandbox-cli`; CI on Windows, macOS and Linux.
- Block contract with capability traits; SI units with conversion; model graph; fixed-step co-simulation master; snapshots and rewind.
- Equation domain: Euler, RK4, Dormand–Prince 5(4); fusion of equation Blocks; Lorenz and coupled-oscillator systems.
- `simsandbox lorenz` command writing CSV.
- ADRs 0001–0004; Apache-2.0 license.
