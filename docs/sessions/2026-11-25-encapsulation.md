# 2026-11-25 — Encapsulation and composition

- **Who:** owner and agent (Claude Code)
- **Goal:** revisit the interface direction ([ADR 0108](../decisions/0108-interface-synchronized-projections.md)): its overhead, whether models are encapsulated, and whether nesting should organize the interface.

## Done

- [Research note](../research/2026-11-25-encapsulation-and-composition.md): the planned work adds no housekeeping for people (notes are agents' only; roles optional; the card generated), but it makes a flat graph legible by labels, which does not scale (the platoon's card lists six inner blocks). Models nest and declare interfaces, but a use may set inner values by path, and the surfaces flatten. It recommends structure first: encapsulation, surfaces by level, then extension, replaceable parts, replication, contracts.
- [Q65](../open-questions.md) raised; the Inspector by intent paused on it.
- [ADR 0110](../decisions/0110-encapsulated-models.md) drafted (proposed): a use changes its model only through its interface; inner values set by path convert to declared parameters (in-file definitions) or are reported (separate files); start states by inner block stay; reading inside stays open and is marked, with "Offer as output"; the card, Overview and Inspector show one level (`explain --part`, `--depth`); Python's `expose`.

- ADR 0110 step 1 built (owner: "start building step 1"): `simsandbox-file/src/encapsulate.rs` converts values set by path, wires and probe links from inner outputs, level by level, before the composition loader reads a file; separate files reported; `expose` and `offer_output`, and Python's `Model.expose` and `Model.offer`, with refusals of inner paths; `upgrade` loads models that use models; a fixture, `tests/encapsulation.rs`, the Python composition test through interfaces. Every example's run gives the same bytes as before.

## Decided

- Owner (2026-11-25): [ADR 0110](../decisions/0110-encapsulated-models.md) accepted (Q65 answered: structure first). Asked whether to close reading too: observing stays open, reading that feeds a result goes through outputs.
- Owner (2026-11-25): [ADR 0111](../decisions/0111-a-declarative-modelling-language.md), SimSandbox is a declarative modelling language, not a library; written into the vision and the architecture overview; the language reference in Next up.

## Left open

- ADR 0110 steps 2 and 3; [Q66](../open-questions.md) (studies and schedules set inner values by path). The setup-script warning at this session's start was a stale file from 2026-11-14 (the old script); the stored script and installed packages are right.

## Notes for the next session

- **Encapsulation's conversion** runs inside `load_composition` and `load_merged_with`, so it covers only files with uses. A file without uses has nothing to convert. It needs the registry: a default the model does not write is the block type's, read by building the block.
- **Definitions used inside definitions** run on their own clocks unless shared (`o/p` in the fixture is a model of its own), so a wire from `o/p/c.x` was refused even before ADR 0110; values reach them by path and convert.

- Wires into a use go only through its interface (`resolve` in `simsandbox-file/src/subsystems.rs`). The leaks are `params` keys with `.` or `/` (`declare`, the loop after `level`), `starts`, wires from inner outputs by path (`a/c.x` passes `check`) and probe links at any depth.
- Owner asked (2026-11-25) whether reading should be closed too; the draft keeps observing open (option E rejected) but closes reading that feeds a result (wires, links, formulas, events).
- No example sets inner values by path; the Inspector's per-use table (`simsandbox-app/src/inspector.rs`, `app/param_rows.rs`) is what writes them.
