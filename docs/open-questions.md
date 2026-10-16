# Open questions

_Last updated: 2026-10-15_

Decisions not yet made. When one is resolved, write an ADR, link it in the **Resolved** table and remove the row from **Open**.

## Open

| ID | Question | Options | Needed by | Notes |
| --- | --- | --- | --- | --- |
| Q1 | Model file format | JSON with a schema; TOML; a small custom DSL | Phase 0 (next up) | Must be plain text, diffable and easy for AI tools to generate |
| Q2 | First audience for the prototype | Students; hobbyists; researchers in the owner's field | Phase 0 gate | Shapes the first examples and UI defaults |
| Q3 | Desktop UI approach | All-native egui + wgpu; Tauri web UI with a native render window | Before the app shell | [ADR 0003](decisions/0003-desktop-first.md) leans all-native |
| Q4 | Symbolic equation parsing | Rust-native parser and symbolic crate; SymPy via Python; Julia Symbolics service | Symbolic equation entry | Affects dependencies and startup |
| Q5 | Algebraic loops in fused groups | Causalisation and tearing; Newton solve per RHS evaluation; forbid and warn | Phase 1 | Current v0 restriction: no direct feedthrough |

## Resolved

| ID | Question | Resolution |
| --- | --- | --- |
| — | Rust core or Julia-first core | [ADR 0001](decisions/0001-rust-core.md) |
| — | Shape of the Block contract | [ADR 0002](decisions/0002-layered-block-contract.md) |
| — | Desktop-only or browser | [ADR 0003](decisions/0003-desktop-first.md) |
| — | "One model, three modes" or per-method classification | [ADR 0004](decisions/0004-method-classification.md) |
| — | Where project knowledge lives | [ADR 0005](decisions/0005-repository-as-source-of-truth.md) |
| — | License | [ADR 0006](decisions/0006-apache-2-license.md) |
