# 0041 — Headless interface: commands, rendering and the Python package

- **Status:** Accepted
- **Date:** 2026-10-22

## Context

[ADR 0039](0039-ai-agent-access.md) requires every feature to be discoverable, checkable, runnable and renderable without the window, through the CLI and a Python package ([ADR 0038](0038-phase-1-revised.md) item 5). Building it raised four questions: where the shared code lives, how views are drawn with no window or GPU, what the machine-readable formats are, and how the Python extension is built alongside the Rust workspace.

## Options considered

| Question | Options | Chosen because |
| --- | --- | --- |
| Shared code | Each interface its own; **one crate all interfaces use** | `simsandbox-session` already held the open model; adding `describe`, `run`, `render` and `views` there keeps the app, CLI and Python identical |
| Rendering | wgpu offscreen (needs a GPU or software driver); a second plotting library (views drawn twice); **egui's own meshes rasterised on the CPU** | The image is what the app shows, from the same view code, with no GPU, display or new drawing code; about 250 lines |
| Formats | Ad hoc text; **JSON for reports, CSV or JSON for runs, PNG for views** | Every language and agent reads them; the model file stays the one input |
| Python build | A workspace member (`cargo test --workspace` would need libpython); **its own Cargo workspace in `python/`, built by maturin** | The extension links Python only when loaded; the main checks stay Python-free; a CI job builds the wheel and runs pytest |

## Decision

- **Shared crate.** `simsandbox-session` provides `describe` (catalogue, block and view types, model checks), `run` (a model file run headless, every output collected), `views` (tiles opened from `controls.views`, moved from the app) and `render` with `raster` (views to an RGBA image and PNG). The CLI, the Python package and the app call these; none reimplements them.
- **Block types describe themselves.** The registry holds a `TypeInfo` per type (summary, methods and their options, settings, an example definition); `describe` builds an example of each type to report its ports, units, parameters, states, tables and method labels, so the catalogue cannot drift from the code.
- **CLI.** `simsandbox describe [TYPE]`, `schema`, `check FILE` (JSON; exit status 1 if the model cannot run), `run FILE --format csv|json` (JSON includes populations; non-finite numbers become `null`) and `render FILE --out IMAGE.png [--t-end] [--view N] [--width] [--height] [--scale] [--light]`.
- **Rendering.** egui runs a few frames headless with its default fonts; its tessellated triangles are rasterised on the CPU with premultiplied-alpha blending in gamma space, as egui's GPU painters do. Tiles are laid out as in the app, with titles and without controls.
- **Python package `simsandbox`** in `python/`: a PyO3 extension (`simsandbox._native`, stable ABI, CPython 3.10 and later) under a pure-Python layer. A `Model` is the model file as a dict, with helpers to add blocks, connect, set parameters, sweep and set run settings, so every edit is a file edit. `run` returns numpy arrays (scalars `(rows,)`, vectors `(rows, n)`, grids `(rows, ny, nx)`, points `(rows, count, dim)`, populations a list of `(n, dim)` arrays); `render` returns PNG bytes. Problems raise `simsandbox.ModelError` with every error `check` finds. Long calls release the GIL.
- **Toolchain.** The CLI now needs Rust 1.95, like the views, because `render` uses egui. The Python package is checked by its own CI job (fmt, clippy, pip install, pytest) on Windows, macOS and Linux.

## Consequences

- An agent can list types, write a file, check it, run it and look at its views from a shell or Python, with the same results as the app ([guide](../guides/headless.md)).
- New block types must register a `TypeInfo` (`describe` tests fail otherwise); new views are rendered headless by the session crate's tests.
- Rendering is single-sample and CPU-bound: fine for figures, not for animation.
- The package is built from source (`pip install ./python`); publishing wheels to PyPI is not decided. An MCP server ([Q29](../open-questions.md)) can wrap these commands.
