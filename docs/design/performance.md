# Performance

Where run time goes, how it was measured, and what is left to gain. Each optimisation keeps results bit for bit (every example and the profiling cases give the same output as before), and none trades one kind of simulation's speed for another's.

## How to measure

- **Cases:** `python3 scripts/profile_cases.py DIR` writes eight heavier versions of the examples, one per kind of simulation: a large grid (Gray–Scott, 128 × 128, typed equations), two such grids on their own clocks, a small ODE (Lorenz, 200 000 steps), a fused group (coupled oscillators), many blocks on one clock (a platoon of 40 cars), linked models on their own clocks, agents with a field (grazing), and a study of a model in time (cruise tuning).
- **Timing:** `cargo run --release -p simsandbox-session --example profile_run -- FILE [T_END]` runs a file headless, as `simsandbox run` does, without writing results. This machine (a 4-core virtual machine) varies by about 10% between runs: take the median of three, and compare against the previous commit built the same way.
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

## What is left, by expected gain

1. **Headless runs keep every output at every step** (131 MB per grid model in the two-grid case): copying and page faults. Keeping only what is asked for (`--model`, named signals, probes) would save memory and time.
3. **The app's engine meets at every instant on new threads** (ADR 0066 step 6): about 10–20% for several heavy models on their own clocks in the app; a redesign of the simulator (each clock's blocks on a thread of their own for the whole run).
4. **Linked models running ahead** where links go one way only (ADR 0066 step 6): removes the meeting at every instant for pipelines; not for feedback loops.
5. **Sums on several cores**: reductions run in order on one core. A fixed-chunk sum is exact on any machine but changes results from earlier runs' last bits; worth it only where reductions are a large share.
