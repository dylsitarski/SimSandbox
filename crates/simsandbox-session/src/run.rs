//! Running a model file headless and collecting every output (ADR 0039),
//! as `simsandbox run` and the Python API do: every communication step of a
//! timed model, or every sample of a swept one (ADR 0028).

use serde_json::{json, Value};
use simsandbox_core::block::{MethodLabels, Shape};
use simsandbox_core::registry::Registry;
use simsandbox_core::sim::Simulator;
use std::io::{self, Write};

/// Used when neither the caller nor the model file gives an end time.
pub const DEFAULT_T_END: f64 = 20.0;

/// One fixed-size output, `block.port`, at every row.
#[derive(Clone, Debug)]
pub struct Output {
    pub name: String,
    pub shape: Shape,
    /// Row by row, `shape.len()` values each.
    pub values: Vec<f64>,
}

/// One population output at every row: its points, `dim` values each.
#[derive(Clone, Debug)]
pub struct Population {
    pub name: String,
    pub dim: usize,
    pub frames: Vec<Vec<f64>>,
}

/// Everything a run produced.
#[derive(Clone, Debug)]
pub struct Results {
    /// What indexes the rows: `t`, or `sample` and the swept parameters.
    pub index: Vec<(String, Vec<f64>)>,
    /// In file block order, then port order.
    pub outputs: Vec<Output>,
    pub populations: Vec<Population>,
    pub labels: Option<MethodLabels>,
    /// Algebraic loops and how they are solved (ADR 0040).
    pub notes: Vec<String>,
    pub warnings: Vec<String>,
}

impl Results {
    pub fn rows(&self) -> usize {
        self.index.first().map_or(0, |(_, v)| v.len())
    }

    /// CSV: the index, then one column per output element
    /// (`block.port[i]` for vectors, grids and point sets). Populations,
    /// whose length varies, are left out.
    pub fn write_csv(&self, out: &mut impl Write) -> io::Result<()> {
        let mut header: Vec<String> = self.index.iter().map(|(n, _)| n.clone()).collect();
        for o in &self.outputs {
            if o.shape == Shape::Scalar {
                header.push(o.name.clone());
            } else {
                header.extend((0..o.shape.len()).map(|i| format!("{}[{i}]", o.name)));
            }
        }
        writeln!(out, "{}", header.join(","))?;
        for r in 0..self.rows() {
            let mut cells: Vec<String> = self.index.iter().map(|(_, v)| v[r].to_string()).collect();
            for o in &self.outputs {
                let n = o.shape.len();
                cells.extend(o.values[r * n..(r + 1) * n].iter().map(f64::to_string));
            }
            writeln!(out, "{}", cells.join(","))?;
        }
        Ok(())
    }

    /// JSON: the index as lists; each output as a list over rows of its
    /// value (a number for a scalar, else a list in the port's element
    /// order); populations as a list over rows of points. JSON has no
    /// infinities or NaN: they become null.
    pub fn to_json(&self) -> Value {
        let list = |v: &[f64]| Value::Array(v.iter().map(|&x| number(x)).collect());
        let index: serde_json::Map<String, Value> = self
            .index
            .iter()
            .map(|(n, v)| (n.clone(), list(v)))
            .collect();
        let outputs: serde_json::Map<String, Value> = self
            .outputs
            .iter()
            .map(|o| {
                let values = if o.shape == Shape::Scalar {
                    list(&o.values)
                } else {
                    Value::Array(o.values.chunks(o.shape.len().max(1)).map(list).collect())
                };
                let shape = crate::describe::shape(&o.shape);
                (o.name.clone(), json!({"shape": shape, "values": values}))
            })
            .collect();
        let populations: serde_json::Map<String, Value> = self
            .populations
            .iter()
            .map(|p| {
                let frames: Vec<Value> = p
                    .frames
                    .iter()
                    .map(|f| Value::Array(f.chunks(p.dim).map(list).collect()))
                    .collect();
                (p.name.clone(), json!({"dim": p.dim, "frames": frames}))
            })
            .collect();
        json!({
            "rows": self.rows(),
            "index": index,
            "outputs": outputs,
            "populations": populations,
            "labels": self.labels.map(crate::describe::labels),
            "notes": self.notes,
            "warnings": self.warnings,
        })
    }
}

fn number(v: f64) -> Value {
    serde_json::Number::from_f64(v).map_or(Value::Null, Value::Number)
}

