//! Older files keep opening (ADR 0044): every older form of the format has
//! a file in `tests/old/` that must load without warnings, report what was
//! converted, and become exactly its `.current.json`, which loads as it is.
//! A change that alters how a form converts shows up here.

use simsandbox_core::registry::Registry;
use simsandbox_file::{load, FileError, FORMAT_VERSION};
use std::path::PathBuf;

fn registry() -> Registry {
    let mut r = Registry::new();
    simsandbox_eq::register(&mut r).unwrap();
    simsandbox_agents::register(&mut r).unwrap();
    r
}

fn old_files() -> Vec<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/old");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.extension().is_some_and(|e| e == "json")
                && !p.to_string_lossy().ends_with(".current.json")
        })
        .collect();
    files.sort();
    files
}

#[test]
fn every_older_form_converts_to_its_current_form() {
    let files = old_files();
    assert!(files.len() >= 5, "{files:?}");
    for old in files {
        let name = old.display().to_string();
        let text = std::fs::read_to_string(&old).unwrap();
        let loaded = load(&text, &registry()).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(loaded.warnings.is_empty(), "{name}: {:?}", loaded.warnings);
        assert!(!loaded.conversions.is_empty(), "{name}: nothing converted");
        let current_path = old.with_extension("current.json");
        let current = std::fs::read_to_string(&current_path)
            .unwrap_or_else(|_| panic!("{name}: no {}", current_path.display()));
        assert_eq!(
            loaded.file.to_json(),
            current,
            "{name} converts differently"
        );
        let again = load(&current, &registry()).unwrap();
        assert!(
            again.conversions.is_empty(),
            "{name}: {:?}",
            again.conversions
        );
        assert!(again.warnings.is_empty(), "{name}: {:?}", again.warnings);
    }
}

#[test]
fn the_examples_are_in_the_current_form() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    for e in std::fs::read_dir(dir).unwrap() {
        let path = e.unwrap().path();
        let text = std::fs::read_to_string(&path).unwrap();
        let loaded = load(&text, &registry()).unwrap();
        assert!(
            loaded.conversions.is_empty(),
            "{}: {:?}",
            path.display(),
            loaded.conversions
        );
    }
}

#[test]
fn files_from_a_newer_simsandbox_say_so() {
    let newer = format!(
        r#"{{"format": {}, "blocks": [], "warp": true}}"#,
        FORMAT_VERSION + 1
    );
    let err = load(&newer, &registry()).err().unwrap();
    assert!(matches!(err, FileError::UnsupportedFormat(_)));
    assert!(err.to_string().contains("newer SimSandbox"), "{err}");
    // A field this build does not know, in the current format.
    let unknown = r#"{"format": 1, "blocks": [], "warp": true}"#;
    let err = load(unknown, &registry()).err().unwrap().to_string();
    assert!(
        err.contains("warp") && err.contains("newer version"),
        "{err}"
    );
}
