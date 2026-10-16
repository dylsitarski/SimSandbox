# AGENTS.md — start here

This file is the entry point for any agent (or new human) working on SimSandbox. The repository is the **single source of truth**: if something about the project is not written down here, it does not exist. Chats, Claude Projects and external docs are scratch space; anything that matters must land in this repository before a session ends.

## 1. Orient (read in this order)

1. [`docs/STATUS.md`](docs/STATUS.md) — what works, what is in progress, the next tasks. **Always read first.**
2. [`docs/vision.md`](docs/vision.md) — what SimSandbox is for and what it is not.
3. [`docs/architecture/overview.md`](docs/architecture/overview.md) — layers, crates and how they fit.
4. [`docs/decisions/README.md`](docs/decisions/README.md) — accepted decisions. Do not contradict one without a new ADR that supersedes it.
5. Whatever the task touches: the [documentation map](docs/README.md) lists every document and what it is for.
6. The latest entry in [`docs/sessions/`](docs/sessions/README.md) — what the previous session did and left open.

## 2. Work

- Pick work from **Next up** in `docs/STATUS.md` unless the user says otherwise.
- The kernel (`crates/simsandbox-core`) contains **no physics**. Solvers and domain logic belong in domain crates.
- Keep the `Block` trait small; new behaviour goes into optional capability traits (see [block contract](docs/architecture/block-contract.md)).
- Numerical code follows the [verification policy](docs/guides/verification-policy.md): every method ships with a test against a known solution, and honest speed/accuracy labels.
- Before running commands, see [development setup](docs/guides/development-setup.md).

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python3 scripts/check_docs.py
```

All four run in CI and must stay green.

## 3. Close the session (required)

Before you stop, update the docs so the next agent can continue without this conversation:

- [ ] `docs/STATUS.md`: move finished items to **Done recently**, update **In progress** and **Next up**, bump `Last updated`.
- [ ] `CHANGELOG.md`: add user-visible changes under **Unreleased**.
- [ ] A decision was made → new ADR in `docs/decisions/` (copy `0000-template.md`) and a row in its index.
- [ ] A question was raised but not decided → `docs/open-questions.md`.
- [ ] Architecture or behaviour changed → update the matching doc in `docs/architecture/` or `docs/design/`.
- [ ] Add a session note in `docs/sessions/` (copy `_template.md`) for any non-trivial session.
- [ ] New document → add it to the map in `docs/README.md` (the docs check fails otherwise).

## 4. Rules of thumb

- Write facts and decisions, not narration. Link instead of duplicating: each fact lives in exactly one document.
- Use dates in ISO format (2026-10-15). Never write "today" or "recently" without a date.
- If the user decides something in chat, record it here in the same session.
- If documentation and code disagree, the code is the truth for behaviour; fix the documentation in the same change.
