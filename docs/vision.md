# Vision

_Last updated: 2026-11-25_

SimSandbox is a desktop environment for building, connecting and exploring simulations of all kinds — from typed equations and physical networks to rigid bodies, agent populations, CFD and ML models — for fun, for learning and for research.

It is a small, solver-agnostic kernel that only knows how to connect, schedule and step blocks, with every kind of physics, math and ML a block type supplied by a domain (plugins later); a model is a graph of blocks and is itself a block ([ADR 0070](decisions/0070-studies-are-blocks.md)). Users build models from deliberately chosen methods, and each method is labelled for speed and accuracy, so it is always clear whether a model is fit for play, exploration or research.

New models are built first from typed equations, the language SimSandbox owns; codes and models that already exist join the same graph as wrapped blocks, without being rewritten ([ADR 0091](decisions/0091-equations-first-hosting-for-existing-codes.md)).

**Audience:** hobbyists who want to build and play with simulations on their own machine, researchers who need results they can defend, and engineers where the same capabilities serve them; none comes first ([ADR 0094](decisions/0094-the-full-audience.md)).

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

## A modelling language, not a library

A model is close to a function: it has an interface (parameters, inputs, outputs) and a body. SimSandbox could have been a library in a programming language, with models written as code and the app built on top. It is not, by choice ([ADR 0111](decisions/0111-a-declarative-modelling-language.md)).

**A model is a description the system reads, not a procedure it calls.** SimSandbox checks units and shapes, orders evaluation, merges equations across models, chooses and labels methods, rewinds and replays bit for bit, and writes the card, the diagram and the Inspector, all from the model itself. Arbitrary code would hide what a model holds, what it depends on and whether it is deterministic.

**The app, the files and agents share one form.** The app must read a model, change it and write it back without loss. That works for data, not for loops, conditions and helper functions. A file does nothing when opened, so a model can be shared safely, and a person can review what an agent made from its generated card.

So SimSandbox is a **declarative modelling language** with three front ends: the app, the file with the CLI, and the Python package. It takes programming languages' *meaning* and keeps its *form* declarative:

| From programming languages | In SimSandbox |
| --- | --- |
| Functions and their signatures | Models and their interfaces |
| Encapsulation | A use changes or feeds on a model only through its interface ([ADR 0110](decisions/0110-encapsulated-models.md)) |
| Types | Units and shapes |
| Modules and packages | Model files, libraries, plugin packages |
| Inheritance | Extension (planned) |
| Interfaces and generics | Replaceable parts (planned) |
| Loops | Replication (planned) |
| Pinned dependencies | Pinned versions of used models (planned) |
| Tests | Contracts and tests attached to models (planned) |

Each of these is a construct in the file, not code that writes the file, so the app, agents and checks all see it. Code belongs inside blocks: typed equations, plugins and wrapped external codes. A Python script may use loops to *write* a model; the model is still data.

What no single language or library offers is the combination: different kinds of simulation in one deterministic model with units; methods chosen and labelled; runs played with, rewound and replayed; and one checked file shared by people and agents. In any one domain a specialized tool is deeper. SimSandbox's case is composition, honest labels and the shared file.

## Non-goals

- Rebuilding what a mature solver does well when it can be wrapped: OpenFOAM, MuJoCo, Modelica tools and ML frameworks join as blocks ([solver backends](architecture/solver-backends.md)). SimSandbox does own the numerics its equation language needs ([ADR 0091](decisions/0091-equations-first-hosting-for-existing-codes.md)).
- Being a browser product. Desktop-first; a web viewer only if demand appears ([ADR 0003](decisions/0003-desktop-first.md)).
- Promising a real-time method for every model.
- Commercial optimisation for now. The project is judged on usefulness and interest; see [market landscape](research/market-landscape.md).
