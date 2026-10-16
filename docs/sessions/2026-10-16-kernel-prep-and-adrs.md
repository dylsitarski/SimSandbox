# 2026-10-16 — Kernel prep and ADRs 0007–0011

- **Who:** Dylan Sitarski with Claude (Claude Code, cloud session)
- **Goal:** review the repository, settle the next steps, prepare the kernel for model files and the desktop UI, and resolve Q1, Q2, Q3 and Q6

## Done

- Reviewed code against STATUS; found that `Simulator` exposed no specs or parameter values, `set_param` accepted any value and misreported unknown names as `BadConnection`, and snapshots did not include parameters.
- Added parameter access (`Model::param`, `Model::params`, `Model::block_ids`, `Simulator::model`, `Simulator::param`), finiteness checks with `SimError::UnknownParam` and `SimError::InvalidParam`, and parameters in snapshots. Tests in `crates/simsandbox-core/tests/params.rs`.
- Wrote ADRs 0007 and 0008; updated STATUS, open questions, architecture overview, block contract and interaction docs.

## Decided

- Snapshots include parameter values, so rewinding also undoes slider changes (owner's decision, recorded in [architecture overview](../architecture/overview.md#data-flow-of-a-run)).
- First `set_param` enforced the `ParamSpec` range; the owner reversed this the same day: ranges are suggestions, any finite value is accepted, and parameters are edited through typed fields with opt-in sliders ([ADR 0011](../decisions/0011-parameter-entry.md)).
- Model files are JSON with a block-type registry ([ADR 0007](../decisions/0007-json-model-files.md)).
- The desktop UI is all-native eframe + egui + wgpu ([ADR 0008](../decisions/0008-native-egui-ui.md)).
- Hobbyists are the first audience, matching the owner's own purposes and sitting between students and researchers ([ADR 0009](../decisions/0009-hobbyists-first.md)).
- The live loop is paced by wall-clock time with a speed control, a per-frame budget and the simulation on the UI thread for Phase 0 ([ADR 0010](../decisions/0010-live-loop-pacing.md)).

## Left open

- Q4 (equation parsing) and Q5 (algebraic loops) remain ([open questions](../open-questions.md)).
- Whether slider choices and ranges are saved in model files (ADR 0011 consequences).

## Notes for the next session

- The owner agreed to the recommendation "JSON or TOML via serde"; this session chose JSON for schema tooling and nested blocks/connections. Revisit by superseding ADR 0007 if the owner prefers TOML.
- A Block whose maths fails for some finite parameter values must report it with `SimError::Block`; the kernel no longer filters by range.
