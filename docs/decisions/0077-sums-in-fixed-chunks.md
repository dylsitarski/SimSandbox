# 0077 — Sums in fixed chunks

- **Status:** Accepted
- **Date:** 2026-11-06

## Context

`sum` and `mean` over an array added its cells one after another on one core ([ADR 0062](0062-probes-recording-and-replay.md) step 3 left them so, to keep earlier results' bits). One long running sum is slow: each addition waits for the one before, so it cannot use vector instructions or several cores. ADR 0062 already decided that exact runs add in a fixed order that does not depend on the machine; this ADR fixes that order. Any new order changes the last bits of sums over large arrays once, compared with earlier runs; from then on results are the same on every machine and any number of cores.

A grid model whose rates read `mean(u)` also ran slower for a second reason: each reduction closed the phase of cell-by-cell work (ADR 0062 step 3), splitting it in two.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| **Fixed chunks of 4096 cells, each in four interleaved lanes, chunks added in order; arrays up to 4096 cells as before** | Same bits on any machine and core count; about 4× faster on one core (vector instructions), more on several; small arrays keep their bits | Sums over more than 4096 cells change once in the last bits |
| A fixed number of chunks (say 64) whatever the size | Simple | Changes every sum, small ones too |
| Chunks per core | Fastest split | Bits depend on the machine: breaks exact replay |
| Keep one after another | No change | Slow; no several cores |

## Decision

- **The rule** (`simsandbox_expr::program::sum`): up to `SUM_CHUNK` = 4096 cells, added one after another, as before. Past it, cut into chunks of 4096 cells from the first; within a chunk, cell `i` goes to lane `i mod 4`, each lane added in order, then `(lane0 + lane1) + (lane2 + lane3)`; the chunks' sums are then added in order. `mean` is that sum divided by the number of cells. The rule depends only on the number of cells.
- **Several cores:** from 2^20 cells the chunks are added on several cores; their sums are the same bits, combined in the same order.
- **Changing the rule** (chunk size, lanes) changes results again; it would need a new ADR.
- **Reductions before a phase:** a reduction or a picked cell whose array the open phase does not write, and whose result no operation in it reads yet, runs before the phase, as scalar instructions already did. The same operations on the same values: no change in bits.
- `minof`, `maxof`, `argminof`, `argmaxof` and `at` are unchanged: their result does not depend on order.

## Consequences

- Sums over more than 4096 cells differ from earlier runs in the last bits: in the examples, Gray–Scott's `v_mean` (16 384 cells) by at most 7 parts in 10^15; every state and every other output is unchanged. Run files recorded before 2026-11-06 of models summing such arrays no longer replay bit for bit.
- Sums alone: about 4× faster on one core (2^14 to 2^22 cells), and 1.5–2.5× faster again on several cores from 2^20 cells. A 512 × 512 Gray–Scott grid whose rates read `mean(u)` and `mean(v)`: 1.37 → 1.19 s, mostly from keeping the phase whole.
- Fixed-chunk sums are usually more accurate than one running sum (rounding error grows more slowly); the tests check one case against an exact integer sum.

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none.
