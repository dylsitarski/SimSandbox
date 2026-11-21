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

    // A model whose only runs are its study's: one row per run (ADR 0071).
    let studied = json(&simsandbox(&[
        "run",
        &example("resonance.ssbx.json"),
        "--format",
        "json",
    ]));
    assert!(studied["outputs"]["curve.w"]["values"].is_array());
    assert_eq!(
        studied["index"]["run"].as_array().unwrap().len(),
        studied["rows"].as_u64().unwrap() as usize
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

/// Standard error's lines, each one JSON object (`--errors json`).
fn json_lines(out: &Output) -> Vec<Value> {
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}")))
        .collect()
}

#[test]
fn help_goes_to_standard_output_and_succeeds() {
    for flag in ["--help", "-h", "help"] {
        let out = simsandbox(&[flag]);
        assert_eq!(out.status.code(), Some(0), "{flag}");
        let text = String::from_utf8(out.stdout).unwrap();
        assert!(text.starts_with("usage: simsandbox"), "{flag}: {text}");
        assert!(text.contains("start-from FILE") && text.contains("Exit status"));
        assert!(out.stderr.is_empty());
    }
    // Each command's own help, two ways.
    for args in [
        &["run", "--help"][..],
        &["help", "run"],
        &["run", "FILE", "-h"],
    ] {
        let out = simsandbox(args);
        assert_eq!(out.status.code(), Some(0), "{args:?}");
        let text = String::from_utf8(out.stdout).unwrap();
        assert!(text.starts_with("usage: simsandbox run FILE"), "{text}");
        assert!(text.contains("--save-probes DIR"));
    }
    for command in [
        "check",
        "describe",
        "schema",
        "render",
        "start-from",
        "upgrade",
    ] {
        let out = simsandbox(&[command, "--help"]);
        assert_eq!(out.status.code(), Some(0), "{command}");
        let text = String::from_utf8(out.stdout).unwrap();
        assert!(
            text.starts_with(&format!("usage: simsandbox {command}")),
            "{text}"
        );
    }
    // No arguments: the usage on standard error, as a usage error.
    let none = simsandbox(&[]);
    assert_eq!(none.status.code(), Some(2));
    assert!(none.stdout.is_empty());
    assert!(String::from_utf8_lossy(&none.stderr).starts_with("usage: simsandbox"));
    // The leftover `lorenz` command is gone.
    assert_eq!(simsandbox(&["lorenz"]).status.code(), Some(2));
}

#[test]
fn exit_status_says_how_a_command_failed() {
    // 2: usage errors.
    let usage = [
        &["frobnicate"][..],
        &["run"],
        &["run", &example("lorenz.ssbx.json"), "--bogus", "1"],
        &["run", &example("lorenz.ssbx.json"), "--t-end", "-1"],
        &["run", &example("lorenz.ssbx.json"), "--format", "xml"],
        &["run", &example("lorenz.ssbx.json"), "--t-end"],
        &["check"],
        &["check", &example("lorenz.ssbx.json"), "extra"],
        &["describe", "eq.nothing"],
        &["schema", "model"],
        &["render", &example("pendulum.ssbx.json")],
        &["start-from", &example("pendulum.ssbx.json")],
        &["help", "nothing"],
        &["--errors", "yaml", "describe"],
    ];
    for args in usage {
        let out = simsandbox(args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.starts_with("error: ") && err.contains("usage: simsandbox"),
            "{err}"
        );
    }
    // 3: a file that cannot be read or written.
    let missing = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("no-such-model.json");
    let missing = missing.to_string_lossy();
    for command in ["run", "check", "upgrade"] {
        let out = simsandbox(&[command, &missing]);
        assert_eq!(out.status.code(), Some(3), "{command}");
        assert!(out.stdout.is_empty());
    }
    let out = simsandbox(&["start-from", &missing, "--t", "1"]);
    assert_eq!(out.status.code(), Some(3));
    let unwritable = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("no-such-dir/out.json");
    let out = simsandbox(&[
        "upgrade",
        &example("lorenz.ssbx.json"),
        "--out",
        &unwritable.to_string_lossy(),
    ]);
    assert_eq!(out.status.code(), Some(3));
    // 1: the model cannot run.
    let bad = scratch(
        "bad-type.json",
        r#"{"format": 1, "blocks": [{"name": "a", "type": "eq.nothing"}]}"#,
    );
    assert_eq!(simsandbox(&["check", &bad]).status.code(), Some(1));
    assert_eq!(simsandbox(&["run", &bad]).status.code(), Some(1));
}

