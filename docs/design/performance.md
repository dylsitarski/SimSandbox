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

**Then multiplying by 1/h² (owner, 2026-11-13: changing the last bits once is acceptable).** Each stencil multiplies by 1/h² (second derivative) or 1/(2h) (first), worked out once per evaluation, instead of dividing at every cell; the same in the phases and in the instructions in order, so a model's results are the same whichever path runs it, and the same on any machine. `fused_speed`'s "fused" kernel does the same and still gives the typed block's bits; its "rewritten" kernel now differs only in adding a cell's four neighbours first (one five-point stencil instead of one per axis). One core, µs per evaluation:

| Grid | Typed | Fused | Rewritten (five-point) |
|---|---|---|---|
| 128 × 128 | 73 | 30 | 26 |
| 256 × 256 | 469 | 118 | 101 |
| 512 × 512 | 1965 | 438 | 411 |

What changed in results, against the version before (`run --t-end 30`, every number of the output compared): the models with stencils (the Gray–Scott and typed heat-rod examples, and grid models built for timing, alone, coupled or through probes) changed in about half their values, by at most 6 × 10⁻¹⁶ of the largest value and 6 × 10⁻¹⁵ relative; models without stencils and all 24 example renders are the same bytes. A 512 × 512 grid stepped at 1 s, unstable and NaN within six steps either way, differs in which cells become NaN first.

From the start of the day typed Gray–Scott went from 5050 to 1965 µs at 512 × 512 (2.6×). Fused code gained as much from the multiplication, so typed equations are still 2.4–4.5× from it.

What is left is the interpreter itself: about 64 instructions per cell against fused code's 25, each operation loading and storing its operands where fused code keeps them in registers. Closing it means compiling the program ([ADR 0084](../decisions/0084-where-typed-equations-stop.md) point 6).

## Compiled routes (ADR 0084 point 6, 2026-11-13)

[ADR 0084](../decisions/0084-where-typed-equations-stop.md) point 6's comparison, in `spikes/compiled-routes` (outside the workspace; [README](../../spikes/compiled-routes/README.md)). `simsandbox-expr` gained a hook for it: a phase of cells is described to a compiler (`PhaseView`: its operations, the arrays it writes and reads, its scalars, and which written arrays a later step reads, `kept`), and compiled code (`PhaseKernel`) may run it in place of the interpreter (`Program::compile_phases`, `EquationSet::compile_phases`). A test runs every phase of a program through a kernel written from the view's rules alone, cell by cell, and requires the interpreter's bits.

**Two changes to the interpreter came out of it, the same bits:**
- **Phases merge:** an array a phase only reads may be read whole (by a stencil) and cell by cell (`F*(1 - u)`) in one phase; that had split Gray–Scott's rates into four phases. Now one phase of 18 operations: on one core 1.2× faster at 256 × 256 and 1.3× at 512 × 512 (1537 µs), the same at 128 × 128; on four cores 512 × 512 takes 589 µs, 1.2× single-core fused code.
- **The kept analysis** (`kept`): a written array that is a temporary and that no later step reads need not be stored. The interpreter still stores it (its operations pass arrays to each other); compiled code keeps it in a register.

Microseconds per evaluation, one core (`taskset -c 0`), best of five; for WebAssembly, inside the module in brackets; every route gives the interpreter's bits:

| Grid | 1. Interpreter | 2. Cranelift | 3. Wasmtime, generated | 4. Wasmtime, by hand | 5. Fused Rust |
|---|---|---|---|---|---|
| 128 × 128 | 72 | 56 | 159 (76) | 77 (66) | 28 |
| 256 × 256 | 389 | 233 | 881 (300) | 445 (275) | 114 |
| 512 × 512 | 1529 | 930 | 3654 (1252) | 1807 (1103) | 471 |

| | Compile after an edit (Gray–Scott, all its programs) | Size added (stripped, on 1.40 MB) |
|---|---|---|
| 2. Cranelift | 4–9 ms (15 phases) | 4.95 MB |
| 3. Wasmtime, generated | 10–16 ms | 12.7 MB (Wasmtime with Cranelift) |
| 4. Wasmtime, by hand | 1.1–1.6 ms (one function) | as route 3 |

