# Code review, 2026-11-14

A whole-project review at commit `7b695b6` (about 101 000 lines of Rust and Python, 91 ADRs, 30 days after the first commit). It asks four questions: does the project serve its [vision](../vision.md); is it efficient; is the code clean; is it usable and general for people and AI agents. Findings are observations, not decisions: each work item needs the owner's choice, and an ADR where it contradicts one.

**How it was done.** Four parallel read-only reviews by crate group, plus a hands-on pass using the release CLI the way an AI agent would (describe, write a model from nothing, check, run, render). Items marked **(reproduced)** were run; the rest were read in code and spot-checked. The desktop window was not run (no display), so the app's feel is not assessed.

## Summary

| Area | Verdict |
| --- | --- |
| Vision: pillars and AI access | Strong. The headless loop works as promised; units, labels, seeds and bit-for-bit replay are real |
| Vision: scope | Drifting. The project is becoming one very capable equation language and solver, not yet a kernel that hosts many solvers; the Phase 2 gate has not started |
| Trust labels | One real hole: a simple model runs 5% wrong under a `Quantitative` label with no warning (W1) |
| Performance | Good culture (measured, bit-identical). Remaining waste is per-step allocation and per-frame copying, not algorithms |
| Cleanliness | Lint hygiene is excellent; structure is not. A 2 458-line function, a 54-field `Session`, a 7 400-line `app.rs`, two engines for nested models |
| Interfaces | Model file, `check` errors and Python builder are good. The typed-equation language has no reference; the JSON Schema has drifted; the CLI is thin on help and structured errors |

The single most useful change of direction: **stop widening typed equations and spend the next stretch on (a) the Phase 2 gate and the plugin interface, which test the Block contract against code SimSandbox did not write, and (b) paying down the five structural items below (W6–W10) while they are still cheap.**

## 1. Vision

**Achieved.**

- The AI-agent promise ([ADR 0039](../decisions/0039-ai-agent-access.md)) holds. Starting from `describe` alone, a two-block model was written, corrected through five deliberate mistakes and rendered. Each `check` error named the path and the fix: an unknown type listed the known ones, a unit mismatch gave both dimensions with line and column, an unknown key listed the valid keys **(reproduced)**.
- Research pillar foundations are in place early: seeds, run files, model hash, `made_with`, results identical bit for bit across interpreter, Cranelift and native routes.
- The concept list in the [architecture overview](../architecture/overview.md) is short and deliberately defended (studies, strategies and optimisers are models, not new machinery).
- `unsafe` is confined to one crate; clippy `-D warnings` on three systems; 582 tests.

**Drifting.**

- **"Wrap mature solvers, do not compete" is not yet true in code.** No external solver is wrapped and no plugin interface exists. Built in-house instead: a supernodal sparse LU, finite volumes on unstructured meshes, adaptive refinement, a register-machine interpreter, a Cranelift JIT, native code generation, Levenberg–Marquardt. `simsandbox-eq`, `-expr` and `-compiled` total about 20 000 lines; 12 of 13 block types are equations, inputs or studies.
- **The Block contract has only been tested by its authors.** The roadmap's own rule is that the contract is proven at gates. Phase 2 items 3 (rigid bodies, the gate) and 4 (plugins) are untouched; ADRs 0062–0089 (probes, nested models, studies, tables, meshes, activation, compiled routes) were added on the way. Each is defensible; together they are the [scope-explosion risk](../risks.md) happening.
- **Audience.** [ADR 0009](../decisions/0009-hobbyists-first.md) puts hobbyists first. Recent work (LU orderings, Gmsh import, hexahedral finite volumes) serves researchers. Either is fine; the documents should say which is true now.
- **The kernel is growing sideways.** `simsandbox-core` holds `sampling.rs` (Sobol and Latin hypercube designs), `hold.rs` (a concrete block), `schedule.rs` (probe triggers) and a thread-local "current folder". `Block` has 14 methods (5 required); `code()`, `reads_time()` and `instants()` are typed-equation and data concerns, not capabilities every block has. `Domain` is a closed enum naming domains that do not exist.
- **The documentation system is straining under its own rule.** `STATUS.md` is 96 KB, of which 62 KB is "Done recently" repeating the 70 KB changelog; every agent is told to read it first. Session notes are required but stop at 2026-10-16. `risks.md` is unchanged since 2026-10-15. ADRs total 605 KB; ADR 0066 alone is 50 KB and is edited as work proceeds, against "never edited after acceptance".

