# Development setup

_Last updated: 2026-10-16_

## Toolchain

SimSandbox needs stable Rust 1.86 or newer; the desktop app (`crates/simsandbox-app`) needs 1.95 or newer because of egui. `rust-toolchain.toml` makes `rustup` select stable with `rustfmt` and `clippy`. Python 3 is needed only for `scripts/check_docs.py`.

| Platform | Install |
| --- | --- |
| Windows (primary desktop target) | Install [rustup](https://rustup.rs) with the default MSVC toolchain; it prompts for the Visual Studio C++ Build Tools. Work in a normal Windows folder and build from PowerShell or your IDE. |
| macOS | `rustup` from rustup.rs; Xcode command-line tools. |
| Linux | `rustup` from rustup.rs; a C toolchain (`build-essential` or equivalent). The desktop app loads windowing and GPU libraries at run time: `libxkbcommon-x11-0` (X11) or the Wayland client libraries, plus a Vulkan or OpenGL driver (e.g. Mesa). |

### Windows and WSL

- Build and run the desktop app natively on Windows; that is what users will run, and GPU access is simplest there.
- Do not build the same checkout from both Windows and WSL: it is slow across the `/mnt/<drive>` boundary and the toolchains share `target/`. For a Linux build, clone a second copy inside the WSL filesystem (`~/...`).
- WSL is useful later for Linux-only solvers such as OpenFOAM, which SimSandbox will drive out of process.
- The repository uses LF line endings (`.gitattributes`), so both environments see identical files.

## Everyday commands

```sh
cargo build --workspace
cargo test --workspace
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
python3 scripts/check_docs.py          # on Windows: py scripts/check_docs.py
cargo run -p simsandbox-cli -- lorenz --t-end 20 --dt 0.01 --method rk4 > lorenz.csv
cargo run -p simsandbox-cli -- run examples/lorenz.ssbx.json --t-end 20 > lorenz.csv
cargo run -p simsandbox-app                                   # desktop app
cargo run -p simsandbox-app -- examples/coupled-oscillators.ssbx.json
cargo run -p simsandbox-app -- --view 3d                     # open in the 3D trajectory view
```

The app can save a screenshot and exit, for documentation and for checking it renders on machines without a display:

```sh
xvfb-run -a cargo run -p simsandbox-app -- --screenshot app.ppm --after 4   # Linux, no display needed
```

The app pins `eframe` and `egui_plot` to exact versions ([ADR 0008](../decisions/0008-native-egui-ui.md)); `egui_plot` version numbers run one ahead of the egui they use (`egui_plot` 0.37 uses egui 0.36), so upgrade them together.

CI (`.github/workflows/ci.yml`) runs formatting, clippy and tests on Windows, macOS and Linux, plus the docs check.

## Dependencies

Prefer the standard library. Add a crate only when it is well maintained and clearly worth it, and say why in the pull request. Large or platform-specific dependencies (GPU, UI, solver bindings) belong in the crate that needs them, never in `simsandbox-core`.
