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

## What is left, by expected gain

1. **One model on several cores** ([ADR 0062](../decisions/0062-probes-recording-and-replay.md) step 3): the large grid's evaluation (`Program::run`) is about 60% of its time and runs on one core. Splitting cell loops and sums into fixed chunks keeps runs exact and uses every core. The largest gain for grids, the heaviest models.
2. **Typed equations copy the state in and out at every evaluation** (`EquationSet::fill`, `rhs`, `outputs`): about 20% of a grid's instructions, 6% of the platoon's. The instructions could read the state and inputs where they are instead of from a copy: a change to the expression engine's layout.
3. **Headless runs keep every output at every step** (131 MB per grid model in the two-grid case): copying and page faults. Keeping only what is asked for (`--model`, named signals, probes) would save memory and time.
4. **The app's engine meets at every instant on new threads** (ADR 0066 step 6): about 10–20% for several heavy models on their own clocks in the app; a redesign of the simulator (each clock's blocks on a thread of their own for the whole run).
5. **Linked models running ahead** where links go one way only (ADR 0066 step 6): removes the meeting at every instant for pipelines; not for feedback loops.