## 2. Correctness and trust

- **W1. A stateless block in a loop with one stateful block is not fused, and nothing says so (reproduced).** A mass (`dx/dt = v`, `dv/dt = F/m`) wired to a spring formula block (`F = -k*x`), RK4, dt 0.01: x(5) = −0.8795 against the exact −0.8391, labelled `Quantitative`, `fused_groups: []`, no note. With an explicit `groups` entry the result is −0.83907154. Cause: automatic fusion requires two or more blocks *with states* (`crates/simsandbox-file/src/lib.rs`, `fuse`, the `count() > 1` filter; [ADR 0051](../decisions/0051-stateless-blocks-join-fused-groups.md) states the rule), so the force is held over each step. This is the first model many users will write. Fix: fuse any connected set holding one or more stateful blocks and at least one other member; independently, make labels honest about coupling, by downgrading accuracy or adding a note whenever a same-instant loop crosses an unfused boundary.
- **W2. Deeply nested expressions abort the process (reproduced).** 200 000 nested parentheses in an equation overflow the stack in `simsandbox-expr`'s recursive parser; `check` dies instead of reporting. The unit parser recurses the same way. Add a depth limit.
- **W3. Unit exponents wrap (reproduced).** `Dimension` stores `i8` exponents with unchecked arithmetic: `m^100 * m^100 / m^56` is reported as `1/m^112`. Contrived, but a unit checker should refuse rather than wrap. Use checked arithmetic.
- **W4. Compiled kernels trust their caller.** `PhaseKernel::run` is a safe method that passes raw pointers without length checks (`simsandbox-compiled/src/jit.rs`, `native.rs`); soundness rests on `cells.rs` alone. Store the expected counts and assert per call. The native cache loads any library found under its key without an integrity check, builds with `target-cpu=native` without the CPU in the key (a cache shared between machines can crash), and never evicts.
- **W5. Implicit solvers.** Newton uses an absolute tolerance floor of 1e-10 and a perturbation floor of 1 in SI values, a Jacobian frozen for the whole step, and no retry; backward Euler iterates in place, so a failure leaves a half-updated state. Dormand–Prince spins 10 000 times on a NaN error estimate before a misleading "step size underflow". Verification covers the implicit methods on linear problems only; add a stiff nonlinear reference (Van der Pol or Robertson), as the [verification policy](../guides/verification-policy.md) intends.
- Smaller: simulation time is accumulated (`t += dt`; t = 4.999999999999938 after 500 steps of 0.01 **(reproduced)**) although exact ticks exist; a model file can read any path through `file` uses and data files (decide a policy for untrusted models); `grid` and `table` sizes have no cap.

## 3. Structure and cleanliness

Local quality is high: almost no `#[allow]`, no stray TODOs, careful error paths, comments that explain why. The problem is size and shape.

