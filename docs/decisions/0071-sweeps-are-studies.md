# 0071 — Sweeps and experiments are studies: one way to run a model at many points

- **Status:** Accepted
- **Date:** 2026-11-04
- **Supersedes:** [ADR 0021](0021-maps-formulas-and-sweeps.md) on sweeps (a model with no states swept by default, the index axis of a sweep), [ADR 0027](0027-explicit-experiments.md) (the experiment as a section of the model file), [ADR 0028](0028-sweeps-as-samples.md) on where a sample lives and how the app records it (its designs stand), [ADR 0067](0067-experiments-and-time-from-data.md) decisions 1 to 6 on where an experiment lives (what a study does is unchanged), and the as-built reading of [ADR 0070](0070-studies-are-blocks.md) step 2 that kept the top-level `sample`

## Context

ADR 0070 made a study a block and said a file's top-level `sample` is read as a definition of the model plus a study over it. Step 2 built only the meaning: the section stayed, headless runs went through the study runner, and the app kept its own sweep (`Session::Sweep`, about 120 references) because study results in the app had only profile views. Sweeps also kept two behaviours no study has:
- a model with no states and no `sample` is swept over its first parameter, by default (ADR 0021);
- two-parameter grid sweeps are reshaped into fields named `grid.<signal>`, for heat maps, surfaces, contours and cuts; unordered designs are shown as samples coloured by a result; the sweep is evaluated in slices on the worker thread, with progress, restarted by any change.

On 2026-11-04 the owner asked why sweeps were still built in, and agreed they move to study blocks. No user file relies on the `sample` section or the default sweep.

## Options considered

| How a study runs in the app | Pros | Cons |
| --- | --- | --- |
| **Within its step, with progress, given up for a newer change** | Results stay a function of the inputs, so rewind, run files and identical runs need nothing new; the window never waits on a step (ADR 0035) | The run waits for the study; a study must be able to stop between points |
| Apart from the step, its results arriving later | The run goes on meanwhile | Outputs that arrive later make a run depend on timing: not reproducible |

| The `sample` section | Pros | Cons |
| --- | --- | --- |
| **Removed; a study block is the only way** | One concept; nothing to explain twice | A model studied as a whole is written as a definition plus a study (helpers make it one step) |
| Kept as shorthand, rewritten as a study when read | Short files | Two ways to write one thing; saving must choose |

## Decision

1. **A model is run at many points only by a study block.** The top-level `sample` section is removed, with no conversion: a file with one is refused, saying to put the model in `models` and a study over it. Experiments over models in time are studies too; the runner is the same.
2. **No default sweep.** A model with no states and no study is evaluated once at the start and then as time passes (constant unless its inputs change), as models with nothing in time already are (ADR 0067). To see a formula against a parameter, study it.
3. **A study's outputs take the shape of its design:**
   - a grid over one parameter, run once per point: lines of cells, the parameter's values their coordinate (as built in ADR 0070 step 3);
   - a grid over two parameters, run once per point: 2D grids of `n1 × n2` cells (the first parameter fastest), both parameters as 2D fields placing their cells, so heat maps, surfaces, contours and cut filters read them directly, with no `grid.` names;
   - any other design (list, random, Latin hypercube, Sobol, file) or replicates: lines of cells indexed by run.
   Formulas read 2D results with a 2D index axis (`grid i = n1`, `grid j = n2`).
4. **Views of unordered studies:** `plot.xy` takes line fields of equal length for `x`, `y` and `colour` and draws their latest values cell by cell as points, the "Samples" view of a study.
5. **In the app, a study runs within its step.** The window shows its progress (runs done of runs); a newer change to anything it reads gives the run up between points and starts again from the new values. Results never depend on timing.
6. **Making a study of the whole model:** "Study this model" also applies to the model open (the canvas background, and Python `Model.as_study(sample)`), moving its blocks into a definition and putting a study over it, as "Make model" and "Study this model" do together.
7. **Removed with the sweep:** `Session::Sweep` and its slices, the `grid.` names, `run_experiment` and `experiment_report` (a file whose only runs are its studies already writes rows, ADR 0070 step 2), the `experiment` and `swept` reports of `check`, and the views' swept index.

**Order of work:**
1. Study outputs shaped by their design (point 3); `plot.xy` over line fields (point 4).
2. Studies in the app: progress, given up for a newer change (point 5).
3. The removals (points 1, 2 and 7), "Study this model" on the whole model and `Model.as_study` (point 6), the resonance examples rewritten as definitions with studies, tests and docs.

## Consequences

- **Concepts removed:** the `sample` section, the default sweep, the app's sweep and its `grid.` names, experiments as a kind apart. **Added:** none; studies, views and the canvas gain what sweeps did.
- Sweeps of formulas are no longer immediate in the app: a study runs within a step. For the examples' sizes (up to a few thousand points of a formula) this is well under a second.
- The resonance examples become a definition and a study each; a heat map of the resonance map reads the study's 2D fields.
- Python code writing `data["sample"]` uses a study block or `Model.as_study`.

## As built

- **Step 1 (2026-11-04).** A grid design over two parameters with one replicate gives `Grid { nx: n1, ny: n2 }` outputs; the two parameters' outputs are the coordinates of every result on axes 0 and 1 (`crates/simsandbox-session/src/study.rs`). Slot kind `Values` (a scalar, `t` or a line of cells) takes `plot.xy`'s `x`, `y` and `colour`; when `x` is a line, the view draws the newest frame of each `y` against it, a marker per cell, coloured in bins by the `colour` line when one `y` is drawn (`Xy::cells`, preset "Cell by cell"). The tuning example adds reach against peak, coloured by gain.
- **Step 3 (2026-11-04).** `ModelFile::sample`, `Loaded::plan` and `Loaded::experiment` removed; `RawFile` reads `sample` only to refuse it; `controls.sweep` is dropped with a warning. A study computes its plan with `Loaded::plan_of` and its points file's digest with `Loaded::points_digest`; a used model with no states is evaluated on new data by `Loaded::stateless`. `Session::Sweep`, its slices, `set_sampling`, `load_deferred`, the app's sweep bar and the views' `Available::swept`, `History::unordered`, whole-history fields and `History::with_index` are removed (the history's index stays `t` for every model). `run_experiment`, `experiment_report`, `Results::series` and Python `Model.sweep` are removed; tests of the study runner use `run::run_sampled`. `keep: "series"` is refused (Q52). A study's default results leave out its model's input blocks. `Edit::StudyModel` ("Study this model" on the canvas background, Python `Model.as_study`) is "Make model" of every block, then "Study this model" on the use. The resonance examples are a spring definition and a study; a model holding a study with 2D results opens on a heat map. Step 2 (progress in the app) follows.