#[test]
fn errors_json_writes_one_object_per_line() {
    // A usage error, the option anywhere on the line.
    for args in [
        &["--errors", "json", "run", "--nope"][..],
        &["run", "--nope", "--errors", "json"],
        &["--errors=json", "run", "--nope"],
    ] {
        let out = simsandbox(args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        let lines = json_lines(&out);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(lines[0]["error"]["kind"], "usage");
        assert!(lines[0]["error"]["usage"]
            .as_str()
            .unwrap()
            .starts_with("usage: simsandbox run"));
    }
    let none = simsandbox(&["--errors", "json"]);
    assert_eq!(none.status.code(), Some(2));
    assert_eq!(json_lines(&none)[0]["error"]["kind"], "usage");

    // An input/output error names the file.
    let missing = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("no-such-run.run.json");
    let missing = missing.to_string_lossy();
    let out = simsandbox(&["--errors", "json", "run", &missing]);
    assert_eq!(out.status.code(), Some(3));
    let e = &json_lines(&out)[0]["error"];
    assert_eq!(
        (e["kind"].as_str(), e["file"].as_str()),
        (Some("io"), Some(&*missing))
    );

    // A model that cannot run: where each error is, from `check`.
    let bad = scratch(
        "bad-method-json.json",
        r#"{"format": 1, "blocks": [{"name": "a", "type": "eq.lorenz", "method": {"name": "leapfrog"}}]}"#,
    );
    for command in ["run", "check"] {
        let out = simsandbox(&["--errors", "json", command, &bad]);
        assert_eq!(out.status.code(), Some(1), "{command}");
        let lines = json_lines(&out);
        assert_eq!(lines.len(), 1);
        let e = &lines[0]["error"];
        assert_eq!(e["kind"], "model");
        assert_eq!(e["file"], bad.as_str());
        assert_eq!(e["errors"][0]["path"], "blocks[0].method", "{command}: {e}");
    }

    // Labels, notes and warnings: one object each.
    let out = simsandbox(&[
        "run",
        &example("lorenz.ssbx.json"),
        "--t-end",
        "0.1",
        "--errors",
        "json",
    ]);
    assert_eq!(out.status.code(), Some(0));
    let lines = json_lines(&out);
    assert!(
        lines.iter().any(|l| l["labels"]["speed"].is_string()),
        "{lines:?}"
    );
    let text = simsandbox(&["run", &example("lorenz.ssbx.json"), "--t-end", "0.1"]);
    assert_eq!(out.stdout, text.stdout);
    assert!(String::from_utf8_lossy(&text.stderr).starts_with("speed: "));

    let old = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../simsandbox-file/tests/old/adr-0014-single-view.json");
    let out = simsandbox(&["--errors", "json", "upgrade", &old.to_string_lossy()]);
    assert_eq!(out.status.code(), Some(0));
    let lines = json_lines(&out);
    assert!(lines.iter().any(|l| l["converted"]["message"]
        .as_str()
        .is_some_and(|m| m.starts_with("controls.view"))));
}

#[test]
fn check_reports_every_block_and_wire_error() {
    let bad = scratch(
        "several-errors.json",
        r#"{"format": 1, "blocks": [
            {"name": "a", "type": "eq.lorenz", "method": {"name": "leapfrog"}},
            {"name": "b", "type": "eq.nothing"},
            {"name": "c", "type": "eq.coupled_oscillator"},
            {"name": "d", "type": "eq.equations", "equations": ["x' = = 1"]}],
          "connections": [
            {"from": "a.xyz", "to": "c.neighbor_x"},
            {"from": "c.x", "to": "c.nope"}]}"#,
    );
    let out = simsandbox(&["check", &bad]);
    assert_eq!(out.status.code(), Some(1));
    let report = json(&out);
    assert_eq!(report["ok"], false);
    let paths: Vec<&str> = report["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["path"].as_str().unwrap())
        .collect();
    // The wire from the failed block `a` is not checked.
    assert_eq!(
        paths,
        [
            "blocks[0].method",
            "blocks[1].type",
            "blocks[3].equations",
            "connections[1]"
        ]
    );
}

