# simsandbox (Python)

Describe, check, build, run and render SimSandbox models from Python. Usage, formats and the CLI equivalents: [docs/guides/headless.md](../docs/guides/headless.md). Design: [ADR 0041](../docs/decisions/0041-headless-interface.md).

- `src/lib.rs`: the extension module `simsandbox._native` (PyO3), a thin layer over `crates/simsandbox-session`.
- `simsandbox/__init__.py`: the Python API (`Model`, `Results`, `describe`, `check`, `schema`, `examples`), annotated to pass `mypy --strict`.
- `simsandbox/_native.pyi`: type stubs for the extension module, and `simsandbox/py.typed`, which tells type checkers the package carries its types. Change the stubs with `src/lib.rs`: `tests/test_stubs.py` compares their names and signatures (argument names and defaults) with the built module, runs mypy's `stubtest` on them, and type-checks `tests/typing_usage.py` with `mypy --strict` (these skip if mypy is not installed).
- `tests/`: pytest tests.

This directory is its own Cargo workspace (excluded from the root one) because an extension module links against Python only when Python loads it. Checks:

```sh
cd python
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
maturin develop               # or: pip install ".[test]"
python -m pytest tests
```

`Model.connect` checks its two ends when called: a block, used model, port or parameter that does not exist, or an input or parameter that already has a wire, raises `ModelError` at once (judged as the node canvas draws wires, building only the blocks named). `Model.add_block` refuses an unknown type.

## Wheels

One abi3 wheel per platform serves every CPython from 3.10 on. `.github/workflows/wheels.yml` builds them with maturin for Linux (manylinux 2.28, x86_64), Windows (x64) and macOS (arm64 and x86_64) on pushes to `main`, on `v*` tags and on demand, installs each into a clean Python and runs the tests, and keeps them as workflow artifacts. They are not published to PyPI yet. To build one locally:

```sh
cd python
maturin build --release --out dist   # dist/simsandbox-<version>-cp310-abi3-<platform>.whl
```

The version is the crate's (`Cargo.toml`). Wheels are never committed (`*.whl` is ignored).
