//! Tables of members in typed equations (ADR 0079 stage 1a): rows with
//! stable ids, changed between steps by rules, checked against a direct
//! simulation of the same rules.

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

/// The model: rows with an age and a size. Each round every row ages and
/// grows; rows past an age die; each young row has a newborn; a cap of
/// rows; and from an index axis, a row for each slot whose threshold the
/// round passes.
fn model() -> serde_json::Value {
    json!({"format": 1, "run": {"dt": 1, "t_end": 30},
        "blocks": [{"name": "pop", "type": "eq.equations", "equations": [
            "table pts max 12",
            "start pts = 3",
            "grid k = 3",
            "param life = 6",
            "state age[pts] = id(pts)",
            "state size[pts] = 1 + id(pts)",
            "state round = 0",
            "round[n+1] = round + 1",
            "age[n+1] = age + 1",
            "size[n+1] = size*1.5",
            "remove pts where age > life",
            "add pts from pts where age < 1.5: age = 0, size = size/2",
            "add pts from k where round > 10*(k + 1) - 0.5: age = 0, size = 100 + k",
            "count = rows(pts)",
            "total_age = sum(age)",
            "total_size = sum(size)",
            "newest = maxof(id(pts))",
            "of_two = of(size, 2)"]}]})
}

#[derive(Clone)]
struct Row {
    id: u64,
    age: f64,
    size: f64,
}

/// The same rules applied directly, round by round.
fn reference(rounds: usize) -> Vec<(f64, f64, f64, f64, f64)> {
    let (max, life) = (12, 6.0);
    let mut rows: Vec<Row> = (0..3)
        .map(|i| Row {
            id: i,
            age: i as f64,
            size: 1.0 + i as f64,
        })
        .collect();
    let mut next_id = 3;
    let mut round = 0.0;
    let mut out = Vec::new();
    let mut record = |rows: &[Row]| {
        let count = rows.len() as f64;
        let age: f64 = rows.iter().map(|r| r.age).sum();
        let size: f64 = rows.iter().map(|r| r.size).sum();
        let newest = rows
            .iter()
            .map(|r| r.id as f64)
            .fold(f64::NEG_INFINITY, f64::max);
        let two = rows.iter().find(|r| r.id == 2).map_or(f64::NAN, |r| r.size);
        out.push((count, age, size, newest, two));
    };
    record(&rows);
    for _ in 0..rounds {
        round += 1.0;
        for r in &mut rows {
            r.age += 1.0;
            r.size *= 1.5;
        }
        rows.retain(|r| r.age <= life);
        let young: Vec<Row> = rows.iter().filter(|r| r.age < 1.5).cloned().collect();
        for y in young {
            if rows.len() >= max {
                break;
            }
            rows.push(Row {
                id: next_id,
                age: 0.0,
                size: y.size / 2.0,
            });
            next_id += 1;
        }
        for k in 0..3 {
            if round > 10.0 * (k as f64 + 1.0) - 0.5 && rows.len() < max {
                rows.push(Row {
                    id: next_id,
                    age: 0.0,
                    size: 100.0 + k as f64,
                });
                next_id += 1;
            }
        }
        record(&rows);
    }
    out
}

#[test]
fn rows_follow_their_rules_with_ids_never_reused() {
    let r = registry();
    let y = run(&model().to_string(), &r, None, None).unwrap();
    let want = reference(30);
    let got: Vec<Vec<f64>> = [
        "pop.count",
        "pop.total_age",
        "pop.total_size",
        "pop.newest",
        "pop.of_two",
    ]
    .iter()
    .map(|n| column(&y, n))
    .collect();
    assert_eq!(got[0].len(), want.len());
    for (k, w) in want.iter().enumerate() {
        let row = [w.0, w.1, w.2, w.3, w.4];
        for (c, (&g, &w)) in got.iter().map(|v| &v[k]).zip(&row).enumerate() {
            assert!(
                g == w || (g.is_nan() && w.is_nan()) || (g - w).abs() < 1e-9 * w.abs().max(1.0),
                "round {k}, column {c}: {g} vs {w}"
            );
        }
    }
    // The cap held at some point, and row 2 died and stayed gone.
    assert!(want.iter().any(|w| w.0 == 12.0));
    assert!(got[4].last().unwrap().is_nan());
}

