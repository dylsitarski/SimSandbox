# 2026-11-15 — The equations' front end (review W6)

- **Who:** Dylan Sitarski with Claude Code
- **Goal:** restructure the typed-equations front end (review W6) and fix the eight faults listed in the reference

## Done

- The eight faults fixed, each with a test that failed before; the reference corrected. 620 workspace tests pass; every example the same bytes.
- Stage 1: `symbolic.rs` split into `symbolic/` (eight files), moves only.
- Stages 2 and 4 (2026-11-16): typed statements; `parse_lines` a pipeline of eight phases with typed results; compared with the old code over 16 568 definitions. Three faults it brought to light fixed (a crash on a starting mesh larger than its table; duplicate parameters per cell).
- 2026-11-17: the three faults left fixed (rules' units, a quantity named like a transfer, the first wrong `start` told); a plain number in a rule now in its column's unit.
- 2026-11-17: stage 3, one table of the array functions (`symbolic/functions/`), matched exhaustively by the three passes; `describe` lists the functions. W6 complete.
- 2026-11-17: `centre` told of its own arguments; `Session` split into five parts (W8, first half), its public surface unchanged.
- 2026-11-17: `app.rs` split by pane, modal state in one enum (W8 complete).
- 2026-11-17: the bugs W8 brought up fixed (flush, the modal holding raw input, the close that never comes); the app run under Xvfb and compared by screenshot.
- 2026-11-17: the kernel tidy ([ADR 0098](../decisions/0098-the-kernel-holds-the-contract.md)); the review's second priority complete.
- 2026-11-17: the review's third priority begun: the step loop and recording, measured first with callgrind, ten commits, 10–50% fewer instructions per step, the same bits ([performance](../design/performance.md#the-step-loop-and-recording-2026-11-17)). Next: cell phases in `simsandbox-expr` (a thread pool, static index tables, implicit methods' buffers), then the views and the app.
- 2026-11-17: the review's third priority, second part: typed equations' per-evaluation costs, measured first, five commits, the same bits ([performance](../design/performance.md#typed-equations-per-evaluation-costs-2026-11-17)): Gather and Along without tables, the implicit methods' arrays in the `Workspace`, Take in place, tables' ids searched and their inputs built once. A thread pool for phases needs rayon (no `unsafe`): tried, no faster, not added. Next: the views and the app.

- 2026-11-17: the review's third priority, third part: the views' frames, measured first (`view_frames`), four commits: `paired` in place, `plot.xy` thinned to pixel columns and kept between frames, heat maps uploaded only on change ([performance](../design/performance.md#the-views-and-the-apps-frames-2026-11-17)). The worker's publish-when-dirty measured and left. Found: `plot.xy` coloured by `t` draws `y` against time (STATUS, known limitations).
- 2026-11-18: the phase-portrait bug fixed (`Curve::new` now always takes the colour's column off first; an `Option::zip` first tried evaluated its argument eagerly and dropped `y` when no colour was bound, caught by rendering the examples). The app's hang with `--view` and `--screenshot`: the view chosen counted as unsaved (the layout is saved with the model, and the baseline was taken before the dock had its views' tabs), and the screenshot's close was held by the unsaved-changes question; both fixed, checked under Xvfb with every `--view` and `--window`.
- 2026-11-18: the review's fourth priority, by two helpers in parallel (Rust in the checkout, Python and CI in a worktree, cherry-picked): CLI help, exit statuses, `--errors json`, several errors per `check` (ADR 0099); `lorenz` command and `group.rs` deleted; Python stubs, checks at the call, wheels in CI; CI cache, minimum Rust, Python 3.10, `cargo deny` (ADR 0100). The hard-coded systems investigated, not replaced (Q62).
- 2026-11-18: how meshes fit views asked; ADR 0101 (primitives are parametric meshes, one cached drawing path, shape meshes from models), accepted by the owner.
- 2026-11-19: the scene view, stage 1 (`plot.scene`, frames, the four primitives, `check`/`describe`/schema/`render`, the pendulum as a scene, which agrees with `eq.pendulum`'s view in the app and headless).
- 2026-11-19: the scene view, stage 2 (grabbing, `picked`, replay), with the example "Grab the bob"; a replayed grab checked in the app under Xvfb.
- 2026-11-19: the scene view, stage 3 (canvas node, renames, Inspector editing, `Model.add_scene`); ADR 0093's list before the gate complete. The container restarted during verification (likely memory): checks rerun with `-j 2`.
- 2026-11-19: parameter bindings in scenes (ADR 0102, owner), frame renames, a scene's check error on its tile and node. A first try recorded a study's swept parameter beside its field output and moved the resonance profiles' axis; caught by the render hashes and fixed (an output of any shape wins).
- 2026-11-19: the rigid-body spike, ADR 0103 proposed (Rapier). Checked: the cart-pole table reproduced exactly by a rebuild; the `mujoco-rs` licence's reservation against text and data mining and machine learning confirmed in its 7.1.0 `LICENSE`.
- 2026-11-19: the rigid-body follow-up: a native 2D ABA, the typed-equations routes and Featherstone as a typed-equations function; ADR 0103 revised to five options, proposing the native core plus Rapier for contacts. Checked by a rebuild: the ABA–CRBA agreement, the double pendulum's errors and orders, the momentum drift and the generated chains' timings reproduced; the typed files pass `check`; no workspace file changed.
- 2026-11-19: ADR 0103 revised on the owner's word: no typed-equations Featherstone function (the block covers it). The two problems the spike found were fixed on branches made from `main` in other sessions and ported here: the implicit methods' structure ([ADR 0104](../decisions/0104-implicit-structure-independent-of-the-state.md), renumbered from 0090; its question Q63, from Q56; the probe merged with this branch's scaled one) and the vector registers after faer ([performance](../design/performance.md#the-vector-registers-after-faer-2026-11-19)). Checked: workspace and Python tests, example hashes unchanged, the register test fails with the clearing disabled.

## Decided

- Owner (2026-11-14): the single build of a model that uses models is deferred ([ADR 0090](../decisions/0090-one-engine-for-nested-models.md), as built).
- The four unmaintained crates `cargo deny` reports are ignored with reasons, not left failing (ADR 0100); the owner may prefer otherwise.

## Left open

- The review is done but for Q62 (the hard-coded systems); before PyPI, the licence in the wheel and absolute README links; the app as taken apart, to try in the window; what ADR 0098 left in the kernel, for the plugin interface's design.

## Notes for the next session

- The app runs under Xvfb (`xvfb-run -a target/release/simsandbox-app FILE --screenshot X.ppm --after 2`) once `libxkbcommon-x11-0` and `mesa-vulkan-drivers` are installed ([development setup](../guides/development-setup.md#testing-gpu-code-without-a-gpu)); this environment lacked them (installed by hand 2026-11-17; the environment's setup script fails, so a new container lacks them again). Running the app builds native code into `~/.cache/simsandbox/native`, after which `check` reports `native`: compare `check` output with `SIMSANDBOX_NATIVE=off`.
- The comparison of old and new front end over a corpus of definitions (harvested from examples, the reference and tests, then mutated) found what the tests did not: build such a corpus before the next change of this size. Some error choices depend on `HashMap` order and differ from run to run; compare those without line and column.
- The safety net for work on the front end: `cargo test --workspace` (bit-for-bit tests against the interpreter and across routes), `crates/simsandbox-eq/tests/reference.rs`, and the hashes of every example's renders, runs and checks before and after.