/// A run file for `model` with these recorded events (`[t, key, value]`).
fn run_file(name: &str, model: &str, track: Value) -> String {
    let run = serde_json::json!({
        "format": 1, "model": model, "model_hash": "made by hand", "track": track
    });
    scratch(name, &run.to_string())
}

/// One output of `run --format json`, as bits.
fn bits(out: &Output, name: &str) -> Vec<u64> {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    json(out)["outputs"][name]["values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap().to_bits())
        .collect()
}

#[test]
fn a_grab_in_a_run_file_replays_headless_and_older_runs_never_pick() {
    // ADR 0093 point 6: a scene's grab is recorded as its input.pointer's
    // x, y, down and picked; headless, a run file gives the same events.
    let model = example("pendulum-grab.ssbx.json");
    let grab = run_file(
        "grab.run.json",
        &model,
        serde_json::json!([
            [0.2, "hand.x", 1.0],
            [0.2, "hand.y", 0.0],
            [0.2, "hand.down", 1.0],
            [0.2, "hand.picked", 0.0],
            [0.6, "hand.down", 0.0],
            [0.6, "hand.picked", -1.0]
        ]),
    );
    let run = |extra: &[&str]| {
        let mut args = vec!["run", &model, "--t-end", "1", "--format", "json"];
        args.extend_from_slice(extra);
        simsandbox(&args)
    };
    let alone = run(&[]);
    let grabbed = run(&["--inputs", &grab]);
    assert_ne!(bits(&grabbed, "bob.theta"), bits(&alone, "bob.theta"));
    // The run file itself replays the same, bit for bit.
    let itself = simsandbox(&["run", &grab, "--t-end", "1", "--format", "json"]);
    assert_eq!(bits(&itself, "bob.theta"), bits(&grabbed, "bob.theta"));
    // Held exactly from the first step at or after 0.2 to the one at 0.6.
    let out = json(&grabbed);
    let t: Vec<f64> = out["index"]["t"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    let held = &out["outputs"]["bob.held"]["values"];
    for (k, t) in t.iter().enumerate() {
        let want = if (0.2 + 1e-9..0.6 + 1e-9).contains(t) {
            1.0
        } else {
            0.0
        };
        assert_eq!(held[k].as_f64(), Some(want), "t = {t}");
    }
    // A run made before `picked` (x, y and down only) reads it as -1
    // throughout: pressed, but on nothing, so the model does not pull.
    let older = run_file(
        "older.run.json",
        &model,
        serde_json::json!([
            [0.2, "hand.x", 1.0],
            [0.2, "hand.down", 1.0],
            [0.6, "hand.down", 0.0]
        ]),
    );
    let older = run(&["--inputs", &older]);
    let picked = &json(&older)["outputs"]["hand.picked"]["values"];
    assert!(picked.as_array().unwrap().iter().all(|v| v == -1.0));
    assert_eq!(bits(&older, "bob.theta"), bits(&alone, "bob.theta"));
}

#[test]
fn check_refuses_a_scene_layer_grabbing_with_no_input_pointer() {
    let text = std::fs::read_to_string(example("pendulum-grab.ssbx.json"))
        .unwrap()
        .replace("\"grab\": \"hand\"", "\"grab\": \"bob\"");
    let out = simsandbox(&["check", &scratch("grab-bob.json", &text)]);
    let report = json(&out);
    let warnings = report["warnings"].as_array().unwrap();
    assert!(
        warnings.iter().any(|w| w
            == "controls.views[0]: layers[3].grab: no input.pointer block 'bob' (grabbing sends the pointer to one); not drawn"),
        "{warnings:?}"
    );
    // `describe` gives the pointer's fourth output.
    let d = json(&simsandbox(&["describe", "input.pointer"]));
    let outputs: Vec<&str> = d["interface"]["ports"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(outputs, ["x", "y", "down", "picked"]);
    let picked = &d["interface"]["params"][3];
    assert_eq!(
        (picked["name"].as_str(), picked["default"].as_f64()),
        (Some("picked"), Some(-1.0))
    );
}

#[test]
fn the_cartpole_balances_and_a_grab_replays_headless() {
    // ADR 0103 and ADR 0105: the mechanism with a typed controller, one
    // fused ODE; a grab of the pole from a run file, replayed bit for bit.
    let model = example("cartpole.ssbx.json");
    let report = json(&simsandbox(&["check", &model]));
    assert_eq!(report["ok"], true, "{report}");
    let groups = report["fused_groups"].as_array().unwrap();
    assert!(groups.iter().any(|g| g["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|b| b == "cartpole")
        && g["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b == "control")));
    let grab = run_file(
        "cartpole-grab.run.json",
        &model,
        serde_json::json!([
            [0.5, "hand.x", 0.6],
            [0.5, "hand.y", 0.6],
            [0.5, "hand.down", 1.0],
            [0.5, "hand.picked", 0.0],
            [1.0, "hand.down", 0.0],
            [1.0, "hand.picked", -1.0]
        ]),
    );
    let run = |extra: &[&str]| {
        let mut args = vec!["run", &model, "--t-end", "6", "--format", "json"];
        args.extend_from_slice(extra);
        simsandbox(&args)
    };
    let alone = run(&[]);
    let grabbed = run(&["--inputs", &grab]);
    let angle = |out: &Output| -> Vec<f64> {
        json(out)["outputs"]["cartpole.hinge_angle"]["values"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect()
    };
    let (a, g) = (angle(&alone), angle(&grabbed));
    assert_ne!(a, g, "the grab moves the pole");
    // Pulled over by the hand, then caught: upright again by 6 s.
    let pulled = g.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    assert!(pulled > 0.15, "{pulled}");
    assert!(a.last().unwrap().abs() < 1e-3 && g.last().unwrap().abs() < 1e-3);
    let itself = simsandbox(&["run", &grab, "--t-end", "6", "--format", "json"]);
    assert_eq!(
        bits(&itself, "cartpole.hinge_angle"),
        bits(&grabbed, "cartpole.hinge_angle")
    );
}

#[test]
fn a_mechanism_is_described_and_its_rows_are_checked_at_their_path() {
    let d = json(&simsandbox(&["describe", "mech.planar"]));
    let text = d.to_string();
    assert!(
        text.contains("<joint>_angle0"),
        "the reference is given whole"
    );
    let model = std::fs::read_to_string(example("cartpole.ssbx.json"))
        .unwrap()
        .replace("\"parent\": \"cart\"", "\"parent\": \"carriage\"");
    let out = simsandbox(&["check", &scratch("cartpole-bad.json", &model)]);
    let report = json(&out);
    assert_eq!(report["ok"], false);
    assert!(report.to_string().contains("bodies[1].parent"), "{report}");
    // Only a mechanism has bodies.
    let wrong = std::fs::read_to_string(example("cartpole.ssbx.json"))
        .unwrap()
        .replace("\"type\": \"mech.planar\"", "\"type\": \"eq.lorenz\"");
    let out = simsandbox(&["check", &scratch("lorenz-bodies.json", &wrong)]);
    assert!(
        json(&out).to_string().contains("blocks[1].bodies"),
        "{}",
        json(&out)
    );
}

#[test]
fn explain_prints_the_models_card() {
    // ADR 0107: generated from the file; Markdown by default, JSON on ask.
    let model = example("cartpole.ssbx.json");
    let out = simsandbox(&["explain", &model]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    for wanted in [
        "# cartpole",
        "## Key parameters (pinned by the model)",
        "### control — controller (`eq.equations`)",
        "| `k1` | -10.0 N/m | tuning | gain on the cart's position |",
        "- control → cartpole: F → slider_force, fx → pole_fx, fy → pole_fy",
    ] {
        assert!(text.contains(wanted), "lacks {wanted:?}:\n{text}");
    }
    let card = json(&simsandbox(&["explain", &model, "--format", "json"]));
    assert_eq!(card["parts"][1]["role"], "system");
    // A model that cannot load fails as check does; a bad format is usage.
    let bad = scratch(
        "explain-bad.json",
        r#"{"format": 1, "blocks": [{"name": "a", "type": "eq.nope"}]}"#,
    );
    assert_eq!(simsandbox(&["explain", &bad]).status.code(), Some(1));
    assert_eq!(
        simsandbox(&["explain", &model, "--format", "html"])
            .status
            .code(),
        Some(2)
    );
}
