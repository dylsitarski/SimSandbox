# simsandbox (Python)

Describe, check, build, run and render SimSandbox models from Python. Usage, formats and the CLI equivalents: [docs/guides/headless.md](../docs/guides/headless.md). Design: [ADR 0041](../docs/decisions/0041-headless-interface.md).

- `src/lib.rs`: the extension module `simsandbox._native` (PyO3), a thin layer over `crates/simsandbox-session`.
- `simsandbox/__init__.py`: the Python API (`Model`, `Results`, `describe`, `check`, `schema`, `examples`).
- `tests/`: pytest tests.

This directory is its own Cargo workspace (excluded from the root one) because an extension module links against Python only when Python loads it. Checks:

```sh
cd python
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
maturin develop               # or: pip install .
python -m pytest tests
```