#[test]
fn rows_are_rewound_with_the_model() {
    let r = registry();
    // With a recorder, a snapshot every 5 rounds.
    let mut m = model();
    m["controls"] = json!({"recorder": {"every": 5}});
    let text = m.to_string();
    let mut s = Session::load("pop", &text, &r).unwrap();
    for _ in 0..12 {
        s.step().unwrap();
    }
    let at = |s: &Session, n: &str| -> f64 {
        let h = s.signals();
        *h.values(h.resolve(n).unwrap()).last().unwrap()
    };
    for _ in 0..10 {
        s.step().unwrap();
    }
    // Back to round 12 (or the snapshot before it), then on to round 13:
    // the rows, their values and the next id are where they were, so it
    // is the reference's round 13.
    // Rewinding goes to the snapshot at or before round 12.
    s.rewind_to(12.0);
    assert!(s.time() <= 12.0);
    while s.time() < 12.5 {
        s.step().unwrap();
    }
    let want = reference(13)[13];
    assert_eq!(s.time(), 13.0);
    assert_eq!(at(&s, "pop.count"), want.0);
    assert!((at(&s, "pop.total_size") - want.2).abs() < 1e-9 * want.2);
    assert_eq!(at(&s, "pop.newest"), want.3);
}

#[test]
fn table_mistakes_are_explained() {
    let r = registry();
    let check = |eqs: &[&str]| -> String {
        let f = json!({"format": 1, "blocks": [{"name": "m", "type": "eq.equations", "equations": eqs}]});
        simsandbox_session::describe::check(&f.to_string(), &r).to_string()
    };
    let base = [
        "table pts",
        "start pts = 2",
        "state a[pts] = 1",
        "a[n+1] = a",
    ];
    let with = |extra: &[&str]| {
        let mut l = base.to_vec();
        l.extend_from_slice(extra);
        check(&l)
    };
    assert!(check(&["table pts", "state a[pts] = 1", "da/dt = -a"]).contains("needs a map"));
    assert!(with(&["param m[pts] = 1"]).contains("a value per row is a state"));
    // An input over a table with its own rows is matched to them by id.
    let joined = with(&["input q[pts]", "c = sum(q)"]);
    assert!(joined.contains("'m.q' is not connected"), "{joined}");
    assert!(with(&["grid k = 2", "b = a*k"]).contains("rows and other axes"));
    assert!(
        with(&["d = ddx(a)"]).contains("not an axis")
            || with(&["d = prev(a)"]).contains("no neighbours")
    );
    assert!(with(&["remove q where a > 1"]).contains("not a table"));
    assert!(with(&["add pts: b = 1"]).contains("not a state over pts"));
    assert!(with(&["start qq = 3"]).contains("not a table"));
    assert!(check(&[
        "table pts max 2",
        "start pts = 3",
        "state a[pts] = 1",
        "a[n+1] = a"
    ])
    .contains("at most 2"));
    assert!(
        with(&["state b[pts] = id(pts)", "b[n+1] = b", "add pts: a = 2"]).contains("must give b")
    );
}

#[test]
fn a_map_with_tables_iterates_and_holds_still() {
    // ADR 0083 with tables: rows that stop changing hold the model still.
    let r = registry();
    let f = json!({"format": 1, "run": {"dt": 1, "t_end": 100},
        "blocks": [{"name": "m", "type": "eq.equations", "equations": [
            "table pts max 5",
            "start pts = 1",
            "state v[pts] = 1",
            "v[n+1] = min(v + 1, 3)",
            "add pts where rows(pts) < 4.5: v = 0",
            "total = sum(v)"]}]});
    let mut s = Session::load("m", &f.to_string(), &r).unwrap();
    assert!(s.iterates());
    s.evaluate_later();
    let mut rounds = 0;
    while s.rounds_pending() {
        s.step().unwrap();
        rounds += 1;
        assert!(rounds <= 100);
    }
    // Four rows added one a round, each growing to 3: still after that.
    assert!(s.held_still() && rounds < 20, "{rounds}");
    let h = s.signals();
    assert_eq!(
        *h.values(h.resolve("m.total").unwrap()).last().unwrap(),
        15.0
    );
}
