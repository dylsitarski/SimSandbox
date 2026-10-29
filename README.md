# SimSandbox

A desktop environment for building, connecting and exploring simulations of all kinds — for fun, for learning and for research.

SimSandbox is a small, solver-agnostic kernel that only knows how to **connect, schedule and observe** models; every kind of physics, math and ML lives behind plugins. Users build models from deliberately chosen methods, and every method is labelled for **speed** and **accuracy**, so it is always clear what a model can be trusted for.

> **Status:** Phase 0 (foundations). The kernel, the first equation-domain methods, model files and a first desktop app (run, pause, rewind, live plots) exist and are tested; parameter controls are next. Details: [docs/STATUS.md](docs/STATUS.md).

## Quick start

Requires stable Rust 1.95+ for the desktop app and the CLI, 1.86+ for the libraries ([setup guide](docs/guides/development-setup.md)).

```sh
cargo test --workspace
cargo run -p simsandbox-app                      # desktop app: opens maximized on its landing page
cargo run -p simsandbox-cli -- lorenz --t-end 20 --dt 0.01 --method rk4 > lorenz.csv
cargo run -p simsandbox-cli -- run examples/coupled-oscillators.ssbx.json > oscillators.csv
cargo run -p simsandbox-cli -- render examples/grazing.ssbx.json --t-end 30 --out grazing.png
pip install ./python                             # Python: import simsandbox
```

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
