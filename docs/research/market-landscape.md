# Market landscape

_Researched: 2026-10-14. Findings come from a web search that returned mostly titles and links, plus prior knowledge; the status of individual companies (HASH, Collimator) should be re-checked before relying on it._

## Summary

Pieces of SimSandbox exist, but nothing combines them. Many tools are excellent within one simulation paradigm; what is missing is something approachable that lets users connect different kinds of simulation, with unit safety, interactivity and shareable, remixable models.

## Existing tools

| Category | Examples | Strength | Gap relative to SimSandbox |
| --- | --- | --- | --- |
| Engineering block diagrams and physical modeling | Simulink, Modelica (OpenModelica, Dymola), JuliaHub Dyad, Collimator | Rigorous, composable, equation-based | Expensive or steep; engineer-focused; weak at agents and discrete events |
| Multimethod business simulation | AnyLogic, Simio | Combines agent-based, discrete-event and system dynamics | Proprietary, business-focused, costly |
| Agent-based modeling | NetLogo, Mesa, HASH | Easy entry; NetLogo strong in education | Mostly one paradigm; HASH appears to have moved away from simulation |
| Educational sandboxes | PhET, Algodoo, sim4edu | Polished and engaging | Closed, pre-built; no building or connecting your own |
| Interoperability standard | FMI 3.0 | Industry-standard model exchange and co-simulation | Plumbing, no friendly front end |

## Market size

MarketsandMarkets projects the simulation-software market at about USD 36 billion by 2030; almost all of it is enterprise engineering (Ansys, Siemens, Dassault, MathWorks). The hobby, learning and open-research space is underserved but harder to monetise. SimSandbox is judged on usefulness and interest, not revenue ([vision](../vision.md)).

## Implications that shaped decisions

- Composition across paradigms with unit safety is the differentiator → [ADR 0002](../decisions/0002-layered-block-contract.md).
- FMI compatibility opens the research and engineering ecosystem → FMI-style cross-domain interface, FMI 3.0 import/export on the roadmap.
- Research-grade CFD should be wrapped, not rebuilt → [solver backends](../architecture/solver-backends.md).
- Natural-language model building is an emerging trend (research papers, vendors such as Simio) → AI assistant in [design](../design/interaction.md).

## Sources

- [MarketsandMarkets — Simulation Software Market](https://marketsandmarkets.com/PressReleases/simulation-software.asp)
- [Digital Engineering 24/7 — market to reach $36.22B by 2030](https://www.digitalengineering247.com/article/simulation-software-market-expected-to-reach-36.22-billion-by-2030)
- [FMI 3.0 release](https://fmi-standard.org/news/2022-05-10-fmi-3.0-release/)
- [OpenModelica FMI documentation](https://openmodelica.org/doc/OpenModelicaUsersGuide/latest/fmitlm.html)
- [JuliaHub — Dyad Studio](https://help.juliahub.com/dyad/v3/tutorials/getting-started.html)
- [HASH — Why not simulate?](https://hash.ai/blog/why-not-simulate)
- [AnyLogic multimethod modeling (System Dynamics Conference)](https://proceedings.systemdynamics.org/2016/proceed/abstracts/1152.html)
- [Simio — natural language in simulation](https://www.simio.com/blog/talking-to-your-models-the-natural-language-revolution-in-ai-simulation)
- [Simulation Agent: LLMs + simulation (arXiv)](https://arxiv.org/html/2505.13761v2)
