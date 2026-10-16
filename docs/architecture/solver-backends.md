# Solver backends

_Last updated: 2026-10-15_

Don't write solvers you can wrap. The platform's value is in connecting and exposing mature engines, not in competing with them. Each backend is a plugin crate that turns its native models into Blocks. Only the equation backend exists today.

| Backend | Covers | Wrap, don't build | Example fast methods | Example rigorous methods | Status |
| --- | --- | --- | --- | --- | --- |
| Equations / math | Typed equations, ODEs, DAEs, simple PDEs, maps, cellular automata | Native Rust integrators now; SUNDIALS, DifferentialEquations.jl or Diffrax later | Fixed-step RK4, symplectic | Adaptive, error-controlled solvers | Started (`simsandbox-eq`) |
| Physical networks | Mechanical, electrical, thermal, hydraulic components | Modelica via OpenModelica/FMI, or ModelingToolkit.jl | Exported FMU stepping | Full DAE solve with index reduction | Planned |
| Rigid body / robotics | Contacts, joints, vehicles | MuJoCo, Rapier, Box2D, Bullet | Native | Fine timesteps, validation against analytic cases | Planned (Phase 2) |
| CFD / continuum | Fluids, heat, structures | OpenFOAM, SU2, FEniCSx, deal.II | Lattice-Boltzmann or stable-fluids on GPU | Batch job with mesh-convergence study | Planned (Phases 2–3) |
| Agents / discrete event | Populations, ecology, traffic, queues | Native ECS; Mesa or SimPy via Python | ECS on GPU | Seeded ensembles with statistics | Planned (Phase 1) |
| ML models | Surrogates, controllers, learned dynamics, perception | ONNX Runtime, PyTorch, JAX | ONNX Runtime on GPU | Full framework with training and uncertainty | Planned (Phase 2) |
| External / legacy | Anything else | FMI 3.0 import, gRPC bridge, Python subprocess | Depends on the tool | Depends on the tool | Planned (Phase 2) |

## Arbitrary math as a first-class citizen

- Users type equations in plain or LaTeX-like notation, e.g. `dx/dt = sigma*(y - x)`; the system parses them, infers states, parameters and inputs, and generates a Block ([Q4](../open-questions.md)).
- Symbolic processing gives Jacobians, simplification, stability analysis and code generation.
- Every parameter becomes a slider and every state a plottable signal, covering oscillators, chaos, population models, epidemics and circuits.
- PDEs start with structured grids and method of lines; heavier cases hand off to the CFD/FEM backend.

## ML in both directions

- **ML as a Block:** a trained model sits in the graph like any other component.
- **ML around the graph:** train surrogates of slow models for real-time use, offered as separate, labelled methods with their training data and error bounds attached; fit parameters to measured data with differentiable backends.

## Running external solvers

Linux-only tools (e.g. OpenFOAM) run out of process — in a container, or in WSL on Windows — behind the same Block traits via a gRPC adapter, so the desktop app stays native on every OS.
