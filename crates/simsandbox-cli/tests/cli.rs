//! The headless interface as an agent uses it (ADR 0039): every command
//! through the built binary, its JSON parsed back.

use serde_json::Value;
use std::path::PathBuf;
use std::process::{Command, Output};

fn simsandbox(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_simsandbox"))
        .args(args)
        .output()
        .expect("the binary runs")
}

fn example(name: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    root.join(name).to_string_lossy().into_owned()
}

fn json(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "{e}: {}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

/// A scratch file in the target directory, unique to the test.
fn scratch(name: &str, text: &str) -> String {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn describe_lists_every_type_and_describes_one() {
    let out = simsandbox(&["describe"]);
    assert!(out.status.success());
    let all = json(&out);
    let names: Vec<&str> = all["block_types"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["type"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"eq.equations") && names.contains(&"agents.grazers"));
    assert!(all["view_types"].as_array().unwrap().len() >= 8);

    let one = json(&simsandbox(&["describe", "eq.lorenz"]));
    assert_eq!(one["interface"]["ports"][0]["name"], "xyz");
    let view = json(&simsandbox(&["describe", "plot.heatmap"]));
    assert_eq!(view["type"], "plot.heatmap");
    let unknown = simsandbox(&["describe", "eq.nothing"]);
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("known: "));
}

#[test]
fn schema_is_json() {
    let schema = json(&simsandbox(&["schema"]));
    assert!(schema["properties"]["blocks"].is_object());
}

#[test]
fn check_reports_problems_with_where_they_are_and_fails() {
    let ok = simsandbox(&["check", &example("grazing.ssbx.json")]);
    assert!(ok.status.success());
    let report = json(&ok);
    assert_eq!(report["ok"], true);
    assert!(report["recorded"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f == "sheep.position"));

    let bad = scratch(
        "bad-method.json",
        r#"{"format": 1, "blocks": [{"name": "a", "type": "eq.lorenz", "method": {"name": "leapfrog"}}]}"#,
    );
    let out = simsandbox(&["check", &bad]);
    assert!(!out.status.success());
    let report = json(&out);
    assert_eq!(report["ok"], false);
    assert_eq!(report["errors"][0]["path"], "blocks[0].method");
}

#[test]
fn run_writes_csv_or_json_with_populations() {
    let csv = simsandbox(&["run", &example("lorenz.ssbx.json"), "--t-end", "0.1"]);
    assert!(csv.status.success());
    let text = String::from_utf8(csv.stdout).unwrap();
    assert_eq!(text.lines().count(), 1 + 11);

    let out = simsandbox(&[
        "run",
        &example("grazing.ssbx.json"),
        "--t-end",
        "1",
        "--format",
        "json",
    ]);
    let run = json(&out);
    let rows = run["rows"].as_u64().unwrap() as usize;
    assert_eq!(run["index"]["t"].as_array().unwrap().len(), rows);
    let grass = &run["outputs"]["grass.grass"];
    assert_eq!(grass["shape"]["nx"], 40);
    assert_eq!(grass["values"][0].as_array().unwrap().len(), 1600);
    let count = &run["outputs"]["sheep.count"]["values"];
    let positions = &run["populations"]["sheep.position"];
    assert_eq!(positions["dim"], 2);
    for k in 0..rows {
        let n = positions["frames"][k].as_array().unwrap().len();
        assert_eq!(n as f64, count[k].as_f64().unwrap());
    }

    // A sweep: one row per sample, indexed by the swept parameter.
    let swept = json(&simsandbox(&[
        "run",
        &example("resonance.ssbx.json"),
        "--format",
        "json",
    ]));
    assert!(swept["index"]["spring.w"].is_array());
    assert_eq!(
        swept["index"]["sample"].as_array().unwrap().len(),
        swept["rows"].as_u64().unwrap() as usize
    );
}

#[test]
fn render_writes_a_png_of_the_views() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let png = dir.join("pendulum.png");
    let _ = std::fs::remove_file(&png);
    let out = simsandbox(&[
        "render",
        &example("pendulum.ssbx.json"),
        "--t-end",
        "2",
        "--out",
        &png.to_string_lossy(),
        "--width",
        "320",
        "--height",
        "200",
        "--scale",
        "2",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let bytes = std::fs::read(&png).unwrap();
    assert_eq!(&bytes[1..4], b"PNG");
    // The IHDR chunk: width and height, big-endian, in pixels.
    let size = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    assert_eq!((size(16), size(20)), (640, 400));

    let missing = simsandbox(&["render", &example("pendulum.ssbx.json")]);
    assert!(String::from_utf8_lossy(&missing.stderr).contains("--out"));
}

#[test]
fn the_diagram_renders_and_start_from_writes_a_file_that_starts_there() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let png = dir.join("cruise-diagram.png");
    let out = simsandbox(&[
        "render",
        &example("cruise-control.ssbx.json"),
        "--diagram",
        "--out",
        &png.to_string_lossy(),
        "--width",
        "400",
        "--height",
        "300",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(&std::fs::read(&png).unwrap()[1..4], b"PNG");

    let started = dir.join("cruise-at-30.json");
    let out = simsandbox(&[
        "start-from",
        &example("cruise-control.ssbx.json"),
        "--t",
        "30",
        "--out",
        &started.to_string_lossy(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("v0 = "));
    let first = json(&simsandbox(&[
        "run",
        &started.to_string_lossy(),
        "--t-end",
        "0.1",
        "--format",
        "json",
    ]));
    let before = json(&simsandbox(&[
        "run",
        &example("cruise-control.ssbx.json"),
        "--t-end",
        "30",
        "--format",
        "json",
    ]));
    let v = |r: &Value, k: usize| r["outputs"]["car.v"]["values"][k].as_f64().unwrap();
    let last = before["rows"].as_u64().unwrap() as usize - 1;
    assert_eq!(v(&first, 0), v(&before, last));
}

#[test]
fn upgrade_writes_the_current_form_of_an_older_file() {
    let old = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../simsandbox-file/tests/old/adr-0014-single-view.json");
    let out = simsandbox(&["upgrade", &old.to_string_lossy()]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("converted: controls.view"));
    let current = std::fs::read_to_string(old.with_extension("current.json")).unwrap();
    assert_eq!(String::from_utf8(out.stdout).unwrap(), current);
    let report = json(&simsandbox(&["check", &old.to_string_lossy()]));
    assert!(!report["conversions"].as_array().unwrap().is_empty());
}
