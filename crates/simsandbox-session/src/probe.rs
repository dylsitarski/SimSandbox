//! Probes (ADR 0062): what a run records, when, and how long it is kept.
//!
//! Every session has an implicit probe: everything its views and filters
//! can show, at every step, thinned when long, which is how recording
//! always worked. Further probes record a chosen subset on their own
//! schedule ([`Every`]) and keep it their own way ([`Keep`]), each into a
//! history of its own. A probe never changes results.

use simsandbox_file::{ProbeEntry, ProbeEvery, ProbeKeep};
use simsandbox_views::history::{FieldShape, SignalRef};
use simsandbox_views::History;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// When a probe records a sample.
#[derive(Debug, Clone, PartialEq)]
pub enum Every {
    /// Every `n` steps of the run (counted from t = 0), the start included.
    Steps(u64),
    /// Every `dt` of simulated time, from t = 0: at the first step at or
    /// after each multiple.
    Seconds(f64),
    /// At the first step at or after each of these times, in order.
    Times(Vec<f64>),
}

/// How long a probe keeps its samples.
#[derive(Debug, Clone, PartialEq)]
pub enum Keep {
    /// The whole run, thinned (every other sample dropped) when it holds
    /// more than `mb` megabytes, or by the fields' size if not given: the
    /// default.
    Thin { mb: Option<f64> },
    /// Only the newest `samples`, forgetting the oldest.
    Last { samples: usize },
    /// Every sample, never thinned.
    All,
}

impl Default for Keep {
    fn default() -> Self {
        Keep::Thin { mb: None }
    }
}

impl Keep {
    /// Apply to a history.
    pub(crate) fn apply(&self, h: &mut History) {
        match *self {
            Keep::Thin { mb: None } => {}
            Keep::Thin { mb: Some(mb) } => {
                let per = h.values_per_sample().max(1) as f64 * 8.0;
                h.thin_past((mb * 1e6 / per) as usize);
            }
            Keep::Last { samples } => h.keep_last(samples),
            Keep::All => h.keep_all(),
        }
    }
}

/// A probe: its name, what it records (signal and field names as views
/// bind them), when and how long.
#[derive(Debug, Clone, PartialEq)]
pub struct ProbeSpec {
    pub name: String,
    pub record: Vec<String>,
    pub every: Every,
    pub keep: Keep,
    /// The CSV file it writes beside the model, if any; grid and point
    /// fields go into `.npy` files beside it.
    pub to: Option<String>,
    /// Where its node sits on the canvas's data layer.
    pub canvas: Option<[f64; 2]>,
}

impl ProbeSpec {
    /// A probe recording `record` at every step, thinned when long.
    pub fn new(name: impl Into<String>, record: Vec<String>) -> Self {
        Self {
            name: name.into(),
            record,
            every: Every::Steps(1),
            keep: Keep::default(),
            to: None,
            canvas: None,
        }
    }

    /// From the model file's entry (checked there).
    pub fn from_entry(e: &ProbeEntry) -> Self {
        let every = match &e.every {
            None => Every::Steps(1),
            Some(ProbeEvery { steps: Some(n), .. }) => Every::Steps(*n),
            Some(ProbeEvery {
                seconds: Some(dt), ..
            }) => Every::Seconds(*dt),
            Some(ProbeEvery {
                times: Some(ts), ..
            }) => Every::Times(ts.clone()),
            Some(_) => Every::Steps(1),
        };
        let keep = match &e.keep {
            Some(ProbeKeep { last: Some(n), .. }) => Keep::Last { samples: *n },
            Some(ProbeKeep { all: true, .. }) => Keep::All,
            Some(ProbeKeep { mb, .. }) => Keep::Thin { mb: *mb },
            None => Keep::default(),
        };
        Self {
            name: e.name.clone(),
            record: e.record.clone(),
            every,
            keep,
            to: e.to.clone(),
            canvas: e.canvas,
        }
    }

    /// As the model file writes it: defaults left out.
    pub fn to_entry(&self) -> ProbeEntry {
        let every = match &self.every {
            Every::Steps(1) => None,
            Every::Steps(n) => Some(ProbeEvery {
                steps: Some(*n),
                ..Default::default()
            }),
            Every::Seconds(dt) => Some(ProbeEvery {
                seconds: Some(*dt),
                ..Default::default()
            }),
            Every::Times(ts) => Some(ProbeEvery {
                times: Some(ts.clone()),
                ..Default::default()
            }),
        };
        let keep = match self.keep {
            Keep::Thin { mb: None } => None,
            Keep::Thin { mb } => Some(ProbeKeep {
                mb,
                ..Default::default()
            }),
            Keep::Last { samples } => Some(ProbeKeep {
                last: Some(samples),
                ..Default::default()
            }),
            Keep::All => Some(ProbeKeep {
                all: true,
                ..Default::default()
            }),
        };
        ProbeEntry {
            name: self.name.clone(),
            record: self.record.clone(),
            every,
            keep,
            to: self.to.clone(),
            canvas: self.canvas,
        }
    }
}

