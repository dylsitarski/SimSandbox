# 0099 — CLI exit codes, JSON errors and several errors per check

- **Status:** Accepted
- **Date:** 2026-11-18
- **Builds on:** [ADR 0039](0039-ai-agent-access.md) (every feature usable by AI agents headless), [ADR 0041](0041-headless-interface.md) (the CLI); [review](../research/2026-11-14-code-review.md) W13

## Context

The review (W13) found the CLI hard for an agent to drive: `--help` printed `error: usage:` and exited 1, there was no help per command, every failure exited 1, errors, warnings and labels were free text on standard error outside `check`, and `check` reported only the first error. An agent could not tell a typo in its own command from a broken model or a missing file without parsing English.

## Options considered

| Question | Options | Chosen because |
| --- | --- | --- |
| Exit status | One failure status; **one per kind of failure** (the BSD `sysexits` numbers, 64–74) | Small numbers are what agents and shells test; `sysexits` is not portable to Windows conventions and has more kinds than a caller acts on |
| Machine-readable errors | Always JSON; JSON only from `check`; **an option, `--errors json`, for every command** | Text stays the default for people and keeps today's output byte for byte; one option covers every command |
| Argument parsing | `clap` (help and errors for free); **the existing hand-written parser, with help texts** | The workspace avoids dependencies; the parser is about 40 lines and the help is a table of texts |
| Several errors per `check` | Make every stage recover; **collect where the stages are independent** | Each block and each wire is checked on its own already; recovering inside the typed-equation parser would restructure its front end (W6) |

## Decision

1. **Help.** `simsandbox --help`, `-h` and `help` print the usage (commands, options, exit statuses) to standard output and exit 0. `simsandbox COMMAND --help` (or `-h`, or `help COMMAND`) prints that command's synopsis and options. With no arguments the usage goes to standard error, exit status 2. A usage error prints `error: MESSAGE`, the command's synopsis and a pointer to its `--help`.
2. **Exit status** by kind of failure:

   | Status | Kind | When |
   | --- | --- | --- |
   | 0 | | Success |
   | 1 | `model` | The model cannot load, or `check` found errors |
   | 1 | `run` | The model loaded, but the run, or the drawing, failed |
   | 2 | `usage` | An unknown command or flag, a missing or bad argument (`--t-end -1`, `--format xml`, an unknown type for `describe`, flags that do not go together) |
   | 3 | `io` | A file could not be read (the model, a run file, the model a run file names, `--inputs`) or written (`--out`, `--save-probes`, standard output) |

   `model` and `run` share status 1 ("the model or the run failed"); the kind tells them apart in JSON. Errors the library finds while running a model (a `--signals` name that keeps nothing, `--inside` naming no use) are `run`, status 1.
3. **`--errors text|json`**, anywhere on the line (`--errors=json` too). With `json`, every line the CLI writes to standard error is one JSON object:
   - a failure: `{"error": {"kind": "model"|"run"|"usage"|"io", "message": TEXT, ...}}`, with `file` (the file it concerns; `-` for standard output) where there is one, `usage` (the synopsis) for a usage error, and `errors` (the `check` report's list, each with `path` or `line` and `column`) when the model has check errors: from `check`, and from `run`, `render` and `start-from`, which then run `check` to say where (and report kind `model`);
   - `{"warning": {"message": TEXT, "file": PATH}}` (`file` when the text form names one), `{"note": {"message": TEXT, "model": NAME}}` (`model` in a model using models), `{"labels": {"speed": ..., "accuracy": ...}}`, `{"converted": {"message": TEXT}}`, `{"wrote": {"file": PATH}}`.

   Text (the default) is unchanged: the same lines as before, so output that people or scripts read stays byte for byte the same.
4. **`check` reports several errors** where the checks are independent: every block (its name, type, method, settings, definition, start and parameters) and every wire between blocks that were built (a wire to a block that failed is not checked, as its error would only repeat that one). `simsandbox_file::load_errors` gives them; its first error is always the one loading stops at, so `errors[0]` is unchanged and the report's form is the same. The rest still stop at the first error: JSON syntax and the file's structure, flattening subsystems and models (including wires naming a block that does not exist), each subsystem built alone, declared outputs, fused groups, controls, the session; and within one typed-equations block, the parser's first error.

## Consequences

- An agent tells its own mistakes (2) from the model's (1) and the file system's (3) without reading the message, and reads every failure, warning and note as JSON with `--errors json`.
- `check` of a missing run file is now an `io` error (exit status 3, no report) where it printed a report with `ok: false` and exited 1; a missing model file already printed no report.
- The `lorenz` command, left from before model files, is removed; `examples/lorenz.ssbx.json` runs the same system.
- [Headless guide](../guides/headless.md) updated. Not done: errors with a time and block name from a failing step (the review's `SimError` note), and collecting errors in the stages listed under 4.

**Concepts added or removed** ([concept list](../architecture/overview.md#concepts)): none.
