# Performance

Where run time goes, how it was measured, and what is left to gain. Each optimisation keeps results bit for bit (every example and the profiling cases give the same output as before), and none trades one kind of simulation's speed for another's.

## How to measure

- **Cases:** `python3 scripts/profile_cases.py DIR` writes eight heavier versions of the examples, one per kind of simulation: a large grid (Gray–Scott, 128 × 128, typed equations), two such grids on their own clocks, a small ODE (Lorenz, 200 000 steps), a fused group (coupled oscillators), many blocks on one clock (a platoon of 40 cars), linked models on their own clocks, agents with a field (grazing), and a study of a model in time (cruise tuning).
- **Timing:** `cargo run --release -p simsandbox-session --example profile_run -- FILE [T_END] [--signals A,B] [--every N]` runs a file headless, as `simsandbox run` does, without writing results. This machine (a 4-core virtual machine) varies by about 10% between runs: take the median of three, and compare against the previous commit built the same way.
- **The app's engine:** `cargo run --release -p simsandbox-session --example profile_steps -- FILE [STEPS]` steps a model through a `Session`, as the app's simulation thread does, and prints the time per step. Gray–Scott (2026-11-06): 1.5–2.2 ms a step in a release build, 27 ms in a debug build; the app steps for 7 ms of each 8.3 ms tick, so a debug build plays it at a few tens of steps a second.
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

## Integrators' updates on several cores (2026-11-06)

Euler's and RK4's vector updates (`x + h·k`) split over the cores from 2^20 values (`integrators::SPLIT`); each value is computed by the same expression, so the same bits in any split. They read and write memory more than they compute, so cores help only on long vectors: one update alone, one core against four, was 126 against 125 µs at 2^17 values (starting threads costs about 0.1 ms), 0.53 against 0.35 ms at 2^19, 2.6 against 1.1 ms at 2^21. In models (medians of seven to thirteen):

| Case (state values) | Before | After |
|---|---|---|
| Grid 1024 × 1024 (2^21), t_end 8 | 1.36–1.41 | 1.26 |
| Grid 512 × 512 (2^19) and smaller, small models | | unchanged |

On the 512 × 512 grid the updates took about 125 ms of 1.0 s; split at 2^17 they made 256 × 256 20% slower and 512 × 512 no faster. Putting the split inside the existing loops also made the 128 × 128 grid 3% slower (the compiler treats the loops differently); the split versions are now functions of their own (`rk4_step_split`, `euler_split`), chosen once per step, and smaller states run the original loops. Dormand–Prince's stages are not split (its error estimate is a sum, and its stages read seven arrays: a separate split version was not worth it here); the implicit methods' solves are their own matter.

## Models feeding each other on long-lived threads (2026-11-06)

