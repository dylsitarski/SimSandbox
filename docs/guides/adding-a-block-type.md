# Adding a block type

_Last updated: 2026-10-17_

How to make a model definition loadable from model files ([ADR 0007](../decisions/0007-json-model-files.md)). The equation domain is the worked example.

## Steps

1. **Write the definition.** In the equation domain, implement `OdeSystem` (spec, equations, initial state) in `crates/simsandbox-eq/src/systems.rs` or a new module. Give every parameter a default, a suggested range and a plain-language description ([ADR 0011](../decisions/0011-parameter-entry.md)), and mark parameters that set initial state with `initial(..)` (`ParamSpec::initial_condition`) so the app shows they apply on Restart.
2. **Pick a type name** namespaced by domain, e.g. `eq.van_der_pol`, and add it as a constant in `crates/simsandbox-eq/src/registry.rs`. Type names appear in users' files: never rename one without a format migration.
3. **Register a factory** in `register()` in the same file. The factory receives the block name from the file and a `BlockConfig`: the method settings and the block's definition lines (the `equations` of a typed block; types with built-in equations reject them); anything that varies per model belongs in parameters, not factory arguments, so files can set it.
4. **Test it:** a verification test for the definition per the [verification policy](verification-policy.md), and a file test in `crates/simsandbox-file/tests/files.rs` showing the type loads and runs identically to the same model built in code.
5. **Add an example** in `examples/` if it is a good first model for hobbyists ([ADR 0009](../decisions/0009-hobbyists-first.md)), with a `description` and sensible `controls` ([ADR 0012](../decisions/0012-controls-in-model-files.md)). Example files must equal their own re-save (`examples_are_written_exactly_as_saved`), so write them with `ModelFile::to_json` or copy its formatting.
6. **Document it:** `CHANGELOG.md` under **Unreleased** and `docs/STATUS.md`.

## Other domains

A new domain crate exposes its own `register(&mut Registry)` and the runtimes (`simsandbox-cli`, later `simsandbox-app`) call it when building their registry. Method settings are a name plus numeric options; parse and validate them in the domain (as `Integrator::from_settings` does) and return `SimError::InvalidMethod` with a clear message.
