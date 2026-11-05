# 0075 — Parameters are inputs with defaults

- **Status:** Accepted
- **Date:** 2026-11-05
- **Amends:** [ADR 0074](0074-model-interfaces.md) decision 1 ("a parameter is never wired") and [ADR 0040](0040-feedthrough-and-algebraic-loops.md) (feedthrough is declared for inputs only)

## Context

ADR 0074 made a model's interface parameters (defaults in the model, values per use or study), inputs (signals, no defaults) and outputs, and said a parameter is never wired: to drive one by a signal, write it as an input. On 2026-11-05 the owner asked what to do when a model's user wants to drive a parameter, such as the spring's force `F`, without copying the model to turn `F` into an input. Copying is what reuse is meant to avoid, and the model's author cannot know which parameters every user will want to drive.

The kernel already passes a block's parameters as a slice on every call (`Derivatives::rhs` and `outputs` take `p`, `crates/simsandbox-core/src/block.rs`), so the engine can drive any declared parameter by writing a signal's value into that slot before each evaluation, for every block type, with no change to block code.

What a driven parameter needs that a constant does not is a record of which outputs read it at the same instant. Feedthrough (ADR 0040) is declared per output for inputs only (`PortSpec::feedthrough`); typed equations work it out from their equations, code blocks declare it with `depends_on`. A parameter's reach into outputs is declared nowhere, and getting it wrong costs unequally:
- assuming a dependency that is not there is safe but costly: loops reported that do not exist, Newton's method on them, a model refused under `run.loops: refuse`, and code-built `EquationGroup`s and population ports (which take no feedthrough) refused as soon as a parameter is driven;
- missing a dependency that is there is silent: the output is computed before its driver is final, so it lags a step, or a real loop goes undetected.

## Options considered

| How a signal drives a value the model calls a parameter | Pros | Cons |
| --- | --- | --- |
| **Every declared parameter of every block may be wired; unwired, it keeps its value** | One rule for all blocks; no copies of models; the author's `param`/`input` still says the intent | Feedthrough of parameters must be known; a rule for when each kind of parameter is read |
| Typed-equation blocks only (`param F` rewritten to `input F` where it is wired) | Exact within steps; no kernel change | A special case: code blocks, starting values and studies left out |
| Never: copy the model and write the value as an input (ADR 0074) | Nothing to build | Reuse lost; the author must foresee every use |
| Updated once per step from the signal, like a slider | No feedthrough analysis | Coarser than an input, silently |

| How outputs' dependence on a driven parameter is known | Pros | Cons |
| --- | --- | --- |
| **Worked out for typed equations; declared by code blocks, with a safe default when not declared; declarations checked by a test that perturbs each parameter** | Never misses a dependency; exact where it is cheap; mistakes in declarations caught in CI | Code blocks gain declarations; the test can miss a dependency (it only flags evidence) |
| Every output depends on every driven parameter | Nothing to declare | Spurious loops everywhere; equation groups and populations refuse driven parameters |
| Detected numerically when the model is built | Automatic | Misses structural zeros (`y = p·u` at `u = 0`), branches, saturation, cells an array parameter reaches only sometimes; a miss is silent |

## Decision

