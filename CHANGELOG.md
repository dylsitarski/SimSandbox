# Changelog

All notable user-visible changes are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/) once it reaches 0.1.0.

## [Unreleased]

### Added

- Documentation system: `AGENTS.md` entry point and session protocol, documentation map, status, vision, roadmap, architecture, design, guides, research, open questions, risks, glossary, session notes, ADRs 0005–0006.
- Docs check (`scripts/check_docs.py`) in CI; pull request and issue templates.
- `Model::param`, `Model::params`, `Model::block_ids`, `Simulator::model` and `Simulator::param` for reading parameters and Block specs.
- ADRs 0007 (JSON model files with a block-type registry), 0008 (all-native egui + wgpu desktop UI), 0009 (hobbyists are the first audience), 0010 (wall-clock pacing of the live loop), 0011 (typed parameter entry; ranges are suggestions), 0012 (exploration controls saved in model files), 0013 (run settings in a `run` section), 0014 (opening view saved in `controls`), 0015 (pluggable views; which style options to expose), 0016 (models choose the views shown) 0017 (built-in expression parser for typed equations), 0018 (one x–y plot), 0019 (typed models without time) and 0020 (method choice only for approximations).

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

### Changed

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
