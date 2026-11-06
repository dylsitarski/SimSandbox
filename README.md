# SimSandbox

A desktop environment for building, connecting and exploring simulations of all kinds — for fun, for learning and for research.

SimSandbox is a small, solver-agnostic kernel that only knows how to **connect, schedule and step** blocks: machines with typed inputs and outputs, parameters and kernel-owned state. Every kind of physics, math and ML is a block type supplied by a domain (through a registry today, plugins later). A model is a graph of blocks and is itself a block, so models nest, repeat and are studied with the same parts. Users build models from deliberately chosen methods, and every method is labelled for **speed** and **accuracy**, so it is always clear what a model can be trusted for.

> **Status:** Phase 2 (breadth). Typed equations with arrays, agents, nested models with their own clocks, inputs and run files, probes, experiments and a desktop app with a node canvas exist and are tested; rigid bodies and a plugin interface are next. Details: [docs/STATUS.md](docs/STATUS.md).

## Quick start

Requires stable Rust 1.95+ for the desktop app and the CLI, 1.86+ for the libraries ([setup guide](docs/guides/development-setup.md)).

```sh
cargo test --workspace
cargo run --release -p simsandbox-app            # desktop app: opens maximized on its landing page
cargo run --release -p simsandbox-cli -- lorenz --t-end 20 --dt 0.01 --method rk4 > lorenz.csv
cargo run --release -p simsandbox-cli -- run examples/coupled-oscillators.ssbx.json > oscillators.csv
cargo run --release -p simsandbox-cli -- render examples/grazing.ssbx.json --t-end 30 --out grazing.png
pip install ./python                             # Python: import simsandbox
```

Use `--release` to run models: a debug build (`cargo run` without it) compiles faster but simulates more than ten times slower, so the app cannot keep up with a large model's speed (Gray–Scott plays at about 57× instead of 200×). Debug builds are for developing and debugging the code ([setup guide](docs/guides/development-setup.md#debug-and-release-builds)).

People and AI agents can describe, check, run and render models without the window: see the [headless guide](docs/guides/headless.md).

## Learn more

| If you want to… | Read |
| --- | --- |
| Work on the project (agents start here) | [AGENTS.md](AGENTS.md) |
| Know what exists and what's next | [docs/STATUS.md](docs/STATUS.md) |
| Understand the purpose and scope | [docs/vision.md](docs/vision.md) |
| See the plan | [docs/roadmap.md](docs/roadmap.md) |
| Understand the design | [docs/architecture/overview.md](docs/architecture/overview.md) |
| Know why things are the way they are | [docs/decisions/](docs/decisions/README.md) |
| Find any document | [docs/README.md](docs/README.md) |
| Contribute | [CONTRIBUTING.md](CONTRIBUTING.md) |

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