Headless, models that feed each other meet at every instant they share; between instants they stepped on threads started for that instant (`parallel::each`). They now step, and record, on a pool kept for their part's run, each model moved to a thread and back (`compose::lend`), as the app's engine does with clocks (ADR 0066 As built). Two Gray–Scott models on their own clocks linked both ways (each driving the other's `F`), keeping one output, medians of five:

| Grid per model | Before | After |
|---|---|---|
| 128 × 128, 200 steps | 0.43 | 0.34 |
| 48 × 48, 3000 steps | 1.43 | 0.96 |
| 24 × 24, 20 000 steps | 3.27 | 2.01 |

Parts of one model (the linked and chain cases), unlinked grids and single models are unchanged; every example and case gives the same bytes.

## Not done: one-way links in the app's engine (2026-11-06)

Decided with the owner: the app's engine does not run a model linked one way ahead of the models it feeds, as headless runs do. Measured in one simulator (`profile_run --merged`) against headless parts:

| One-way case | One simulator (the app's engine) | Parts (headless) |
|---|---|---|
| A driver feeding two springs | 0.31–0.33 s | 0.16–0.21 s |
| A chain of five small models | 0.58–0.71 s | 0.22–0.27 s |
| Two 128 × 128 grids, one feeding the other | 0.37–0.40 s | 0.37–0.48 s |
| Two 48 × 48 grids, one feeding the other | 0.84–0.92 s | 0.64–0.70 s |

- **Little to gain in the app:** it plays at a set speed. Through its stepping path (`profile_steps`) the chain manages about 250 000 steps a second and the 48 × 48 pair about 1 600, where both cases need about 100 (the chain at 1× with its 0.01 s step, the grids at their 100× default); a faster engine shows only when the app cannot keep up. Heavy models already step on the pool at each instant: for 128 × 128 grids one simulator is as fast as parts.
- **It conflicts with the app:** a model run ahead would have computed its future before it is shown. A slider, an input driven by hand or a diagram edit acts at the current time, so that future would have to be thrown away and computed again; rewind, probes and views would need each model's history to stop at a different time and be brought back into line on every pause and rewind. A redesign of the engine and its recording, with risk to exact replay, for a gain rarely seen.
- Headless runs, where nothing changes during the run, already have it. If a model cannot keep up in the app, profile that model rather than reopening this.

## Not done: mid-size updates on a pool (2026-11-06)

Decided with the owner. Integrator updates on states of 2^17 to 2^20 values gain from several cores only on threads already running, working on the integrator's vectors where they are. The project's own pool takes only owned work, and without `unsafe` the vectors cannot be lent to it (the state belongs to the calling block); copying chunks to it costs more than the update. A rayon pool can, and was measured (one update, one core against the pool): 2^17 values 182–188 → 106–143 µs, 2^18 352–375 → 210–226 µs, 2^19 0.78–0.93 → 0.31 ms, 2^20 1.6 → 0.46 ms. In models the updates take about 60 ms of a 256 × 256 grid's 0.66 s and 121 ms of a 512 × 512 grid's 0.96 s, so the gain would be about 2–4% and 8%. Not worth a new dependency on its own; if rayon is adopted for another reason, this is a small follow-up.

## The list is closed (2026-11-06)

Every item found by the profiling pass of 2026-11-05 is done or decided against, above. New work starts from a new profile of the model that is slow (`profile_run`, `profile_steps`, callgrind), timed in release builds and compared bit for bit.

## Typed equations against fused code (2026-11-07)

Asked by the owner: are typed equations much slower than compiled code, and what do exact bits cost? `cargo run --release -p simsandbox-eq --example fused_speed` evaluates Gray–Scott's right-hand side once, typed, and as hand-written Rust in one loop over the cells, in several forms. Microseconds per evaluation, one core (`taskset -c 0`), this machine (Xeon at 2.1 GHz); in brackets the largest difference from "fused" against the largest rate:

| Grid | Typed | Fused, same operations | Rewritten (×1/h² once) | FMA only | Rewritten + FMA |
|---|---|---|---|---|---|
| Default build: | | | | | |
| 128 × 128 | 213 | 50 | 30 (9e-15) | 293 (3e-16) | 289 (9e-15) |
| 256 × 256 | 1094 | 210 | 145 (3e-14) | 1188 (3e-16) | 1178 (3e-14) |
| 512 × 512 | 5355 | 842 | 647 (1e-13) | 4745 (3e-16) | 4733 (1e-13) |
| Built for this CPU (`-C target-cpu=native`: AVX2, FMA): | | | | | |
| 128 × 128 | 213 | 47 | 24 (9e-15) | 49 (3e-16) | 25 (9e-15) |
| 256 × 256 | 1126 | 187 | 107 (3e-14) | 196 (3e-16) | 109 (3e-14) |
| 512 × 512 | 5184 | 745 | 499 (1e-13) | 793 (3e-16) | 526 (1e-13) |

On four cores the typed 512 × 512 evaluation takes 3.2–3.5 ms (its phases split); the others are as on one.

- **Typed equations are 4–6× slower than fused code of the same operations on large grids, with the same bits.** The fused kernel gives exactly the typed block's values. The gap is how the program runs: one instruction at a time over whole arrays, each a pass through memory with temporaries between. Fusing (each cell's operations together while it is in the cache) keeps every operation and its order, so it keeps exact bits. This corrects the earlier reading of "near compiled speed" (ADR 0060: 1.0× the built-in heat rod of 50 cells): that holds against the built-in blocks, which are also written array by array, and for grids that fit in the cache, not against fused code on large grids.
- **Exactness costs nothing here.** Fused multiply-add gives no gain (slightly slower; a library call, 6× slower, unless built for a CPU that has it). Building for this CPU's instructions gains 5–20% for fused code and nothing for typed code.
- **A cheaper formula keeps exactness on any machine.** Multiplying by 1/h² once instead of dividing by h² twice is 1.4–2× faster. It changes the last bits once, as ADR 0077's sums did, and is then the same on every machine.
- **What would gain:** evaluating a phase's cells together (an interpreter over cells within chunks, or code compiled in the process such as Cranelift) with the same operations in the same order; stencils multiplying by 1/h². Both keep exact bits ([ADR 0082](../decisions/0082-exactness-is-labelled.md) point 4). Listed in [STATUS](../STATUS.md) Next up.

## Typed equations in blocks of cells (2026-11-13)

Item 3 of STATUS's Next up, first step: the interpreter keeps every operation and its order, and runs faster. Profiling Gray–Scott (`callgrind`) put all of an evaluation in the phases' operations (`cells::run_op`), most of it in stencils: each cell worked out where it lies along its line and went through closures to find its neighbours. Two changes, `crates/simsandbox-expr/src/cells.rs`:
- **Stencils and shifts by runs:** cells inside a line read their neighbours a stride away, in plain loops over three slices; only the two end cells of each line take the edge rules. Same expression, so the same bits.
- **Blocks of cells:** a phase runs all its operations on 2048 cells before the next block, on one core and within each core's share, so the arrays it writes stay in the cache. A program with large arrays now runs by phases on one core too (before, only on several).

Microseconds per evaluation, `fused_speed`, one core (`taskset -c 0`), default build, the same machine as above; typed equations before, then after:

| Grid | Typed before | Typed after | Fused | After against fused |
|---|---|---|---|---|
| 128 × 128 | 162 | 88 | 41 | 2.2× |
| 256 × 256 | 953 | 627 | 161 | 3.9× |
| 512 × 512 | 5050 | 2518 | 661 | 3.8× |

Blocks alone (before the stencil change) gained 5–15%; at 512 × 512 after it, whole arrays took 4280 µs, blocks of 256 cells 3280, of 1024–4096 cells 2600–2670. On four cores 512 × 512 went from 3110 to 2460 µs. Small models (`typed_speed`: Lorenz, heat rod, pendulum, stiff rod) are unchanged: their arrays are below the phases' threshold. All results are the same bits (the benchmark checks typed against fused; the phases' test runs every block size from 1 cell against the instructions in order).

