# 0092 — What a model file may read: its project folder, and folders the user grants

- **Status:** Proposed (asked by the owner, 2026-11-14: limit what a model reads without forcing copies of shared data)
- **Date:** 2026-11-14
- **Builds on:** [ADR 0063](0063-probes-on-wires.md), [ADR 0066](0066-models-nest-as-blocks.md); answers [Q59](../open-questions.md)

## Context

A model file names other files: models it uses (`file`), data (`input.file`, `start … = file`, meshes), replacements (`files`). Absolute paths and `../` are accepted, so a model received from someone else can read any JSON, CSV or mesh file its user can, and its results can carry the contents out. AI agents run models they did not write. Confining a model to its own folder would force each model to keep a copy of data several models share.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| A. Anything (as now) | Nothing to build | A shared model reads what it likes |
| B. The model's own folder only | Simple | Copies of shared data; models in sibling folders cannot share |
| C. A project folder, plus folders the user grants | Models of one project share data without copies; reads outside need the user's word, given outside the model file | A marker file and a grant to explain |

## Decision (proposed)

C.

1. **Project folder.** The nearest folder at or above the opened model file that holds a file named `simsandbox.project.json`, else the model file's own folder. Every path the model and the models it uses name must resolve, after following links, inside it. Models in one project share `data/` beside them by relative path (`../data/wind.csv`), with no copies.
2. **Grants.** Folders outside the project are read only when the user grants them, never from inside a model file: `--allow-read DIR` (repeatable) and `SIMSANDBOX_READ` headless, `allow_read=[…]` in Python; in the app a question when a model is opened ("reads `/data/shared/wind.csv`: allow this folder?"), remembered in the app's settings, not in the file.
3. **Refusal.** A path outside both is an error at its place in the file, naming the path and the option that grants it. `check` lists every file a model reads (`reads`), so a person or agent sees them before running.
4. The marker file holds `{"format": 1}` for now. It gives no rights beyond its own folder.
5. Unchanged: file contents are in the model hash; probes write only plain names beside the model.

## Consequences

- Every example keeps working (each reads beside itself). A file with absolute or outward paths needs a project marker or a grant; `upgrade` cannot fix that and says so.
- One load context carrying the project folder and grants replaces the thread-local folder (review, section 3), which this needs anyway.
- Documents to update when built: [headless guide](../guides/headless.md), [changing the file format](../guides/changing-the-file-format.md).

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none user-visible in models; the project folder is a setting of where files live, not of a model.
