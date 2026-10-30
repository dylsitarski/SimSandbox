//! `input.file` (ADR 0063 step 4): recorded data as an input over time —
//! a probe's file, or any CSV or `.npy`. The data is read when the model
//! is built; the outputs are a function of time alone (no state), so
//! snapshots, rewind and replay need nothing new.
//!
//! A CSV's first column is the time, every other column a scalar output
//! named by its header (characters other than letters, digits and `_`
//! become `_`: `car.v` gives the output `car_v`). A header may give a
//! unit in brackets, `car.v [m/s]` (ADR 0048); otherwise the setting
//! `unit.COLUMN` or `unit` gives it, else it is dimensionless. A `.npy`
//! of `f64` (or `f32`) is one output `y`, its first axis the samples:
//! `(samples)` a scalar, `(samples, n)` a vector, `(samples, a, b)` a grid
//! of `a` rows of `b` (or with `shape: points`, `a` points of `b`
//! coordinates), its times the first column of the CSV named by `times`
//! (a probe's CSV beside its `.npy` files) or every `dt` seconds from 0.
//!
//! Between samples the value is held (`interpolate: hold`, the default:
//! what a probe feeding a block gives) or interpolated linearly
//! (`linear`); before the first sample it is the first, after the last
//! the last. Like any block's output, it is taken at each communication
//! point and held over the step; the block never joins a fused group.

use simsandbox_core::block::{
    AccuracyClass, Block, BlockSpec, Domain, MethodLabels, MethodSpec, PortSpec, Shape, SpeedClass,
    StepContext, StepIo, TimeSemantics,
};
use simsandbox_core::registry::BlockConfig;
use simsandbox_core::units::Unit;
use simsandbox_core::{Result, SimError};
use std::path::Path;

/// The type name.
pub const FILE: &str = "input.file";
/// Its one method: samples are read from the file.
pub const READ: &str = "read";

/// Recorded data read from a file.
pub struct FileInput {
    spec: BlockSpec,
    method: MethodSpec,
    times: Vec<f64>,
    /// Per output, its values sample after sample.
    data: Vec<Vec<f64>>,
    linear: bool,
}

fn setting_error(setting: &str, reason: impl Into<String>) -> SimError {
    SimError::InvalidSetting {
        setting: format!("settings.{setting}"),
        reason: reason.into(),
    }
}

/// A port name from a column's header: letters, digits and `_`.
pub fn port_name(column: &str) -> String {
    column
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// A CSV: its header (name and unit in brackets, if any, per column) and
/// its rows of numbers.
struct Csv {
    header: Vec<(String, Option<String>)>,
    rows: Vec<Vec<f64>>,
}

fn read_csv(path: &Path, setting: &str) -> Result<Csv> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| setting_error(setting, format!("cannot read {}: {e}", path.display())))?;
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header = lines
        .next()
        .ok_or_else(|| setting_error(setting, format!("{} is empty", path.display())))?
        .split(',')
        .map(|h| {
            let h = h.trim();
            match h.strip_suffix(']').and_then(|r| r.rsplit_once('[')) {
                Some((name, unit)) => (name.trim().to_string(), Some(unit.trim().to_string())),
                None => (h.to_string(), None),
            }
        })
        .collect::<Vec<_>>();
    let mut rows = Vec::new();
    for (k, line) in lines.enumerate() {
        let row = line
            .split(',')
            .map(|v| v.trim().parse::<f64>())
            .collect::<std::result::Result<Vec<f64>, _>>()
            .map_err(|_| {
                setting_error(
                    setting,
                    format!("{} row {}: not all numbers", path.display(), k + 2),
                )
            })?;
        if row.len() != header.len() {
            return Err(setting_error(
                setting,
                format!(
                    "{} row {}: {} values for {} columns",
                    path.display(),
                    k + 2,
                    row.len(),
                    header.len()
                ),
            ));
        }
        rows.push(row);
    }
    Ok(Csv { header, rows })
}