- **W6. `EquationSet::parse_with_rows` is one 2 458-line function** (`simsandbox-eq/src/symbolic.rs:2208–4665`), about 113 locals and 26 closures, tied together by 55 `expect("checked…")` calls. Statements are matched as strings in four passes; built-in functions are dispatched by name in three places (shape, lowering, units). Introduce a `Statement` enum, an analysed-declarations struct and one function table; split the 6 700-line file.
- **W7. Two engines run nested models.** The CLI and Python run a composition as several simulators (`simsandbox-session/src/compose.rs`, about 1 200 lines); the app, studies and render run the same file as one simulator with clocks. Equivalence rests on one helper called five times. Retire one, or compare them over every composition example.
- **W8. `Session` has 54 fields and about 72 public methods; `app.rs` is 7 428 lines** with a 40-field struct of independent flags and a 413-line `param_panel`. Split `Session` into recording, rewind, inputs and catalogue; split `app.rs` by pane and group modal state in one enum.
- **W9. The interactive canvas lives in the headless crate.** `simsandbox-session/src/canvas.rs` (about 2 000 lines of egui input handling) and egui itself compile into the CLI and the Python wheel; `History` stores `egui_plot::PlotPoint`. Move interaction to the app, keep a paint-only path, and consider a `render` feature.
- **W10. The same logic in three front ends.** Run-file reading, checking and writing exist in the CLI, in Python (which rebuilds the format by hand) and in the app; render setup exists twice. Move both into `simsandbox-session`.
- Also: 8 thread-locals carry load context, and study worker threads re-establish only the folder, so a study probably reads the saved version of a file with unsaved edits (not reproduced); `"probe:"` prefixes are handled in 122 places and names split on `.` in about 59 (a parsed signal reference type would remove both); `group.rs` (266 lines) is used only by a test; five hard-coded systems (about 630 lines) duplicate what typed equations express; the integrator and block-type guides predate `Workspace` and `implicit.rs`.
- **Two wheels are committed at the repository root** (`numpy-…whl` 16.7 MB, `scipy-…whl` 35.3 MB): 52 of the repository's 56 MB. Remove them and ignore `*.whl`.

## 4. Performance and memory

The discipline is unusually good: measurements in [performance](../design/performance.md), bit-identical results, memory budgets, streamed output. What remains is constant-factor waste on hot paths, none of it measured in this review:

| Where | Waste | Fix |
| --- | --- | --- |
| `sim.rs` step | Clones every clock (with its list of sample times) each instant; rebuilds a slot per block; O(groups × members) membership scan; fresh vectors per fused group; a block with k feedthrough outputs is evaluated k times per resolve | Precompute at build; borrow |
| `Session::record` | Copies every field output each step, kept or not; looks ports up by name per scalar; with a study present, a full snapshot every step | Resolve offsets at load; copy only what is kept |
| `expr/cells.rs` | Spawns OS threads per phase per evaluation; about 8 vectors per run | A scoped pool; layout per phase |
| `expr/program.rs` | `Gather` and `Along` rebuild static index tables per evaluation | Build once |
| Implicit methods | Allocate residuals, Jacobian rows and solve buffers per substep; fused groups probe sparsity densely | Reuse `Workspace`; a sparsity hook |
| Tables | Each new row count re-parses the equations from text | Make row counts runtime values |
| Loading | A composition is built twice and parsed about five times; a study evaluation rebuilds the registry | A parsed-file cache |
| Views | `plot.xy` copies up to 100 000 samples per curve per frame with a `Vec` per sample; the heat map re-uploads its texture when paused | Borrow the stored points; decimate to pixel width; cache |
| App worker | Deep-clones the model file after every command batch, including each slider tick | Publish only when dirty |
| Python | Results are copied twice on the way to numpy | One copy |

Measure before fixing: the first three rows are the likely real costs for small real-time models, which is the Fun pillar's case.

## 5. Usability and generality

**For AI agents.**

- **W11. The typed-equation language has no reference.** It is now the main way to build anything (tables, edges, meshes, `solve`, named axes), but its description is one paragraph in `describe` that omits half of it, one several-thousand-word paragraph in the [headless guide](../guides/headless.md), and 30 ADRs. Brackets are context-sensitive: `F[m]` is a unit or an axis depending on whether an axis `m` exists. Write a reference with a grammar and a function table, and generate the `describe` text from the same table as W6's.
- **W12. The JSON Schema is hand-written and has drifted.** `activate` and `restart` (blocks) and `open` and `expand` (controls) parse but are absent from `model.schema.json`, which forbids extra properties, so valid files fail the published schema. Generate it from the structs or test every example against it.
- **W13. CLI.** `--help` prints `error: usage:` and exits 1; there is no per-command help; every failure exits 1; outside `check`, errors, warnings and labels are free text on standard error; `check` reports only the first error; a leftover `lorenz` subcommand remains. Add help, distinct exit codes, an option for JSON errors, and several errors per check.
- Python: no `py.typed` or stub; validation is lazy (`connect` accepts anything until run); no wheels are built, so use needs a Rust toolchain. The planned MCP server (Q29) would be cheap once W10 and W13 are done.

