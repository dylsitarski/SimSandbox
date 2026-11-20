# 0095 — One block with states is fused with the stateless blocks it is wired to; loops stepped apart are noted

- **Status:** Accepted (owner, 2026-11-14: asked for the review's work item W1, which proposed both parts)
- **Date:** 2026-11-14
- **Amends:** [ADR 0051](0051-stateless-blocks-join-fused-groups.md) (the rule "a set is fused if it has two or more blocks with states"); builds on [ADR 0023](0023-automatic-fusion.md)

## Context

The [review of 2026-11-14](../research/2026-11-14-code-review.md) (W1) ran a mass (`dx/dt = v`, `dv/dt = F/m`) wired in a circle with a spring written as a formula block (`F = -k*x`), RK4, dt 0.01. x(5) was −0.8795 against the exact −0.8391, under the label `Quantitative`, with nothing said. ADR 0051 fused a set only when it held two or more blocks with states, so one block with states and its formula were stepped apart, and the force reached the mass a step late: an error of the order of the step, whatever the method. This is the first model many people write. ADR 0051's own reason (a formula or conversion must not silently cost accuracy) applies to it unchanged.

Other loops are stepped apart for reasons that stay: different methods, `fusion: off`, different clocks, the edge of an activated part. Their labels are the methods' and say nothing of the coupling.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Leave the rule; document it | No change | The simplest model with a formula block is wrong under an honest-looking label |
| **Fuse any connected set of two or more candidates that has a block with states** | The formula is evaluated at every stage; one rule for one block with states or several | A group may hold a single block with states (a little more work per step than the block alone) |
| Lower the accuracy label of every model with a loop stepped apart | The label itself warns | Labels are of methods ([ADR 0004](0004-method-classification.md)); a coupling error is of the step, and a label cannot say how large |
| **Say it in the run's notes** | Seen by the CLI, `check`, Python and the app, with what to do | A note can be overlooked |

## Decision

1. Automatic fusion fuses a connected set of candidates when it has two or more blocks, at least one with states. The rest of ADR 0051 stands: stateless blocks take the method of the set they join and never merge sets with different methods.
2. A loop of blocks that could be fused (continuous or stateless equation blocks, at least one with states) and are not all in one group gives a note, with algebraic loops' notes (`run` and `check` output, standard error, the app's warnings): the blocks, that each reads the others' outputs from the start of the step, and how to solve them together.
3. Labels do not change. Loops across domains (an ODE and agents) are not noted: nothing can fuse them yet, and the fixed-step master without a coupling-error estimate remains a known limitation.

## Consequences

- The mass and spring give −0.83907154 at t = 5 (exact −0.83907153), tested.
- Results change for models where one block with states was wired to formula, conversion or input blocks and nothing else fused them: they are now more accurate. Of the examples only `pendulum-drag` changes its grouping (its pointer input joins the pendulum); its results are the same bytes.
- `fused_groups` in `check` lists such sets.

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none.
