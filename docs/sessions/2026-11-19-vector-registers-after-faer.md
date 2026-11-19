# 2026-11-19 — The vector registers after faer

- **Who:** agent (Claude Code)
- **Goal:** find why a 200-link typed chain from the rigid-body spike stepped 25× slower after a 50-link chain had run with TR-BDF2 in the same process, and fix it.

## Done

- Reproduced it with chain models of the same form. The spike's `typed/` models and its `typed-cost` subcommand were not committed on any branch.
- Ruled out the core's pool, the compiled-code cache, malloc, page faults and denormals: same bits and the same instructions in both orders, all of it user time on one thread.
- Cause: faer's AVX/AVX-512 matrix kernels (`private-gemm-x86`) return without `vzeroupper`, so later SSE code on that thread is slow on Intel processors.
- Fix: `simsandbox_compiled::vector_state::clear_upper()`, called by the implicit methods after factorising and after faer's pivoted solve. Regression test reads `XGETBV(1)`. Findings and timings are in [performance](../design/performance.md#the-vector-registers-after-faer-2026-11-19).

## Decided

- Nothing needing an ADR. Unsafe code stays in `simsandbox-compiled`, which now holds the two small functions this needed.

## Left open

- Any other dependency with hand-written AVX code would need the same call. faer is the only one called from the simulation today.
- The spike's own `typed-cost` harness and chain models should be committed with the spike, so its numbers can be checked again.

## Notes for the next session

- The session's setup script failed (`install` with a missing destination operand). It needs fixing in the environment's settings.
- To tell a slowdown of the same work from different work: compare callgrind's instructions per step. Valgrind hides vector-state, denormal and cache effects.
- Ported to `claude/great-ramanujan-shfh9u` the same day (from branch `claude/cool-thompson-mcyveu`): `factorise` there takes the state as one argument and the working arrays, so the wrapper follows that signature. The new test was checked to fail there with the clearing disabled.