/// A `.npy` file's shape and values (format 1 to 3, little-endian `f8` or
/// `f4`, C order).
pub fn read_npy(bytes: &[u8]) -> std::result::Result<(Vec<usize>, Vec<f64>), String> {
    if bytes.len() < 10 || &bytes[..6] != b"\x93NUMPY" {
        return Err("not a .npy file".into());
    }
    let (len, start) = match bytes[6] {
        1 => (u16::from_le_bytes([bytes[8], bytes[9]]) as usize, 10),
        2 | 3 if bytes.len() >= 12 => (
            u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize,
            12,
        ),
        v => return Err(format!(".npy format version {v} is not supported")),
    };
    let header = bytes
        .get(start..start + len)
        .and_then(|h| std::str::from_utf8(h).ok())
        .ok_or("the .npy header is cut short")?;
    let value = |key: &str| -> Option<&str> {
        let at = header.find(&format!("'{key}'"))? + key.len() + 2;
        Some(header[at..].trim_start().strip_prefix(':')?.trim_start())
    };
    let descr = value("descr").ok_or("the .npy header has no 'descr'")?;
    let size = if descr.starts_with("'<f8'") {
        8
    } else if descr.starts_with("'<f4'") {
        4
    } else {
        return Err("only little-endian float64 or float32 .npy files are read".into());
    };
    if value("fortran_order").is_some_and(|v| v.starts_with("True")) {
        return Err("Fortran-ordered .npy files are not read; save in C order".into());
    }
    let shape_text = value("shape").ok_or("the .npy header has no 'shape'")?;
    let shape_text = shape_text
        .strip_prefix('(')
        .and_then(|s| s.split_once(')'))
        .map(|(inner, _)| inner)
        .ok_or("the .npy shape is not a tuple")?;
    let shape = shape_text
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.parse::<usize>()
                .map_err(|_| "the .npy shape is not numbers".to_string())
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let count: usize = shape.iter().product();
    let data = &bytes[start + len..];
    if data.len() < count * size {
        return Err(format!(
            ".npy holds {} values, its shape needs {count}",
            data.len() / size
        ));
    }
    let values = data[..count * size]
        .chunks_exact(size)
        .map(|c| match size {
            8 => f64::from_le_bytes(c.try_into().expect("8 bytes")),
            _ => f32::from_le_bytes(c.try_into().expect("4 bytes")) as f64,
        })
        .collect();
    Ok((shape, values))
}

const SETTINGS: &str =
    "'file', 'interpolate', 'unit', 'unit.COLUMN', and for a .npy 'times' or 'dt' and 'shape'";

impl FileInput {
    /// Data in memory: `times` strictly increasing, each output's values
    /// sample after sample.
    pub fn new(
        name: &str,
        times: Vec<f64>,
        outputs: Vec<(PortSpec, Vec<f64>)>,
        linear: bool,
    ) -> Result<Self> {
        if times.is_empty() {
            return Err(setting_error("file", "the file has no samples"));
        }
        if let Some(k) =
            (0..times.len()).find(|&k| !times[k].is_finite() || (k > 0 && times[k] <= times[k - 1]))
        {
            return Err(setting_error(
                "file",
                format!(
                    "times must be finite and increase: sample {} is at {}",
                    k + 1,
                    times[k]
                ),
            ));
        }
        let (ports, data): (Vec<PortSpec>, Vec<Vec<f64>>) = outputs.into_iter().unzip();
        for (p, d) in ports.iter().zip(&data) {
            assert_eq!(d.len(), times.len() * p.shape.len(), "{} sized", p.name);
        }
        Ok(Self {
            spec: BlockSpec {
                name: name.into(),
                domain: Domain::Equations,
                ports,
                params: Vec::new(),
                states: Vec::new(),
                tables: Vec::new(),
                time: TimeSemantics::Stateless,
            },
            method: MethodSpec {
                name: READ.into(),
                labels: MethodLabels {
                    speed: SpeedClass::RealTime,
                    // Exact at the samples; linear between them is an
                    // assumption about the data (ADR 0061).
                    accuracy: AccuracyClass::Quantitative,
                },
            },
            times,
            data,
            linear,
        })
    }

