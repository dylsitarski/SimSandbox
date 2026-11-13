# 0089 — Native code for typed equations: generated Rust, compiled where a toolchain exists

- **Status:** Accepted (owner, 2026-11-13: native code switches on when a toolchain is found)
- **Date:** 2026-11-13
- **Builds on:** [ADR 0084](0084-where-typed-equations-stop.md) points 5 and 6 (no outside compiler needed; speed from compiling our own program), [Q55](../open-questions.md) (Cranelift in the build)

## Context

The owner asked (2026-11-13) why compiled typed equations cannot match fused Rust, and for a way that models wanting speed can rely on it. The comparison of ADR 0084 point 6 ([performance](../design/performance.md#compiled-routes-adr-0084-point-6-2026-11-13)) found:
- **Cranelift in the process** is 2.0–2.8× faster than the interpreter on Gray–Scott with two-lane vector code, and still 1.3× slower than fused Rust.
- **The remaining gap is LLVM:** its scheduling and unrolling, and AVX's four lanes where Cranelift's x86 back end has two.
- **The bits match exactly.** Rust never reorders or fuses floating-point operations, so code that does the program's operations in its order gives the interpreter's bits, whatever compiles it.

SimSandbox already describes each phase of cells exactly (`PhaseView`, with the rules a kernel must follow). Fused Rust for a model can therefore be generated, not written. ADR 0084 point 5 says model files never *need* an outside compiler, so any such code can only be optional.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| **Generate Rust from each phase's view; compile it with the user's toolchain when one is installed; cache it by hash; load it; check its bits; fall back otherwise (proposed)** | Fused speed (LLVM, this CPU's vector width), the same bits; nothing to write by hand; the model file unchanged and portable | Needs a Rust toolchain (about 1 GB) for the speed; seconds to a minute for a first build; loads unsandboxed native code; per-platform library loading; unsafe code in one crate |
| Cranelift with vector code only (Q55) | Always there, 6–10 ms per edit, no toolchain | 1.3× from fused speed |
| Export a Rust crate for a custom build of SimSandbox or its Python package | No loading at run time | A manual step and a rebuild at every change of the model |
| Embed LLVM in SimSandbox | Fused speed without a toolchain | Tens of MB; hundreds of ms per edit; a heavy, fast-moving dependency |
| A block written by hand in Rust, built into SimSandbox | Fastest possible | Not the model's equations; a rebuild; what typed equations exist to avoid |

## Proposal

1. **A setting of the machine, not of the model:** "native code" (app setting, `--native` in the CLI, `native=True` in Python). It changes no result (point 4), so model files and run files stay as they are and replay the same bits anywhere.
2. **What is generated:** for each phase of a program large enough to have phases, a Rust function from its `PhaseView`, following its rules: runs of cells inside lines, edges one cell at a time, temporaries not stored. Functions and powers call the same Rust functions the interpreter calls. Names from the model never reach the source, only numbers and indices, so a model file cannot inject code.
3. **Where it is built:**
   - A crate in a cache folder (`<cache>/simsandbox/native/<hash>/`), the hash over the generated source, SimSandbox's version and `rustc --version`.
   - Built with `cargo build --release` as a library, for this CPU (`-C target-cpu=native`; still the same bits, since Rust does not contract `a*b + c`).
   - In the background: the model runs on the interpreter (or Cranelift) meanwhile and switches when the library is ready, invisibly, since the bits are the same.
   - Reused while the hash matches; old entries are removed beyond a size limit.
4. **Checked before use:** each loaded phase runs once against the interpreter on the current values; any difference in bits and it is not used, with a warning in the log. The generator is tested as the hook is now, against a kernel written from the view's rules, in any chunking.
5. **Falls back** with no toolchain, a failed build or a failed check: to Cranelift if Q55 puts it in the build, else to the interpreter. `check` and the app's status say which is in use per block.
6. **Loading:** one crate (`simsandbox-native`) holds the generator, the build and the loading (`libloading`), the only one that allows unsafe code. The library exports one `extern "C"` function per phase with the signature of route 2's kernels.

## Consequences

- Models that want fused speed get it from their equations, on machines with Rust installed, with the same results as everywhere else.
- A first build costs seconds to a minute per model version, not per edit of a parameter (parameters are not compiled in).
- The workspace gains a crate that allows unsafe code, and SimSandbox learns to find and run cargo.
- Updated when built: headless guide (setting, `check`), performance, STATUS.

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none. "Native code" is a setting of how fast the machine runs a model, not a concept of models.

## As built (2026-11-13)

`crates/simsandbox-compiled/src/native.rs`; tests `crates/simsandbox-eq/tests/native.rs`.
- **Where it switches on:** the app builds when a toolchain is found (`Mode::Build`, its toolbar says "Native code: building…", then "Native code"); the CLI and Python use libraries already built (by the app, say) but start no builds, because a short-lived process would leave a build running after it exits; `SIMSANDBOX_NATIVE=build` makes them build too, `SIMSANDBOX_NATIVE=off` turns it off everywhere. Point 1's CLI flag and Python option are this variable for now.
- **Generated, built, loaded** as point 2 to 6 say: one library per program (one `phase_N` function each), a crate in `<cache>/simsandbox/native/crates/ssn_<hash>` (`SIMSANDBOX_CACHE` moves the cache), built `--offline` for this CPU on one background thread, loaded with `libloading`.
- **Checked:** each phase's first call with the library runs Cranelift's code and the library's on copies and compares the arrays kept; equal, the library alone from then on; different, Cranelift's code and a warning. `check` reports per block how it runs (`code`: `interpreted`, `compiled`, `native`, or `compiled` and why not native, programs named when they differ; `Block::code`, `PhaseKernel::describe`); the app's toolbar counts the libraries in use, being built and not used.
- **Measured** (Gray–Scott, `fused_speed`, one core): 24, 104, 516 µs at 128², 256², 512² against fused Rust's 28, 113, 471 and Cranelift's 39, 162, 733; the benchmark asserts the bits equal fused Rust's. A library built in under a second here.
- **Tested:** Gray–Scott and a model using every kind of operation built natively in a fresh cache, the rates the interpreter's bits with parameters as given and moved, both libraries in use, none failed; skipped where `cargo` is missing.
