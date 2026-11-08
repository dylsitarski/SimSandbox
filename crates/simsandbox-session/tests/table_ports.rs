//! Ports carrying a table's rows (ADR 0079 stage 1b): values with the
//! rows' ids and the table's version, wired between typed-equation
//! blocks. A block downstream follows the rows by id, at the same instant.

use serde_json::json;
use simsandbox_session::examples::registry;
use simsandbox_session::run::run;
use simsandbox_session::session::Session;

fn column(r: &simsandbox_session::run::Results, name: &str) -> Vec<f64> {
    r.outputs
        .iter()
        .find(|o| o.name == name)
        .unwrap_or_else(|| panic!("no output {name}"))
        .values
        .clone()
}

/// Rows that age, die, have newborns and are capped (as in the stage 1a
/// test); `size` leaves as rows. `acc` follows them: it sums each row's
/// size over the rounds it has seen it, and passes twice the size on to
/// `twice`, which follows `acc`'s rows in turn.
fn model() -> serde_json::Value {
    json!({"format": 1, "run": {"dt": 1, "t_end": 30},
        "blocks": [
            {"name": "pop", "type": "eq.equations", "equations": [
                "table pts max 12",
                "start pts = 3",
                "param life = 6",
                "state age[pts] = id(pts)",
                "state size[pts] = 1 + id(pts)",
                "age[n+1] = age + 1",
                "size[n+1] = size*1.5",
                "remove pts where age > life",
                "add pts from pts where age < 1.5: age = 0, size = size/2",
                "count = rows(pts)",
                "total = sum(size)",
                "newest = maxof(id(pts))",
                "of_two = of(size, 2)"]},
            {"name": "acc", "type": "eq.equations", "equations": [
                "table q",
                "input size[q]",
                "state seen[q] = 0",
                "seen[n+1] = seen + size",
                "doubled = 2*size",
                "count = rows(q)",
                "total = sum(size)",
                "newest = maxof(id(q))",
                "of_two = of(seen, 2)"]},
            {"name": "twice", "type": "eq.equations", "equations": [
                "table r",
                "input doubled[r]",
                "total = sum(doubled)",
                "newest = maxof(id(r))"]}],
        "connections": [
            {"from": "pop.size", "to": "acc.size"},
            {"from": "acc.doubled", "to": "twice.doubled"}]})
}

#[test]
fn rows_pass_between_blocks_by_id_at_the_same_instant() {
    let r = registry();
    let y = run(&model().to_string(), &r, None, None).unwrap();
    let c = |n: &str| column(&y, n);
    let (pop_total, pop_two) = (c("pop.total"), c("pop.of_two"));
    assert_eq!(pop_total.len(), 31);
    // The same rows, in the same order, at every instant: counts, ids and
    // sums agree to the bit.
    assert_eq!(c("acc.count"), c("pop.count"));
    assert_eq!(c("acc.newest"), c("pop.newest"));
    assert_eq!(c("acc.total"), pop_total);
    assert_eq!(c("twice.newest"), c("pop.newest"));
    for (k, (&t, &p)) in c("twice.total").iter().zip(&pop_total).enumerate() {
        assert!((t - 2.0 * p).abs() <= 1e-12 * p, "round {k}: {t} vs 2×{p}");
    }
    // Row 2's own sum, kept by id while other rows come and go: the sizes
    // it had in the rounds before, gone with it.
    let seen = c("acc.of_two");
    let mut sum = 0.0;
    for (k, &s) in seen.iter().enumerate() {
        if pop_two[k].is_nan() {
            assert!(s.is_nan(), "round {k}: row 2 is gone, but {s}");
        } else {
            assert_eq!(s, sum, "round {k}");
            sum += pop_two[k];
        }
    }
    assert!(pop_two.iter().any(|v| v.is_nan()) && sum > 0.0);
}

#[test]
fn rows_ports_are_described_and_checked() {
    let r = registry();
    let text = model().to_string();
    let d = simsandbox_session::describe::check(&text, &r);
    assert_eq!(d["ok"], json!(true), "{d}");
    let check = |m: serde_json::Value| simsandbox_session::describe::check(&m.to_string(), &r);
    // Rows into a scalar, and a scalar into rows: refused by shape.
    let mut m = model();
    m["blocks"][2]["equations"] = json!(["input doubled", "total = doubled"]);
    let e = check(m).to_string();
    assert!(e.contains("rows"), "{e}");
    // A followed table's rows come from its input only.
    let mut m = model();
    m["blocks"][1]["equations"][0] = json!("table q max 20");
    m["blocks"][1]["equations"]
        .as_array_mut()
        .unwrap()
        .push(json!("start q = 2"));
    assert!(check(m)
        .to_string()
        .contains("follows the rows of the input size"));
    let mut m = model();
    m["blocks"][1]["equations"]
        .as_array_mut()
        .unwrap()
        .push(json!("remove q where seen > 3"));
    assert!(check(m)
        .to_string()
        .contains("follows the rows of the input size"));
}

