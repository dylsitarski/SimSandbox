# Changing the model file format

How to add to or change the model file format without breaking files people already have. The rule is [ADR 0044](../decisions/0044-unsaved-changes-and-older-files.md); the format is `crates/simsandbox-file` and its [JSON Schema](../../crates/simsandbox-file/model.schema.json).

## Adding (the usual case)

1. Add the field as optional (`#[serde(default, skip_serializing_if = ...)]`), so older files without it load and files that do not use it are written as before.
2. Describe it in `model.schema.json` and, if it affects meaning, check it in `load` with an error at its path (`blocks[2].at`).
3. Test that it round-trips (`ModelFile::from_json` then `to_json`) and loads.
4. Mention it in `CHANGELOG.md`; the examples stay in the current form (`the_examples_are_in_the_current_form`).

## Replacing a spelling within the format

When a newer form replaces an older one (as `controls.views` replaced `controls.view`):

1. Keep reading the older form, and convert it as the file is read (in `ModelFile::check_controls` for controls, or `from_json` for the rest). Push a sentence to `conversions` naming what became what and the ADR.
2. Save only the new form.
3. Add an older-form file to `crates/simsandbox-file/tests/old/` named after the ADR (`adr-NNNN-what.json`), then write its current form with `simsandbox upgrade tests/old/adr-NNNN-what.json --out tests/old/adr-NNNN-what.current.json` and read it to check it is right. `tests/older_files.rs` then keeps both honest.

## Raising the format version

Only when a change cannot be made by adding (a field removed or redefined):

1. Write an ADR.
2. Raise `FORMAT_VERSION`, and convert the previous version's JSON into the new one before parsing, as one step in a chain, so every older version still opens.
3. Keep the old fixtures; add the new version's examples as `.current.json`.
