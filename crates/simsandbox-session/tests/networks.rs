//! Networks read from files (ADR 0069 step 3): tables and edges started
//! from CSV files, parameters over rows, flows along edges summed onto
//! their ends (`outflow`), and array parameters read from a file.

use serde_json::json;
use simsandbox_core::registry::in_folder;
use simsandbox_session::examples::registry;
use simsandbox_session::run::{run, Results};
use std::path::{Path, PathBuf};

/// Values of an output over time; `name#id` is a quantity over a table's
/// rows at the row with that id.
fn series(y: &Results, name: &str) -> Vec<f64> {
    if let Some((pop, id)) = name.split_once('#') {
        let id: f64 = id.parse().unwrap();
        let p = y
            .populations
            .iter()
            .find(|p| p.name == pop)
            .unwrap_or_else(|| panic!("no rows {pop}"));
        return (0..p.frames.len())
            .map(|k| {
                let (_, ids, values) = p.rows_at(k).unwrap();
                ids.iter()
                    .position(|&i| i == id)
                    .map_or(f64::NAN, |r| values[r])
            })
            .collect();
    }
    y.outputs
        .iter()
        .find(|o| o.name == name)
        .unwrap_or_else(|| panic!("no output {name}"))
        .values
        .clone()
}

fn last(y: &Results, name: &str) -> f64 {
    *series(y, name).last().unwrap()
}

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/networks")
}

/// A fresh folder with the bridge's files, for a test to change.
fn copy_of_bridge(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("simsandbox-net-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    for f in ["bridge.ssbx.json", "nodes.csv", "wires.csv"] {
        std::fs::copy(examples().join(f), d.join(f)).unwrap();
    }
    d
}

#[test]
fn the_bridge_settles_at_kirchhoffs_solution() {
    let dir = examples();
    let text = std::fs::read_to_string(dir.join("bridge.ssbx.json")).unwrap();
    let y = in_folder(Some(&dir), || run(&text, &registry(), None, None)).unwrap();
    // Node 11: 1(10 − v₁) + 5(v₂ − v₁) + 3(0 − v₁) = 0; node 12: 2(10 − v₂)
    // + 5(v₁ − v₂) + 4(0 − v₂) = 0 (conductances in mS): v₂ = 230/74,
    // v₁ = (10 + 5 v₂)/9.
    let v2 = 230.0 / 74.0;
    let v1 = (10.0 + 5.0 * v2) / 9.0;
    assert!(
        (last(&y, "bridge.u#11") - v1).abs() < 1e-9,
        "{}",
        last(&y, "bridge.u#11")
    );
    assert!(
        (last(&y, "bridge.u#12") - v2).abs() < 1e-9,
        "{}",
        last(&y, "bridge.u#12")
    );
    // The file's ids, and held nodes held exactly.
    assert_eq!(last(&y, "bridge.u#10"), 10.0);
    assert_eq!(last(&y, "bridge.u#13"), 0.0);
    // Conductances given in mS, read in S.
    assert_eq!(last(&y, "bridge.g#2"), 0.005);
    // The power the held nodes supply is what the wires dissipate.
    let (s, d) = (last(&y, "bridge.supplied"), last(&y, "bridge.dissipated"));
    assert!((s - d).abs() < 1e-12 * d, "{s} {d}");
    // A current out of each free node balances: none flows through
    // capacitances any more.
    let i = |k: usize| last(&y, &format!("bridge.I#{k}"));
    assert!((i(0) - i(2) - i(3)).abs() < 1e-12);
    assert!((i(1) + i(2) - i(4)).abs() < 1e-12);
}

