//! Every registered block type's parameter feedthrough declarations
//! (ADR 0075), checked by changing each parameter and evaluating the
//! outputs again with the same state and inputs. An output that changes
//! with a parameter its declaration does not list fails; outputs that
//! declare nothing (read as "every parameter") and listed parameters no
//! trial shows are reported, not failed.

use simsandbox_core::block::{Block, Causality, StepIo};
use simsandbox_core::table::Table;

/// A small deterministic generator: the test gives the same trials on
/// every machine.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// The outputs at state and inputs fixed by `init` and `inputs`, with
/// parameters `p`.
fn outputs(block: &dyn Block, p: &[f64], state: &[f64], tables: &[Table], u: &[f64]) -> Vec<f64> {
    let spec = block.spec();
    let mut y = vec![0.0; spec.output_len()];
    let mut x = state.to_vec();
    let mut tables = tables.to_vec();
    let count = |c: Causality| {
        spec.ports
            .iter()
            .filter(|q| q.causality == c && q.shape.is_variable())
            .count()
    };
    let pin = vec![Vec::new(); count(Causality::Input)];
    let mut pout = vec![Vec::new(); count(Causality::Output)];
    let ok = block.compute_outputs(
        0.5,
        StepIo {
            params: p,
            inputs: u,
            state: &mut x,
            outputs: &mut y,
            tables: &mut tables,
            population_inputs: &pin,
            population_outputs: &mut pout,
        },
    );
    if ok.is_err() {
        y.fill(f64::NAN);
    }
    y
}

fn differ(a: &[f64], b: &[f64]) -> bool {
    a.iter()
        .zip(b)
        .any(|(x, y)| x.to_bits() != y.to_bits() && !(x.is_nan() && y.is_nan()))
}

#[test]
fn outputs_change_only_with_the_parameters_they_declare() {
    let registry = simsandbox_session::examples::registry();
    let mut failures = Vec::new();
    let mut report = Vec::new();
    let mut checked = 0;
    let mut blocks: Vec<(String, Box<dyn Block>)> = Vec::new();
    for name in registry.type_names() {
        match registry.example(name) {
            Ok(b) => blocks.push((name.to_string(), b)),
            Err(_) => report.push(format!("{name}: no example to check")),
        }
    }
    // Typed equations whose dependencies run through defined quantities,
    // array parameters and grid axes.
    let typed = [
        vec![
            "input u",
            "param a = 2",
            "param b = 3",
            "param c = 5",
            "state x = 1",
            "dx/dt = -a*x",
            "p = b*u",
            "q = p + c",
            "r = x + 1",
        ],
        vec![
            "param L [m] = 1",
            "param k = 2",
            "grid i [m] = 0 .. L, 4",
            "param w[i] = 1",
            "s = w*k + i/L",
            "total = sum(s)",
        ],
    ];
    for (n, lines) in typed.iter().enumerate() {
        let config = simsandbox_core::registry::BlockConfig {
            definition: lines.iter().map(|l| l.to_string()).collect(),
            ..Default::default()
        };
        let b = registry
            .build("eq.equations", &format!("typed{n}"), &config)
            .unwrap_or_else(|e| panic!("typed{n}: {e}"));
        blocks.push((format!("typed{n}"), b));
    }
    for (name, mut block) in blocks {
        let spec = block.spec().clone();
        let mut rng = Lcg(1);
        // Per output: the parameters some trial showed it reads.
        let outs: Vec<_> = spec.outputs().filter(|q| !q.shape.is_variable()).collect();
        let mut seen = vec![Vec::<String>::new(); outs.len()];
        for _trial in 0..3 {
            let p: Vec<f64> = spec
                .params
                .iter()
                .map(|q| {
                    let (lo, hi) = if q.suggested_min < q.suggested_max {
                        (q.suggested_min, q.suggested_max)
                    } else {
                        (q.default - 1.0, q.default + 1.0)
                    };
                    lo + (hi - lo) * rng.next()
                })
                .collect();
            let mut state = vec![0.0; spec.states.len()];
            if block.init(&p, &mut state).is_err() {
                continue;
            }
            let mut tables: Vec<Table> = spec
                .tables
                .iter()
                .map(|t| Table::new(t.columns.len()))
                .collect();
            if block.init_tables(&p, 1, &mut tables).is_err() {
                continue;
            }
            let u: Vec<f64> = (0..spec.input_len())
                .map(|_| 2.0 * rng.next() - 1.0)
                .collect();
            let base = outputs(block.as_ref(), &p, &state, &tables, &u);
            for (i, q) in spec.params.iter().enumerate() {
                // Starting values are read when the block starts only.
                if q.initial_condition {
                    continue;
                }
                let mut moved = p.clone();
                moved[i] += (0.1 * p[i].abs()).max(0.1);
                let y = outputs(block.as_ref(), &moved, &state, &tables, &u);
                let mut at = 0;
                for (k, port) in outs.iter().enumerate() {
                    let n = port.shape.len();
                    if differ(&base[at..at + n], &y[at..at + n]) && !seen[k].contains(&q.name) {
                        seen[k].push(q.name.clone());
                    }
                    at += n;
                }
            }
        }
        checked += 1;
        for (k, port) in outs.iter().enumerate() {
            match &port.param_feedthrough {
                Some(list) => {
                    for q in seen[k].iter().filter(|q| !list.contains(q)) {
                        failures.push(format!(
                            "{name}.{} changes with parameter {q}, which it does not declare",
                            port.name
                        ));
                    }
                    for q in list.iter().filter(|q| !seen[k].contains(q)) {
                        report.push(format!(
                            "{name}.{} declares {q}, which no trial showed it reads",
                            port.name
                        ));
                    }
                }
                None if port.state.is_none() && !spec.params.is_empty() => {
                    report.push(format!(
                        "{name}.{} declares nothing (read as every parameter); trials show {:?}",
                        port.name, seen[k]
                    ));
                }
                None => {}
            }
        }
    }
    println!("{checked} block types checked");
    for r in &report {
        println!("{r}");
    }
    assert!(checked > 0);
    assert!(failures.is_empty(), "{failures:#?}");
}
