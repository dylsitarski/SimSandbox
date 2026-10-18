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
- 2026-10-17: owner accepted ADR 0017 and the recommendations for Q12–Q14 (ADRs 0018–0020); `plot.time` and `plot.phase` merged into `plot.xy` with view presets. 79 tests.
- 2026-10-17: typed models without time: maps, formulas, sweeps (ADR 0021); logistic-map and resonance examples. 86 tests.
- 2026-10-17: views step 4, derived signals. 88 tests.
- 2026-10-17: owner feedback round: sweep-to-infinity crash, status line, description, Derived signals widths, heat rod length/position, `plot.profile`, heat map axes; Q15, Q16. 103 tests.
- 2026-10-17: Q17 (implicit and IMEX methods) raised; automatic fusion built (ADR 0023). 111 tests.
- 2026-10-17: minimum plot span; two-parameter sweeps with heat map coordinates and log colour, resonance-map example. 115 tests.
- 2026-10-18: owner questions on heat map axes, index-dependent settings, profiles of 2D grids, run versus sweep, unstructured sweeps recorded as Q18–Q22.
- 2026-10-18: ADR 0029 (profile cuts, supersedes 0026); view axes built (ADRs 0024–0025). 118 tests.
- 2026-10-18: profile cuts built (ADR 0029); views crate gains serde for selections. 122 tests.
- 2026-10-18: swap axes for `plot.xy` (transpose); Q23 and Q24 raised.
- 2026-10-18: ADRs 0030–0031; Q25 (filters and views on the node canvas) raised.
- 2026-10-17: messages moved into a log tile beside the rewind bar (`crates/simsandbox-app/src/log.rs`) at the owner's request; duplicate description tooltip removed (egui already shows cut text on hover). 104 tests.
- 2026-10-17: views steps 5–6: `Shape::Grid`/`Shape::Points`, fields in the history, `plot.heatmap`, `plot.scatter`, slot kinds and `Available`; `eq.heat_rod`, `eq.orbits` and `eq.pendulum` with verification tests; the `eq.pendulum` view (`applies_to`) behind `simsandbox-eq`'s `views` feature; heat-rod, orbits and pendulum examples, checked by screenshot. 100 tests.
- Wrote ADR 0015 and the [visualization architecture](../architecture/visualization.md) for Q9 and Q10.
- Wrote ADRs 0007 to 0015; updated STATUS, open questions, architecture overview, block contract and interaction docs.

## Decided

- Visualization becomes sources → filters → views, with filters as a flat list in `controls` and views that only draw; native visualization is the live loop's and is not capped (3D views and rendering grow with production needs); export complements it; no Rerun ([ADR 0030](../decisions/0030-filters-and-native-visualization.md)). Cuts become filters with one mode each: along x at fixed y, along y at fixed x, or free lines, with a whole-field default ([ADR 0031](../decisions/0031-cut-filters.md), superseding 0029).
- Profiles of 2D grids are straight cuts in any direction, with a chosen horizontal axis (fraction, x, y or distance) and nearest or bilinear interpolation; supersedes ADR 0026 at the owner's request ([ADR 0029](../decisions/0029-profile-cuts.md)).
- Field views and sweeps (2026-10-18, owner agreed to the recommendations): heat map axes take the index or declared coordinate fields, with swap ([ADR 0024](../decisions/0024-heat-map-axes-and-coordinates.md)); slots and settings declare where they apply ([ADR 0025](../decisions/0025-index-aware-view-settings.md)); profiles of 2D grids are slices ([ADR 0026](../decisions/0026-profile-slices.md)); experiments are explicit, sampling × run × reduction, decided at model level ([ADR 0027](../decisions/0027-explicit-experiments.md)); sweeps are recorded as samples with grid structure as metadata, superseding ADR 0022's storage ([ADR 0028](../decisions/0028-sweeps-as-samples.md)).
- Sweeps cover one or two parameters; two give grid fields shown by heat map, then surface and contour views; sweeping models with states waits for bifurcation diagrams ([ADR 0022](../decisions/0022-two-parameter-sweeps.md)).
- Equation blocks are fused automatically when connected and using identical methods; the owner asked that fusion can be turned off (`fusion: off`) and that explicit groups stay optional, so different methods remain possible across groups ([ADR 0023](../decisions/0023-automatic-fusion.md)).
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

- Q5 (algebraic loops) remains ([open questions](../open-questions.md)).

## Notes for the next session

- View selections are JSON (`ViewEntry::selection`, `View::selection`/`set_selection`/`edit_selection`); the views crate depends on serde for them. A selection that does not parse drops the view with a warning, like other bad controls.
- Heat map axes: `heat_axes` resolves `x`/`y` to time or a grid axis; unbound axes take the directions left in order. `axis_map` maps texels to cells, one per cell when even, resampled at the smallest spacing (≤ 1024 texels) when not. A coordinate that is not strictly monotonic falls back to cell numbers at draw time (the picker and load check prevent binding non-coordinates).
- Two-parameter sweep grids are stored row by row with the first parameter along x (cell (i, j) at j·nx + i); parameter values use the same lerp as one-parameter sweeps, so range ends are exact. The session keeps `output_names` and `base_fields` and rebuilds the history whenever the sweep's shape changes (`rebuild_history`).
- A linear colour scale hides everything but the resonance peak (amplitude ~ 1/c); the example uses `log_colour`.
- Fusion was built in the master rather than as ADR 0023's consequences sketched (the loader building an `EquationGroup` and renaming ports): members stay separate Blocks and only their stepping changes, so names, ids, snapshots and `set_param` needed no mapping. The master's group right-hand side is bit-identical to `FusedSystem`'s (test `master_fusion_matches_the_fused_system_bit_for_bit`).
- With fusion the oscillator energy plot autoscaled to rounding noise (10⁻¹⁴), which looked like drift; plots now keep a minimum span of 10⁻⁶ of the largest magnitude (owner's request).
- egui panics in debug builds when asked to draw a NaN or infinite point ("Comparing NaN vectors"); `crates/simsandbox-views/tests/render.rs` renders every built-in view headlessly with such values. New views should be added to it.
- The coupled-oscillator energy drift was measured with `simsandbox run` at dt = 0.01, 0.005, 0.0025: +11.5 %, +4.9 %, +2.3 % at t = 100, first order in dt, matching a coupling delay of dt/2 acting as negative damping on the antisymmetric mode (Q16).
- egui `TextEdit`s inside a `Grid` collapsed to a few characters in the Derived signals window; fixed sizes (`add_sized`) work.
- Fields are recorded whole and never split into scalars; derived signals cannot use them yet. A missing or misshapen frame is stored as NaN so frames stay in step with times.
- The heat rod with RK4 is stable only while α·dt_sub < about 0.7 h² (h = 1/51); the example uses 10 substeps at dt 0.05, enough up to α ≈ 0.05 (the slider's suggested maximum). Larger α needs more substeps or an implicit method.
- The pendulum view's preset binds the first `B.x`/`B.y` pair it finds; `Available` lists block types but not which block has which type, so a model mixing a pendulum with another block that has `x` and `y` outputs may bind the wrong one. Give `Available` block names per type if that matters.
- Saved 3D cameras are written in their shortest single-precision form (`tidy` in `crates/simsandbox-app/src/views.rs`); before, saving the Lorenz example wrote `-0.6000000238418579`.
- Sweep points use `lo*(1 - f) + hi*f`, so both ends are hit exactly (`lo + (hi - lo)*f` gave 0.9999999999999999 for 1.0).
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
