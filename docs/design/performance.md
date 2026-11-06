# Performance

Where run time goes, how it was measured, and what is left to gain. Each optimisation keeps results bit for bit (every example and the profiling cases give the same output as before), and none trades one kind of simulation's speed for another's.

## How to measure

- **Cases:** `python3 scripts/profile_cases.py DIR` writes eight heavier versions of the examples, one per kind of simulation: a large grid (Gray–Scott, 128 × 128, typed equations), two such grids on their own clocks, a small ODE (Lorenz, 200 000 steps), a fused group (coupled oscillators), many blocks on one clock (a platoon of 40 cars), linked models on their own clocks, agents with a field (grazing), and a study of a model in time (cruise tuning).
- **Timing:** `cargo run --release -p simsandbox-session --example profile_run -- FILE [T_END] [--signals A,B] [--every N]` runs a file headless, as `simsandbox run` does, without writing results. This machine (a 4-core virtual machine) varies by about 10% between runs: take the median of three, and compare against the previous commit built the same way.
- **Profiling:** `valgrind --tool=callgrind` on the same program, built with `debug = true` in the release profile, at a shortened `T_END`; `callgrind_annotate` for the functions with the most instructions of their own, and `--tree=caller` for who calls them. Instruction counts miss time spent in the system (reading files, page faults): confirm with timings.
- **Exactness:** run every example and case with `simsandbox run --format json` before and after, and compare.

## Profile of 2026-11-05 and what was done

Share of instructions in each case, before the changes:

| Finding | Cases | Share | Done |
|---|---|---|---|
| A fused group's right-hand side allocated each member's inputs, outputs and parameters at every evaluation | fused group, many blocks, study | about 50%, 30%, 25% | Kept in arrays per group (`GroupScratch`); parameters copied only when some are driven |
| Integrators allocated their stage arrays at every step | small ODE; all with RK4 | about 50% | Kept in the integrator's `Workspace` between steps, not zeroed (a right-hand side writes all of `dx`, now part of the `Equations::rhs` contract) |
| The number of cores was asked of the system at every instant (`available_parallelism`, which reads control-group files on Linux) | linked models | about 16% of instructions, most of the time | Asked once |
| Values passed between blocks one by one, times a unit factor of 1 | agents | about 10% | Copied as a block when the factor is 1 (the same bits) |
| Headless results grew their arrays as rows came | grid, agents | a few % | Room reserved when the number of rows is known |

Timings, median of three, before and after (seconds):

| Case | Before | After |
|---|---|---|
| Large grid | 3.12 | 2.49 |
| Two grids | 0.51 | 0.42 |
| Small ODE | 0.031 | 0.017 |
| Fused group | 1.65 | 0.61 |
| Many blocks | 1.03 | 0.67 |
| Linked models | 2.41 | 0.27 |
| Agents | 0.175 | 0.164 |
| Study | 0.018 | 0.012 |

## One model on several cores (2026-11-06)

[ADR 0062](../decisions/0062-probes-recording-and-replay.md) step 3: a program over large arrays runs in phases, each split into chunks of cells, one per core, with the same bits in any split (the ADR's As built says how).

- **First designs were slower.** Splitting each array operation on its own gave about eight parallel sections per evaluation; starting threads costs about 100 µs here (`std::thread::scope`, 3 threads), more than most sections took. The phases now take whole runs of operations (stencils included), scalar temporaries lie apart from arrays so their instructions can run before a phase, and arrays read cell by cell are split with those written: the Gray–Scott right-hand side is one phase.
- **Sharing memory costs too.** With one section per evaluation, a 128 × 128 grid's phases (about 50 µs each on one core) took about 95 µs on four, with plain threads and with rayon alike: the cells move between the cores' caches. So phases below 2^20 cell operations stay on one core, and larger ones measure both ways and keep the faster. No new dependency: rayon was no faster than plain threads once sections were large.

Gray–Scott with typed equations, medians of five, this machine (seconds):

| Grid | Before | Phases on one core | Now |
|---|---|---|---|
| 128 × 128, t_end 600 | 0.98 | 1.02 | 1.02 |
| 256 × 256, t_end 100 | 0.86 | 0.88 | 0.87 |
| 512 × 512, t_end 40 | 2.41 | 1.80 | 1.42 |

The 128 × 128 and 256 × 256 rows are equal within this machine's noise: no phase there is large enough to share, and the program runs as before. At 512 × 512 the phases are faster even on one core, since a phase runs all its operations on a chunk while it is in the cache.

## States and rates in place (2026-11-06)

Typed equations copied the states into the program's registers and the rates out at every evaluation, and the states again for the outputs. Timed (not counted: callgrind counts `memcpy` at about 25% of a grid's instructions, which overstates its time), the copies were about 7% of the 128 × 128 grid's run and 14% of the 512 × 512 one, where the rest runs on four cores. Now arrays that only phases of cells read or write stay where the integrator keeps them ([ADR 0060](../decisions/0060-arrays-in-typed-equations.md) As built). Small models keep the old path unchanged: a first version that went through the new bookkeeping on every call made the 40-car platoon 15% slower.

