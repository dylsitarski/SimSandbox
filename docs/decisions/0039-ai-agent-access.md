# 0039 — AI agents are first-class users

- **Status:** Accepted
- **Date:** 2026-10-22

## Context

The owner wants as much of SimSandbox as possible available to AI agents (2026-10-22): users could prompt an AI to build a complete working model, and AI agents doing research could clone SimSandbox and use it to set up and validate research models quickly. The Python API would give some of this, but an AI agent is helped or blocked by more than an API: whether every feature exists outside the window, whether it can discover what exists, whether errors tell it how to fix a file, whether it can see a plot, and whether results are reproducible enough to check.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Leave it to the Python API | No new rule | Features added to the app only stay out of reach; discovery and checking are left to each agent |
| **A design principle with concrete rules**, served by the files, the CLI and the Python API | Every feature stays reachable; cheap to keep if followed from now | A rule every feature must meet |
| A dedicated agent protocol first (e.g. an MCP server) | Direct tool use from AI clients | Duplicates what the CLI and API should offer; worth adding on top of them, not instead |

## Decision

- **Principle:** everything a person can do in SimSandbox, an AI agent can do without the window, and check. This joins the [vision](../vision.md) as a design principle.
- **Rules every feature meets:**
  - **No window-only features.** Whatever the app can make or change (blocks, connections, equations, parameters, sweeps, filters, views, layout) is expressible in the model file, and the app's edits are file edits ([ADR 0007](0007-json-model-files.md)). Live interaction stays in the app, but its outcome can be saved.
  - **Discoverable.** The CLI and Python API list, as JSON, every registered block type with its ports, units, parameters (defaults, ranges, descriptions), states, methods and labels, and every view and filter type with its slots and settings. The JSON Schema describes the file.
  - **Checkable.** Validation is a command that reports every problem as JSON with its file path (`blocks[2].method`), line and column, and a message saying what would fix it.
  - **Runnable and observable headless.** Runs and sweeps write numbers (CSV, JSON, numpy); views render to images without a window, so an agent can look at the same plots a person sees.
  - **Reproducible.** Seeds and method settings live in the file ([ADR 0034](0034-agent-domain.md)); the same file gives the same numbers, so an agent's result can be checked by rerunning it.
  - **Labelled.** Speed and accuracy labels and verification results appear in headless output, so an agent can tell whether a model is fit for research.
  - **Documented for agents.** `AGENTS.md`, the documentation map and the examples stay the entry point; examples double as templates; documents state facts an agent can act on.
- **Where it is built:** the Phase 1 item "headless interface" ([ADR 0038](0038-phase-1-revised.md)): the Python API, and CLI commands `simsandbox describe` (types as JSON), `simsandbox check` (validation as JSON) and `simsandbox render` (views to PNG). An MCP server for AI clients, and an AI agent driving the running app in front of the user, are built on these later ([Q29](../open-questions.md)); the in-app AI model assistant stays on the [roadmap](../roadmap.md) and gains from both.

## Consequences

- New features are reviewed against the rules above; a feature that only works in the window is incomplete.
- The app's `--screenshot` remains for checking the app itself; `simsandbox render` becomes the way to look at results.
- Error messages are written to be acted on, by people and agents alike.