1. **A parameter is an input with a default; an input is a parameter without one.** Every parameter a block declares may be wired, for every block type. Unwired, it has its value (a number, or a formula of the model's declared parameters); wired, the signal replaces it. A parameter is set or wired, not both: `check` refuses one with both, naming it. An input stays a port with no default that must be wired (ADR 0074). Typed equations keep both words because they state the author's intent: `param` (set me), `input` (wire me).
2. **Written as any wire is.** A connection's `to` names the parameter as it names an input: `{"from": "leader.v", "to": "spring.F"}`, a used model's exposed parameter by path (`car1/mass`), a study's parameter as `s.m`. A block's inputs and parameters, and a model's exposed inputs, outputs and parameters, share one namespace: a block type declaring an input and a parameter of the same name is refused when registered, and exposing (ADR 0074) keeps names unique across all three. The wire is checked for unit and shape as for an input: a scalar parameter takes a scalar signal, an array parameter ([ADR 0069](0069-repetition-in-blocks.md)) a vector of its length.
3. **A driven parameter is read whenever its owner reads it:**

   | Parameter | Read when | Driving it means |
   | --- | --- | --- |
   | Ordinary (`k`, `F`) | Every evaluation | It follows the signal, exactly as an input does, including at every right-hand-side evaluation inside a fused group |
   | Starting value (`v0`) | When the block starts | It takes the signal's value at the start and at Restart; initial outputs are computed in dependency order including these, and a cycle through starting values is refused, naming it |
   | Study parameter | When the study runs | The study runs again when the value changes, as it does for a changed input ([ADR 0071](0071-sweeps-are-studies.md)), at that cost |

   Where an input from outside is held over a step (into a fused group's step, or a block that steps itself), a driven parameter is held the same way; the speed and accuracy labels say so as they do for inputs.
4. **Which outputs a driven parameter reaches at the same instant:**
   - Each output port says which parameters it depends on directly: `PortSpec::param_feedthrough`, `None` when not declared, `Some(names)` when declared (set with `depends_on_params`).
   - **Typed equations** fill it exactly, by the same walk that finds inputs (ADR 0040): a defined quantity that uses a parameter, directly or through earlier quantities, depends on it; states do not.
   - **Code blocks** declare it. Not declared means every output depends on every parameter, except outputs that are exact copies of the state (`PortSpec::state`), which cannot change with a parameter at the same instant.
   - Loop analysis and the order of feedthrough outputs count only driven parameters; an undriven one is a constant and adds no edge.
   - **A workspace test checks the declarations** of every registered block type: for each parameter, it changes the value and evaluates the outputs again at several random points over the suggested ranges, with the same state and inputs. An output that changes when its declaration does not list the parameter fails the test; a listed dependency no trial shows is reported, not failed. Numerical trials check declarations; they never replace them at run time.
5. **In the app** a value row on a node has a port; wired, the row shows its source instead of a number, and the Inspector and Controls show "driven by `leader.v`", with no slider. Lifting the wire gives the value back. An exposed parameter of a used model or study shows on its node the same way.
6. **Activation is a separate decision.** Driving starting values and study parameters is most useful for models and studies that run or restart when an event fires, reading their driven values then. That is a new concept with its own questions (what counts as an event, what runs, what outputs hold between activations) and gets its own ADR ([Q53](../open-questions.md)), built on decision 3.

**Order of work:**
1. Kernel: parameter slots fed by connections, written before each evaluation (master and fused groups); `param_feedthrough` with its default, in loop analysis and output order; starting values read from drivers at start, ordered, cycles refused. Typed equations' exact dependencies. The declaration test over every registered block type, and declarations added to the built-in blocks it flags. A test against an exact solution: `dx/dt = -k·x` with `k` driven by a ramp `a·t` gives `x = x0·exp(-a·t²/2)`.
2. Files and headless: connections to parameters (blocks, uses by path, studies), the refusals of decision 2 and of set-and-wired, `check` listing driven parameters, schema; Python's `Model.connect` to a parameter.
3. App: value-row ports and wiring to them, "driven by" in the Inspector and Controls, sliders removed while driven; an example driving a parameter; docs.

## Consequences

- No model needs copying to drive one of its parameters; the author decides what has a default, the user what is driven.
- `PortSpec` gains `param_feedthrough`; code blocks with outputs that are not state copies should declare it, or driving their parameters reports loops that are not there. The declaration test makes forgetting visible.
- A driven parameter of a member of a code-built `EquationGroup`, or one reaching a population port, is refused when it feeds through, as inputs with feedthrough are now.
- Run files and the model hash need nothing new: driven values are computed from the model, and the hash covers connections.
- [ADR 0074](0074-model-interfaces.md): "a parameter is never wired" is replaced by decision 1; its exposing and drawing stay.

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none added. "Parameter" widens: a named, unit-aware value with a default that a wire may drive. The concept list is updated when step 1 is built.

## As built

- **Step 1 (2026-11-05).** `Model::drive(from, (block, param))` checks a scalar output in the parameter's unit and one driver per parameter; `Model::drives`, `is_driven`. In the `Simulator` a driven parameter is an input slot of its Block's own after its ports: the routes, the exchange, the order of feedthrough outputs and loops, and fused groups carry it as they carry inputs, and the value is written into the parameter before each evaluation (into a copy for each right-hand-side evaluation of a fused group); Blocks see their ports' inputs only. `Simulator::set_param` refuses a driven parameter. A Block with driven parameters keeps its outputs in snapshots. `PortSpec::param_feedthrough` (`depends_on_params`, `reads_param`); `OutputPlan` adds edges for driven parameters an output reads, never for starting values, and refuses a declaration naming no parameter. Typed equations fill it exactly, through defined quantities, an array parameter counting as all its elements; grid axes read the parameters in their bounds; `EquationGroup` renames its members' declarations. The built-in blocks the declaration test flagged declare theirs (input blocks, the pendulum, the heat rod, grass, orbits, grazers). Driven starting values: blocks are started again, drivers first, until what they start from stops changing; `starting_order` refuses a starting value that reaches itself at the start through outputs, feedthrough or driven parameters.
  - Tests: the decay `dx/dt = -k·x` with `k = a·t` against `x = exp(-a·t²/2)`, fourth order fused and first order exchanged (held over the step, as an input from outside); loops through driven parameters found, solved and refused under `refuse`; units, double drivers and hand-set values refused; rewind; a chain of driven starting values and a refused cycle; an undeclared code block counted as reading every parameter. The declaration test over all registered types and two typed definitions. The 19 examples run bit for bit as before.
  - Not yet: a vector signal driving an array parameter (each element `m[2]` is a parameter of its own and can be driven by a scalar); refusing a block type whose input and parameter share a name (step 2, with the file format).
- **Step 2 (2026-11-05).** The loader drives a parameter when a connection's `to` names one (not an input) of the block. Flattening resolves a wire into a shared use's declared parameter to the inner parameters set to it alone, through nested uses; a formula reading it otherwise refuses the wire, naming the formula. A wire into a shared use passes a declared parameter through as it passes an input. `compose::check_driven` refuses a parameter set in `params` and wired (blocks, uses by `use/param`, studies), and inputs or outputs named as a declared parameter; `Registry::build` refuses a block type whose input and parameter share a name. `check` lists `driven` (per model in a composition) and `driven_by` on parameter rows (a use's declared parameter too); `Session::set_key` leaves driven parameters to their signals when a declared value's formulas are re-applied. Schema descriptions updated; Python's `Model.connect` documents it.
  - Tests: a file's driven decay against its exact solution, set-and-wired refused; the Platoon's `car2/mass` driven (refused while car2 sets it; refused when the follower writes `2*mass`); the resonance study with `m` driven peaks at `1/sqrt(2)`; names shared by inputs and parameters refused; Python's driven decay.
  - Not yet: driving a parameter of a model on its own clock (refused by the existing rule that models on their own clocks are linked through probes).