**For people.** Not assessed in the window. From the code: the app is tested by calling methods, not by simulated interaction; several built features are still "not yet tried by the owner in the window" ([STATUS](../STATUS.md), In progress), and that queue should empty before more UI is added.

**Generality of concepts.**

- `Shape` has seven variants with overlap: `Vector(n)` and a one-row `Grid` have the same axes but do not connect; `Points`, `Population` and `Rows` differ mainly in encoding. [ADR 0081](../decisions/0081-shapes-are-lists-of-axes.md) points the right way; finish it down to fixed axes plus rows.
- A block entry in the file is now a union of block, use and study (17 keys). Still learnable; watch it.
- `SimError` is mostly strings and a failing `do_step` carries no block name or time.
- A trivial block takes about 50 lines because `BlockSpec` has no default or builder. This matters most for the plugin interface: write the third-party "hello block" first and let it shape the contract.

## Work items

Priority 1 protects trust, 2 pays structural debt, 3 is speed, 4 is polish. Size: S under a day, M a few days, L a week or more.

| # | Item | Priority | Size |
| --- | --- | --- | --- |
| W1 | Fuse one stateful block with stateless neighbours; note or downgrade labels on unfused same-instant loops | 1 | S |
| W2 | Depth limit in the expression and unit parsers | 1 | S |
| W3 | Checked unit-exponent arithmetic | 1 | S |
| W12 | Schema generated from, or tested against, the structs | 1 | S |
| — | Remove the committed wheels; ignore `*.whl` | 1 | S |
| W5 | Newton tolerances, retry and state restore; DOPRI5 non-finite guard; stiff nonlinear verification case | 1 | M |
| W4 | Length assertions in compiled kernels; CPU in the native cache key; cache size cap | 1 | M |
| W11 | Typed-equation reference with grammar and function table | 2 | M |
| W6 | Split `parse_with_rows` into parse, analyse, lower with typed intermediate forms | 2 | L |
| W7 | One engine for nested models, or exhaustive equivalence tests | 2 | L |
| W9 | Canvas interaction out of `simsandbox-session`; egui-free history | 2 | M |
| W10 | Run-file and render operations shared by CLI, Python and app | 2 | M |
| W8 | Split `Session` and `app.rs` | 2 | L |
| — | Kernel tidy: move `sampling`, `hold`, `schedule` out of core; `code`, `reads_time`, `instants` into capability traits; `BlockSpec` builder | 2 | M |
| — | Docs: cap "Done recently" at two weeks (the changelog holds the rest); resume session notes or drop the rule; refresh `risks.md`, roadmap and the two stale guides | 2 | S |
| — | Step-loop and recording allocations (section 4, rows 1–2), measured first | 3 | M |
| — | Thread pool for cell phases; static index tables; implicit-method buffers | 3 | M |
| — | `plot.xy` decimation and caching; publish the model only when dirty | 3 | M |
| W13 | CLI help, exit codes, JSON errors, several errors per `check` | 4 | M |
| — | Python `py.typed`, stubs, wheels in CI | 4 | M |
| — | CI: build cache, minimum-Rust job, Python 3.10, `cargo deny` | 4 | S |
| — | Delete `group.rs` and the `lorenz` subcommand; hard-coded systems as typed definitions | 4 | S |

## Questions for the owner

1. Is SimSandbox for now an equation sandbox that will later host other solvers, or a host first? The vision says host; the code says sandbox. Whichever it is, the vision, roadmap and audience ADR should say so.
2. Should the Phase 2 gate (rigid bodies, then plugins) start before any further typed-equation feature?
3. Which nested-model engine survives (W7)?
4. May a model file read paths outside its own folder?