    /// From a model file's settings (see the module's documentation).
    pub fn from_config(name: &str, c: &BlockConfig) -> Result<Self> {
        if let Some(k) = c.settings.keys().find(|k| {
            !matches!(
                k.as_str(),
                "file" | "interpolate" | "unit" | "times" | "dt" | "shape"
            ) && !k.starts_with("unit.")
        }) {
            return Err(setting_error(
                k,
                format!("{FILE} has the settings {SETTINGS}"),
            ));
        }
        if c.method.name != READ || !c.method.options.is_empty() {
            return Err(SimError::InvalidMethod(format!(
                "{FILE} reads its samples: its method is '{READ}', with no options"
            )));
        }
        if !c.definition.is_empty() || c.update.is_some() {
            return Err(SimError::InvalidDefinition(format!(
                "{FILE} takes only the settings {SETTINGS}"
            )));
        }
        let linear = match c.settings.get("interpolate").map(String::as_str) {
            None | Some("hold") => false,
            Some("linear") => true,
            Some(o) => {
                return Err(setting_error(
                    "interpolate",
                    format!("'{o}' is not 'hold' or 'linear'"),
                ))
            }
        };
        let unit = |text: &str, setting: &str| {
            Unit::parse(text).map_err(|e| setting_error(setting, e.to_string()))
        };
        let default_unit = match c.settings.get("unit") {
            Some(u) => unit(u, "unit")?,
            None => Unit::DIMENSIONLESS,
        };
        let path = c
            .file("file")
            .ok_or_else(|| setting_error("file", "the file to read, a .csv or .npy"))??;
        let is = |ext: &str| {
            path.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case(ext))
        };
        let only_npy = ["times", "dt", "shape"];
        if is("csv") {
            if let Some(k) = only_npy.iter().find(|k| c.settings.contains_key(**k)) {
                return Err(setting_error(
                    k,
                    "only for a .npy file; a CSV's first column is its time",
                ));
            }
            let csv = read_csv(&path, "file")?;
            if csv.header.len() < 2 {
                return Err(setting_error(
                    "file",
                    "a CSV needs a time column and at least one more",
                ));
            }
            let times = csv.rows.iter().map(|r| r[0]).collect();
            let mut outputs: Vec<(PortSpec, Vec<f64>)> = Vec::new();
            for (i, (column, in_header)) in csv.header.iter().enumerate().skip(1) {
                let port = port_name(column);
                if port.is_empty() || outputs.iter().any(|(p, _)| p.name == port) {
                    return Err(setting_error(
                        "file",
                        format!("column '{column}' gives the output '{port}' twice or not at all"),
                    ));
                }
                let key = format!("unit.{column}");
                let u = match (c.settings.get(&key), in_header) {
                    (Some(u), _) => unit(u, &key)?,
                    (None, Some(u)) => unit(u, "file").map_err(|_| {
                        setting_error("file", format!("column '{column}': '{u}' is not a unit"))
                    })?,
                    (None, None) => default_unit,
                };
                outputs.push((
                    PortSpec::output(&port, u, Shape::Scalar),
                    csv.rows.iter().map(|r| r[i]).collect(),
                ));
            }
            if let Some(k) = c.settings.keys().find(|k| {
                k.strip_prefix("unit.")
                    .is_some_and(|col| !csv.header.iter().skip(1).any(|(h, _)| h == col))
            }) {
                return Err(setting_error(k, "the file has no such column"));
            }
            Self::new(name, times, outputs, linear)
        } else if is("npy") {
            if let Some(k) = c.settings.keys().find(|k| k.starts_with("unit.")) {
                return Err(setting_error(k, "a .npy has one output, y: use 'unit'"));
            }
            let bytes = std::fs::read(&path).map_err(|e| {
                setting_error("file", format!("cannot read {}: {e}", path.display()))
            })?;
            let (shape, values) = read_npy(&bytes)
                .map_err(|e| setting_error("file", format!("{}: {e}", path.display())))?;
            let samples = *shape.first().ok_or_else(|| {
                setting_error("file", "a .npy of samples needs at least one axis")
            })?;
            let points = match c.settings.get("shape").map(String::as_str) {
                None | Some("grid") => false,
                Some("points") => true,
                Some(o) => {
                    return Err(setting_error(
                        "shape",
                        format!("'{o}' is not 'grid' or 'points'"),
                    ))
                }
            };
            let port_shape = match shape[1..] {
                [] => Shape::Scalar,
                [n] => Shape::Vector(n),
                [count, dim] if points => Shape::Points { count, dim },
                [ny, nx] => Shape::Grid { nx, ny },
                _ => {
                    return Err(setting_error(
                        "file",
                        format!("a .npy of shape {shape:?}: at most 3 axes are read"),
                    ))
                }
            };
            if c.settings.contains_key("shape") && shape.len() != 3 {
                return Err(setting_error("shape", "only for a .npy of 3 axes"));
            }
            let times = match (c.file("times"), c.settings.get("dt")) {
                (Some(_), Some(_)) | (None, None) => return Err(setting_error(
                    "times",
                    "a .npy needs its times: 'times' (a CSV whose first column they are) or 'dt'",
                )),
                (Some(p), None) => read_csv(&p?, "times")?
                    .rows
                    .iter()
                    .map(|r| r[0])
                    .collect::<Vec<_>>(),
                (None, Some(dt)) => {
                    let dt = dt
                        .trim()
                        .parse::<f64>()
                        .ok()
                        .filter(|d| d.is_finite() && *d > 0.0)
                        .ok_or_else(|| {
                            setting_error("dt", format!("'{dt}' is not a positive number"))
                        })?;
                    (0..samples).map(|k| k as f64 * dt).collect()
                }
            };
            if times.len() != samples {
                return Err(setting_error(
                    "times",
                    format!("{} times for {samples} samples", times.len()),
                ));
            }
            Self::new(
                name,
                times,
                vec![(PortSpec::output("y", default_unit, port_shape), values)],
                linear,
            )
        } else {
            Err(setting_error("file", "a .csv or .npy file"))
        }
    }

    /// The samples' times.
    pub fn times(&self) -> &[f64] {
        &self.times
    }
}

