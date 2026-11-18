# 0100 — Python wheels, typing and dependency checks

- **Status:** Accepted
- **Date:** 2026-11-18

## Context

The [review](../research/2026-11-14-code-review.md) (section 5, priority 4) found that the Python package needs a Rust toolchain to install (no wheels are built), that type checkers cannot see its native module (no `py.typed`, no stubs), and that it validates lazily: `Model.connect` accepted any text until the model ran. CI had no build cache, did not test the declared minimum Rust (1.97) or Python (3.10), and did not check dependencies' licences or advisories.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Wheels built and tested in CI, kept as artifacts; PyPI later | Installs without Rust; each wheel tested in a clean Python | Four release builds per push to main |
| Publish to PyPI now | Installable with `pip install simsandbox` | The wheel lacks the licence file; the README's links are relative; the API is not yet stable |
| Stubs written by hand, held to `lib.rs` by tests | Exact names, defaults and result shapes (`TypedDict`s) | Kept in step by hand |
| Stubs generated (`pyo3-stub-gen`) | No hand upkeep | A new dependency; result dicts typed only as `dict` |
| Every advisory fails `cargo deny`, with named exceptions | New advisories cannot slip in | Each unmaintained crate we cannot leave needs a written reason |
| Unmaintained crates only warn | Quieter | An unmaintained crate is added unnoticed |

## Decision

1. **Wheels.** CI (`.github/workflows/wheels.yml`) builds one abi3 wheel per platform (cp310-abi3: manylinux 2_28 x86_64, Windows x64, macOS arm64 and x86_64) with maturin, on pushes to main, `v*` tags and on demand, not on pull requests. Each wheel is installed from the artifacts alone into a clean Python (3.10 and 3.13 on Linux, 3.12 elsewhere) and tested, and kept as a workflow artifact. Nothing is published to PyPI until a separate decision (trusted publishing from tags), which must also bundle the licence and give the README absolute links.
2. **Typing.** The package ships `py.typed` and hand-written stubs for `simsandbox._native`, held to `python/src/lib.rs` by an introspection test, mypy's `stubtest` and `mypy --strict` over a usage file (`python/tests/test_stubs.py`).
3. **Validation at the call.** Builder calls check what is cheap where they are made: `Model.connect` judges a wire as the node canvas does, with only the one or two blocks it names (`connection_problem`), so blocks are added before they are wired; `Model.add_block` refuses an unknown type. Whole-model errors stay with `check`.
4. **CI.** A build cache for every Rust job; a job checking the workspace with the minimum Rust (`rust-version`); Python 3.10 as well as 3.12; `cargo deny` over both workspaces (`deny.toml`): licences allowed by name, sources from crates.io only, path wildcards denied (our crates are `publish = false`), duplicate versions warned. Every advisory fails, unmaintained ones included, unless `deny.toml` ignores it with a reason. On 2026-11-18 four are ignored: `paste` (through faer and egui_dock) and `zvariant`, `zvariant_derive`, `zbus_names` (through eframe's accessibility support), all unmaintained, none vulnerable, none with a maintained release to move to.

## Consequences

- The package installs without Rust from a CI artifact; PyPI publishing is a later decision.
- A change to `lib.rs`'s functions fails the stub tests until the stubs follow.
- Python code that wired blocks before adding them now gets `ModelError` at `connect`.
- The ignored advisories are revisited when faer, egui_dock or eframe move off those crates.
- Licences in the dependency tree (2026-11-18): 0BSD, Apache-2.0 (with the LLVM exception), BSD-2/3-Clause, BSL-1.0, ISC, MIT, Unicode-3.0, Unlicense, Zlib, and the font licences OFL-1.1 and Ubuntu-font-1.0 for egui's embedded fonts (their texts must ship with the app). No copyleft licence applies: `self_cell` and `r-efi` offer GPL/LGPL only as an alternative.
- Documents updated: [python/README.md](../../python/README.md), [development setup](../guides/development-setup.md).

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none.
