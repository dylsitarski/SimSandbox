# 0004 — Method classification instead of "one model, three modes"

- **Status:** accepted
- **Date:** 2026-10-15

## Context

An earlier idea promised that every model could run in interactive, exploratory and research modes. Heavy models (e.g. CFD) have no faithful real-time version, and users usually want one mode at a time.

## Decision

A Block separates its **model definition** from one or more **methods**. Each method carries two independent labels:

| Speed | Meaning |
| --- | --- |
| Real-time | Keeps a 30–60 fps loop and accepts live input |
| Responsive | Seconds to minutes; change a parameter, then rerun |
| Batch | Hours or more; runs as a job |

| Accuracy | Meaning |
| --- | --- |
| Qualitative | Right behaviour; numbers not to be trusted |
| Quantitative | Numerically controlled, not yet checked for this case |
| Verified | Error estimated for this case; benchmarks pass |

Shorthand: Interactive = real-time; Exploratory = responsive + quantitative; Research = verified at any speed.

## Consequences

- No promise that a fast method exists for every model. Surrogates and reduced-order models are separate, labelled methods.
- A composite model runs only as fast as its slowest method and is capped by its least accurate (`MethodLabels::combine`).
- Verified applies to a specific case and is lost when the model changes.
