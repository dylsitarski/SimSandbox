//! Run files as every interface uses them (ADR 0059): finding a run's
//! model beside it, the model text and recorded input to replay, checking
//! a run against its model, and the runs kept beside a model. The format
//! itself is [`simsandbox_file::runs`].

use crate::session::InputEvent;
use crate::{describe, Session};
use serde_json::Value;
use simsandbox_core::registry::Registry;
use simsandbox_file::runs::run_file_name;
pub use simsandbox_file::runs::{version_note, RunFile};
use simsandbox_file::ModelFile;
use std::fmt;
use std::path::{Path, PathBuf};

/// Whether `path` names a run file.
pub fn is_run_file(path: &str) -> bool {
    path.ends_with(".run.json")
}

/// A run's recorded track as the input a session replays.
pub fn events(track: &[(f64, String, f64)]) -> Vec<InputEvent> {
    track
        .iter()
        .map(|(t, key, value)| InputEvent {
            t: *t,
            key: key.clone(),
            value: *value,
        })
        .collect()
}

/// Where the model a run names is: relative to the run file.
pub fn model_beside(run_path: &Path, run: &RunFile) -> PathBuf {
    run_path.parent().unwrap_or(Path::new("")).join(&run.model)
}

/// Why a run file could not be read. The interfaces word the model's case
/// differently, so the parts are kept apart; `Display` is the CLI's.
#[derive(Debug, Clone, PartialEq)]
pub enum ReadError {
    /// The run file itself: unreadable or not a run file.
    Run { path: PathBuf, message: String },
    /// The model it names cannot be read.
    Model {
        run: PathBuf,
        model: PathBuf,
        message: String,
    },
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Run { path, message } => write!(f, "{}: {message}", path.display()),
            Self::Model {
                run,
                model,
                message,
            } => write!(
                f,
                "{}: its model: {}: {message}",
                run.display(),
                model.display()
            ),
        }
    }
}

/// A run file read with the model it names.
#[derive(Debug, Clone)]
pub struct Opened {
    pub run: RunFile,
    /// The model file, beside the run file.
    pub model_path: PathBuf,
    /// The model file's text, as saved.
    pub text: String,
}

impl Opened {
    /// The recorded input to replay.
    pub fn track(&self) -> Vec<InputEvent> {
        events(&self.run.track)
    }
}

/// Read a run file and the text of the model it names.
pub fn read_run(path: &Path) -> Result<Opened, ReadError> {
    let run = std::fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|t| RunFile::from_json(&t).map_err(|e| e.to_string()))
        .map_err(|message| ReadError::Run {
            path: path.to_path_buf(),
            message,
        })?;
    let model_path = model_beside(path, &run);
    let text = std::fs::read_to_string(&model_path).map_err(|e| ReadError::Model {
        run: path.to_path_buf(),
        model: model_path.clone(),
        message: e.to_string(),
    })?;
    Ok(Opened {
        run,
        model_path,
        text,
    })
}

/// Whether the model saved at `model` is the one `run` was made with.
/// A model that cannot be read or loaded is not.
pub fn fits_disk(run: &RunFile, model: &Path, registry: &Registry) -> bool {
    std::fs::read_to_string(model)
        .ok()
        .and_then(|t| simsandbox_file::load_any(&t, registry, model.parent()).ok())
        .and_then(|l| run.fits(&l).ok())
        .unwrap_or(false)
}

/// What a headless command runs: the model text, the recorded input to
/// replay with it, and warnings about the run file it came from.
#[derive(Debug, Clone, Default)]
pub struct Replay {
    pub text: String,
    pub track: Vec<InputEvent>,
    /// Each starts with the run file's path.
    pub warnings: Vec<String>,
}

