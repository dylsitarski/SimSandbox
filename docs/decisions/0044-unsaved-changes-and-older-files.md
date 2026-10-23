# 0044 — Unsaved changes, and the rule for older files

- **Status:** Accepted
- **Date:** 2026-10-23

## Context

Phase 1 item 8 finishes shareable model files ([ADR 0038](0038-phase-1-revised.md)): files were already self-contained, but the app replaced a changed model without asking, and older spellings of the format were converted silently with no stated rule. The format has changed often within version 1 (views, filters, sweeps, subsystems), always by adding; files from those days must keep opening, and people and agents need to know when a file was converted and what a newer file means.

## Options considered

| Question | Options | Chosen |
| --- | --- | --- |
| What counts as unsaved | A flag set by each edit; **compare what a save would write with what was opened or saved** | Comparing: nothing to forget when a new kind of edit appears; parameters, edits, sliders, views, speed and step all count |
| When to ask | Never (autosave); **before replacing the model or closing the window** | Asking: the person keeps control of the file |
| Older files | Refuse them; convert with a migration tool only; **convert on reading, say so, write the current form on saving** | Reading converts, so nothing breaks; the notice and `upgrade` make it visible |

## Decision

- **Unsaved changes.** The app keeps the text a save would have written when a model is opened or saved. Before opening another file or example, applying a new model from the equation editor, or closing the window, it compares that with what a save would write now; if they differ it asks: Save, Don't save or Cancel (a cancelled Save as cancels too). The toolbar shows "● unsaved" while they differ (checked on the worker about once a second). Edits that rebuild the same model (canvas, equations, solving) never ask.
- **The rule for older files.**
  1. Within a format version, changes only add: new optional fields and sections, new block, view and filter types. A field's meaning never changes in place.
  2. An older spelling within the version is converted as the file is read, never refused; the conversion is reported (`Loaded::conversions`, the app's log, `check`'s `conversions`), and saving writes only the current form. Conversions are lenient: what cannot be converted is dropped with a warning, never a load failure, unless it affects results.
  3. A change that cannot be expressed by adding (removing or redefining a field) raises the format version. The new version reads every older version by converting it step by step, so a file of any age opens.
  4. A file of a newer format is refused with a message saying it comes from a newer SimSandbox; an unknown field in the current format names the field and suggests a typo or a newer SimSandbox.
  5. Every older form has a file in `crates/simsandbox-file/tests/old/` that must keep loading without warnings and become exactly its `.current.json`, and the examples must be in the current form. Adding a conversion adds a fixture ([guide](../guides/changing-the-file-format.md)).
- **Headless** ([ADR 0039](0039-ai-agent-access.md)): `simsandbox upgrade FILE [--out NEW]` and Python `simsandbox.upgrade(model)` write a file in the current form and list what was converted.

## Consequences

- Phase 1 is complete.
- Every format change costs a fixture and, if it converts, a note; in exchange, old files are tested rather than hoped about.
- The comparison costs one serialisation on the worker per check; for very large models (big start states) the marker may lag.