impl ProbeSpec {
    /// Check the settings; an error says what is wrong.
    pub fn check(&self) -> Result<(), String> {
        let ok = match &self.every {
            Every::Steps(n) => *n >= 1,
            Every::Seconds(dt) => dt.is_finite() && *dt > 0.0,
            Every::Times(ts) => ts.iter().all(|t| t.is_finite() && *t >= 0.0),
        };
        if !ok {
            return Err(format!(
                "probe {}: 'every' is a whole number of steps of at least 1, a positive time, or times of 0 or more",
                self.name
            ));
        }
        let keep_ok = match self.keep {
            Keep::Thin { mb } => mb.is_none_or(|mb| mb.is_finite() && mb > 0.0),
            Keep::Last { samples } => samples >= 1,
            Keep::All => true,
        };
        if !keep_ok {
            return Err(format!(
                "probe {}: 'keep' is a positive size in MB or at least 1 sample",
                self.name
            ));
        }
        Ok(())
    }
}

/// What a probe has recorded, written to files in `dir` (ADR 0062):
/// `to`, a CSV of `t` and its signals, and for each grid or point field
/// `<stem>.<field>.npy` beside it (NumPy, little-endian `f64`), shaped
/// `(samples, ny, nx)` for a grid and `(samples, count, dim)` for points.
/// Populations, whose size varies, are not written: a note says so.
/// Returns the files written and the notes.
pub fn write_files(h: &History, dir: &Path, to: &str) -> io::Result<(Vec<PathBuf>, Vec<String>)> {
    let mut written = Vec::new();
    let mut notes = Vec::new();
    let csv = dir.join(to);
    let mut out = io::BufWriter::new(std::fs::File::create(&csv)?);
    write_csv(h, &mut out)?;
    out.flush()?;
    written.push(csv);
    let stem = to.strip_suffix(".csv").unwrap_or(to);
    for (i, f) in h.fields().iter().enumerate().filter(|(_, f)| !f.whole) {
        let shape = match f.shape {
            FieldShape::Grid { nx, ny } => [h.len(), ny, nx],
            FieldShape::Points { count, dim } => [h.len(), count, dim],
            FieldShape::Population { .. } => {
                notes.push(format!(
                    "{} is a population (its size varies) and is not written",
                    f.name
                ));
                continue;
            }
        };
        let path = dir.join(format!("{stem}.{}.npy", f.name));
        let mut out = io::BufWriter::new(std::fs::File::create(&path)?);
        write_npy(
            &mut out,
            shape,
            (0..h.len()).map(|k| h.fields()[i].frame(k)),
        )?;
        out.flush()?;
        written.push(path);
    }
    Ok((written, notes))
}

/// CSV: `t`, then one column per signal, one row per sample.
pub fn write_csv(h: &History, out: &mut impl Write) -> io::Result<()> {
    let mut header = vec![h.index().to_string()];
    header.extend(h.names().iter().cloned());
    writeln!(out, "{}", header.join(","))?;
    for k in 0..h.len() {
        let mut row = vec![h.times()[k].to_string()];
        row.extend((0..h.names().len()).map(|i| h.value(SignalRef::Signal(i), k).to_string()));
        writeln!(out, "{}", row.join(","))?;
    }
    Ok(())
}

/// JSON: `rows`, `t`, each signal as a list over samples, each field
/// with its shape and its frames (a list per sample). JSON has no
/// infinities or NaN: they become null.
pub fn to_json(h: &History) -> serde_json::Value {
    use serde_json::{json, Value};
    let number = |v: f64| serde_json::Number::from_f64(v).map_or(Value::Null, Value::Number);
    let list = |v: &[f64]| Value::Array(v.iter().map(|&x| number(x)).collect());
    let signals: serde_json::Map<String, Value> = h
        .names()
        .iter()
        .enumerate()
        .map(|(i, n)| (n.clone(), list(&h.values(SignalRef::Signal(i)))))
        .collect();
    let fields: serde_json::Map<String, Value> = h
        .fields()
        .iter()
        .filter(|f| !f.whole)
        .map(|f| {
            let shape = match f.shape {
                FieldShape::Grid { nx, ny } => json!({"grid": [ny, nx]}),
                FieldShape::Points { count, dim } => json!({"points": [count, dim]}),
                FieldShape::Population { dim } => json!({"population": dim}),
            };
            let frames: Vec<Value> = (0..h.len()).map(|k| list(f.frame(k))).collect();
            (f.name.clone(), json!({"shape": shape, "frames": frames}))
        })
        .collect();
    json!({"rows": h.len(), "t": list(h.times()), "signals": signals, "fields": fields})
}

