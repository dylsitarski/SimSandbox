# 0005 — Repository as single source of truth

- **Status:** Accepted
- **Date:** 2026-10-15

## Context

SimSandbox is developed largely with AI agents across many sessions, machines and tools (Claude chats, Claude Projects, Claude Code, a Claude doc holding the original plan). Knowledge scattered across those places is lost between sessions, and a new agent cannot work effectively without it.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Repository holds everything | Versioned with the code; available to every agent and tool; reviewable in pull requests | Requires discipline to write things down |
| External doc or wiki as primary | Rich editing | Drifts from the code; not visible to coding agents by default |
| Chat and project memory | No effort | Not durable, not shareable, not reviewable |

## Decision

The GitHub repository `dylsitarski/SimSandbox` is the single source of truth for code, plans, decisions, status and history. The documentation system is defined in [`docs/README.md`](../README.md):

- `AGENTS.md` is the entry point and defines a session protocol: read `docs/STATUS.md` first, and before stopping update status, changelog, ADRs, open questions and a session note.
- Every document has a type with a defined purpose and update trigger, and appears in the documentation map.
- `scripts/check_docs.py` runs in CI and fails on unmapped documents, broken relative links, or ADRs missing from the index.

External places (Claude chats, the Claude Project, the original plan doc) are scratch space or mirrors. Anything decided there is copied into the repository in the same session.

## Consequences

- A new agent can start from `AGENTS.md` with no other context.
- Every session ends with a small documentation update; this is part of the definition of done.
- The original plan doc (Claude doc "SimSandbox — Implementation Plan") is superseded by `docs/`; its content was migrated into vision, roadmap, architecture, design, risks and open-questions documents.
