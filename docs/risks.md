# Risks

_Last updated: 2026-11-14_

| Risk | Why it matters | Mitigation |
| --- | --- | --- |
| Scope explosion | "All simulations" can absorb unlimited effort | Freeze the Block contract early; add domains only as plugins; ship the equation backend first |
| Coupling accuracy | Naively connecting solvers produces plausible-looking but wrong results | Fusion within domains; proper co-simulation master algorithms, coupling-error estimates and UI warnings across domains |
| Real-time vs accuracy confusion | Users may trust a real-time result as if it were research-grade | Speed and accuracy labels visible on plots and exports; reports require verified methods |
| Unverified AI-written numerics | AI-generated solver code can look right and be subtly wrong | [Verification policy](guides/verification-policy.md): every method tested against known solutions and convergence order |
| Sharing requires an install | Desktop-only means no click-to-try links, which weakens the learning pillar | Portable model files, a free standalone player, recorded runs; keep the core free of desktop-only assumptions |
| Being a thin wrapper | Value may feel low if it only re-exposes other tools | Differentiate on composition, unit safety, interactivity and provenance |
| Solo-developer bandwidth | A platform this broad is a multi-year effort | Heavy AI-assisted development with strict docs and tests; open-source core to attract backend contributors; wrap existing solvers |
| Integration pain with external solvers | Building and packaging tools like OpenFOAM across platforms is slow | Run them out of process (containers, WSL on Windows) behind the same Block traits |
| Owning numerics | Typed equations are the primary way to build models ([ADR 0091](decisions/0091-equations-first-hosting-for-existing-codes.md)), so SimSandbox maintains its own solvers, linear algebra and compiled code, each a place to be subtly wrong | The verification policy for every primitive; results bit for bit across routes; [ADR 0084](decisions/0084-where-typed-equations-stop.md)'s bounds on the language; wrap a mature solver where one fits |
