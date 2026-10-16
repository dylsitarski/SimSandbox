# Decision records

Architecture decision records (ADRs) capture one significant decision each: the context, the decision and its consequences. They are the project's memory of *why*.

## Index

| ADR | Title | Status | Date |
| --- | --- | --- | --- |
| [0001](0001-rust-core.md) | Rust core | Accepted | 2026-10-15 |
| [0002](0002-layered-block-contract.md) | Layered Block contract | Accepted | 2026-10-15 |
| [0003](0003-desktop-first.md) | Desktop-first | Accepted | 2026-10-15 |
| [0004](0004-method-classification.md) | Method classification instead of "one model, three modes" | Accepted | 2026-10-15 |
| [0005](0005-repository-as-source-of-truth.md) | Repository as single source of truth | Accepted | 2026-10-15 |
| [0006](0006-apache-2-license.md) | Apache-2.0 license | Accepted | 2026-10-15 |

## When to write one

Write an ADR when a choice is hard to reverse, affects more than one crate or document, or settles an item in [open questions](../open-questions.md). Small, local choices belong in code comments or the pull request.

## How

1. Copy [`0000-template.md`](0000-template.md) to the next number: `NNNN-short-title.md`.
2. Fill it in; keep it to one page. Status starts as `Proposed` if the owner has not agreed yet, otherwise `Accepted`.
3. Add a row to the index above (the docs check fails otherwise) and to the map in [`docs/README.md`](../README.md).
4. Update the documents the decision affects, and remove the item from open questions if it resolves one.

## Rules

- Accepted ADRs are not rewritten. To change a decision, write a new ADR and set the old one's status to `Superseded by NNNN`.
- Statuses: `Proposed`, `Accepted`, `Superseded by NNNN`, `Rejected`.
