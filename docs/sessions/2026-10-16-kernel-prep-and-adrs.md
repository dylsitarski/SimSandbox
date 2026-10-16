# 2026-10-16 — Kernel prep and ADRs 0007–0008

- **Who:** Dylan Sitarski with Claude (Claude Code, cloud session)
- **Goal:** review the repository, settle the next steps, prepare the kernel for model files and the desktop UI, and resolve Q1 and Q3

## Done

- Reviewed code against STATUS; found that `Simulator` exposed no specs or parameter values, `set_param` accepted any value and misreported unknown names as `BadConnection`, and snapshots did not include parameters.
- Added parameter access (`Model::param`, `Model::params`, `Model::block_ids`, `Simulator::model`, `Simulator::param`), range and finiteness checks with `SimError::UnknownParam` and `SimError::ParamOutOfRange`, and parameters in snapshots. Tests in `crates/simsandbox-core/tests/params.rs`.
- Wrote ADRs 0007 and 0008; updated STATUS, open questions, architecture overview, block contract and interaction docs.

## Decided

- Snapshots include parameter values, so rewinding also undoes slider changes (owner's decision, recorded in [architecture overview](../architecture/overview.md#data-flow-of-a-run)).
- `set_param` enforces the `ParamSpec` range; the range is a hard limit, not only a slider hint.
- Model files are JSON with a block-type registry ([ADR 0007](../decisions/0007-json-model-files.md)).
- The desktop UI is all-native eframe + egui + wgpu ([ADR 0008](../decisions/0008-native-egui-ui.md)).

## Left open

- Q2 (first audience) should be answered before the app shell is finished ([open questions](../open-questions.md)).
- Steps per frame in the app loop (STATUS item 2).

## Notes for the next session

- The owner agreed to the recommendation "JSON or TOML via serde"; this session chose JSON for schema tooling and nested blocks/connections. Revisit by superseding ADR 0007 if the owner prefers TOML.
- Because ranges are now enforced, any new `ParamSpec` must have a range wide enough for legitimate use.
