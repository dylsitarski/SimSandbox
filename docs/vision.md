# Vision

_Last updated: 2026-10-22_

SimSandbox is a desktop environment for building, connecting and exploring simulations of all kinds — from typed equations and physical networks to rigid bodies, agent populations, CFD and ML models — for fun, for learning and for research.

It is a small, solver-agnostic kernel that only knows how to connect, schedule and observe models, with every kind of physics, math and ML behind plugins. Users build models from deliberately chosen methods, and each method is labelled for speed and accuracy, so it is always clear whether a model is fit for play, exploration or research.

**First audience:** hobbyists who want to build and play with simulations on their own machine ([ADR 0009](decisions/0009-hobbyists-first.md)). Students and researchers follow in later phases.

## The three pillars

**Choose the method, know what you get.** Users usually want one thing at a time: to play, to explore or to produce defensible results. SimSandbox does not promise that every model runs in every mode ([ADR 0004](decisions/0004-method-classification.md)). The user picks methods suited to the goal, and the platform labels what those choices can deliver.

| Pillar | What it means for the user | Design rule it imposes |
| --- | --- | --- |
| Fun | Real-time play: drag, poke, steer, watch it react | Real-time methods run in a time-budgeted loop with live input; the model shows clearly when it contains a method that can't |
| Learning | Exposed, labelled parameters; see cause and effect instantly | Parameters, units and equations are first-class metadata, never buried in code |
| Research | Results accurate and reproducible enough to publish | Every run is versioned, seeded and traceable; accuracy is declared and testable |

The pillars pull in different directions: real-time favours speed, research favours accuracy. SimSandbox handles that by classifying methods on both axes ([method classification](architecture/method-classification.md)) rather than forcing every model to serve every pillar.

## Where the value is

- **Equations to live, tweakable simulations.** Type an equation and get parameter controls, plots and phase portraits immediately.
- **Composition across simulation types** with unit safety: an ODE wired to an agent population wired to an ML controller, without hand-written glue.
- **Explorable research models** that others can open and poke instead of reading a static figure.
- **Honest trust labels and provenance**, which no single solver offers.

## Open to AI agents

Everything a person can do in SimSandbox, an AI agent can do without the window, and check ([ADR 0039](decisions/0039-ai-agent-access.md)): models are plain files, every type is discoverable, files are validated with errors that say what to fix, and runs, sweeps and views work headless and reproducibly. A user can ask an AI to build a complete working model; an AI doing research can clone SimSandbox and set up and validate research models quickly.

## Non-goals

- Replacing mature solvers. SimSandbox wraps OpenFOAM, MuJoCo, Modelica tools and ML frameworks; it does not compete with them ([solver backends](architecture/solver-backends.md)).
- Being a browser product. Desktop-first; a web viewer only if demand appears ([ADR 0003](decisions/0003-desktop-first.md)).
- Promising a real-time method for every model.
- Commercial optimisation for now. The project is judged on usefulness and interest; see [market landscape](research/market-landscape.md).