impl Block for FileInput {
    fn spec(&self) -> &BlockSpec {
        &self.spec
    }

    fn method(&self) -> &MethodSpec {
        &self.method
    }

    fn init(&mut self, _params: &[f64], _state: &mut [f64]) -> Result<()> {
        Ok(())
    }

    fn do_step(&mut self, ctx: StepContext, io: StepIo<'_>) -> Result<()> {
        self.compute_outputs(ctx.t + ctx.dt, io)
    }

    fn compute_outputs(&self, t: f64, io: StepIo<'_>) -> Result<()> {
        // The last sample at or before `t`, the first before them all.
        let k = self.times.partition_point(|&s| s <= t).saturating_sub(1);
        let next = (k + 1 < self.times.len() && t > self.times[k]).then_some(k + 1);
        let mut at = 0;
        for (p, d) in self.spec.ports.iter().zip(&self.data) {
            let n = p.shape.len();
            let a = &d[k * n..(k + 1) * n];
            let out = &mut io.outputs[at..at + n];
            match next.filter(|_| self.linear) {
                Some(j) => {
                    let w = (t - self.times[k]) / (self.times[j] - self.times[k]);
                    let b = &d[j * n..(j + 1) * n];
                    for ((o, &a), &b) in out.iter_mut().zip(a).zip(b) {
                        *o = a + (b - a) * w;
                    }
                }
                None => out.copy_from_slice(a),
            }
            at += n;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use simsandbox_core::graph::Model;
    use simsandbox_core::registry::MethodSettings;
    use simsandbox_core::sim::Simulator;

    fn dir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("simsandbox-file-input-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn config(settings: &[(&str, &str)]) -> BlockConfig {
        BlockConfig {
            method: MethodSettings::new(READ),
            settings: settings
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            folder: Some(dir()),
            ..Default::default()
        }
    }

    fn run(block: FileInput, steps: usize, dt: f64) -> Vec<Vec<f64>> {
        let mut m = Model::new();
        let id = m.add(Box::new(block));
        let mut s = Simulator::new(m, 0.0).unwrap();
        let names: Vec<String> = s
            .model()
            .block(id)
            .unwrap()
            .spec()
            .outputs()
            .map(|p| p.name.clone())
            .collect();
        let all = |s: &Simulator| {
            names
                .iter()
                .flat_map(|n| s.output(id, n).unwrap().to_vec())
                .collect::<Vec<f64>>()
        };
        let mut out = vec![all(&s)];
        for _ in 0..steps {
            s.step(dt).unwrap();
            out.push(all(&s));
        }
        out
    }

    #[test]
    fn a_csv_is_held_or_interpolated_between_samples() {
        std::fs::write(
            dir().join("speed.csv"),
            "t,car.v [m/s],F\n0,1,10\n1,3,20\n2,2,NaN\n",
        )
        .unwrap();
        let held = FileInput::from_config("data", &config(&[("file", "speed.csv"), ("unit", "N")]))
            .unwrap();
        let names: Vec<&str> = held.spec.ports.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["car_v", "F"]);
        assert_eq!(held.spec.ports[0].unit, Unit::parse("m/s").unwrap());
        assert_eq!(held.spec.ports[1].unit, Unit::parse("N").unwrap());
        let v: Vec<f64> = run(held, 6, 0.5).iter().map(|o| o[0]).collect();
        assert_eq!(v, [1.0, 1.0, 3.0, 3.0, 2.0, 2.0, 2.0]);
        let linear = FileInput::from_config(
            "data",
            &config(&[("file", "speed.csv"), ("interpolate", "linear")]),
        )
        .unwrap();
        let v: Vec<f64> = run(linear, 6, 0.5).iter().map(|o| o[0]).collect();
        assert_eq!(v, [1.0, 2.0, 3.0, 2.5, 2.0, 2.0, 2.0]);
    }

    #[test]
    fn a_npy_grid_takes_its_times_from_a_csv() {
        let mut bytes = b"\x93NUMPY\x01\x00".to_vec();
        let mut header =
            "{'descr': '<f8', 'fortran_order': False, 'shape': (2, 2, 3), }".to_string();
        while (10 + header.len() + 1) % 64 != 0 {
            header.push(' ');
        }
        header.push('\n');
        bytes.extend((header.len() as u16).to_le_bytes());
        bytes.extend(header.as_bytes());
        for v in 0..12 {
            bytes.extend((v as f64).to_le_bytes());
        }
        std::fs::write(dir().join("grid.npy"), &bytes).unwrap();
        std::fs::write(dir().join("grid.csv"), "t\n0.5\n1.5\n").unwrap();
        let b = FileInput::from_config(
            "g",
            &config(&[("file", "grid.npy"), ("times", "grid.csv"), ("unit", "K")]),
        )
        .unwrap();
        assert_eq!(b.spec.ports[0].shape, Shape::Grid { nx: 3, ny: 2 });
        let out = run(b, 2, 1.0);
        assert_eq!(out[0], [0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(out[2], [6.0, 7.0, 8.0, 9.0, 10.0, 11.0]);
        let p = FileInput::from_config(
            "p",
            &config(&[("file", "grid.npy"), ("dt", "1"), ("shape", "points")]),
        )
        .unwrap();
        assert_eq!(p.spec.ports[0].shape, Shape::Points { count: 2, dim: 3 });
    }

    #[test]
    fn bad_settings_and_files_are_refused() {
        std::fs::write(dir().join("back.csv"), "t,x\n0,1\n0,2\n").unwrap();
        std::fs::write(dir().join("ok.csv"), "t,x\n0,1\n").unwrap();
        for s in [
            vec![("file", "back.csv")],
            vec![("file", "missing.csv")],
            vec![("file", "ok.csv"), ("dt", "1")],
            vec![("file", "ok.csv"), ("unit.y", "m")],
            vec![("file", "ok.csv"), ("interpolate", "cubic")],
            vec![("file", "ok.csv"), ("colour", "red")],
            vec![("file", "ok.txt")],
            vec![],
        ] {
            assert!(FileInput::from_config("d", &config(&s)).is_err(), "{s:?}");
        }
        let mut unsaved = config(&[("file", "ok.csv")]);
        unsaved.folder = None;
        let e = FileInput::from_config("d", &unsaved)
            .err()
            .unwrap()
            .to_string();
        assert!(e.contains("save the model first"), "{e}");
    }
}
