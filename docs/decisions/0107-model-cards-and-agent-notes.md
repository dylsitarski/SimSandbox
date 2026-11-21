# 0107 — Model cards are generated; only agents must write notes; what an agent made is marked

- **Status:** Accepted (owner, 2026-11-20); field names and the card's layout to be settled when built
- **Date:** 2026-11-20
- **Builds on:** [ADR 0039](0039-ai-agent-access.md) (AI agents as first-class users), [ADR 0094](0094-the-full-audience.md) (the full audience)

## Context

Models made by AI agents must be easy for a person to understand: what the model is for, how it is built, which methods it uses and how far its results can be trusted. The owner (2026-11-20) liked a model card generated from the file, and decided that manual entries are required only of agents, and that both a check for agents and a record in the file mark what agents made.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| A hand-written description for every model and block | Rich | Burden on people; goes stale |
| **A card generated from the file; notes required only where an agent made the part** | Nothing to write for people; agents explain what cannot be derived; the card never goes stale on what is derived | Needs the file to say who made what |
| Notes required of agents by convention only (a check they run) | No file change | The person reading cannot tell agents' parts from theirs |
| Provenance in the file only | The card can show it | Nothing makes agents write notes |

## Decision

1. **The model card is generated** from the model file, in the app and headless (a command and a Python call). It gives the model's structure with its parts' roles, each block's equations, units, methods, speed and accuracy labels, fusion and verification status, and what each view shows. People never have to write anything for it; notes they write are shown.
2. **Notes say what cannot be derived:** a model's or block's purpose, its assumptions, and why a value was chosen. They are optional fields of the model and its blocks.
3. **Agents must write them.** `check` has an agent level (`simsandbox check --agent`, the same in Python and any later agent interface). It fails when a part an agent made lacks a purpose, or a value it set by hand lacks a reason. A person's ordinary `check` does not report these.
4. **What an agent made is recorded in the file.** A model, block, wire or value made or last changed through an agent interface carries a mark. The card shows it beside the agent's notes. A person who edits that part, or marks it as reviewed, clears it.
5. AGENTS.md, `describe` and the headless guide tell agents to run `check --agent`.

## Consequences

- New optional fields in the model format (notes, the mark), added as [changing the file format](../guides/changing-the-file-format.md) describes, and a new command or option for the card; their names are settled when built.
- The agent interfaces (CLI, Python, a later MCP server, [Q29](../open-questions.md)) need to know they act for an agent: a flag or a mode, settled when built.
- The card is the "Overview" of the [interface report](../research/2026-11-20-interface-report.md).

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none; the card is a view of a model, notes and the mark are data on it.

## Built (2026-11-21)

- Point 1, headless:
  - `simsandbox explain FILE [--format markdown|json]`, `ss.explain` and `Model.explain` (Markdown or a dict), from `simsandbox-session/src/card.rs`;
  - parts' roles are inferred (an input block is interaction; a block with states a system; a stateless block that reads a system and drives one a controller; other stateless blocks formulas; a study block a study);
  - a part without a summary of its own shows its type's;
  - key parameters are the pinned controls, else the types' suggestions ([ADR 0109](0109-parameter-roles-key-parameters-port-groups.md)).
- Points 2–5 (2026-11-21), with the names left open settled:
  - **Notes:** an optional `notes` object on blocks (`purpose`, `assumptions`, `reasons` by parameter) and on the model (`assumptions`, `reasons`; its purpose is its `description`).
  - **The mark:** `"agent": true` on the model (its description, run, views, declarations), on blocks (their values included) and on wires.
  - **Python:** `ss.agent_mode()` (or `SIMSANDBOX_AGENT=1`) marks what a `Model` adds or changes; `Model.note()` writes notes.
  - **`check --agent`** (`check(model, agent=True)`, `describe::check_with`; `card::agent_problems`) reads the file as written. It fails at each path where a marked part lacks:
    - a model's description;
    - a block's purpose (a typed-equations block's summary will do);
    - a reason for each value set on it;
    - a description for each typed-equations parameter (the stand-in "Parameter of the equations" counts as none) and input.
  - **The card** shows the marks and the notes.
  - **Clearing:** `Edit::Review` clears a block's mark and those of the wires into it (an empty name clears the model's). The app adds it after a person's edit of a block's content (`Edit::edited_block`), and the Inspector shows a "Made by an AI agent" note with "Mark as reviewed".
  - **Guidance:** in `describe`'s catalogue (`for_agents`), the headless guide and AGENTS.md.
- Limits:
  - A review is an edit of the model, so marking a block reviewed in the app rebuilds and restarts the run, as other edits do.
  - Values changed live with a slider do not clear a mark.
- Not yet: the card in the app (the Overview tab, [ADR 0108](0108-interface-synchronized-projections.md)).