/// A NumPy `.npy` file (format 1.0) of `f64` values in C order.
fn write_npy<'a>(
    out: &mut impl Write,
    shape: [usize; 3],
    frames: impl Iterator<Item = &'a [f64]>,
) -> io::Result<()> {
    let mut header = format!(
        "{{'descr': '<f8', 'fortran_order': False, 'shape': ({}, {}, {}), }}",
        shape[0], shape[1], shape[2]
    );
    // Magic (6), version (2), header length (2), header: a multiple of
    // 64 bytes, ending in a newline.
    let total = (10 + header.len() + 1).div_ceil(64) * 64;
    header.push_str(&" ".repeat(total - 10 - header.len() - 1));
    header.push('\n');
    out.write_all(b"\x93NUMPY\x01\x00")?;
    out.write_all(&(header.len() as u16).to_le_bytes())?;
    out.write_all(header.as_bytes())?;
    // A frame not recorded (empty) is written as NaN, so every frame has
    // the field's shape.
    let len = shape[1] * shape[2];
    for frame in frames {
        if frame.len() == len {
            for v in frame {
                out.write_all(&v.to_le_bytes())?;
            }
        } else {
            for _ in 0..len {
                out.write_all(&f64::NAN.to_le_bytes())?;
            }
        }
    }
    Ok(())
}

/// Decides, step by step, when a probe records.
#[derive(Debug, Clone)]
pub(crate) struct Trigger {
    every: Every,
    /// For `Seconds`: the next multiple due. For `Times`: the next index.
    next_time: f64,
    next_index: usize,
}

impl Trigger {
    pub(crate) fn new(every: Every) -> Self {
        let mut t = Self {
            every,
            next_time: 0.0,
            next_index: 0,
        };
        t.seek(None, 0.0);
        t
    }

    /// Whether to record the sample at time `t` after `step` steps; `eps`
    /// absorbs rounding in the times. Call once per step, in order.
    pub(crate) fn due(&mut self, t: f64, step: u64, eps: f64) -> bool {
        match &self.every {
            Every::Steps(n) => step.is_multiple_of(*n),
            Every::Seconds(dt) => {
                if t + eps < self.next_time {
                    return false;
                }
                // The next multiple strictly after t.
                self.next_time = ((t + eps) / dt).floor() * dt + dt;
                true
            }
            Every::Times(ts) => {
                let mut due = false;
                while self.next_index < ts.len() && ts[self.next_index] <= t + eps {
                    self.next_index += 1;
                    due = true;
                }
                due
            }
        }
    }

