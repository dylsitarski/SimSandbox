# Changelog

All notable user-visible changes are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses [Semantic Versioning](https://semver.org/) once it reaches 0.1.0.

## [Unreleased]

### Added

- Documentation system: `AGENTS.md` entry point and session protocol, documentation map, status, vision, roadmap, architecture, design, guides, research, open questions, risks, glossary, session notes, ADRs 0005–0006.
- Docs check (`scripts/check_docs.py`) in CI; pull request and issue templates.

## [0.0.1] — 2026-10-15

### Added

- Rust workspace with `simsandbox-core`, `simsandbox-eq` and `simsandbox-cli`; CI on Windows, macOS and Linux.
- Block contract with capability traits; SI units with conversion; model graph; fixed-step co-simulation master; snapshots and rewind.
- Equation domain: Euler, RK4, Dormand–Prince 5(4); fusion of equation Blocks; Lorenz and coupled-oscillator systems.
- `simsandbox lorenz` command writing CSV.
- ADRs 0001–0004; Apache-2.0 license.
