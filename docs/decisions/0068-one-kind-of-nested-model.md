# 0068 — One kind of nested model: subsystems become models defined in the file

- **Status:** Accepted
- **Date:** 2026-11-02
- **Supersedes:** the form of [ADR 0043](0043-subsystems.md) (subsystems, their ports and instances), once its order of work is done; declared parameters with formulas stay as ADR 0043 and [ADR 0056](0056-formulas-and-parameters-in-the-app.md) decided.
- **Extends:** [ADR 0066](0066-models-nest-as-blocks.md) (models nest as blocks)

## Context

Since ADR 0066, a model file can use other model files, each on its own clock or on its user's. A model on a shared clock is already flattened by turning it into a subsystem definition (step 2b). The owner asked on 2026-11-02 whether nested models make subsystems obsolete ([Q49](../open-questions.md)).

Subsystems still had four things uses of model files lacked:
- one definition used several times in a file, edited once for every use;
- a light form, with no `run` or `controls`;
- start states for each use (`starts`);
- the fullest editor: rename, ports, nested definitions, "Make subsystem".

Two kinds of nested thing mean two formats, two node kinds, two editors and two sets of rules. Step 4d of ADR 0066 (expanding in place) would otherwise be built for both. The owner agreed to the recommendation below on 2026-11-02.

## Options considered

| Option | Pros | Cons |
| --- | --- | --- |
| Keep both | No migration | Two formats, node kinds and editors; rules drift apart |
| **One kind: a subsystem becomes a model defined in the file** | One format, one node, one editor; a definition can be moved to its own file and back; its ports are input blocks, so it can run alone | A migration of files, names and the app's subsystem editor |
| Drop definitions in the file: reuse only through files and local copies | Simplest format | Using one definition several times needs a file per definition |

## Decision

**1. One kind of model.** A model is written in a file of its own, or as a definition in a model file's `models` section, by name:

```json
"models": {"wheel": {"blocks": [...], "connections": [...], "outputs": [...]}},
"blocks": [{"name": "front", "model": "wheel"}, {"name": "rear", "model": "wheel", "params": {"r": 0.3}}]
```

- A definition is written like a model file: blocks, connections, `params`, `outputs`, `groups`, `fusion`, optionally `run`, `sample` and its own `models`. `format` is optional. Its `controls` hold only probes that feed its links; views and the rest belong to the file opened.
- A use names a `model` (a definition) or a `file`, never both. Its `params`, `clock`, `files` and canvas place work for both as ADR 0066 says.

**2. Its ports** are its input blocks (`input.value`) and its declared `outputs`, as for a model file. On a shared clock, wires go into the input blocks and from the outputs, and the input blocks are wired through (ADR 0066 step 2b). Units come from the input blocks' settings.

**3. Finding a definition.** A use's `model` names a definition of the model it is written in, else of the models holding that one, outward to the file. A file used sees only its own definitions. A definition that uses itself, directly or not, is refused.

**4. The default clock.** A use without `clock` runs on its own clock at the model's `run.dt` if the model writes one, else on its user's clock. A former subsystem has no `run`, so it stays on its user's clock. A model file that writes no `run.dt` now shares its user's clock when used, where it ran on its own at 0.01 s.

**5. Each use's values.** `params` as in ADR 0066 (numbers or formulas, at any depth by path). `starts` gives start states by the path of the inner block (`hub`, `axle/hub`), as subsystem uses had.

**6. Local copies are definitions.** "Make a local copy" (ADR 0066 step 4c) adds a definition named after the use, used by that use only, in the file open; a deeper one goes in the definition holding it. This replaces the `"model": {…}` object of step 4c's second part.

**7. The edit question** (ADR 0066 step 4c) covers definitions:
- a definition used N times in this file: **edit the definition** (every use), **edit as a new copy with its own file**, or **make a local copy for this use** (a new definition);
- a model file: as step 4c says.

**8. One editor.** Opening any use shows its inside with the full editor: blocks, wires, input blocks and declared outputs as ports, renaming (a definition's uses follow), nested definitions. "Make model" (was "Make subsystem") moves a selection into a new definition: crossing wires become input blocks and declared outputs. "Move to its own file" writes a definition out as a model file and points its uses at it; "Put in this file" does the reverse.

**9. Names** are paths everywhere: `front/hub.w`, and `controls.open` lists nodes inside uses by path (`front/hub`).

**10. Older files.** `subsystems` are read and converted (as ADR 0044 converts older spellings, listed in `converted`):
- each subsystem becomes a definition with no `run`;
- each input port becomes an input block of the port's name, wired to what the port fed;
- each output port becomes a declared output;
- an instance (`"type": "follower"`) becomes a use (`"model": "follower"`);
- wires to `car1.u` become `car1/u`, and wires from `car1.y` become `car1/y`;
- `starts` and `params` keep their keys; `subsystem::block` in `controls.open` becomes a path through the first use.

Results are the same bits as before. Saving writes the new form.

**11. Headless:**
- `check` lists `models` (definitions, their ports and uses).
- `render --diagram --inside PATH` draws a use's inside; `--subsystem` stays as another name for a while.
- Python: `Model.define(name, model)`, `add_model(name, model=...)`, and `Model.group` makes a definition.

**Order of work:**
1. Definitions: `models` and `model` uses, the lookup, the default clock, `starts` on uses, local copies as definitions; `check`, `run` and Python.
2. The diagram and the app: definitions are use nodes with the full editor inside any use, paths in place of `Scope::Subsystem`, the edit question for definitions, "Make model", moving a definition to a file and back, and the Inspector.
3. Older files: `subsystems` converted on reading with the same bits (every example and test file), the Platoon example rewritten, names in `controls.open`, headless renames; then the subsystem code paths go. ADR 0043's form is then superseded.
4. ADR 0066 step 4d (expanding in place) for one kind of node.

## Consequences

- One format and one editor for every nested model. A definition can move to its own file and back without changing what uses it.
- Ports as input blocks mean any definition runs alone for testing.
- Shared-clock flattening stays the implementation (ADR 0066 step 2b), so solver fusion across boundaries is unchanged.
- A model file with states but no `run.dt` changes timing when used: it now shares its user's clock. `check` shows each model's clock.
- Files, Python code and names that use subsystems keep working through the conversion, then are written in the new form.