    /// After a rewind or restart to `t`: the samples at `t` and before are
    /// recorded (`Some(t)`), or nothing is (`None`, a fresh start).
    pub(crate) fn seek(&mut self, recorded_up_to: Option<f64>, eps: f64) {
        match (&self.every, recorded_up_to) {
            (Every::Seconds(_), None) | (Every::Times(_), None) => {
                self.next_time = 0.0;
                self.next_index = 0;
            }
            (Every::Seconds(dt), Some(t)) => {
                self.next_time = ((t + eps) / dt).floor() * dt + dt;
            }
            (Every::Times(ts), Some(t)) => {
                self.next_index = ts.partition_point(|&x| x <= t + eps);
            }
            (Every::Steps(_), _) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The times a trigger records at over `steps` steps of `dt`.
    fn fired(every: Every, dt: f64, steps: u64) -> Vec<f64> {
        let mut tr = Trigger::new(every);
        (0..=steps)
            .map(|k| (k, k as f64 * dt))
            .filter(|&(k, t)| tr.due(t, k, 1e-9))
            .map(|(_, t)| t)
            .collect()
    }

    #[test]
    fn triggers_fire_by_steps_seconds_and_times() {
        assert_eq!(fired(Every::Steps(3), 0.1, 7).len(), 3);
        // Every 0.25 s with 0.1 s steps: the first step at or after each.
        let t = fired(Every::Seconds(0.25), 0.1, 10);
        let r: Vec<f64> = t.iter().map(|x| (x * 10.0).round() / 10.0).collect();
        assert_eq!(r, [0.0, 0.3, 0.5, 0.8, 1.0]);
        // Listed times: each once, even if two fall in one step.
        let t = fired(Every::Times(vec![0.0, 0.31, 0.32, 0.7]), 0.1, 10);
        let r: Vec<f64> = t.iter().map(|x| (x * 10.0).round() / 10.0).collect();
        assert_eq!(r, [0.0, 0.4, 0.7]);
    }

    #[test]
    fn a_rewound_trigger_fires_again_where_it_did() {
        let mut tr = Trigger::new(Every::Seconds(0.25));
        let first: Vec<u64> = (0..=10)
            .filter(|&k| tr.due(k as f64 * 0.1, k, 1e-9))
            .collect();
        // Back to t = 0.4 (recorded up to there): fires at 0.5 and 0.8 again.
        tr.seek(Some(0.4), 1e-9);
        let again: Vec<u64> = (5..=10)
            .filter(|&k| tr.due(k as f64 * 0.1, k, 1e-9))
            .collect();
        assert_eq!(
            again,
            first.into_iter().filter(|&k| k >= 5).collect::<Vec<_>>()
        );
        let mut times = Trigger::new(Every::Times(vec![0.2, 0.6]));
        assert!(times.due(0.2, 2, 1e-9));
        times.seek(Some(0.1), 1e-9);
        assert!(times.due(0.2, 2, 1e-9));
    }

    #[test]
    fn probe_settings_are_checked() {
        let p = |every, keep| ProbeSpec {
            every,
            keep,
            ..ProbeSpec::new("p", vec!["a.x".into()])
        };
        assert!(p(Every::Steps(1), Keep::default()).check().is_ok());
        assert!(p(Every::Steps(0), Keep::All).check().is_err());
        assert!(p(Every::Seconds(-1.0), Keep::All).check().is_err());
        assert!(p(Every::Times(vec![f64::NAN]), Keep::All).check().is_err());
        assert!(p(Every::Steps(1), Keep::Last { samples: 0 })
            .check()
            .is_err());
        assert!(p(Every::Steps(1), Keep::Thin { mb: Some(0.0) })
            .check()
            .is_err());
        // A probe just added on the canvas records nothing yet.
        let mut none = p(Every::Steps(1), Keep::All);
        none.record.clear();
        assert!(none.check().is_ok());
    }

    #[test]
    fn probes_go_to_the_file_and_back() {
        for (every, keep) in [
            (Every::Steps(1), Keep::default()),
            (Every::Steps(4), Keep::Thin { mb: Some(5.0) }),
            (Every::Seconds(0.5), Keep::Last { samples: 10 }),
            (Every::Times(vec![1.0, 2.0]), Keep::All),
        ] {
            let spec = ProbeSpec {
                every,
                keep,
                to: Some("out.csv".into()),
                canvas: Some([1.0, 2.0]),
                ..ProbeSpec::new("p", vec!["a.x".into()])
            };
            assert_eq!(ProbeSpec::from_entry(&spec.to_entry()), spec);
        }
        let plain = ProbeSpec::new("p", vec!["a.x".into()]).to_entry();
        assert!(plain.every.is_none() && plain.keep.is_none());
    }

    #[test]
    fn probe_files_hold_the_samples() {
        let mut h = History::with_fields(
            "t",
            vec!["a.x".into()],
            vec![("g.u".into(), FieldShape::Grid { nx: 3, ny: 2 })],
        );
        h.keep_all();
        h.push_sample(0.0, &[1.0], &[&[0.0, 1.0, 2.0, 3.0, 4.0, 5.0]]);
        h.push_sample(0.5, &[2.5], &[&[6.0, 7.0, 8.0, 9.0, 10.0, 11.0]]);
        let dir = std::env::temp_dir().join(format!("simsandbox-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (files, notes) = write_files(&h, &dir, "out.csv").unwrap();
        assert!(notes.is_empty());
        assert_eq!(files.len(), 2);
        let csv = std::fs::read_to_string(dir.join("out.csv")).unwrap();
        assert_eq!(csv, "t,a.x\n0,1\n0.5,2.5\n");
        let npy = std::fs::read(dir.join("out.g.u.npy")).unwrap();
        assert_eq!(&npy[..8], b"\x93NUMPY\x01\x00");
        let len = u16::from_le_bytes([npy[8], npy[9]]) as usize;
        assert_eq!((10 + len) % 64, 0);
        let header = std::str::from_utf8(&npy[10..10 + len]).unwrap();
        assert!(header.contains("'shape': (2, 2, 3)"), "{header}");
        let values: Vec<f64> = npy[10 + len..]
            .chunks(8)
            .map(|b| f64::from_le_bytes(b.try_into().unwrap()))
            .collect();
        assert_eq!(values, (0..12).map(f64::from).collect::<Vec<_>>());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
