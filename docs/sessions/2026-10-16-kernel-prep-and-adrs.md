# 2026-10-16 — Kernel prep and ADRs 0007–0012

- **Who:** Dylan Sitarski with Claude (Claude Code, cloud session)
- **Goal:** review the repository, settle the next steps, prepare the kernel for model files and the desktop UI, and resolve Q1, Q2, Q3 and Q6

## Done

- Reviewed code against STATUS; found that `Simulator` exposed no specs or parameter values, `set_param` accepted any value and misreported unknown names as `BadConnection`, and snapshots did not include parameters.
- Added parameter access (`Model::param`, `Model::params`, `Model::block_ids`, `Simulator::model`, `Simulator::param`), finiteness checks with `SimError::UnknownParam` and `SimError::InvalidParam`, and parameters in snapshots. Tests in `crates/simsandbox-core/tests/params.rs`.
- Implemented the model file format: kernel `Registry` and `MethodSettings`, `simsandbox_eq::register`, the `simsandbox-file` crate (load, save, lenient controls, JSON Schema), two example files, `simsandbox run`, and a guide for adding block types. 30 tests.
- Built the desktop app shell (`crates/simsandbox-app`): pacing, session with rewind history, egui window, built-in examples, `--screenshot`. Verified under Xvfb with screenshots of both examples running at 1×. 38 tests.
- Wrote ADRs 0007 to 0012; updated STATUS, open questions, architecture overview, block contract and interaction docs.

## Decided

- Snapshots include parameter values, so rewinding also undoes slider changes (owner's decision, recorded in [architecture overview](../architecture/overview.md#data-flow-of-a-run)).
- First `set_param` enforced the `ParamSpec` range; the owner reversed this the same day: ranges are suggestions, any finite value is accepted, and parameters are edited through typed fields with opt-in sliders ([ADR 0011](../decisions/0011-parameter-entry.md)).
- Slider choices and ranges, default playback speed and default plots are saved in an optional `controls` section of the model file; per-person UI state is not ([ADR 0012](../decisions/0012-controls-in-model-files.md)).
- Model files are JSON with a block-type registry ([ADR 0007](../decisions/0007-json-model-files.md)).
- The desktop UI is all-native eframe + egui + wgpu ([ADR 0008](../decisions/0008-native-egui-ui.md)).
- Hobbyists are the first audience, matching the owner's own purposes and sitting between students and researchers ([ADR 0009](../decisions/0009-hobbyists-first.md)).
- The live loop is paced by wall-clock time with a speed control, a per-frame budget and the simulation on the UI thread for Phase 0 ([ADR 0010](../decisions/0010-live-loop-pacing.md)).

## Left open

- Q7: where the communication step `dt` is saved ([open questions](../open-questions.md)).
- Q4 (equation parsing) and Q5 (algebraic loops) remain ([open questions](../open-questions.md)).

## Notes for the next session

- `egui_plot` 0.37 is built against egui 0.36, so the app pins `eframe = 0.36.2`; pinning eframe 0.37 produced two egui versions and type errors. Upgrade both together.
- egui 0.36 changed `App::update` to `App::ui(&mut self, ui, frame)` and panels to `Panel::top(..).show(ui, ..)`; screenshots arrive as `Event::Screenshot` after `ViewportCommand::Screenshot`.
- The cloud container lacked `libxkbcommon-x11-0`; installing it (and `mesa-vulkan-drivers`) made `xvfb-run` work for screenshots.
- Interactive controls (buttons, rewind slider, drag and drop) were checked by reading the code and through `Session`/`Pacer` tests, not by clicking in a running window.
- Saving goes through the `ModelFile` that was loaded (`capture_params` then `to_json`); Blocks carry no type name, so a model built only in code cannot be saved. Revisit if the app needs "new model" before the node canvas.
- A first coupled-oscillator file test used a guessed tolerance (1e-2) and failed at 1.09e-2; the file tests now check bit-identity with the model built in code, since accuracy is verified in `simsandbox-eq`.

- The owner agreed to the recommendation "JSON or TOML via serde"; this session chose JSON for schema tooling and nested blocks/connections. Revisit by superseding ADR 0007 if the owner prefers TOML.
- A Block whose maths fails for some finite parameter values must report it with `SimError::Block`; the kernel no longer filters by range.
