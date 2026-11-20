# 2026-11-20 — The mechanism block

- **Who:** owner and agent (Claude Code)
- **Goal:** settle the mechanism block's file form and build it ([ADR 0103](../decisions/0103-rigid-body-engine.md) point 1).

## Done

- [ADR 0103](../decisions/0103-rigid-body-engine.md) accepted; [ADR 0105](../decisions/0105-mechanism-file-form.md) (the file form) drafted and accepted.
- `crates/simsandbox-mech`:
  - `tree.rs`: the spike's 2D solver, with `fixed` joints, joint springs, and loads at a body's centre of mass added. Its numbers are separate from its structure, so they are read from parameters at every evaluation.
  - `rows.rs`: the rows, read and checked.
  - `block.rs`: `Mechanism` (an `OdeSystem`), `MechBlock` (which adds the rows outputs) and the factory.
- The model file's `bodies` field: schema, load checks, routed to the block as definition lines. Registered in the session's registry and in the file crate's test registry.
- `docs/reference/mechanisms.md`, given by `describe mech.planar`.
- `examples/cartpole.ssbx.json` ("Cart-pole"), and the CLI tests: it balances, a grab replays bit for bit from a run file, rows refused at their path.
- Python: `add_block(bodies=...)`, checked at the call by `_native.block_problem` (stubs updated).
- Checks: fmt, clippy, workspace tests, the Python tests, the docs check; every other example's hashes unchanged.
- After the owner tried the cart-pole in the app:
  - **The Inspector's table for `bodies`** (`simsandbox-app/src/app/mechanism.rs`). Each change is a new diagram edit, `Edit::SetMechanism`. It renames the ports and parameters a renamed body or joint makes, in wires, parameter values, settings and everything in the file outside the blocks (views, sliders, schedules), and drops wires to ports that go. Tested in the session and the app: renames, numbers, add and remove, a refused change kept as a draft, an input unlisted.
  - **Per-platform hashes** (`simsandbox-cli/tests/platform_hashes.rs`, `tests/platform-hashes.json`): four mechanism cases through the binary, pinned for Linux x86_64. The test build and the release build give the same bits, and a changed parameter was caught.

## Decided

- The owner: [ADR 0103](../decisions/0103-rigid-body-engine.md) and [ADR 0105](../decisions/0105-mechanism-file-form.md) accepted.
- Recorded in ADR 0105's "Built" section:
  - the block is in the equation domain, so it fuses with typed equations;
  - `bodies` reaches the block as definition lines.

## Left open

- Before the gate ([STATUS](../STATUS.md), Next up item 3): Windows' and macOS' pinned hashes. CI runs only on `main` and pull requests, and the test fails there until their lines are added (the failure prints them). Intel macOS is not in CI.

## Notes for the next session

- **The table's renames:** a name belongs to the body or joint whose name it starts with (the longest match, then `_`), so the rows' names, not a second list of suffixes, decide what follows a rename. If the new rows do not build, nothing is dropped: the edit waits as a draft.
- **Convergence order:** on the double pendulum, RK4's order measured at 4 ms and 2 ms steps is 3.79 (not yet asymptotic); at 2 ms and 1 ms it is 4.0. The test uses the latter.
- **Unwired inputs:** an unwired input is an error, not 0. That is why ADR 0105 creates inputs only for listed joints and bodies.
- **The cart-pole's grab:** the spring pulls the pole's centre of mass (a load). The spike pulled its tip.
- **`StepIo`:** it has no reborrow. `MechBlock` destructures it to pass it to the inner `OdeBlock` and then write its rows.