/// The model text to run and the recorded input to replay with it: a run
/// file's model with the run's seed, settings and schedule (warning if the
/// model changed since the run was made, or another version made it); or a
/// model file, with `inputs` (a run file) applying that run's schedule and
/// track.
pub fn model_and_track(
    path: &str,
    inputs: Option<&str>,
    registry: &Registry,
) -> Result<Replay, String> {
    let read = |p: &str| std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"));
    if is_run_file(path) {
        if inputs.is_some() {
            return Err("--inputs applies a run to a model file; this is a run file".into());
        }
        let opened = read_run(Path::new(path)).map_err(|e| e.to_string())?;
        let model_path = opened.model_path.display();
        let (file, _) =
            ModelFile::from_json(&opened.text).map_err(|e| format!("{model_path}: {e}"))?;
        let loaded = simsandbox_file::load_any(&opened.text, registry, opened.model_path.parent())
            .map_err(|e| format!("{model_path}: {e}"))?;
        let mut warnings = Vec::new();
        if !opened.run.fits(&loaded).map_err(|e| e.to_string())? {
            warnings.push(format!(
                "{path}: {model_path} has changed since this run was recorded; \
                 replaying it anyway"
            ));
        }
        if let Some(note) = opened.run.version_note() {
            warnings.push(format!("{path}: {note}"));
        }
        return Ok(Replay {
            text: opened.run.apply_to(&file).to_json(),
            track: opened.track(),
            warnings,
        });
    }
    let text = read(path)?;
    let Some(run_path) = inputs else {
        return Ok(Replay {
            text,
            ..Replay::default()
        });
    };
    let run = RunFile::from_json(&read(run_path)?).map_err(|e| format!("{run_path}: {e}"))?;
    let (mut file, _) = ModelFile::from_json(&text).map_err(|e| format!("{path}: {e}"))?;
    file.run.inputs = run.inputs.clone();
    Ok(Replay {
        text: file.to_json(),
        track: events(&run.track),
        warnings: Vec::new(),
    })
}

/// Check a run file, as [`describe::check`] reports: its model is found
/// and checked with the run's settings and schedule, the recorded input
/// names parameters that can be set, and a model changed since the run was
/// made, or a run another version made, is a warning.
pub fn check_run(path: &str, registry: &Registry) -> Value {
    let fail = |message: String| serde_json::json!({"ok": false, "errors": [{"message": message}], "warnings": []});
    let opened = match read_run(Path::new(path)) {
        Ok(o) => o,
        Err(e) => return fail(e.to_string()),
    };
    let model_path = opened.model_path.display();
    let run = &opened.run;
    let (file, _) = match ModelFile::from_json(&opened.text) {
        Ok(f) => f,
        Err(e) => return fail(format!("{model_path}: {e}")),
    };
    let replay = run.apply_to(&file).to_json();
    let mut report = describe::check(&replay, registry);
    if report["ok"] != true {
        return report;
    }
    let mut warnings: Vec<Value> = report["warnings"].as_array().cloned().unwrap_or_default();
    // Files are found from the thread's folder, which the caller sets.
    if let Ok(loaded) = simsandbox_file::load_any(&opened.text, registry, None) {
        if !run.fits(&loaded).unwrap_or(false) {
            warnings.push(Value::from(format!(
                "{model_path} has changed since this run was recorded"
            )));
        }
        if let Some(note) = run.version_note() {
            warnings.push(Value::from(note));
        }
    }
    match Session::load(path, &replay, registry) {
        Ok(mut s) => {
            if let Err(e) = s.replay(opened.track()) {
                return fail(e);
            }
            warnings.extend(s.warnings.iter().cloned().map(Value::from));
        }
        Err(e) => return fail(e),
    }
    report["warnings"] = Value::from(warnings);
    report
}

/// The run files beside `model` that name it, by name.
pub fn runs_beside(model: &Path) -> Vec<(PathBuf, RunFile)> {
    let Some(model_name) = model.file_name().map(|n| n.to_string_lossy().into_owned()) else {
        return Vec::new();
    };
    let dir = match model.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut runs: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_run_file(&p.to_string_lossy()))
        .filter_map(|p| {
            let text = std::fs::read_to_string(&p).ok()?;
            let run = RunFile::from_json(&text).ok()?;
            (run.model == model_name).then_some((p, run))
        })
        .collect();
    runs.sort_by(|a, b| a.1.name.cmp(&b.1.name).then(a.0.cmp(&b.0)));
    runs
}

