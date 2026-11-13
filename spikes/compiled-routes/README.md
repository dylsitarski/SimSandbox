# Compiled routes for typed equations (ADR 0084 point 6)

The comparison [ADR 0084](../../docs/decisions/0084-where-typed-equations-stop.md) point 6 asks for: five ways to evaluate Gray–Scott's right-hand side, by time per evaluation, compile time after an edit, whether the bits equal the interpreter's, and the size each adds. Results and conclusions: [performance](../../docs/design/performance.md#compiled-routes-adr-0084-point-6-2026-11-13).

This crate is not part of the workspace, so its compilers stay out of SimSandbox's build and CI. It uses the hook in `simsandbox-expr` that lets compiled code run a phase of cells in place of the interpreter (`PhaseView`, `PhaseKernel`, `EquationSet::compile_phases`).

| Route | Where |
| --- | --- |
| 1. The interpreter | `simsandbox-expr` |
| 2. Our program compiled by Cranelift into the process | `src/jit.rs` |
| 3. Our program compiled to WebAssembly, run by Wasmtime | `src/wasm.rs` |
| 4. A WebAssembly block written by hand | `src/gray_scott.wat` |
| 5. Native fused Rust | `fused` in `src/main.rs` |

Routes 2 and 3 compile each phase from its view alone: runs of cells inside every line in a tight loop, neighbours a stride away; cells at a line's end one at a time with their edges; temporaries no later step reads kept in registers. Functions and powers call the interpreter's own (`routes::host_call`), so every route gives the same bits.

```sh
cd spikes/compiled-routes
cargo build --release                      # both compilers
taskset -c 0 target/release/compiled-routes
SPIKE_SIZES=128,256 SPIKE_EVALS=50 target/release/compiled-routes
cargo build --release --no-default-features --features cranelift   # one route, for its size
```

Measured 2026-11-13 with Cranelift 0.137 and Wasmtime 50 (which uses the same Cranelift).
