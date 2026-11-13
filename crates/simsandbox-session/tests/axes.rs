//! Outputs over three or more axes (ADR 0081 stages 1–2): recorded, and
//! written by run output in JSON and CSV with their axes named.

use serde_json::json;
use simsandbox_session::examples::registry;
use simsandbox_session::run::run;

/// q = x + 10y + 100z over index axes of 2, 3 and 4 cells: each cell's
/// value says where it is.
fn model() -> String {
    json!({"format": 1, "run": {"dt": 1, "t_end": 1},
        "blocks": [{"name": "m", "type": "eq.equations", "equations": [
            "grid x = 2", "grid y = 3", "grid z = 4",
            "q = x + 10*y + 100*z", "s = sum(q, z)"]}]})
    .to_string()
}

#[test]
fn an_output_over_three_axes_is_written_with_its_axes() {
    let r = registry();
    let y = run(&model(), &r, None, None).unwrap();
    let q = y.outputs.iter().find(|o| o.name == "m.q").unwrap();
    assert_eq!(q.shape.dims(), Some(vec![2, 3, 4]));
    assert_eq!(q.axes, ["x", "y", "z"]);
    // The first axis fastest.
    for (k, &v) in q.values[..24].iter().enumerate() {
        let (i, j, l) = (k % 2, (k / 2) % 3, k / 6);
        assert_eq!(v, (i + 10 * j + 100 * l) as f64);
    }
    // JSON: the axes with their names and lengths.
    let v = y.to_json();
    let shape = &v["outputs"]["m.q"]["shape"];
    assert_eq!(shape["kind"], "axes");
    assert_eq!(
        shape["axes"],
        json!([{"name": "x", "length": 2}, {"name": "y", "length": 3}, {"name": "z", "length": 4}])
    );
    // One or two axes keep their kind, nx and ny, and gain the axes.
    let s = &v["outputs"]["m.s"]["shape"];
    assert_eq!(
        (s["kind"].as_str(), s["nx"].as_u64(), s["ny"].as_u64()),
        (Some("grid"), Some(2), Some(3))
    );
    assert_eq!(s["axes"][1]["name"], "y");
    // CSV: a cell by its places.
    let mut csv = Vec::new();
    y.write_csv(&mut csv).unwrap();
    let header = String::from_utf8(csv)
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string();
    assert!(
        header.contains("m.q[0,0,0],m.q[1,0,0],m.q[0,1,0]"),
        "{header}"
    );
    assert!(
        header.contains("m.q[1,2,3]") && header.contains("m.s[5]"),
        "{header}"
    );
}

#[test]
fn the_session_records_an_output_over_three_axes() {
    let r = registry();
    let mut s = simsandbox_session::session::Session::load("m", &model(), &r).unwrap();
    s.step().unwrap();
    let h = s.signals();
    let f = h
        .fields()
        .iter()
        .find(|f| f.name == "m.q")
        .expect("recorded");
    assert_eq!(f.shape.dims(), Some(vec![2, 3, 4]));
    assert_eq!(f.frame(h.len() - 1)[23], 1.0 + 20.0 + 300.0);
}
