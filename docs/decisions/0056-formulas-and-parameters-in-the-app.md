# 0056 — Formulas and model parameters made in the app

- **Status:** Accepted
- **Date:** 2026-10-24

## Context

Until now the app only changed numbers. A formula (`"m": "mass"`) or a parameter of the model (top-level `params`) was written in the file, or with Python's `Model.set` and `Model.declare`. Values that formulas set were shown and not edited ([ADR 0043](0043-subsystems.md)). The owner asked how a person sets them, and chose the recommendation of [Q34](../open-questions.md) (2026-10-24).

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| **`= expression` in any value field, "+ parameter" in the Model group** | Works wherever a value is typed (inspector, tables, nodes); familiar from spreadsheets; no new window | Typing `=` changes what a field means |
| A dialog per block with a number/formula choice | Explicit | One more window; slower for one value |
| File and Python only | Nothing to build | The app cannot make the models it shows |

## Decision

- **Formulas.** Any value field takes a number or `= expression`. This covers inspector rows, Controls, tables of uses, values of blocks not yet running, and values on nodes. A value that a formula sets shows `= expression`, with its current value beside it. Typing a number there replaces the formula; typing a new `= expression` changes it. A formula reads the declared parameters in scope: the model's for a block or a use's own value on the model level, and the subsystem's for a block inside a subsystem.
- **Where formulas go.** A block value on the model level, a use's own value (`car1.mass`), and a block value inside a subsystem (every use's default, [ADR 0055](0055-open-nodes-subsystem-values-deferred-windows.md)) take formulas. The model's own parameters are numbers. So are values inside one use (`car1/car.c`): the message says to open the subsystem or declare a subsystem parameter instead.
- **Declaring and removing.** The inspector's Model group, now always shown, has a name, value and unit and a "+ parameter" button. Right-clicking a model parameter offers "Remove this parameter".
- **Edits.** These are canvas edits, `SetFormula`, `DeclareParam` and `RemoveParam` (with `SetParam` for a number replacing a formula), so they can be undone and they restart the run. One that leaves the model unable to load is refused with the reason: an unknown name, a unit that does not fit, or a parameter still read by a formula. It is not kept as a draft. A changed number without a formula still applies at once, without a restart.
- **Headless** stays as it was: Python `Model.set(key, "expression")` and `Model.declare(...)`, or the file.

## Consequences

- A model with shared values can be built entirely in the app.
- ADR 0043's "values a formula sets are shown with the formula, not edited" no longer holds in the app.
- Declared parameters of a subsystem are still made by "Make subsystem" or in the file; the inspector's form declares the model's own.