Medians of seven paired runs (eleven for 128 × 128), seconds:

| Case | Before | After |
|---|---|---|
| Grid 128 × 128, t_end 600 | 1.12 | 0.96 (faster in 11 of 11) |
| Grid 256 × 256, t_end 100 | 0.95 | 0.77 |
| Grid 512 × 512, t_end 40 | 1.68 | 1.44 |
| Two grids | 0.42 | 0.38 |
| Many blocks, fused group, small ODE, agents, linked, study | | unchanged within noise |

The machine was slower this day than for the table above (the same grid's "before" here is the "now" there); compare within a table.

## Keeping what is asked for (2026-11-06)

Headless runs kept every output at every step ([ADR 0076](../decisions/0076-headless-runs-keep-what-is-asked.md)); they still do by default, but now keep only the outputs and steps asked for (`--signals`, `--every`, `--model`). Medians of five, with the largest memory held:

| Case | Kept | Time (s) | Memory (MB) |
|---|---|---|---|
| Grid 128 × 128, t_end 600 | everything | 1.04 | 392 |
| | `gs.v_mean` | 0.80 | 17 |
| | `gs.u` | 0.88 | 92 |
| | every 10th step | 0.81 | 55 |
| Two grids, t_end 200 | everything | 0.39 | 279 |
| | one model (`a`) | 0.37 | 154 |
| | `a/gs.v_mean,b/gs.v_mean` | 0.29 | 28 |
| | every 10th step | 0.31 | 54 |

## The app's engine on long-lived threads (2026-11-06)

Several models on their own clocks in one simulator (the app's engine, and headless runs with inputs, probes or run files) met at every instant on threads started for that instant. On mid-size models this was slower on four cores than on one: starting threads cost more than the step. The simulator now keeps a pool of threads for the run and lends each clock's blocks to it, owned, at each instant worth sharing ([ADR 0066](../decisions/0066-models-nest-as-blocks.md) As built). Idle threads check for work for 50 µs before sleeping.

Two unlinked Gray–Scott models on their own clocks in one simulator (`profile_run --merged --signals a`), medians of five, seconds:

| Grid per model | Before | After | One core | Apart (headless, a thread per model) |
|---|---|---|---|---|
| 128 × 128, 200 steps | 0.46 | 0.43 | 0.64 | 0.29 |
| 48 × 48, 3000 steps | 1.31 | 0.87 | 1.44 | 0.60 |
| 24 × 24, 20 000 steps | 1.78 | 1.11 | 1.78 | 0.73 |

Single-clock cases (grid, platoon, small ODE, fused group) are unchanged: they never share an instant. The rest of the gap to "apart" is the meeting itself: outputs, links and recording after each instant run on one thread.

## Models linked one way run ahead (2026-11-06)

Headless, linked models met at every instant they shared, even where the links run one way. Now a model that only feeds others runs ahead of them on a thread of its own, passing its samples on in batches ([ADR 0066](../decisions/0066-models-nest-as-blocks.md) As built). Medians of five or seven, keeping one output:

| Case | Before | After |
|---|---|---|
| A driver feeding two springs (linked case) | 0.30 | 0.18 |
| A chain of five models (driver, spring, three followers) | 0.61 | 0.24 |
| Two unlinked grids, fused group, small ODE, study | | unchanged |

A first version passing samples on at every instant made the linked case four times slower: waking the receiving thread for each sample costs more than a step of these small models. Passing them on every 64 instants as one array, and removing per-instant allocations in the instant loop, gave the gains above.

## Sums in fixed chunks (2026-11-06)

[ADR 0077](../decisions/0077-sums-in-fixed-chunks.md). Sums alone, one after another against the fixed rule (four lanes per chunk of 4096 cells): 2^14 cells 11.5 → 2.9 µs, 2^18 187 → 49 µs, 2^22 3.4 → 0.8 ms; from 2^20 cells, chunks on several cores 1.5–2.5× faster than four lanes on one. In a model the sums were a small share: a 512 × 512 Gray–Scott grid with `mean(u)` and `mean(v)` in its rates gained little from the faster sum itself (1.35 → 1.34 s); what cost time was each mean splitting the phase of cell-by-cell work in two. Running such reductions before the phase took it to 1.19 s (the grid without means: 0.97 s).

## What is left, by expected gain

1. **Integrators' vector updates** (`x + h·k` in RK4 and the like) run on one core over the whole state: for large grids a growing share now that evaluation is split and copies are gone.
2. **Models feeding each other headless meet on new threads**: within a part of models that feed each other, the headless runner still starts threads at each instant they share (`parallel::each` in `compose::point`); the pool would serve there too.
3. **Models linked one way in the app**: the app's engine is one simulator; running a feeding model ahead there would mean recording and rewinding models at different times.
