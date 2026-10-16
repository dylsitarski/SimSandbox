# 0007 — JSON model files with a block-type registry

- **Status:** Accepted
- **Date:** 2026-10-16

## Context

Models exist only as Rust code ([open question Q1](../open-questions.md#resolved)). Saving, loading, sharing, the CLI and the desktop app all need a file format. Requirements: plain text, diffable, easy for people and AI tools to write, and loadable without recompiling. Blocks are trait objects with no type identity, so a file also needs a way to name a Block type and rebuild it.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| JSON with a schema | Validated by JSON Schema; editor completion; AI tools generate it reliably; `serde_json` is the standard Rust crate | No comments; verbose by hand |
| TOML | Comments; pleasant for flat config | Lists of nested tables (blocks, connections) read awkwardly; weaker schema tooling |
| Small custom DSL | Most readable; could hold equations directly | Parser, error messages, editor support and AI familiarity all to build ourselves |

## Decision

- Model files are **JSON** (`.ssbx.json`), pretty-printed with a stable key order so diffs stay small. A JSON Schema for the format is published in the repository alongside the code that reads it.
- Top level: `format` (integer version, starts at 1), optional `description`, `blocks`, `connections`.
- Each block: `name` (unique in the file), `type` (registered type name, namespaced by domain, e.g. `eq.lorenz`), `method` (`{"name": "rk4", ...settings}`), and `params` (name → value in the parameter's declared unit; omitted parameters take their defaults).
- Each connection: `{"from": "block.port", "to": "block.port"}`. Units are checked and converted on load exactly as in `Model::connect`.
- Type registry: the kernel defines a serde-free registry mapping a type name to a factory that builds a `Box<dyn Block>` from method settings. Domain crates register their types (`simsandbox_eq::register`). `serde` stays out of `simsandbox-core`; reading and writing JSON lives in a separate crate that depends on the kernel and the registry.
- Loading validates everything it can before running (unknown type, unknown parameter, out-of-range value, bad connection) and reports the file location.

## Consequences

- First external dependencies: `serde` and `serde_json`, outside the kernel.
- Comments go in `description` fields; a human-friendlier front end (TOML or a DSL) can be added later as another reader of the same in-memory model.
- Fused equation groups need a file representation too; the first version covers single Blocks and master-level connections, groups follow.
- The `format` version allows migrations; breaking changes bump it.