/// Where a new run called `name` goes beside `model`: its usual name
/// ([`run_file_name`]), numbered from 2 if that is taken.
pub fn new_run_path(model: &Path, name: &str) -> PathBuf {
    let model_name = model
        .file_name()
        .map_or(String::new(), |n| n.to_string_lossy().into_owned());
    let dir = model.parent().unwrap_or(Path::new(""));
    let file = run_file_name(&model_name, name);
    let mut path = dir.join(&file);
    let mut k = 2;
    while path.exists() {
        let stem = file.trim_end_matches(".run.json");
        path = dir.join(format!("{stem}-{k}.run.json"));
        k += 1;
    }
    path
}

/// Write a run file. Errors name the path.
pub fn write_run(path: &Path, run: &RunFile) -> Result<(), String> {
    std::fs::write(path, run.to_json()).map_err(|e| format!("{}: {e}", path.display()))
}

/// Now as RFC 3339 in UTC, to the second: a run's `created`.
pub fn utc_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()) as i64;
    rfc3339(secs)
}

/// Seconds since 1970-01-01T00:00:00Z as RFC 3339 in UTC.
fn rfc3339(secs: i64) -> String {
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::examples::registry;

    const MODEL: &str = r#"{"format": 1, "blocks": [
        {"name": "push", "type": "input.value", "settings": {"unit": "N"}}],
        "run": {"dt": 0.1, "t_end": 1.0}}"#;

    /// A fresh folder with the model saved in it, and a run made of it.
    fn saved(test: &str) -> (PathBuf, PathBuf, RunFile) {
        let dir = std::env::temp_dir().join(format!("ssbx-runs-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let model = dir.join("push.ssbx.json");
        std::fs::write(&model, MODEL).unwrap();
        let mut s = Session::load_in("push", MODEL, &registry(), Some(&dir)).unwrap();
        s.replay(vec![InputEvent {
            t: 0.5,
            key: "push.y".into(),
            value: 0.5,
        }])
        .unwrap();
        s.run_until(1.0).unwrap();
        let run = s.run_file("push.ssbx.json", "Push test", "", "2026-11-14T10:00:00Z");
        (dir, model, run)
    }

    #[test]
    fn a_run_is_written_beside_its_model_and_read_back_with_it() {
        let (dir, model, run) = saved("round-trip");
        assert_eq!(run.track.len(), 1);
        let path = new_run_path(&model, &run.name);
        assert_eq!(path, dir.join("push.push-test.run.json"));
        write_run(&path, &run).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), run.to_json());
        // A taken name is numbered, not overwritten.
        let second = new_run_path(&model, &run.name);
        assert_eq!(second, dir.join("push.push-test-2.run.json"));
        write_run(&second, &run).unwrap();
        assert!(new_run_path(&model, &run.name).ends_with("push.push-test-3.run.json"));

        let opened = read_run(&path).unwrap();
        assert_eq!(opened.run, run);
        assert_eq!(
            (opened.model_path.as_path(), opened.text.as_str()),
            (model.as_path(), MODEL)
        );
        assert_eq!(opened.track(), events(&run.track));
        assert!(fits_disk(&run, &model, &registry()));

        // Runs of another model, and files that are not runs, are left out.
        let mut other = run.clone();
        other.model = "other.ssbx.json".into();
        write_run(&dir.join("other.a.run.json"), &other).unwrap();
        std::fs::write(dir.join("broken.run.json"), "{").unwrap();
        let beside: Vec<PathBuf> = runs_beside(&model).into_iter().map(|(p, _)| p).collect();
        // Both are called "Push test": then by path.
        assert_eq!(beside, [second, path]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_run_replays_with_its_settings_and_warns_when_its_model_changed() {
        let (dir, model, mut run) = saved("replay");
        run.seed = Some(7);
        let path = dir.join("push.a.run.json");
        write_run(&path, &run).unwrap();
        let (p, m) = (path.to_str().unwrap(), model.to_str().unwrap());

        let r = model_and_track(p, None, &registry()).unwrap();
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert_eq!(r.track, events(&run.track));
        let (file, _) = ModelFile::from_json(&r.text).unwrap();
        assert_eq!(file.run.seed, Some(7));
        let report = check_run(p, &registry());
        assert_eq!(report["ok"], true, "{report}");
        assert_eq!(report["warnings"], serde_json::json!([]));

        // A model file alone, or with a run's schedule and track applied.
        let plain = model_and_track(m, None, &registry()).unwrap();
        assert_eq!((plain.text.as_str(), plain.track.len()), (MODEL, 0));
        let with = model_and_track(m, Some(p), &registry()).unwrap();
        assert_eq!(with.track, events(&run.track));
        let e = model_and_track(p, Some(p), &registry()).unwrap_err();
        assert!(e.contains("this is a run file"), "{e}");

        // Another version made it: a note, replayed all the same.
        let mut older = run.clone();
        older.made_with = "simsandbox 0.0.0".into();
        write_run(&path, &older).unwrap();
        let r = model_and_track(p, None, &registry()).unwrap();
        assert_eq!(r.warnings.len(), 1);
        assert!(
            r.warnings[0].starts_with(&format!("{p}: recorded with simsandbox 0.0.0")),
            "{:?}",
            r.warnings
        );
        let note = older.version_note().unwrap();
        assert_eq!(
            check_run(p, &registry())["warnings"],
            serde_json::json!([note])
        );

        // The model changed since: both say so, each in its words.
        write_run(&path, &run).unwrap();
        let changed = MODEL.replace(r#""unit": "N""#, r#""unit": "m""#);
        std::fs::write(&model, changed).unwrap();
        assert!(!fits_disk(&run, &model, &registry()));
        let r = model_and_track(p, None, &registry()).unwrap();
        assert_eq!(
            r.warnings,
            [format!(
                "{p}: {m} has changed since this run was recorded; replaying it anyway"
            )]
        );
        assert_eq!(
            check_run(p, &registry())["warnings"],
            serde_json::json!([format!("{m} has changed since this run was recorded")])
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_run_that_cannot_be_read_says_which_file() {
        let (dir, model, run) = saved("errors");
        let path = dir.join("push.a.run.json");
        let p = path.to_str().unwrap();
        // No run file.
        let e = read_run(&path).unwrap_err();
        assert!(matches!(e, ReadError::Run { .. }));
        assert!(e.to_string().starts_with(&format!("{p}: ")), "{e}");
        // Its model is gone.
        write_run(&path, &run).unwrap();
        std::fs::remove_file(&model).unwrap();
        let e = read_run(&path).unwrap_err();
        assert!(matches!(&e, ReadError::Model { model: m, .. } if *m == model));
        let text = e.to_string();
        assert!(
            text.starts_with(&format!("{p}: its model: {}: ", model.display())),
            "{text}"
        );
        assert_eq!(model_and_track(p, None, &registry()).unwrap_err(), text);
        let report = check_run(p, &registry());
        assert_eq!(report["ok"], false);
        assert_eq!(report["errors"][0]["message"], text.as_str());
        assert!(!fits_disk(&run, &model, &registry()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn times_are_rfc_3339_in_utc() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        // A leap day and the second before a new year.
        assert_eq!(rfc3339(1_709_210_096), "2024-02-29T12:34:56Z");
        assert_eq!(rfc3339(1_798_761_599), "2026-12-31T23:59:59Z");
        assert!(utc_now().ends_with('Z') && utc_now().len() == 20);
    }
}
