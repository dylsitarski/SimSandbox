# 0068 — One kind of nested model: subsystems become models defined in the file

- **Status:** Accepted; built 2026-11-02 (steps 1 to 3)
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

**4. The default clock.** A use without `clock` runs on its user's clock when a wire reaches it (wires never cross clocks) and its model writes no `run.dt`. Otherwise ADR 0066 holds: its own clock at `run.dt` if it has states, on new data if not. A former subsystem has no `run` and is wired, so it stays on its user's clock. (As built, 2026-11-02: the first form, "shared whenever no `run.dt` is written", would have stopped models without states from running on new data, which ADR 0067 keeps; and files saved before this ADR always wrote `run.dt`. So `run.dt` decides only for a wired model. `run.dt` is now written only when it was read or differs from the default.)

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
- `check` lists `definitions` (their ports, uses and step; `models` already lists the clock domains).
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

## As built

**Step 1 (2026-11-02): models defined in the file.**
- **The file:** `ModelFile::models` holds definitions as written (JSON). A use's `model` names one. `BlockEntry::resolved` holds it as read: `ModelFile::from_json` resolves every use (`compose::resolve_models`: its own `models`, then the outer ones, refusing unknown names and models that use themselves), and `ModelFile::resolve_models` does it again after an edit in memory. Every definition is read, used or not. A definition's `format` is optional; its `controls` hold only probes.
- **`run.dt`:** written only when it was read or differs from the default (`RunSettings::dt_written`, `writes_dt`); the `run` section is left out when empty.
- **The clock:** `compose::runs_shared(use, folder, wired)` and `compose::wired`, used by the loader, the diagram's clock label and wiring on the canvas (a model with no step of its own shares once a wire reaches it).
- **`starts`** on uses: `compose::set_starts`, by the path of the inner block, for uses of files and of definitions.
- **Local copies are definitions** (`edit_use`): "Make a local copy" adds a definition named after the use (`free_key` if taken) to the file open, or to the definition holding it for a deeper use; models along the path become definitions held one in another. `edit_use::Target` names a file and a key path through nested `models`. A copy holds the definitions it uses (`self_contained`), and its files are rebased, `models` included. `uses_of` counts the uses of a definition, for the question.
- **Conversion, ahead of step 3:** `compose::convert_subsystems` writes a file's subsystems as definitions: input ports as input blocks, output ports as declared outputs, instances as uses, and port wires as paths. It is not yet applied on reading.
- **Headless:** `check` lists `definitions` (inputs, outputs, uses, step). Python: `Model.define(name, model)`, and `add_model(name, model=...)`.
- **Undo:** `undo::canvas_state` carries `models` and `outputs`.
- **Tests:**
  - the Platoon example converted gives the same bits as with its subsystem, on one clock, and reads back unchanged;
  - definitions are found in the file and from inside a definition, and an inner one of the same name wins;
  - wired definitions share the user's clock; one with states and its own `run.dt` runs on its own clock;
  - start states by use;
  - refusals: an unknown name, a model using itself, `controls` other than probes;
  - local copies as definitions, headless and in the app (undo and redo included);
  - Python.

**Step 2 (2026-11-02): definitions in the diagram and the app.**
- **Nodes:** a use of a definition is a use node ("model wheel", its clock: "shared clock" when wired with no step of its own), with its input blocks and declared outputs as ports. It opens like a use of a file. The Inspector labels its group "uses the model wheel".
- **Editing inside** goes through the same path as for files (ADR 0066 step 4c), so the editor inside a use is the model-level editor: blocks, wires, values, input blocks and declared outputs.
- **The edit question for definitions** (`DiagramTab::begin_edit`): for a definition used N times here, "Edit the definition (N uses)", "Edit as a new copy" (named after the definition) or "Make a local copy" (a new definition for this use). A definition used once (as a local copy is) is edited without asking.
- **Make model** (`Edit::GroupModel`): at a model's level, the canvas's "Make model" groups the selection as "Make subsystem" did, then writes it as a definition `modelN` (`compose::convert_some_subsystems`, which brings along the subsystems it uses). Crossing wires become its input blocks and declared outputs. Not wired, its use is put on the user's clock explicitly (`"clock": "shared"`). Inside a subsystem the menu still says "Make subsystem".
- **Moving a definition to its own file and back** (`edit_use::definition_to_file`, `file_to_definition`; canvas "Move to its own file…" and "Put in this file"; the app's `move_use`):
  - *Out:* the definition, with those it uses, rebased, goes to a new file, kept with the files edited until Save. Every use of it, in definitions too unless they define the name again, points at the file.
  - *In:* the file a use reads becomes a definition named after it, used by every use of that file at this level. The file stays as it is.
  - Undo covers both.
- **Moved to step 3:** replacing `Scope::Subsystem` by paths of uses, which only makes sense once subsystems are converted on reading.
- **Tests:**
  - Make model gives the original model's bits, wired or not;
  - a file moved in and out again gives the same bits, the new file written only on Save;
  - in the app: a definition used twice asks and changes both, a local copy changes one, and one used once is edited without asking;
  - moving in and out in the app, with undo and Save.

**Step 3 (2026-11-02): subsystems removed.** No saved file had subsystems (owner, 2026-11-02), so they are refused rather than converted.
- **The format:** a file with `subsystems` is refused, saying how to write it under `models`. The JSON Schema lists `models`, `model` and `files` and no longer `subsystems`. The flattening keeps subsystems inside: the loader alone reads its own files holding them (`compose::with_subsystems`).
- **Errors inside a model on a shared clock** are reported where they are written: `models.follower.blocks[0].equations`, or the use and its file's path (`Loader::inside_error`, by the names of the blocks as written). Errors inside a definition's text carry the full path (`compose::nest`).
- **The Platoon example** defines `follower` once (an input block `lead`, the declared output `v`) and uses it three times.
- **Diagram, canvas, app:** `Scope` keeps only `Model`. Instance and port nodes, `Edit::Group`, `AddPort` and `RenameSubsystem` are gone, with the subsystem breadcrumb.
  - "Make model" is the only grouping. It understands ends written by path, so uses of definitions can be grouped too.
  - Added to the editor: "Offer X as an output" and "Stop offering X" on a block (`Edit::DeclareOutput`, `RemoveOutput`); renaming a definition from its inside view (`Edit::RenameModel`; uses follow, in nested definitions too); the type menu adding a definition as a use.
  - "Edit…" on a block inside a used model opens the equation editor (`use/path::block`). Applying goes through the edit question (`Edit::SetEquations`).
  - Use nodes show the values set for their use (declared parameters, not those set by formulas). A declared output carries what it shows, so a view of `car1/car.v` is drawn from `car1`'s `v`.
- **The merged model:** a model with nothing in time is paced by its file's step in the app.
- **Headless:** `render --diagram --inside PATH` (`--subsystem` stays as an older name); Python `render(inside=...)` and `Model.group` making a definition. `check` lists `definitions`, and for models that use models also `params`.
- **Not yet:** open nodes inside used models are not saved by path (the canvas's open set is shared with the model's level). A definition no use names is read but not built, so a problem inside it shows once it is used.
- **Tests** moved to definitions with the same checks:
  - flattening equal to the model written by hand;
  - declared parameters and formulas;
  - nesting;
  - problems where written;
  - saving only what changed;
  - units through a used model's parameters;
  - Make model on blocks and on uses, renaming, offered outputs;
  - the equation editor inside a used model.

