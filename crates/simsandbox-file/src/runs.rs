//! Run files (ADR 0059): one recorded or scripted run of a model, kept
//! beside it as `<model>.<run>.run.json`. A run holds the events applied in
//! it (the schedule in effect and the recorded live input, ADR 0058), the
//! seed and settings used, and which model it was made with: a path and a
//! hash of the model as saved, `controls` left out, so rearranging panes
//! does not make a run stale. Results are not stored; replaying the run
//! reproduces them.

use crate::{FileError, Loaded, ModelFile, ScheduledChange};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The run file format this build reads and writes.
pub const RUN_FORMAT: u32 = 1;
/// The run file's JSON Schema, for `simsandbox schema run`.
pub const RUN_SCHEMA: &str = include_str!("../run.schema.json");

/// The run settings a run used, in place of its model's.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunOverrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dt: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t_end: Option<f64>,
}

/// One run of a model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunFile {
    pub format: u32,
    /// The model file, relative to the run file.
    pub model: String,
    /// [`model_hash`] of the model the run was made with.
    pub model_hash: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// When it was saved, as RFC 3339 (UTC).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub created: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<RunOverrides>,
    /// The schedule in effect, copied from the model or written by hand.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<ScheduledChange>,
    /// Recorded live input: `[t, key, value]` in the order it happened.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub track: Vec<(f64, String, f64)>,
}

impl RunFile {
    /// Read a run file. Unknown fields and a newer format are refused,
    /// as for model files (ADR 0044).
    pub fn from_json(text: &str) -> Result<Self, FileError> {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
            if let Some(n) = v.get("format").and_then(serde_json::Value::as_u64) {
                if n > RUN_FORMAT as u64 {
                    return Err(FileError::UnsupportedFormat(n as u32));
                }
            }
        }
        let run: RunFile = serde_json::from_str(text).map_err(|e| FileError::Syntax {
            line: e.line(),
            column: e.column(),
            message: e.to_string(),
        })?;
        if run.format != RUN_FORMAT {
            return Err(FileError::UnsupportedFormat(run.format));
        }
        let bad = |path: String, message: String| FileError::Invalid { path, message };
        for (k, c) in run.inputs.iter().enumerate() {
            if !(c.t.is_finite() && c.t >= 0.0 && c.to.is_finite()) {
                return Err(bad(
                    format!("inputs[{k}]"),
                    "times are 0 or more and values finite".into(),
                ));
            }
        }
        for (k, (t, _, v)) in run.track.iter().enumerate() {
            if !(t.is_finite() && *t >= 0.0 && v.is_finite()) {
                return Err(bad(
                    format!("track[{k}]"),
                    "times are 0 or more and values finite".into(),
                ));
            }
        }
        if let Some(r) = &run.run {
            for (key, v) in [("dt", r.dt), ("t_end", r.t_end)] {
                if v.is_some_and(|v| !(v.is_finite() && v > 0.0)) {
                    return Err(bad(format!("run.{key}"), "must be positive".into()));
                }
            }
        }
        Ok(run)
    }

    pub fn to_json(&self) -> String {
        let mut s = serde_json::to_string_pretty(self).expect("run files always serialize");
        s.push('\n');
        s
    }

    /// The model file to replay this run with: `model` with the run's
    /// seed, settings and schedule in place of its own. The recorded track
    /// is applied by the session as it runs.
    pub fn apply_to(&self, model: &ModelFile) -> ModelFile {
        let mut m = model.clone();
        if let Some(seed) = self.seed {
            m.run.seed = Some(seed);
        }
        if let Some(r) = &self.run {
            if let Some(dt) = r.dt {
                m.run.dt = dt;
            }
            if r.t_end.is_some() {
                m.run.t_end = r.t_end;
            }
        }
        m.run.inputs = self.inputs.clone();
        m
    }

    /// Whether `model` (loaded) is the model this run was made with.
    pub fn fits(&self, model: &Loaded) -> Result<bool, FileError> {
        Ok(self.model_hash == loaded_hash(model)?)
    }
}

