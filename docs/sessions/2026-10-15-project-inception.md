# 2026-10-15 — Project inception

- **Who:** Dylan Sitarski with Claude (Cowork), sessions on 2026-10-14 and 2026-10-15
- **Goal:** test whether the idea is worth building, plan it, make the core decisions, and set up the repository

## Done

- Market research: no existing tool combines approachable building, cross-paradigm composition and research-grade trust ([market landscape](../research/market-landscape.md)).
- Wrote an implementation plan as a Claude doc ("SimSandbox — Implementation Plan"), then migrated its content into `docs/` (vision, roadmap, architecture, design, risks, open questions).
- Scaffolded the Rust workspace: kernel, equation domain, CLI, CI, 13 tests.
- Designed and implemented this documentation system.

## Decided (in order)

1. Pillars fun, learning and research guide development ([vision](../vision.md)).
2. Desktop-first instead of browser, for performance across all methods ([ADR 0003](../decisions/0003-desktop-first.md)).
3. Replace "one model, three modes" with per-method speed and accuracy labels; the owner pointed out heavy models have no real-time method and users want one mode at a time ([ADR 0004](../decisions/0004-method-classification.md)).
4. Project name: SimSandbox.
5. Rust core, chosen for its more established base ([ADR 0001](../decisions/0001-rust-core.md)).
6. Layered hybrid Block contract after comparing FMI-only, Modelica-only, DEVS and fixed-tick dataflow ([ADR 0002](../decisions/0002-layered-block-contract.md)).
7. Apache-2.0 license ([ADR 0006](../decisions/0006-apache-2-license.md)).
8. The repository is the single source of truth ([ADR 0005](../decisions/0005-repository-as-source-of-truth.md)).

## Left open

- Model file format, first audience, UI approach, equation parsing, algebraic loops ([open questions](../open-questions.md)).

## Notes for the next session

- Development is AI-assisted; the owner noted the solo-bandwidth risk is reduced by this, and the main replacement risk is unverified AI-written numerics ([verification policy](../guides/verification-policy.md)).
- The owner works on Windows (repository at `D:\SimSandbox` on one machine) and also has WSL; build natively on Windows ([development setup](../guides/development-setup.md)).
- The agent sessions so far could not push to GitHub directly; changes were delivered for the owner to push.
- The original Claude doc and the Claude Project overview are now mirrors; do not update them instead of the repository.