/// Run a model file: to `t_end` in steps of `dt` (each defaulting to the
/// file's `run` section), or through its sweep if it has no states.
pub fn run(
    text: &str,
    registry: &Registry,
    t_end: Option<f64>,
    dt: Option<f64>,
) -> Result<Results, String> {
    for v in [t_end, dt].into_iter().flatten() {
        if !(v.is_finite() && v > 0.0) {
            return Err("the end time and step must be positive".into());
        }
    }
    let loaded = simsandbox_file::load(text, registry).map_err(|e| e.to_string())?;
    if loaded.plan.is_some() && (dt.is_some() || t_end.is_some()) {
        return Err("the end time and step do not apply to a swept model".into());
    }
    let dt = dt.unwrap_or(loaded.file.run.dt);
    let t_end = t_end.or(loaded.file.run.t_end).unwrap_or(DEFAULT_T_END);

    let mut fixed = Vec::new();
    let mut variable = Vec::new();
    for entry in &loaded.file.blocks {
        let id = loaded.ids[&entry.name];
        let spec = loaded.model.block(id).map_err(|e| e.to_string())?.spec();
        for port in spec.outputs() {
            let name = format!("{}.{}", entry.name, port.name);
            if let Shape::Population { dim } = port.shape {
                variable.push((id, port.name.clone(), name, dim));
            } else {
                fixed.push((id, port.name.clone(), name, port.shape));
            }
        }
    }
    let mut sim = Simulator::new(loaded.model, 0.0).map_err(|e| e.to_string())?;
    let mut r = Results {
        index: Vec::new(),
        outputs: fixed
            .iter()
            .map(|(_, _, name, shape)| Output {
                name: name.clone(),
                shape: *shape,
                values: Vec::new(),
            })
            .collect(),
        populations: variable
            .iter()
            .map(|(_, _, name, dim)| Population {
                name: name.clone(),
                dim: *dim,
                frames: Vec::new(),
            })
            .collect(),
        labels: sim.labels(),
        notes: crate::session::loop_notes(&sim),
        warnings: loaded.warnings.clone(),
    };
    let record = |r: &mut Results, sim: &Simulator| {
        for (o, (id, port, _, _)) in r.outputs.iter_mut().zip(&fixed) {
            o.values
                .extend_from_slice(sim.output(*id, port).unwrap_or(&[]));
        }
        for (p, (id, port, _, _)) in r.populations.iter_mut().zip(&variable) {
            p.frames.push(sim.output(*id, port).unwrap_or(&[]).to_vec());
        }
    };
    if let Some(plan) = &loaded.plan {
        let targets: Vec<_> = plan
            .keys
            .iter()
            .map(|k| {
                let (block, param) = k.split_once('.').expect("plan keys are block.param");
                (loaded.ids[block], param)
            })
            .collect();
        let mut index = vec![("sample".to_string(), Vec::new())];
        index.extend(plan.keys.iter().map(|k| (k.clone(), Vec::new())));
        for (k, point) in plan.points.iter().enumerate() {
            for (&(id, param), &v) in targets.iter().zip(point) {
                sim.set_param(id, param, v).map_err(|e| e.to_string())?;
            }
            sim.restart(0.0).map_err(|e| e.to_string())?;
            index[0].1.push(k as f64);
            for (column, &v) in index[1..].iter_mut().zip(point) {
                column.1.push(v);
            }
            record(&mut r, &sim);
        }
        r.index = index;
    } else {
        let mut t = vec![sim.time()];
        record(&mut r, &sim);
        for _ in 0..(t_end / dt).round() as usize {
            sim.step(dt).map_err(|e| e.to_string())?;
            t.push(sim.time());
            record(&mut r, &sim);
        }
        r.index = vec![("t".to_string(), t)];
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::examples::{registry, EXAMPLES};

    fn example(part: &str) -> &'static str {
        EXAMPLES
            .iter()
            .find(|e| e.text.contains(part))
            .unwrap()
            .text
    }

    #[test]
    fn timed_runs_record_every_step_and_populations() {
        let r = run(example("agents.grazers"), &registry(), Some(1.0), None).unwrap();
        let rows = r.rows();
        assert_eq!(rows, 11);
        let count = r.outputs.iter().find(|o| o.name == "sheep.count").unwrap();
        let positions = &r
            .populations
            .iter()
            .find(|p| p.name == "sheep.position")
            .unwrap();
        assert_eq!(positions.dim, 2);
        for k in 0..rows {
            assert_eq!(positions.frames[k].len() as f64, 2.0 * count.values[k]);
        }
        let grass = r.outputs.iter().find(|o| o.name == "grass.grass").unwrap();
        assert_eq!(grass.values.len(), rows * 1600);
        let mut csv = Vec::new();
        r.write_csv(&mut csv).unwrap();
        let csv = String::from_utf8(csv).unwrap();
        assert_eq!(csv.lines().count(), rows + 1);
        assert!(csv.starts_with("t,grass.grass[0],"));
    }

    #[test]
    fn swept_runs_are_indexed_by_sample_and_parameters() {
        let r = run(example("\"points\": 600"), &registry(), None, None).unwrap();
        assert_eq!(r.index[0].0, "sample");
        assert_eq!(r.index[1].0, "spring.w");
        assert_eq!(r.rows(), 600);
        let j = r.to_json();
        assert_eq!(j["rows"], 600);
        assert!(j["outputs"]["spring.amplitude"]["values"][0].is_number());
        assert!(run(example("\"points\": 600"), &registry(), Some(1.0), None).is_err());
        assert!(run(example("lorenz"), &registry(), Some(-1.0), None).is_err());
    }
}