What is left is the interpreter itself: about 64 instructions per cell against fused code's 25, each operation loading and storing its operands where fused code keeps them in registers. Closing it means compiling the program ([ADR 0084](../decisions/0084-where-typed-equations-stop.md) point 6).

## Implicit methods on 2D grids (2026-11-13)

TR-BDF2 on Gray–Scott at 128 × 128 (`grid_cost` on a copy of the example with `"method": {"name": "trbdf2"}`) cost 520 ms a step, 300–330 ms of it in `faer`'s sparse LU with partial pivoting. A better ordering for that LU gained at most 28% (its fill is bounded by AᵀA's structure, which COLAMD already orders for). [ADR 0087](../decisions/0087-implicit-lu-on-the-matrix-pattern.md): our own multifrontal LU on the pattern of A + Aᵀ, pivots on the diagonal, nested dissection: 165 ms a step (factorising 75 ms, solves 5 ms each). `crates/simsandbox-eq/examples/lu_speed.rs` compares the two LUs on a 2D grid, a chain and a ring; 1D systems cost the same as before (`typed_speed`).

## Test builds (2026-11-13)

The checks had become slow: `cargo test --workspace` ran for 77 s after a 187 s build, most of it in our own numerical code unoptimised (`finite_volumes` 34 s, `gray_scott` 14 s, the session's unit tests 14 s). Measured with each configuration built from nothing in its own target folder, on the cloud machine (4 cores, `-j3`), then all tests run, then a rebuild after touching `symbolic.rs`:

| Debug profile | Build | Tests | Rebuild |
| --- | --- | --- | --- |
| Unoptimised (before) | 187 s | 77 s | 32 s |
| Dependencies at `opt-level = 2` | 520 s | 81 s | 39 s |
| Dependencies at 2, our crates at 1 | 644 s | 17 s | 42 s |
| Everything at 1 | 506 s | 18 s | 42 s |
| **Our crates at 1, dependencies at 0 (chosen)** | 274 s | 27 s | 31 s |
| As chosen, with faer and its kernels at 2 | 292 s | 28 s | 31 s |

Optimising dependencies (faer, the UI crates) costs most of the build time and buys nothing in the tests, whose time is in typed equations, integrators and meshes. The chosen profile halves an edit-and-test cycle (109 s → 58 s) and adds 37 s to a run from nothing (CI, which keeps no cache). All 572 tests passed in every configuration; Rust keeps IEEE floating point at every optimisation level, so results are the same bits.