/// A SHA-256 of a model as saved, leaving out `controls` (they never
/// change results, ADR 0012) and the run settings a run file records
/// itself (seed, `dt`, `t_end`, the schedule), so replaying a run under its
/// own settings does not make it look stale.
pub fn model_hash(model: &ModelFile) -> String {
    let mut m = model.clone();
    // Probes feeding the model change its results (ADR 0063): their
    // names and when they record are part of it.
    let feeding = |p: &crate::ProbeEntry| {
        m.connections
            .iter()
            .any(|c| c.from.starts_with(&format!("probe:{}.", p.name)))
    };
    let probes: Vec<crate::ProbeEntry> = model
        .controls
        .probes
        .iter()
        .filter(|p| feeding(p))
        .map(|p| crate::ProbeEntry {
            name: p.name.clone(),
            record: p.record.clone(),
            every: p.every.clone(),
            keep: None,
            to: None,
            canvas: None,
        })
        .collect();
    m.controls = Default::default();
    m.controls.probes = probes;
    m.run.seed = None;
    m.run.dt = crate::RunSettings::default().dt;
    m.run.t_end = None;
    m.run.inputs.clear();
    let digest = Sha256::digest(m.to_json().as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

/// The hash of a loaded model, in the form a run records it: every
/// parameter written out with its value, as a running session saves it,
/// so a file that leaves defaults implicit and one that spells them out
/// hash the same.
pub fn loaded_hash(loaded: &Loaded) -> Result<String, FileError> {
    let mut flat = loaded.flat.clone();
    flat.capture_params(&loaded.model, &loaded.ids)?;
    let mut file = loaded.file.clone();
    file.capture_from(&flat, &loaded.params);
    Ok(model_hash(&file))
}

/// The usual name of a run file beside `model_path`:
/// `cart-pole.ssbx.json` and "push test" give
/// `cart-pole.push-test.run.json`.
pub fn run_file_name(model_path: &str, run_name: &str) -> String {
    let stem = model_path
        .strip_suffix(".ssbx.json")
        .or_else(|| model_path.strip_suffix(".json"))
        .unwrap_or(model_path);
    let slug: String = run_name
        .trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = if slug.is_empty() {
        "run".to_string()
    } else {
        slug
    };
    format!("{stem}.{slug}.run.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODEL: &str = r#"{"format": 1, "blocks": [
        {"name": "push", "type": "input.value", "settings": {"unit": "N"}}],
        "controls": {"speed": 2.0}}"#;

    #[test]
    fn a_probe_feeding_the_model_is_part_of_its_hash() {
        let with = |every: f64, to: Option<&str>, feeds: bool| {
            let mut v: serde_json::Value = serde_json::from_str(MODEL).unwrap();
            v["controls"]["probes"] = serde_json::json!([{
                "name": "p", "record": ["push.y"], "every": {"seconds": every}
            }]);
            if let Some(to) = to {
                v["controls"]["probes"][0]["to"] = to.into();
            }
            if feeds {
                v["blocks"].as_array_mut().unwrap().push(serde_json::json!(
                    {"name": "q", "type": "input.value", "settings": {"unit": "N"}}
                ));
                v["connections"] = serde_json::json!([]);
            }
            let (mut f, _) = ModelFile::from_json(&v.to_string()).unwrap();
            if feeds {
                f.connections.push(crate::ConnectionEntry {
                    from: "probe:p.push.y".into(),
                    to: "q.u".into(),
                });
            }
            model_hash(&f)
        };
        // Not feeding the model: a control, outside the hash.
        assert_eq!(with(1.0, None, false), with(2.0, None, false));
        // Feeding it: when it records is part of the model; its file is not.
        assert_ne!(with(1.0, None, true), with(2.0, None, true));
        assert_eq!(with(1.0, None, true), with(1.0, Some("p.csv"), true));
    }

    #[test]
    fn runs_read_write_and_know_their_model() {
        let (model, _) = ModelFile::from_json(MODEL).unwrap();
        let run = RunFile {
            format: RUN_FORMAT,
            model: "push.ssbx.json".into(),
            model_hash: model_hash(&model),
            name: "push test".into(),
            note: String::new(),
            created: "2026-10-26T14:03:00Z".into(),
            seed: Some(7),
            run: Some(RunOverrides {
                dt: Some(0.02),
                t_end: Some(3.0),
            }),
            inputs: vec![ScheduledChange {
                t: 1.0,
                set: "push.y".into(),
                to: 40.0,
            }],
            track: vec![(0.5, "push.y".into(), 10.0)],
        };
        let again = RunFile::from_json(&run.to_json()).unwrap();
        assert_eq!(again, run);
        let fits = |m: &ModelFile| run.model_hash == model_hash(m);
        assert!(fits(&model));
        // Controls never change results: the hash ignores them.
        let mut panes = model.clone();
        panes.controls.speed = Some(8.0);
        assert!(fits(&panes));
        // Nor do the settings the run records itself.
        let mut faster = model.clone();
        faster.run.dt = 0.5;
        faster.run.seed = Some(3);
        assert!(fits(&faster));
        // A block does.
        let mut edited = model.clone();
        edited.blocks[0].name = "shove".into();
        assert!(!fits(&edited));
        let replay = run.apply_to(&model);
        assert_eq!(
            (replay.run.seed, replay.run.dt, replay.run.t_end),
            (Some(7), 0.02, Some(3.0))
        );
        assert_eq!(replay.run.inputs, run.inputs);
        // Unknown fields and newer formats are refused.
        let extra = run.to_json().replacen("{", r#"{"colour": 1,"#, 1);
        assert!(RunFile::from_json(&extra).is_err());
        let newer = run.to_json().replace(r#""format": 1"#, r#""format": 2"#);
        assert!(matches!(
            RunFile::from_json(&newer),
            Err(FileError::UnsupportedFormat(2))
        ));
        assert_eq!(
            run_file_name("cart-pole.ssbx.json", "Push test!"),
            "cart-pole.push-test.run.json"
        );
    }
}
