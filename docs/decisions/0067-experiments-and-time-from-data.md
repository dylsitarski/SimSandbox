# 0067 — Experiments over whole models, and time from data

- **Status:** Accepted
- **Date:** 2026-11-02
- **Extends:** [ADR 0027](0027-explicit-experiments.md) and [ADR 0028](0028-sweeps-as-samples.md) (experiments and samples, now over any model), [ADR 0066](0066-models-nest-as-blocks.md) (clocks of models that use model files)

## Context

ADR 0066 step 3 (2026-11-02) let models without states run on new data, but refused a model in which nothing runs in time. The owner asked why: a chain with no time in it, such as features → an ML model → post-processing, is a real model. The question widened to how SimSandbox should run any experiment on any model:
- sweeps and Monte Carlo over models with or without states, composed or not;
- an ML model over a test set;
- recorded data replayed through a model.

ADR 0027 already put an experiment (sampling, run, reductions) in the model file, and left the sampling of models with states for later. ADR 0028 defined the sample designs.

The owner agreed (2026-11-02) to the proposal below, with its four defaults.

## Options considered

| Experiments | Pros | Cons |
| --- | --- | --- |
| **The opened file's `sample` runs the whole model at each point; a used file's `sample` makes that model a curve** | One rule at every scale; sweeps inside models and experiments over them compose | The same section means two things by where it is |
| A separate experiment file referencing the model | Model and experiment apart | A second kind of file; the experiments of ADR 0027 already live in the model file |

| Data in time | Pros | Cons |
| --- | --- | --- |
| **A model without states runs on new data, which includes its own data files' sample times** | A data-driven chain needs no `dt` and steps exactly at the data, even irregular | A data file's times act without being named as a clock |
| Data drives a model only when its use says `clock: "data"` | Explicit | One more setting for the usual case |

## Decision

**1. The file opened is the experiment.** Its `sample` section always means: run the whole model at each point. This holds whether or not the model has states, uses files or nests. A `sample` section in a *used* file keeps its meaning from ADR 0066 step 3: that model is a sweep, giving a curve at each delivery. A single model alone behaves as today, since its experiment and its sweep coincide.

**2. Points.** A point sets any parameter or input block at any depth, by path (`vehicle/plant/car.m`, `ml/u.y`), with the path rules of ADR 0066 step 2b. Designs are those of ADR 0028, plus **`file`**: each row of a CSV is a point, with columns named by path. This gives an ML model over a test set, or measured operating conditions.

**3. Each point runs the whole model.**
- **Nothing in time:** one evaluation of every model, in link order. This replaces ADR 0066 step 3's refusal.
- **Something in time:** a full run to `t_end`. Models with states can then be sampled, as ADR 0027 planned.

**4. What is kept per point.**
- **By default (owner):** the final value of every output, one row per point, so memory stays bounded however long the runs.
- **`reduce`:** named results, one value per point, written as formulas over the run, e.g. `{"v_max": "max(plant/car.v)", "v_end": "final(plant/car.v)", "settle": "first_time(abs(controller/cruise.e) < 0.1)"}`. They start with the reductions of ADR 0027 (final, mean, min, max, amplitude, period) plus `first_time`, and more as needed.
- **`keep: "series"`:** the full time series of each point, opt-in.

**5. Randomness.**
- **By default (owner):** every point uses the run's seed (common random numbers), so differences between points come from the parameters.
- `seed: "per point"` mixes the point's index into the seed, for Monte Carlo.
- `replicates: n` runs each point n times with seeds mixed from the replicate's index.

**6. Several cores, same results.** Points are independent and their seeds are fixed by their indices, so they run on several cores with the same results whatever the count of cores.

**7. Time from data (owner: automatic).**
- **Data deliveries:** a model without states runs on new data, which now includes its own data files: the sample times of its `input.file` blocks are instants at which it is evaluated, as a link's delivery is. A chain driven by recorded data needs no `dt` and steps where the data has samples, even irregular ones. CSV times are written as decimals, so they join the exact ticks of ADR 0066.
- **Clock forms for a use:**

  | `clock` | Meaning |
  | --- | --- |
  | absent | Its file's `run.dt` if it has states; on new data (links and its own data files) if not |
  | `{"dt": step}` | Every step |
  | `{"times": [t1, t2, …]}` | At the listed instants: schedules, events known in advance, irregular sampling |
  | `"shared"` | Its parent's clock (ADR 0066 step 2) |

- **The end of a run:** a run ends at `t_end` if given. Otherwise, when only data drives it, it ends at the last data sample. With nothing in time and no data, it is the one evaluation at t = 0.

**8. Later, through the same section:** experiments that choose their next points from earlier results, such as optimisation, fitting to data, and continuation or bifurcation, as designs that call a strategy (built in or Python). Events found during a run (a block announcing its next event time) would need a kernel hook and their own ADR. Training an ML model is a model with states, whose iterations are its steps. A block that evaluates a trained ML model (e.g. ONNX) comes through the plugin interface ([Q28](../open-questions.md)).

**Order of work:**
1. Remove ADR 0066 step 3's refusal: a model with nothing in time is evaluated once, in link order (2026-11-02, with this ADR).
2. The experiment over any model: points by path, designs including `file`, one evaluation or a full run per point, final values by default, `keep: "series"`, seeds and replicates, several cores with the same results. Headless first (`run`, `check`, Python), then views of experiment results in the app.
3. `reduce` formulas, starting with ADR 0027's reductions and `first_time`.
4. Time from data: `input.file` sample times as deliveries, `clock: {"times": …}`, and the end of a data-driven run.
5. Adaptive experiments (optimisation, fitting, continuation): their own ADR.

## Consequences

- One notion of experiment covers sweeps, Monte Carlo, sensitivity, test sets and replays, over any model at any depth. It closes the known limitation that models with states cannot be sampled.
- The same `sample` section means "experiment" in the file opened and "this model is a sweep" in a file used. `check` says which.
- Experiments that change nothing about a model run on several cores without changing its results.
- ADR 0066's order of work is unchanged. Its step 4 (the app) and step 5 (rewind, run files) apply to experiments too: a run file records the experiment's points and seeds.