#[test]
fn the_files_are_part_of_the_model() {
    let dir = copy_of_bridge("hash");
    let text = std::fs::read_to_string(dir.join("bridge.ssbx.json")).unwrap();
    let hash = || {
        let s = simsandbox_session::session::Session::load_in("b", &text, &registry(), Some(&dir))
            .unwrap();
        s.run_file("b.json", "", "", "").model_hash
    };
    let before = hash();
    // States over rows integrated in time: it runs in time, not in rounds.
    let s =
        simsandbox_session::session::Session::load_in("b", &text, &registry(), Some(&dir)).unwrap();
    assert!(!s.rounds_pending());
    drop(s);
    let wires = std::fs::read_to_string(dir.join("wires.csv")).unwrap();
    std::fs::write(dir.join("wires.csv"), wires.replace("11,12,5", "11,12,6")).unwrap();
    let after = hash();
    assert_ne!(before, after);
    // And they change the results.
    let y = in_folder(Some(&dir), || run(&text, &registry(), None, None)).unwrap();
    assert_eq!(last(&y, "bridge.g#2"), 0.006);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn bad_files_are_refused_with_reasons() {
    let dir = copy_of_bridge("bad");
    let text = std::fs::read_to_string(dir.join("bridge.ssbx.json")).unwrap();
    let error = |nodes: &str, wires: &str| -> String {
        std::fs::write(dir.join("nodes.csv"), nodes).unwrap();
        std::fs::write(dir.join("wires.csv"), wires).unwrap();
        in_folder(Some(&dir), || run(&text, &registry(), None, None))
            .map(|_| ())
            .expect_err("refused")
    };
    let nodes = "id,fixed,share\n10,1,1\n11,0,0\n12,0,0\n13,1,0\n";
    let wires = "from,to,g [mS]\n10,11,1\n10,12,2\n11,12,5\n11,13,3\n12,13,4\n";
    let e = error(&nodes.replace("fixed", "held"), wires);
    assert!(e.contains("'held' is not a quantity over nodes"), "{e}");
    let e = error(nodes, &wires.replace("12,13,4", "12,14,4"));
    assert!(
        e.contains("14 is not the id of a starting row of nodes"),
        "{e}"
    );
    let e = error(nodes, &wires.replace("from,to", "a,b"));
    assert!(e.contains("'from' and 'to'"), "{e}");
    let e = error(nodes, &wires.replace("g [mS]", "g [s]"));
    assert!(e.contains("the column 'g' is in s"), "{e}");
    let e = error(&nodes.replace("11,0,0", "11,0"), wires);
    assert!(e.contains("2 values for 3 columns"), "{e}");
    let e = error(&nodes.replace("12,0,0", "11,0,0"), wires);
    assert!(e.contains("the id 11 comes twice"), "{e}");
    // A relative file needs the model's folder.
    let e = run(&text, &registry(), None, None).err().unwrap();
    assert!(e.contains("save the model first"), "{e}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The Platoon as one block (ADR 0069 step 2) with its masses given
/// `inline` or read from a file.
fn platoon(masses: &str, inline: bool) -> String {
    let params = if inline {
        json!({"mass[1]": 1500, "mass[2]": 2500})
    } else {
        json!({})
    };
    json!({"format": 1, "run": {"dt": 0.05, "t_end": 20},
        "blocks": [{"name": "cars", "type": "eq.equations", "method": {"name": "rk4"},
            "params": params,
            "equations": [
                "grid k = 0 .. 3, 3",
                "input lead [m/s]",
                "state v[k] [m/s] = 20",
                masses,
                "param kp [N*s/m] = 400",
                "e = prev(v) - v",
                "edge v: k- = lead",
                "dv/dt = kp*e/mass"
            ]},
            {"name": "leader", "type": "input.value", "params": {"y": 25}, "settings": {"unit": "m/s"}}],
        "connections": [{"from": "leader.y", "to": "cars.lead"}]})
    .to_string()
}

#[test]
fn an_array_parameter_from_a_file_is_the_same_as_written_out() {
    let dir = std::env::temp_dir().join(format!("simsandbox-net-masses-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // A file's values, in tonnes: converted to the parameter's kg.
    std::fs::write(dir.join("masses.csv"), "mass [t]\n1\n1.5\n2.5\n").unwrap();
    let written = platoon("param mass[k] [kg] = 1000", true);
    let from_file = platoon("param mass[k] [kg] = file \"masses.csv\"", false);
    let a = run(&written, &registry(), None, None).unwrap();
    let b = in_folder(Some(&dir), || run(&from_file, &registry(), None, None)).unwrap();
    let bits = |y: &Results| {
        series(y, "cars.v")
            .iter()
            .map(|x| x.to_bits())
            .collect::<Vec<_>>()
    };
    assert_eq!(bits(&a), bits(&b));
    // The masses differ, so the followers differ from the leader.
    assert_ne!(
        series(&a, "cars.v")[3 * 20 + 1],
        series(&a, "cars.v")[3 * 20 + 2]
    );
    // One value is not an array; a file of the wrong length is refused.
    let e = in_folder(Some(&dir), || {
        run(
            &platoon("param mass [kg] = file \"masses.csv\"", false),
            &registry(),
            None,
            None,
        )
    })
    .err()
    .unwrap();
    assert!(e.contains("a file gives an array's values"), "{e}");
    std::fs::write(dir.join("masses.csv"), "mass\n1000\n1500\n").unwrap();
    let e = in_folder(Some(&dir), || run(&from_file, &registry(), None, None))
        .err()
        .unwrap();
    assert!(e.contains("2 values of mass, for 3 cells"), "{e}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn parameters_over_rows_hold_in_maps_and_flows_conserve() {
    // A map: each row's value grows by its own rate, a parameter per row.
    let maps = json!({"format": 1, "run": {"dt": 1, "t_end": 3},
    "blocks": [{"name": "m", "type": "eq.equations", "equations": [
        "table pts",
        "start pts = 3",
        "state x[pts] = 1",
        "param r[pts] = 1 + id(pts)",
        "x[n+1] = x*r",
        "total = sum(x)"
    ]}]})
    .to_string();
    let y = run(&maps, &registry(), None, None).unwrap();
    assert_eq!(last(&y, "m.x#2"), 27.0);
    assert_eq!(series(&y, "m.r#1"), vec![2.0; 4]);
    // Diffusion along a chain's edges: the total stays to rounding and
    // every node reaches the mean.
    let chain = json!({"format": 1, "run": {"dt": 0.1, "t_end": 40},
    "blocks": [{"name": "m", "type": "eq.equations", "method": {"name": "rk4"},
        "equations": [
        "table pts",
        "start pts = 4",
        "state p[pts] = (id(pts) < 0.5)*4",
        "edges segs of pts",
        "start segs = chain",
        "param g[segs] = 1 + id(segs)",
        "Q = g*(from_end(p, segs) - to_end(p, segs))",
        "dp/dt = -outflow(Q, segs)",
        "total = sum(p)"
    ]}]})
    .to_string();
    let y = run(&chain, &registry(), None, None).unwrap();
    assert!(series(&y, "m.total")
        .iter()
        .all(|t| (t - 4.0).abs() < 1e-12));
    for k in 0..4 {
        assert!((last(&y, &format!("m.p#{k}")) - 1.0).abs() < 1e-6);
    }
}
