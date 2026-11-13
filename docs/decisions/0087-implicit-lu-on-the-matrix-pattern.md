# 0087 — Implicit methods factorise on the matrix's own pattern

- **Status:** Accepted (owner, 2026-11-13: build our own LU without row interchanges; the rest proposed here, mine)
- **Date:** 2026-11-13
- **Builds on:** [ADR 0060](0060-arrays-in-typed-equations.md) (implicit methods, step 4; 2D grids, step 5), [ADR 0082](0082-exactness-is-labelled.md) (exact runs)

## Context

Backward Euler and TR-BDF2 solve `(I − c·h·J) Δ = r` by a sparse LU, factorised once per communication step. ADR 0060 used `faer`'s LU with partial pivoting and its COLAMD ordering, and measured TR-BDF2 at 730 ms a step on Gray–Scott at 128 × 128 (32 768 unknowns), most of it factorising: implicit methods were not useful on 2D grids of that size (STATUS item 1, 2026-11-13).

**Measured on 2026-11-13** (`crates/simsandbox-eq/examples/lu_speed.rs`, release, one thread; a step then cost 520 ms, of which faer's LU was 300–330 ms):
- An LU with partial pivoting can pick any row as pivot, so `faer` bounds its fill by the structure of AᵀA, and COLAMD orders for exactly that. Other orderings fed to the same LU through `faer`'s lower-level functions did no better than −28% (nested dissection on AᵀA's graph: 238 against 330 ms); AMD or nested dissection on A's own graph were 4–7× worse.
- `faer`'s Cholesky on a matrix with the same pattern took 65 ms: a factorisation that keeps A's own symmetric pattern could be several times faster.

The matrices are `I − c·h·J` of diffusion and reactions: diagonally dominant in the cases that matter, so pivots on the diagonal are almost always good.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| A better ordering for `faer`'s LU | Small, low risk | At most −28% at 128 × 128 |
| **Our own LU on A + Aᵀ's pattern, pivots on the diagonal (owner's choice)** | Fill of a symmetric factorisation; about 3.5× faster at 128 × 128, 6× at 200 × 200 | A few hundred lines of numerical code to verify; needs a fallback when a pivot is too small |
| Iterative solvers (GMRES with ILU) | Little memory | Iteration counts depend on stiffness; tolerances to choose; a different kind of method |
| Record and move on | No work | Implicit methods stay unusable on large 2D grids |

## Decision

1. **The implicit methods factorise with `simsandbox_eq::sparse_lu`:** a multifrontal, supernodal LU of `A` without row interchanges, on the pattern of `A + Aᵀ`.
2. **Ordering:** approximate minimum degree (`faer`'s AMD) on the graph of `A + Aᵀ`, or nested dissection when minimum degree needs more than 32 operations a column (it fills in, as on 2D and 3D grids and meshes) and nested dissection needs a tenth fewer. Nested dissection splits each part at the middle level of a breadth-first search from a far vertex, so it works on any graph, not only grids. Both are computed once with the structure.
3. **Pivots:** a pivot that is zero, not finite, or smaller than 1/100 of the largest entry below it in its column refuses the factorisation, and that step falls back to `faer`'s LU with partial pivoting (its structure made the first time it is needed). Which path a step takes depends only on the matrix, so runs replay bit for bit.
4. **Exactness:** one thread, a fixed order of operations (dense products by `faer`'s matmul, sequential), so the same matrix gives the same bits every time. Results of implicit methods change in their last bits from before this decision (different rounding); no example uses an implicit method.

## Consequences

- TR-BDF2 on Gray–Scott at 128 × 128: 165 ms a step, from 520 ms. 1D systems cost as before.
- Implicit methods' errors against exact solutions (ADR 0060's tests) are unchanged; their numbers differ from earlier versions in the last bits.
- `faer` stays, for the fallback and dense products.
- Updated: ADR 0060 (note in step 5), [performance](../design/performance.md), STATUS.

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none. It is how an existing method solves its equations.

## As built (2026-11-13)

`crates/simsandbox-eq/src/sparse_lu.rs`, used by `crates/simsandbox-eq/src/implicit.rs`.
- **Symbolic** (`Symbolic::new`, once per structure): the graph of `A + Aᵀ`; the ordering (point 2); the elimination tree, postordered; each column's rows below the diagonal; supernodes: consecutive columns along a chain of parents, merged while their rows below agree, or while at most 32 columns with explicit zeros under a fifth of what they store; for each entry of `A`, its place in its supernode's front.
- **Numeric** (`Symbolic::factorise`): supernodes in order, children before parents. Each front is assembled from `A`'s entries and its children's updates, its columns factorised in blocks of 32 (column by column, then the panel to their right updated by matrix products), its factor kept (the columns, then U's rows to their right), and its update `F₂₂ − L₂₁·U₁₂` passed to its parent (by plain loops below 4096 multiplications, else `faer`'s matmul).
- **Solve:** forward through the supernodes with L (unit diagonal), back with U; a single-column supernode has its own short path.
- **Measured** (`lu_speed`, release, one thread):
  - two coupled fields on a periodic 128 × 128 grid: factorise 75 ms against `faer`'s 265–330 ms, solve 5 against 10 ms; symbolic 87 ms once (faer's 29); 64 × 64: 9.5 against 38 ms; 200 × 200: 476 against 2944 ms;
  - a chain and a ring of 800 cells: factorise 0.03 ms against 0.04–0.06 ms, solves within a few microseconds of `faer`'s; the typed rod's TR-BDF2 step as before (0.14–0.16 ms at 800 cells, 0.04 at 200).
- **Tested:** solves of grids (3, 8, 31 cells a side) and a chain in two pieces to 1e-12; the same bits twice; a pivot needing a row interchange refuses, and in a step falls back to `faer`'s LU and gives backward Euler's exact answer; nested dissection chosen on a 64 × 64 grid (5.2·10⁷ operations against 6.8·10⁷); ADR 0060's convergence tests of both methods pass unchanged.
