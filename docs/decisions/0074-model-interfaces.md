# 0074 — A model's interface: parameters, inputs and outputs, exposed where they are

- **Status:** Accepted
- **Date:** 2026-11-05
- **Amends:** [ADR 0068](0068-one-kind-of-nested-model.md) decision 2 (a model's ports are its input blocks and declared outputs) and [ADR 0073](0073-ports-and-offered-outputs.md) (how outputs are offered and ports shown)

## Context

On 2026-11-05 the owner found a used model's interface awkward to see and to build:
- outputs were offered from block menus or the Inspector, other ports revealed from a searched list, and inputs were blocks of their own, an asymmetry with no reason a user can see;
- parameters and inputs blur: a value set from the Controls or the Inspector feels like an input, so the resonance spring had input blocks `c` and `m` inside its model and again at the top level only so a study could be given them. Making them parameters seemed to cost reuse, as a model's parameter values are saved with it.

Reuse is not at stake: a model's declared parameters (`params`, [ADR 0043](0043-subsystems.md)) are defaults, and each use sets its own values on its entry (ADR 0068). What was missing is that studies could not take parameters as uses do, and that nothing exposed a block's parameter, input or output in one step.

## Options considered

| What a value set by the user is | Pros | Cons |
| --- | --- | --- |
| **A parameter of the model: a default in the model, a value on each use or study** | Constants stay constants; reuse kept; shown and pinned like any parameter | A signal-driven value must be written as an input |
| An input with a default | One kind of port | An input is a signal: a default blurs it with a parameter |
| A parameter that can also be wired | Flexible | When a wired constant takes its value, and sizes fixed at build time, become rules to learn |

| How the interface is built | Pros | Cons |
| --- | --- | --- |
| **A toggle on each parameter and port, inside the model; wiring from outside to an expanded model exposes the port** | Exposed where the thing is; no menus or lists | A control on every port inside a model |
| Output blocks beside input blocks | Symmetric | A block per output crowds models |
| Menus and lists (ADR 0073) | Built | What the owner found awkward |

## Decision

1. **A model's interface** has three kinds of thing:
   - **parameters**, constants each use or study sets, with a default in the model (its declared `params`);
   - **inputs**, signals wired in, with **no default**: an exposed input of a used model or study that nothing feeds is refused by `check`, naming it, as any unwired block input is;
   - **outputs**, signals offered out (`outputs`, ADR 0066 step 4b).
   A value the user sets is a parameter; a value another block drives is an input. A parameter is never wired: to drive it by a signal, write it as an input (`input c` instead of `param c`).
2. **Exposed inputs** are written beside `outputs`: `"inputs": [{"name": "lead", "to": "control.lead"}]`, each naming the block inputs it feeds. Input blocks (`input.value`) still act as a model's inputs, as before; they remain what live and scheduled input (keys, pointer, `run.inputs`) drive, with their value, which is theirs, not a default of the model's input.
3. **Exposing in one step.** Inside a model (opened or expanded in place), each block parameter, input port and output port has a toggle:
   - on a parameter, it declares a model parameter of that name (made unique) with the current value and unit as its default, and sets the block's parameter to it by formula; off, the block keeps the value and the declaration goes;
   - on a port, it adds the port to the model's `inputs` or `outputs`, by its name (made unique); off, removes it.
   Wiring from outside to an inner port of a model expanded in place exposes that port. "Show a port" and "Offer … to models using this one" go.
4. **Exposed things look different**: an exposed port or parameter is drawn with a ring and is what the outer node shows; others stay plain.
5. **Studies take parameters as uses do.** A study block's parameters are its model's declared parameters, with the model's values as defaults, written in the study's `params`, shown on its node and in the Inspector, pinned to Controls like any parameter; a change runs the study again. Its points may vary them by name (as ADR 0067 allowed).
6. **Examples**: the resonance spring declares its quantities as parameters and has no input blocks; its studies set `c` and `m` as parameters, pinned as sliders. The Platoon's follower exposes its controller's input; the tuning example is checked against these rules.

**Order of work:**
1. Studies take parameters (decision 5) and the resonance examples rewritten (6).
2. Exposed inputs in the format, loader, check and diagram (2; 1's refusal).
3. The toggles, wiring that exposes, the drawing of exposed things, and removing the menus and the list (3, 4); the Platoon rewritten (6).

## Consequences

- A model whose interface has inputs runs only where they are wired; to try it alone, wire a source to it, or keep an input block, whose value it then runs with.
- ADR 0073's Inspector list of offered outputs stays, as the place to see and rename them; the block-menu offer and the port list go.

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none added; a model's inputs no longer need input blocks.

## As built

- **Step 1 (2026-11-05).** A study block's `spec.params` are its model's declared parameters (unit, range, description; the model's value as default); `StudyBlock::evaluate` takes them with the inputs, keys its kept results by both, and sets them before every point. `Edit::Study` copies the use's `params`. The resonance examples rewritten: the spring declares `c`, `m`, `w`; `F` and `k` stay block parameters.
- **Step 2 (2026-11-05).** `ModelFile::inputs` (`InputPort` with a `unit`), checked by `compose::check_inputs`. `compose::expose_inputs` turns them into input blocks where a model is used (`Loader::read_use`, which also refuses a use that does not wire one, by `use.name` or `use/name`), studied (`study::with_exposed_inputs`) or drawn as a use's node (`diagram::use_node`); never where it is edited (`used_model`) or written. `compose::runs_alone` refuses a model with exposed inputs run on its own. Schema updated.
