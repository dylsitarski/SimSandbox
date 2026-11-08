//! Ports carrying a table's rows (ADR 0079 stage 1b): values with the
//! rows' ids and the table's version, wired between typed-equation
//! blocks. A block downstream follows the rows by id, at the same instant.

use serde_json::json;
use simsandbox_session::examples::registry;
use simsandbox_session::run::run;

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
