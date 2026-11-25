# 2026-11-25 — Encapsulation and composition

- **Who:** owner and agent (Claude Code)
- **Goal:** revisit the interface direction ([ADR 0108](../decisions/0108-interface-synchronized-projections.md)): its overhead, whether models are encapsulated, and whether nesting should organize the interface.

## Done

- [Research note](../research/2026-11-25-encapsulation-and-composition.md): the planned work adds no housekeeping for people (notes are agents' only; roles optional; the card generated), but it makes a flat graph legible by labels, which does not scale (the platoon's card lists six inner blocks). Models nest and declare interfaces, but a use may set inner values by path, and the surfaces flatten. It recommends structure first: encapsulation, surfaces by level, then extension, replaceable parts, replication, contracts.
- [Q65](../open-questions.md) raised; the Inspector by intent paused on it.
- [ADR 0110](../decisions/0110-encapsulated-models.md) drafted (proposed): a use changes its model only through its interface; inner values set by path convert to declared parameters (in-file definitions) or are reported (separate files); start states by inner block stay; reading inside stays open and is marked, with "Offer as output"; the card, Overview and Inspector show one level (`explain --part`, `--depth`); Python's `expose`.

## Decided

- Owner (2026-11-25): draft the encapsulation ADR. Not yet accepted.

## Left open

- ADR 0110's acceptance (answers Q65). The setup-script warning at this session's start was a stale file from 2026-11-14 (the old script); the stored script and installed packages are right.

## Notes for the next session

- Wires into a use go only through its interface (`resolve` in `simsandbox-file/src/subsystems.rs`). The leaks are `params` keys with `.` or `/` (`declare`, the loop after `level`), `starts`, wires from inner outputs by path (`a/c.x` passes `check`) and probe links at any depth.
- Owner asked (2026-11-25) whether reading should be closed too; the draft keeps observing open (option E rejected) but closes reading that feeds a result (wires, links, formulas, events).
- No example sets inner values by path; the Inspector's per-use table (`simsandbox-app/src/inspector.rs`, `app/param_rows.rs`) is what writes them.
