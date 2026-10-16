# Contributing

Agents: start with [AGENTS.md](AGENTS.md). Everyone: the [documentation map](docs/README.md) explains where things go.

## Workflow

1. Pick a task from **Next up** in [docs/STATUS.md](docs/STATUS.md), or open an issue.
2. Branch from `main`: `feat/<topic>`, `fix/<topic>`, `docs/<topic>` or `chore/<topic>`.
3. Commit in small steps. Messages: an imperative summary line (≤ 72 characters), a blank line, then what and why.
4. Open a pull request using the template; CI must be green.

## Definition of done

- [ ] Code formatted, clippy clean, tests pass on all platforms (CI)
- [ ] Numerical code meets the [verification policy](docs/guides/verification-policy.md)
- [ ] `docs/STATUS.md` updated; roadmap boxes ticked if a roadmap item finished
- [ ] `CHANGELOG.md` updated for user-visible changes
- [ ] Affected architecture or design docs updated; ADR written for any significant decision
- [ ] `python3 scripts/check_docs.py` passes

## Licensing

By contributing you agree that your contributions are licensed under the [Apache License 2.0](LICENSE).
