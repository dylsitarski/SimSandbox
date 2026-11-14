# 0096 — The native cache: trusted like a folder of programs, named by processor, bounded

- **Status:** Accepted (owner, 2026-11-14: asked for the review's work item W4)
- **Date:** 2026-11-14
- **Builds on:** [ADR 0089](0089-native-code-for-typed-equations.md); [review](../research/2026-11-14-code-review.md) W4

## Context

Native code for typed equations is a library per program in a cache folder, loaded when a model is built. The review found three gaps. Loading a library runs what the file holds, and the bit-for-bit check comes after loading, so anyone able to write to the cache could run code as the user. Libraries are built for the machine's own processor (`target-cpu=native`) but named only by source, compiler and version, so a cache carried to another machine (a shared home folder, a container image) could meet an instruction it lacks and stop the program. Nothing was ever removed.

Compiled code (Cranelift's and the libraries') also reads and writes through raw pointers, trusting its caller for every length.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Sign or digest each library | Detects a changed file | A digest beside the file is written by whoever writes the file; a key needs somewhere safer than the cache to live |
| **Trust the cache as the user's own, and refuse one others can write to** | What a folder of programs is trusted by; no keys; one check of permissions | Unix only: on Windows the cache is in the user's profile and its access lists are not read |
| Load nothing built elsewhere than this process | No trust in files | The cache is the point: a model opens at native speed |

## Decision

1. **Trust.** The cache is trusted like any folder of the user's programs. SimSandbox makes its folders for the user alone (mode 700 whatever the umask) and takes other users' write access off each library it builds. On Unix it loads nothing from a cache whose folder, build folder or library can be written by group or others, and says why (`check`'s `code`: `compiled (native code not built: …)`). `SIMSANDBOX_CACHE` must name a folder of the user's own; the folders above it are the user's concern.
2. **Name.** A library's name is the hash of its source, the compiler, SimSandbox's version and the processor: its architecture, system and the instruction sets it has.
3. **Size.** The cache keeps the 200 libraries used most recently; after a build the least recently used go, with what they were built from, never one the running process holds.
4. **Calls.** Compiled code is made only from a phase whose operations fit its arrays and whose lines fill its cells, and every call checks the arrays and scalars it is given against what the code was compiled for: a mismatch is a panic that names it, not a read or write outside an array.

## Consequences

- Libraries built before this have other names and are built again (in the app, in the background); the old ones go as the cache fills.
- A cache made under a umask that leaves folders group-writable is refused until deleted; the message says so.
- The check at each call costs a few comparisons per chunk of cells, not per cell.
- Still as before: a library is checked bit for bit on its first call only, over the cells of that call.

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none.
