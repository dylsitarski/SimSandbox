# 0098 — The kernel holds the contract: what a block is is data, what some blocks do is a capability

- **Status:** Accepted (owner, 2026-11-17: "Begin the kernel tidy", the review's work item)
- **Date:** 2026-11-17
- **Builds on:** [ADR 0002](0002-layered-block-contract.md) (the layered contract); [review](../research/2026-11-14-code-review.md) section 1, "the kernel is growing sideways"

## Context

The kernel (`simsandbox-core`) is to hold only what connects, schedules and steps blocks (AGENTS.md: no physics; keep `Block` small, new behaviour in optional capability traits). By 2026-11-14 it also held study designs (`sampling.rs`), a concrete block (`hold.rs`, a probe feeding its model) and probe schedules (`schedule.rs`), none used by the kernel itself; and `Block` had grown to 14 methods, three of them for particular domains: `instants()` (a data block's sample times), `reads_time()` (whether a block reads `t`, ADR 0083) and `code()` (how a typed-equations block runs, ADR 0089). The smallest block took about 50 lines, most of them filling a `BlockSpec` field by field, which a plugin interface (Phase 2 item 4) would ask of every author.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Leave the kernel as it is | No work | The contract keeps taking in what one domain needs; a plugin author meets it all |
| **Move what the kernel does not use to the crate that does; a block's fixed properties as `BlockSpec` data; what changes while a block runs as a capability; a builder for `BlockSpec`** | `Block` shrinks; one rule says where a new need goes; the smallest block is short | Paths change for code in the workspace (none outside it yet) |
| Every domain need as a capability trait | One mechanism | A capability per fixed fact adds an accessor to `Block` each time, which is what was to be avoided |

## Decision

1. **Only what the kernel uses is in it.** `sampling`, `schedule` and `hold` are `simsandbox_file::{sampling, schedule, hold}`: the model file plans studies, reads probe schedules and builds the hold block; no dependency edge was added.
2. **What a block is, fixed once it is built, is data on its `BlockSpec`:** `reads_time: Option<bool>` and `instants: Option<Vec<f64>>`.
3. **What a domain needs of some blocks while they run is a capability**, reached by an `as_*` accessor as `Equations` and the others are: `GeneratedCode` (`code()`, which changes as native code is built), through `Block::as_generated_code()`.
4. **`BlockSpec` has a builder** (`BlockSpec::new(name, domain)`, `.input`, `.output`, `.port`, `.param`, `.state`, `.table`, `.time`), with `ParamSpec::new` and `MethodSpec::new`. The smallest block, about 25 lines, is the doc-tested example on `Block`.

## Consequences

- `Block` has 12 methods (5 required), from 14; `simsandbox-core/src` 5 892 lines, from 6 294. Results the same bits; every example the same bytes.
- New needs follow the rule above. Left in the kernel and to look at when a plugin interface is designed: the `Domain` enum lists domains nothing implements (`RigidBody`, `Fields`, `Ml`); `BlockConfig` carries the agents' `update` order and typed equations' `definition`; the registry's thread-local folder (`in_folder`, `current_folder`), which typed equations read for start files; `PortSpec::mesh` (how mesh cells are drawn, a views concern); activation (`graph.rs` `Activation`, ADR 0088), which is scheduling and stays.
- [Block contract](../architecture/block-contract.md) and [overview](../architecture/overview.md) updated.

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none.