- **Route 2 is 1.3–1.7× faster than the interpreter, still 2× slower than fused Rust**, with the same bits. Cranelift emits one scalar operation per cell; LLVM vectorises and schedules the fused loop, and the interpreter's own loops over blocks of cells. Getting there took the same care as the interpreter: runs of cells inside every line (no per-cell bookkeeping, neighbours at constant offsets) and temporaries kept in registers (storing them all cost 2×).
- **Route 3 is not close to route 2:** its code alone is 1.3–1.4× slower (Wasm's 32-bit addresses and locals), and copying the arrays into the module's memory and out again at each call makes it 2.8–3.9× slower than route 2 in all, slower than the interpreter. Sharing memory without copying would mean typed equations' registers living inside the module's memory.
- **Route 4, a block written by hand, runs inside WebAssembly at 2.3–2.4× fused Rust's time**; with its state copied in and rates out at each call, 1.1–1.2× the interpreter's. That is the measure for user blocks as WebAssembly plugins ([Q28](../open-questions.md)): fine for blocks whose work is large against their inputs and outputs.
- **By ADR 0084's rule** (route 3 close to route 2, else route 2 for typed equations), typed equations' compiled route is Cranelift in the process. Whether it goes into the build now, for 1.3–1.7× and 5 MB, is the owner's ([Q55](../open-questions.md)).

**Then vector code (2026-11-13, asked by the owner: why can fused Rust not be matched?).** Cranelift does not vectorise by itself; LLVM does, which was most of the gap. Route 2 now emits two-lane code (`f64x2`) for runs of cells inside every line: each lane does exactly the scalar operations, so the bits are the same; powers, comparisons, functions and axes go lane by lane through the scalar code. One core, µs per evaluation (the same run):

| Grid | Interpreter | Cranelift, one lane | Cranelift, two lanes | Fused Rust | Two lanes against fused |
|---|---|---|---|---|---|
| 128 × 128 | 78 | 56 | 39 | 29 | 1.34× |
| 256 × 256 | 481 | 230 | 172 | 131 | 1.31× |
| 512 × 512 | 1339 | 937 | 586 | 434 | 1.35× |

Two lanes are 1.3–1.6× one lane and 2.0–2.8× the interpreter; compile time is unchanged (6–10 ms). A second model using every kind of operation (value, gradient and periodic edges, a first derivative, both shifts, a comparison, a power, a function, an axis) gives the interpreter's bits through both routes, with and without vector code. What remains is LLVM's scheduling and unrolling, and wider vectors: Cranelift's x86 back end has 128-bit vectors only, where fused Rust built for this CPU (`-C target-cpu=native`) uses AVX's four lanes (5–20% more, 2026-11-07). Matching fused Rust exactly needs LLVM: [ADR 0089](../decisions/0089-native-code-for-typed-equations.md) proposes generating Rust and compiling it where a toolchain is installed.

## Compiled typed equations in the build (2026-11-13)

The owner put Cranelift with vector code into the build (Q55). `simsandbox-eq` compiles the phases of every typed-equations program large enough to have them (`simsandbox-compiled`), once its arrays are placed; compiled code is kept by the phase it was compiled from, so a model built again compiles nothing. `fused_speed`, µs per evaluation, the same build:

| Grid | Interpreted (`SIMSANDBOX_INTERPRET=1`) | Compiled | Fused Rust | Compiled, four cores |
|---|---|---|---|---|
| 128 × 128 | 74 | 41 | 29 | 39 |
| 256 × 256 | 416 | 173 | 127–135 | 181 |
| 512 × 512 | 1752 | 741 | 489–510 | 449 |

Compiled is 1.4–1.5× fused Rust on one core (the spike's 1.3× had its kernel's layout; here the rates' copies and the program around them count too). Every example's outputs and renders are the same bytes as before (90 and 24); a test builds Gray–Scott (256 × 256, shared among cores) and a model using every kind of operation both ways and requires the same rates and outputs, with parameters as given and moved.

## Native code (ADR 0089, 2026-11-13)

Rust generated from Gray–Scott's phases, built for this CPU with the installed toolchain (`SIMSANDBOX_NATIVE=build`), µs per evaluation, one core, `fused_speed` (which waits for the library and asserts the fused bits):

| Grid | Native | Fused Rust | Cranelift |
|---|---|---|---|
| 128 × 128 | 24 | 28 | 39 |
| 256 × 256 | 104 | 113 | 162 |
| 512 × 512 | 516 | 471 | 733 |

Native code is fused code's speed (faster on small grids: built for this CPU, AVX; within 1.1× at 512 × 512, where memory bounds both). Building a library took under a second for Gray–Scott.

## Build size (2026-11-14)

The cloud machines filled their disk several times on 2026-11-13. A fresh build of everything `cargo test --workspace` needs was 20 GB: 14.1 GB of test executables (43, each a linked binary with every crate's full debug info), 2.1 GB of libraries, the rest incremental caches, examples and build scripts. Debug info for our crates as line tables only (file and line in panics and backtraces) and none for dependencies (`Cargo.toml`, and `python/Cargo.toml` for the Python package):

| | Full debug info | Line tables, none for dependencies |
|---|---|---|
| Fresh test build | 20 GB, 350 s | 5.7 GB, 220 s |
| Test executables | 14.1 GB | 2.8 GB |
| Rebuild after editing `symbolic.rs` | about 31 s | 14 s |
| `cargo test --workspace` | 27 s | 26 s |
| Python package build and clippy | 2.7 GB, 133 s | 1.5 GB, 112 s |

Also measured and not taken: the Python package sharing the workspace's `target/` saved 0.3 GB, its lock file and features differing; the comparison script's two release builds sharing one folder were taken (0.7 GB instead of 1.4, the second build 51 s instead of 189 s).

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

## Loading models of large grids (2026-11-14)

Found while removing a repeated build from the loading of models that use models ([ADR 0090](../decisions/0090-one-engine-for-nested-models.md)), by callgrind on a 512 × 512 Gray–Scott block (2 states, 524 288 state values):

- **Naming the states was over half of building the block**: `T[0]`, `T[1]`, … for every cell with `format!`, 1.1 of 1.7 thousand million instructions. Written out by hand (`element_name`) the build is 0.57 thousand million, a third.
- **A model using models built each used model three times**; now twice (ADR 0090, as built).
- Together, loading a model that uses four such models: 11.5 → 5.1 thousand million instructions; `simsandbox run` of it for one step 1.71 → 1.12 s. Results the same bytes.
- Still large in a build, not yet looked at: the half million names themselves (0.26), `ArrIns::touches` and `gather_places` (0.23), `Program::reads` (0.15), compiling (0.19, the first time).

## The step loop and recording (2026-11-17)

The [code review of 2026-11-14](../research/2026-11-14-code-review.md) §4 listed, unmeasured, constant-factor waste in the kernel's step loop (`sim.rs`, `loops.rs`, `hold.rs`) and the session's recording. Measured first, on small models where overhead and not arithmetic dominates:

- **Cases:** through a `Session` (`profile_steps`, the app's path): the examples cruise-control, lorenz, coupled-oscillators (filters), pendulum, units, forced-spring (models on their own clocks), platoon, heat-rod, the bridge network (tables), grazing (agents) and cruise-search (a study, held still after its search), and from `scripts/profile_cases.py` many-blocks (40 cars) and linked (a probe feeding two models on their own clocks); linked also with its probe on 2001 listed times. Headless (`profile_run`): small ODE, fused, many-blocks, linked, cruise-control, units.
- **Instructions per step:** callgrind's total for 2N steps less that for N (N from 300 to 4000), over N: loading drops out, and the count is the same from run to run. Linked cases are within about 5% only: the pool's idle threads count instructions while they wait.
- **Wall time:** best of five, the same builds.

Where the instructions went, before (inclusive shares):

- **cruise-control:** the fused group's right-hand side 70%, of which its output plan 38% (`EquationSet::outputs` 31%: the PI block was evaluated once for each of its two feedthrough outputs); `Session::record` 8% (looking every recorded output up by name, `Simulator::output`, 4%).
- **many-blocks:** the group's right-hand side 61%, its output plan 32%; `record` 6%; finding each block's fused group (`group_of` over every group's members) 2%.
- **cruise-search:** `Simulator::step` 77% (the study block evaluated once per output, from its cache), `record` 16%, the snapshot taken before each step and dropped after it 6%.

Changed, each with the same results (every example's and case's output, `run --format json` and renders, compared byte for byte; the session's tests):

| Change | Where | Gain |
|---|---|---|
| A block's feedthrough outputs next to each other in the order share one evaluation, when the block is not wired to itself (within a loop's pass, always) | `loops.rs`, `OutputPlan::new` | the largest: cruise-control −15%, many-blocks −18%, cruise-search −28% |
| Fused group members' numbers of states and outputs kept in the plan, not asked of the spec at each evaluation | `sim.rs`, `GroupPlan` | 2–8% on fused models |
| Recorded signals and fields found by place once (`Simulator::outputs_of`), not by name each step; fields borrowed from the simulator, not copied | `session/catalogue.rs`, `Session::record` | 5–29% through a session |
| A sample-and-hold keeps its trigger instead of cloning its schedule (a whole list of times) every step | `hold.rs` | linked with 2001 listed times −23% |
| Group membership kept per block; clocks borrowed, not cloned, at each instant | `sim.rs`, `step`, `advance` | many-blocks −3%, others ≤1% |
| The snapshot before a step taken only when a study is watched (`watch_studies`), the only case in which it can be given up and the step undone; the app's worker watches, so it still takes it | `Session::step` | cruise-search −11% |
| The output plan's working array, the group's combined state and the recording's kept signals kept between steps | `loops.rs`, `sim.rs`, `recording.rs` | about 1–8% each |

Instructions per step, before → after:

| Case | Before | After | |
|---|---|---|---|
| lorenz | 2 929 | 2 065 | −30% |
| pendulum | 3 838 | 2 539 | −34% |
| cruise-control | 23 791 | 16 915 | −29% |
| units | 24 262 | 19 872 | −18% |
| coupled-oscillators | 29 443 | 25 539 | −13% |
| forced-spring | 21 988 | 19 277 | −12% |
| platoon | 72 032 | 52 759 | −27% |
| heat-rod | 59 368 | 57 791 | −3% |
| bridge | 50 362 | 47 188 | −6% |
| grazing | 627 902 | 570 183 | −9% |
| cruise-search | 54 360 | 26 936 | −50% |
| linked | 61 458 | 54 384 | −12% |
| linked, 2001 listed times | 75 868 | 52 965 | −30% |
| many-blocks | 834 997 | 589 822 | −29% |
| Headless: small ODE | 1 442 | 1 440 | 0 |
| Headless: fused | 24 497 | 21 761 | −11% |
| Headless: cruise-control | 22 826 | 17 155 | −25% |
| Headless: units | 23 267 | 20 494 | −12% |
| Headless: many-blocks | 820 338 | 619 108 | −25% |
| Headless: linked | 56 397 | 53 846 | −5% |

Wall time, best of five (seconds): through a session, cruise-control 200 000 steps 0.402 → 0.272, lorenz 1 000 000 steps 0.243 → 0.168, coupled-oscillators 0.482 → 0.380, units 0.407 → 0.312, forced-spring 0.421 → 0.365, platoon 100 000 steps 0.650 → 0.432, many-blocks 5 000 steps 0.380 → 0.279, cruise-search 100 000 steps 0.689 → 0.371, linked 0.352 → 0.300, linked with listed times 0.409 → 0.302; headless, fused 0.733 → 0.577, many-blocks 0.886 → 0.702, cruise-control 0.358 → 0.270, units 0.378 → 0.330, small ODE 0.021 and study 0.014 → 0.012 unchanged within noise.

Measured and left:

- **Members read in place:** a fused group still gathers its members' blocks, parameters and inputs into three small vectors per step. Reading them where they are, through an enum, cost as much at every access in the right-hand side as the allocations saved (coupled-oscillators +3%, fused +3%).
- **Slots and units per instant** (`step_here`) with several clocks: under 1% (forced-spring).
- **Comparing block types with "study"** each step, and **`state_bits` twice per step** for maps that iterate: under 0.1% and about 1% (cruise-search).
- **The `Vec`s `push_kept` builds for fields:** none is allocated when nothing is kept; the rest is the history's own `push_sample`.
- **Not measured, no case uses them:** an activated part's `clone` per step in `gate` (no example has one), and `run_derived` copying its inputs per sample (only filters with clocks of their own on probes).
- **Out of this pass:** typed equations' own evaluation (`simsandbox-expr`, `EquationSet::outputs` and `rhs`), now most of every small model's step; its allocations per call show in forced-spring (`sin`).

## Typed equations' per-evaluation costs (2026-11-17)

The second part of the [review](../research/2026-11-14-code-review.md) §4: what typed equations and the implicit methods pay at every evaluation besides their arithmetic. Measured as [above](#the-step-loop-and-recording-2026-11-17) (callgrind's instructions for 2N steps less those for N, over N; wall time best of five through a `Session`, `profile_steps`), on cases where these costs show: the examples gray-scott (128 × 128), heat-rod-typed and the bridge network (tables); Gray–Scott at 64 × 64 with a sum along an axis broadcast back into the rates (`sum(v, y)`, `maxof(u, x)`: Gather and Along at every evaluation); TR-BDF2 on the typed and the built-in heat rods and on Gray–Scott at 48 × 48, backward Euler on the bridge; Gray–Scott at 256 × 256 and 512 × 512 (above 2^20 cell operations a phase: shared among the cores, so wall time only).

Changed, each with the same results (every example's and profiling case's output, `run --format json` and renders, and this pass's cases, compared byte for byte; 636 tests):

| Change | Where | Gain |
|---|---|---|
| Gather and Along walk their places with an odometer instead of building a table of them (a `Vec` per cell for Along) at every evaluation; a line along the first axis is reduced in place | `expr/program.rs` | Gray–Scott with a sum along an axis −29% |
| The implicit methods' working arrays (residuals, the Jacobian's columns, the stages, the LU's fronts, updates and factor, a solve's array) kept in their `Workspace`, each set where a fresh one was allocated; the Jacobian's values written straight into the matrix's pattern | `eq/implicit.rs`, `eq/sparse_lu.rs` (`Work`) | heat rods with TR-BDF2 −22% and −24%, Gray–Scott 48 × 48 −7% |
| Take writes in place when its result shares no registers with what it reads (known when compiled) | `expr/program.rs` | bridge −4%, with backward Euler −6% |
| Edges' ends found among ascending ids by binary search, not by a `HashMap` built twice a step; other ids still by the map | `eq/symbolic/evaluate.rs` | bridge −11%, with backward Euler −3% |
| A table block's inputs built in one array reserved at their length | `eq/symbolic/evaluate.rs` | bridge −9% |

Instructions per step, before → after:

| Case | Before | After | |
|---|---|---|---|
| gray-scott 128 × 128 | 3 597 333 | 3 597 389 | 0 |
| Gray–Scott 64 × 64, sum along an axis | 3 069 909 | 2 190 717 | −29% |
| heat-rod-typed | 108 804 | 109 493 | +0.6% |
| bridge | 47 212 | 36 577 | −23% |
| heat-rod-typed, TR-BDF2 | 143 652 | 111 488 | −22% |
| heat-rod, TR-BDF2 | 131 746 | 99 667 | −24% |
| bridge, backward Euler | 174 985 | 144 265 | −18% |
| Gray–Scott 48 × 48, TR-BDF2 | 113 528 549 | 105 550 185 | −7% |

Wall time, best of five (seconds): Gray–Scott with a sum along an axis 3 000 steps 0.778 → 0.593, bridge 200 000 steps 0.782 → 0.593, with backward Euler 100 000 steps 1.465 → 1.150, heat-rod-typed with TR-BDF2 100 000 steps 1.242 → 0.949, heat-rod with TR-BDF2 1.116 → 0.846, Gray–Scott 48 × 48 with TR-BDF2 20 steps 0.896 → 0.887; unchanged within noise: gray-scott 3 000 steps 0.773 → 0.761, 256 × 256 1 000 steps 1.587 → 1.571, 512 × 512 200 steps 1.343 → 1.296. The previous pass's cases are as they were, within 1% (heat-rod-typed's +0.6% and the typed-equation cases' +0.3–0.9% are in the interpreter's `Program::one`, compiled a little differently: about 17 instructions more per evaluation).

Measured and left:

- **A pool for phases of cells.** Above 2^20 cell operations a phase starts its threads at every evaluation (`std::thread::scope`): three threads started and joined cost about 95 µs alone on this machine. The project's own `Pool` takes only owned work, and without `unsafe` (forbidden in the workspace) borrowed arrays cannot be lent to long-lived threads, so only a dependency (rayon) could keep them. Tried with `rayon_core::scope` in `parallel::each` and measured, then taken out: Gray–Scott per evaluation (`fused_speed`, four cores) 512 × 512 377–434 µs against 360–432 without, 256 × 256 161–196 against 195–209; through a session 512 × 512 1.28 s against 1.20–1.34 s, 256 × 256 1.60 against 1.58. The calling thread works on its own share while the others start, which hides most of their start. The same finding as 2026-11-06 (rayon no faster for phases; the owner then chose no new dependency), so none was added.
- **Phases' setup per run** (`Phase::run_in`'s operand list and splits, "about 8 vectors per run" in the review): under 1% of a step at 64 × 64 and 128 × 128 (gray-scott: 25 thousand of 3.6 million instructions).
- **A sparsity hook for fused groups.** A fused group's structure is probed once per run (n + 1 evaluations of the group, kept in the `Workspace`), not per step: two coupled 40 × 40 typed grids with TR-BDF2 (3 200 states) take 0.24 s for the first step against 0.05–0.06 s for the next; the difference is the probe and building the LU's structure once (`sparse_lu::Symbolic::new`: 0.6 of 6.2 thousand million instructions over five steps). The steady cost there is the Jacobian's colouring, which the coupling through `mean` makes dense: a hook giving the exact structure would find the same.
- **`FusedSystem::rhs`** (`EquationGroup`, built in code) allocated per call; no model file reached it (only a test built one). Deleted with `EquationGroup` on 2026-11-18; the master's fused-group right-hand side keeps its arrays since the [previous pass](#the-step-loop-and-recording-2026-11-17).
- **Still per evaluation, not taken:** the LU's fronts zeroed for assembly (10% of Gray–Scott 48 × 48 with TR-BDF2; they must start at zero); `faer`'s pivoting LU, used when the LU on the pattern refuses a pivot (the bridge with backward Euler), allocating inside `faer`; a table block's states and inputs gathered into fresh arrays twice a step (the rest of the bridge's allocations, about a fifth of its step); `EquationSet::outputs` copying the states into the outputs (15% of gray-scott).

## The views and the app's frames (2026-11-17)

The third part of the [review](../research/2026-11-14-code-review.md) §4: what the views and the app pay at every frame. Measured first:

- **Frames:** `cargo run --release -p simsandbox-session --example view_frames -- FILE --t-end T [--view K] [--steps S]` runs a model to `T`, then draws its views frame after frame as the app does (egui's frame and its tessellation, without a GPU) and prints the time, the vertices and the texture bytes uploaded per frame; `--steps S` steps the model between frames (running), else it is still (paused). Instructions per frame: callgrind's total for 8 frames less that for 4, over 4. Wall time: the best of three runs of 20 frames, 1200 × 800 points.
- **Cases:** recordings at the history's limit (99 901 samples of Lorenz, predator-prey and pendulum-drag, `--t-end 999` or `499`), each view alone: Lorenz's three signals against time (`plot.xy`) and its `plot.3d`, predator-prey against time and its phase portrait coloured by time, pendulum-drag against time and the bob's path; Lorenz to t = 5 (500 samples); heat maps of gray-scott (128 × 128), Gray–Scott at 512 × 512 and heat-rod.
- **The app's worker:** the time `Model::of` (what the worker publishes after each batch of commands) takes, in a test build, for every example and profiling case.
- **Pictures:** `simsandbox render` of the cases before and after, compared pixel by pixel; screenshots of the app under Xvfb.

Before, a `plot.xy` at the history's limit took 40–80 ms a frame: `paired` built a `Vec` per sample (half the instructions), egui_plot transformed every sample and egui tessellated a line through each (1.2 million vertices for Lorenz). A paused heat map coloured its field and uploaded it every frame (64 KB at 128 × 128, 1 MB at 512 × 512). `Model::of` takes 3–70 µs per example and 0.7 ms for many-blocks (40 cars), in a test build (opt-level 1, dependencies unoptimised).

Changed, each with the same simulation results (no change reaches a simulation):

| Change | Where | Gain |
|---|---|---|
| One recording's signals paired in place, several with one array of rows; a coloured line's scale found once, not once per segment | `history.rs` (`paired`), `colour.rs` (`segments`) | Lorenz against time −47%, its `plot.3d` −87%, phase portrait coloured by time −76% |
| `plot.xy` reads its samples where the history keeps them; a curve with more samples than twice the plot's width in pixels is thinned: of consecutive points in one column (four to a pixel) the first, last, lowest, highest, leftmost and rightmost are kept, in order (`decimate::Columns`) | `builtin.rs`, `decimate.rs` | against time −42% to −71% further, 5 ms a frame instead of 37–70 |
| `plot.xy` keeps its ranges and thinned lines, and lends them to the plot, while the recordings' stamps (`History::stamp`), the bindings, the trail length, the plot's width and orientation are the same | `builtin.rs` (`Kept`), `history.rs` | paused: a further −82% to −86% |
| A heat map colours and uploads its textures only when the field's stamp, the bindings, the slicing, the log scale, the trail length or the facets change | `fields.rs` | paused: −88% to −99%, nothing uploaded |

Per frame, instructions (millions) and wall time, before → after:

| Case | Before | After, paused | After, running |
|---|---|---|---|
| Lorenz against time | 280.1, 80.7 ms | 15.3, 2.3 ms | 84.7, 9.2 ms |
| Lorenz `plot.3d` | 202.9, 32.6 ms | 26.8, 4.3 ms | |
| predator-prey against time | 184.1, 44.9 ms | 8.5, 0.9 ms | |
| predator-prey coloured by time | 230.7, 46.1 ms | 33.6, 14.4 ms | |
| pendulum-drag against time | 184.3, 40.5 ms | 8.1, 0.9 ms | |
| pendulum-drag's path | 94.7, 15.4 ms | 3.9, 0.9 ms | |
| Lorenz, 500 samples | 2.36, 0.36 ms | 1.49, 0.28 ms | |
| gray-scott heat map | 5.86, 0.58 ms, 64 KB | 0.72, 0.08 ms, 0 | 9.9, 0.77 ms (as before) |
| Gray–Scott 512 × 512 heat map | 83.1, 10.9 ms, 1 MB | 0.74, 0.09 ms, 0 | |
| heat-rod heat map | 8.88, 1.0 ms | 0.44, 0.06 ms | |

Running, Lorenz's frame is the thinning pass over every sample (about 35 instructions a point); predator-prey's view coloured by time thins little, because it draws `prey` against time (see below), whose columns hold a point or two each.

Pictures: renders of a few hundred samples (every example's `render --t-end 1`, in the hash check) are identical. At 50 000–100 000 samples the lines are smoothed a little differently where many overlap: Lorenz against time, 15% of pixels differ, 0.06% by more than 32/255; coupled oscillators 21% and 0.04%; units, predator-prey and pendulum-drag less; none visible at three times the size. One column to a pixel differed by more (1.3% of Lorenz's pixels by more than 32/255: faint stripes in dense areas), two columns 0.2%; four were kept. App screenshots under Xvfb (Lorenz against time and its phase portrait, Gray–Scott) differ only by the run's progress.

Measured and left:

- **Publishing the model only when it changed.** The worker clones the model (`Model::of`) after every batch of commands. Commands that change nothing it publishes are rare: Run, Pause and speed, and the unsaved check once a second (which itself serialises the model, 30–490 µs). A slider or a view's pointer changes a parameter's value, which the published rows hold, so they need it. Once a second at most 0.7 ms (the largest case, in a test build) is under 0.1% of the worker's time.
- **Phase portraits thinned by columns** keep most points when the path retraces itself (an orbit): pendulum-drag's path draws 3.9 M instructions a frame paused, but predator-prey's coloured view still 33.6 M.
- **Thinning while running** reads every sample each frame; keeping the columns of the samples already thinned would make it proportional to the new samples. Not done: 9 ms a frame at the history's limit, within the frame.
- **`plot.3d`** still projects and draws every point (at most 20 000 drawn), 4.3 ms a frame for Lorenz.
