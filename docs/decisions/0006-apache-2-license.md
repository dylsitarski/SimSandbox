# 0006 — Apache-2.0 license

- **Status:** Accepted
- **Date:** 2026-10-15

## Context

The core is meant to be open source to attract backend contributors and to be usable in research. The options were permissive (Apache-2.0, MIT, or the Rust-customary MIT OR Apache-2.0 dual license) or copyleft (GPL-3.0).

## Decision

License the repository under Apache-2.0 (`LICENSE`; `license = "Apache-2.0"` in the workspace manifest).

## Consequences

- Permissive use, including commercial and research, with an explicit patent grant.
- Contributions are accepted under the same license.
- Wrapped solvers keep their own licenses; plugins that link copyleft libraries (e.g. some CFD codes) should run out of process and document their license separately.
