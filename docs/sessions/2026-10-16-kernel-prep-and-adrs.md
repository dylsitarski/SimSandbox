# 2026-10-16 — Kernel prep and ADRs 0007–0015

- **Who:** Dylan Sitarski with Claude (Claude Code, cloud session)
- **Goal:** review the repository, settle the next steps, prepare the kernel for model files and the desktop UI, and resolve Q1, Q2, Q3 and Q6

## Done

- Reviewed code against STATUS; found that `Simulator` exposed no specs or parameter values, `set_param` accepted any value and misreported unknown names as `BadConnection`, and snapshots did not include parameters.
- Added parameter access (`Model::param`, `Model::params`, `Model::block_ids`, `Simulator::model`, `Simulator::param`), finiteness checks with `SimError::UnknownParam` and `SimError::InvalidParam`, and parameters in snapshots. Tests in `crates/simsandbox-core/tests/params.rs`.
- Implemented the model file format: kernel `Registry` and `MethodSettings`, `simsandbox_eq::register`, the `simsandbox-file` crate (load, save, lenient controls, JSON Schema), two example files, `simsandbox run`, and a guide for adding block types. 30 tests.
- Built the desktop app shell (`crates/simsandbox-app`): pacing, session with rewind history, egui window, built-in examples, `--screenshot`. Verified under Xvfb with screenshots of both examples running at 1×. 38 tests.
- Added the `run` section (ADR 0013), `Simulator::restart`, `ParamSpec::initial_condition`, and the app's parameter panel, Restart and Save. Verified headless with screenshots of both examples. 44 tests.
- Added phase-portrait and 3D trajectory views (`view3d.rs`: CPU projection, drag to rotate, scroll to zoom, fit from the bounding sphere so the box never clips). 49 tests.
- Saved the opening view in `controls` (ADR 0014) and made the scroll wheel zoom 2D plots. 52 tests.
- Added native Open, Save and Save as dialogs (`rfd`, XDG portal backend on Linux, so no GTK build dependency) and keyboard shortcuts; the path field is gone. 54 tests.
- Pluggable views steps 1–2: `simsandbox-views` (history, view contract and registry, colour encodings, built-in views), `controls.views` with conversion of the older `view`, slot pickers, colour slot, `trail_length`. Verified with screenshots of all three views and a 3 s trail. 63 tests.
- 2026-10-17: `trail_length` 0 now shows no trail and -1 the whole run (owner's request); Q11 raised (which views a model offers).
- 2026-10-17: ADR 0016 (models choose the views shown, "More views…" for the rest) and views step 3 (tiles). 64 tests.
- 2026-10-17: symbolic equation entry: `simsandbox-expr`, `eq.equations` (`EquationSet`), `equations` in model files, equation editor, predator–prey example; ADR 0017 written as Proposed (the parser approach was chosen by the agent and awaits the owner's confirmation).
- Wrote ADR 0015 and the [visualization architecture](../architecture/visualization.md) for Q9 and Q10.
- Wrote ADRs 0007 to 0015; updated STATUS, open questions, architecture overview, block contract and interaction docs.

## Decided

- Snapshots include parameter values, so rewinding also undoes slider changes (owner's decision, recorded in [architecture overview](../architecture/overview.md#data-flow-of-a-run)).
- First `set_param` enforced the `ParamSpec` range; the owner reversed this the same day: ranges are suggestions, any finite value is accepted, and parameters are edited through typed fields with opt-in sliders ([ADR 0011](../decisions/0011-parameter-entry.md)).
- Run settings (`dt`, optional `t_end`) are saved in a `run` section; invalid values are load errors; the app pauses once at `t_end` ([ADR 0013](../decisions/0013-run-section.md)).
- Models choose the views shown as tiles; other fitting views are added from "More views…"; domain views fit only their own block types ([ADR 0016](../decisions/0016-model-curated-views.md)).
- Views become pluggable like Blocks: view types with slots bound to signals (outputs, `t`, derived signals), declared style properties, a registry with domain-provided views, several views at once; style options are exposed only as encodings (slots) or readability properties, and viewer needs such as palettes are app settings ([ADR 0015](../decisions/0015-pluggable-views.md)).
- Initial-condition parameters are flagged in `ParamSpec` and apply on Restart, or at once before the run starts; Restart replaces Reset in the app.
- Slider choices and ranges, default playback speed and default plots are saved in an optional `controls` section of the model file; per-person UI state is not ([ADR 0012](../decisions/0012-controls-in-model-files.md)).
- Model files are JSON with a block-type registry ([ADR 0007](../decisions/0007-json-model-files.md)).
- The desktop UI is all-native eframe + egui + wgpu ([ADR 0008](../decisions/0008-native-egui-ui.md)).
- Hobbyists are the first audience, matching the owner's own purposes and sitting between students and researchers ([ADR 0009](../decisions/0009-hobbyists-first.md)).
- The live loop is paced by wall-clock time with a speed control, a per-frame budget and the simulation on the UI thread for Phase 0 ([ADR 0010](../decisions/0010-live-loop-pacing.md)).

## Left open

- Owner to confirm ADR 0017 (typed equations, proposed). Q5 (algebraic loops) remains ([open questions](../open-questions.md)).

## Notes for the next session

- The equation editor was checked through `apply_equations` tests and a screenshot of the typed predator–prey example, not by typing into the window.
- egui's `horizontal_wrapped` did not wrap the slot combo boxes, so a 3D tile's header spilled into the next tile; slot pickers are now laid out in fixed-width rows sized to the tile. The default font lacks "✕"; the close button uses "×".
- The app keeps one `ViewState` per visited view type so switching keeps bindings, style and camera; Save writes only the active one. Multi-view (step 3) should save the whole list.
- Old `controls.view` entries are converted in `simsandbox-file` (`ViewControl::into_entry`), adding `colour: t` so trails still fade.
- The file dialogs were not exercised in this container (no desktop portal); `save_to`, `open_path` and the naming helpers are unit-tested, the dialog calls are thin wrappers around `rfd`.
- GPU code can be tested here: the app already renders through wgpu on lavapipe (Mesa software Vulkan; `VK_LOADER_DEBUG=driver` shows `lvp_icd.json`). Packages needed: `libxkbcommon-x11-0 mesa-vulkan-drivers xvfb`. An earlier claim this session that GPU code could not be tested here was wrong.
- The first 3D fit used a fixed fraction of the panel and clipped near corners under perspective; the fit now uses the box's bounding sphere (test `the_box_fits_at_every_rotation`).
- With x0 = y0 = z0 = 1, Lorenz spirals on one lobe until about t = 17 before switching; screenshots at t = 30 show the butterfly.
- Parameter fields show at most 6 significant digits and only apply text the user changed, so the display's rounding never alters a value (e.g. beta = 8/3).
- Phase 0 gate counted as met from headless screenshots; the owner should confirm on Windows (`cargo run -p simsandbox-app`).
- `egui_plot` 0.37 is built against egui 0.36, so the app pins `eframe = 0.36.2`; pinning eframe 0.37 produced two egui versions and type errors. Upgrade both together.
- egui 0.36 changed `App::update` to `App::ui(&mut self, ui, frame)` and panels to `Panel::top(..).show(ui, ..)`; screenshots arrive as `Event::Screenshot` after `ViewportCommand::Screenshot`.
- The cloud container lacked `libxkbcommon-x11-0`; installing it (and `mesa-vulkan-drivers`) made `xvfb-run` work for screenshots.
- Interactive controls (buttons, rewind slider, drag and drop) were checked by reading the code and through `Session`/`Pacer` tests, not by clicking in a running window.
- Saving goes through the `ModelFile` that was loaded (`capture_params` then `to_json`); Blocks carry no type name, so a model built only in code cannot be saved. Revisit if the app needs "new model" before the node canvas.
- A first coupled-oscillator file test used a guessed tolerance (1e-2) and failed at 1.09e-2; the file tests now check bit-identity with the model built in code, since accuracy is verified in `simsandbox-eq`.

- The owner agreed to the recommendation "JSON or TOML via serde"; this session chose JSON for schema tooling and nested blocks/connections. Revisit by superseding ADR 0007 if the owner prefers TOML.
- A Block whose maths fails for some finite parameter values must report it with `SimError::Block`; the kernel no longer filters by range.
