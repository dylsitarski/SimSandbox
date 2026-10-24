# Glossary

_Last updated: 2026-10-15_

| Term | Meaning |
| --- | --- |
| Block | The unit of simulation: a model definition plus the method computing it, with ports, parameters and state. Implements the `Block` trait. |
| Capability trait | An optional trait a Block may implement (`Equations`, `Rollback`, `Derivatives`, `Realtime`); the kernel uses it to decide how to schedule the Block. |
| Domain | The family a Block belongs to (equations, rigid body, agents, fields, ML, external). Blocks in the same domain can be fused. |
| Fused group | Several same-domain Blocks merged into one problem, e.g. equation Blocks into one ODE system with one integrator (`FusedSystem`). |
| Co-simulation master | The kernel component that steps Blocks or fused groups of different domains and exchanges data between them (`Simulator`). |
| Communication step | The interval at which the master exchanges data between Blocks. |
| Coupling error | Error introduced by exchanging values only at communication steps instead of solving coupled equations together. |
| Model definition | What is simulated: equations or rules, ports, parameters, states. |
| Method | How a model definition is computed: an integrator, solver or discretisation with its settings. |
| Speed class | Real-time, Responsive or Batch. |
| Accuracy class | Qualitative, Quantitative or Verified. |
| Verified | An accuracy label earned by a specific case after its convergence check and benchmarks pass; lost when the model changes. |
| Port | A typed, unit-aware input or output of a Block. |
| Signal connection | A causal connection: an output drives an input. |
| Physical connection | An acausal connection between across/through variable pairs (e.g. voltage and current); only within the equation domain. Not yet implemented. |
| Direct feedthrough | An output that depends on the current input, declared per output and input ([ADR 0040](decisions/0040-feedthrough-and-algebraic-loops.md)). |
| Algebraic loop | Outputs that depend on each other through feedthrough at the same instant, with no state between them; solved together (Newton's method by default, `run.loops`). |
| FMI / FMU | Functional Mock-up Interface, the industry standard for exchanging simulation models; an FMU is a packaged model. |
| ADR | Architecture decision record, in `docs/decisions/`. |
