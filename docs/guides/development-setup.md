# Development setup

_Last updated: 2026-11-13_

## Toolchain

SimSandbox needs stable Rust 1.97 or newer (Cranelift, which compiles typed equations in `simsandbox-compiled`, needs 1.97; egui 1.95). `rust-toolchain.toml` makes `rustup` select stable with `rustfmt` and `clippy`. Python 3 is needed for `scripts/check_docs.py`, and Python 3.10 or newer with `maturin` for the Python package (`python/`, [README](../../python/README.md)).

| Platform | Install |
| --- | --- |
| Windows (primary desktop target) | Install [rustup](https://rustup.rs) with the default MSVC toolchain; it prompts for the Visual Studio C++ Build Tools. Work in a normal Windows folder and build from PowerShell or your IDE. |
| macOS | `rustup` from rustup.rs; Xcode command-line tools. |
| Linux | `rustup` from rustup.rs; a C toolchain (`build-essential` or equivalent). The desktop app loads windowing and GPU libraries at run time: `libxkbcommon-x11-0` (X11) or the Wayland client libraries, plus a Vulkan or OpenGL driver (e.g. Mesa). Its file dialogs use the XDG desktop portal (`xdg-desktop-portal` with a GTK or KDE backend, standard on GNOME and KDE). |

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
cargo run --release -p simsandbox-cli -- lorenz --t-end 20 --dt 0.01 --method rk4 > lorenz.csv
cargo run --release -p simsandbox-cli -- run examples/lorenz.ssbx.json --t-end 20 > lorenz.csv
cargo run --release -p simsandbox-cli -- check examples/grazing.ssbx.json      # JSON report
cargo run --release -p simsandbox-cli -- render examples/grazing.ssbx.json --t-end 30 --out grazing.png
cargo run --release -p simsandbox-app                                   # desktop app: maximized, landing page (F11: full screen)
cargo run --release -p simsandbox-app -- examples/coupled-oscillators.ssbx.json
cargo run --release -p simsandbox-app -- examples/lorenz.ssbx.json --view 3d   # open in the 3D trajectory view
```

### Debug and release builds

| Build | Command | Use it for | Cost |
|---|---|---|---|
| **Release** | `cargo run --release …`, `cargo build --release` (binaries in `target/release/`) | Running models: the app, `simsandbox run` and `render`, timing (`docs/design/performance.md`) | Slower to compile |
| **Debug** | `cargo run …`, `cargo build`, `cargo test` (binaries in `target/debug/`) | Changing and debugging the code: quick rebuilds, debug assertions, overflow checks, readable backtraces | Simulates several times slower |

The difference matters for any model with real work per step. Gray–Scott (a 128 × 128 grid) takes 1.5–2.2 ms a step in a release build and about 27 ms in a debug build (measured 2026-11-06 with `profile_steps`). The app steps for at most 7 ms of each 8.3 ms tick and drops what it cannot do, so in a debug build it plays Gray–Scott at about 57× instead of the 200× it asks for; in a release build it keeps up. Small models (Lorenz, the cruise control) keep up either way. Tests run in debug builds; the checks in [AGENTS.md](../../AGENTS.md) need no `--release`. The two builds keep separate outputs, so switching does not rebuild the other.

Since 2026-11-13 the debug build optimises SimSandbox's own crates a little (`[profile.dev] opt-level = 1` in `Cargo.toml`) and leaves dependencies unoptimised: `cargo test --workspace` runs in about 27 s instead of 77 s, a rebuild after an edit costs the same, and a build from nothing takes about 4.5 minutes instead of 3 ([performance](../design/performance.md#test-builds-2026-11-13)). Debug assertions and overflow checks stay on; a debugger may show fewer local variables. `target/` grows with every build (incremental caches, release and debug outputs: 22 GB after a long session); `cargo clean` empties it when the disk fills.

### Checking that results stay the same bits

A change to how models are evaluated (the interpreter, compiled code, a solver, a reduction) should leave results bit for bit unless it is meant not to. `scripts/compare_outputs.sh [BASE]` builds the CLI at `BASE` (default `HEAD`, in a git worktree) and from the working tree, runs every example to t = 30 as CSV and JSON and renders it, and prints `DIFF` or `RDIFF` for each difference, `FAILED` for a run that failed or was killed on either side (never counted as the same), and a summary (`outputs same=48 diff=0 failed=0 renders same=24 diff=0` for the examples). Before 2026-11-14 `resonance-map`'s JSON output was killed for memory, so a base older than that reports it as failed. The two release builds take several GB; they are removed afterwards unless `KEEP=1`. When results are meant to change (the 1/h² stencils, 2026-11-13), measure by how much in each differing output ([performance](../design/performance.md)) and say so in CHANGELOG.

`SIMSANDBOX_INTERPRET=1` runs typed equations on the interpreter (the reference), and `SIMSANDBOX_NATIVE=off|cache|build` sets native code ([headless guide](headless.md#speed-of-typed-equations)), for comparing the ways they run.

### Cloud agent sessions

The cloud machines have about 4 cores, a memory limit shared by every process and a fixed disk allowance:
- Build with `-j3` (`cargo test --workspace -j3`), and run the workspace tests, the comparison script and the Python checks one after another: together they get processes killed for lack of memory.
- `target/` grows past 20 GB with debug, release and example builds. When the disk fills, remove `target/debug/incremental`, `target/debug/examples` and `target/release` first (rebuilt quickly); removing `target/debug/deps` costs a full rebuild (about 5 minutes).

The app can save a screenshot and exit, for documentation and for checking it renders on machines without a display:

```sh
xvfb-run -a cargo run --release -p simsandbox-app -- --screenshot app.ppm --after 4   # Linux, no display needed
```

### Testing GPU code without a GPU

wgpu runs on software drivers, so rendering and compute code can be tested on machines without a GPU, including cloud agent sessions and CI:

- **Linux:** Mesa's lavapipe (`mesa-vulkan-drivers`) is a software Vulkan driver. With it, `libxkbcommon-x11-0` and `xvfb`, the app renders headless (`--screenshot`), and offscreen wgpu tests (render to a texture, read it back, compare with a reference image or a CPU result) run in `cargo test`.
- **Windows:** WARP, the software DirectX 12 adapter, ships with Windows and is available on CI runners.
- Software drivers show whether rendering and shaders are correct, not how fast they are or how a particular vendor's driver behaves; check those on real hardware.
- Cloud sessions start from a fresh container: install the Linux packages in the environment's setup script so they are always present.

The app pins `eframe` and `egui_plot` to exact versions ([ADR 0008](../decisions/0008-native-egui-ui.md)); `egui_plot` version numbers run one ahead of the egui they use (`egui_plot` 0.37 uses egui 0.36), so upgrade them together.

CI (`.github/workflows/ci.yml`) uses the newest stable Rust, whose clippy can add lints that an older local toolchain does not flag: run `rustup update` (or `cargo +stable clippy ...` after it) before pushing. It runs formatting, clippy and tests on Windows, macOS and Linux, the same for the Python package (with pytest), plus the docs check. The [headless guide](headless.md) covers the CLI's machine-readable commands and the Python API.

## Dependencies

Prefer the standard library. Add a crate only when it is well maintained and clearly worth it, and say why in the pull request. Large or platform-specific dependencies (GPU, UI, solver bindings) belong in the crate that needs them, never in `simsandbox-core`.