#[test]
fn a_loop_through_rows_is_refused() {
    // b's rows follow a's at the same instant, and a's follow b's.
    let r = registry();
    let m = json!({"format": 1, "run": {"dt": 1, "t_end": 3},
        "blocks": [
            {"name": "a", "type": "eq.equations", "equations": [
                "table p", "input u[p]", "y = u + 1"]},
            {"name": "b", "type": "eq.equations", "equations": [
                "table p", "input u[p]", "y = u + 1"]}],
        "connections": [
            {"from": "a.y", "to": "b.u"},
            {"from": "b.y", "to": "a.u"}]});
    let e = run(&m.to_string(), &r, None, None).unwrap_err().to_string();
    assert!(e.contains("loop through a table's rows"), "{e}");
}

/// Row 2's size at each round, as `pop.of_two` gives it (NaN once gone).
fn row_two(y: &simsandbox_session::run::Results) -> Vec<f64> {
    column(y, "pop.of_two")
}

#[test]
fn rows_are_recorded_by_id_and_written_out() {
    // ADR 0079 stage 1c: the history, `run` (CSV and JSON) and probes keep
    // rows with their ids, so a row is followed by id.
    let r = registry();
    let text = model().to_string();
    let y = run(&text, &r, None, None).unwrap();
    let two = row_two(&y);
    let size = y.populations.iter().find(|p| p.name == "pop.size").unwrap();
    assert!(size.rows && size.dim == 1 && size.frames.len() == 31);
    // The same rows downstream, by id, at every round.
    let seen = y.populations.iter().find(|p| p.name == "acc.seen").unwrap();
    for k in 0..31 {
        let (v1, ids1, _) = size.rows_at(k).unwrap();
        let (_, ids2, _) = seen.rows_at(k).unwrap();
        assert_eq!(ids1, ids2, "round {k}");
        assert!(k == 0 || v1 >= size.rows_at(k - 1).unwrap().0);
    }
    // CSV: a column per id ever seen; row 2's is `of_two`, NaN once gone.
    let mut csv = Vec::new();
    y.write_csv(&mut csv).unwrap();
    let csv = String::from_utf8(csv).unwrap();
    let header: Vec<&str> = csv.lines().next().unwrap().split(',').collect();
    let col = header.iter().position(|h| *h == "pop.size#2").unwrap();
    let ids = size.ids();
    assert_eq!(
        header.iter().filter(|h| h.starts_with("pop.size#")).count(),
        ids.len()
    );
    for (k, line) in csv.lines().skip(1).enumerate() {
        let v: f64 = line.split(',').nth(col).unwrap().parse().unwrap();
        assert!(v == two[k] || (v.is_nan() && two[k].is_nan()), "round {k}");
    }
    // JSON: versions, ids and values per round.
    let j = y.to_json();
    let s = &j["over_rows"]["pop.size"];
    assert_eq!(s["ids"][0], json!([0, 1, 2]));
    assert_eq!(s["values"][0], json!([1.0, 2.0, 3.0]));
    assert_eq!(s["versions"].as_array().unwrap().len(), 31);
    assert!(j["populations"].as_object().unwrap().is_empty());

    // The session's history, followed by id.
    let mut s = Session::load("m", &text, &r).unwrap();
    for _ in 0..30 {
        s.step().unwrap();
    }
    let h = s.signals();
    let f = h.fields().iter().find(|f| f.name == "pop.size").unwrap();
    let by_id = f.by_id(2);
    assert_eq!(by_id.len(), 31);
    for (k, (&a, &b)) in by_id.iter().zip(&two).enumerate() {
        assert!(
            a == b || (a.is_nan() && b.is_nan()),
            "round {k}: {a} vs {b}"
        );
    }
    assert_eq!(f.ids(), ids);
}

#[test]
fn a_probe_writes_rows_by_id_and_a_rewind_cuts_them_back() {
    let r = registry();
    let mut m = model();
    m["controls"] = json!({"probes": [{"name": "sizes", "record": ["pop.size"],
        "to": "sizes.csv", "stream": true}]});
    let dir = std::env::temp_dir().join(format!("simsandbox-rows-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::load("m", &m.to_string(), &r).unwrap();
    s.set_recorder(Some(Default::default()));
    s.set_probe_dir(Some(dir.clone()));
    s.restart().unwrap();
    s.run_until(20.0).unwrap();
    s.flush(true);
    let rows = || std::fs::read_to_string(dir.join("sizes.pop.size.csv")).unwrap();
    let all = rows();
    assert!(
        all.starts_with("t,version,id,value\n0,3,0,1\n0,3,1,2\n0,3,2,3\n"),
        "{all}"
    );
    // One line per row per round: as many as the rows the run had.
    let y = run(&model().to_string(), &r, Some(20.0), None).unwrap();
    let counts = column(&y, "pop.count");
    assert_eq!(all.lines().count() - 1, counts.iter().sum::<f64>() as usize);
    // A rewind cuts the rows back; running on writes the same again.
    s.rewind_to(10.0);
    s.flush(true);
    let cut = rows();
    assert!(cut
        .lines()
        .skip(1)
        .all(|l| l.split(',').next().unwrap().parse::<f64>().unwrap() <= 10.0));
    assert!(cut.len() < all.len() && all.starts_with(&cut));
    s.run_until(20.0).unwrap();
    s.flush(true);
    assert_eq!(rows(), all);
    std::fs::remove_dir_all(&dir).unwrap();
}
