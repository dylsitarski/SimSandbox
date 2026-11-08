//! Edge tables (ADR 0079 stage 2): rows that join two rows of a table,
//! values at their ends, and rules that split and merge them, checked
//! against the same rules applied directly.

use serde_json::json;
use simsandbox_session::examples::registry;
use simsandbox_session::run::{run, Results};
use simsandbox_session::session::Session;

fn rows_of<'a>(y: &'a Results, name: &str) -> &'a simsandbox_session::run::Population {
    y.populations
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no rows {name}"))
}

fn column(y: &Results, name: &str) -> Vec<f64> {
    y.outputs
        .iter()
        .find(|o| o.name == name)
        .unwrap_or_else(|| panic!("no output {name}"))
        .values
        .clone()
}

fn block(eqs: &[&str], t_end: f64) -> String {
    json!({"format": 1, "run": {"dt": 1, "t_end": t_end},
        "blocks": [{"name": "m", "type": "eq.equations", "equations": eqs}]})
    .to_string()
}

/// Refinement of a step over [0, 1]: split every interval whose ends
/// differ, until it is shorter than a tolerance.
const REFINE: &[&str] = &[
    "table pts max 200",
    "start pts = 5",
    "state x[pts] = id(pts)/4",
    "state y[pts] = 7*id(pts)",
    "x[n+1] = x",
    "y[n+1] = y",
    "param at = 0.37",
    "param tol = 0.01",
    "f = (x > at)",
    "edges segs of pts",
    "start segs = chain",
    "len = to_end(x, segs) - from_end(x, segs)",
    "jump = abs(to_end(f, segs) - from_end(f, segs))",
    "split segs where (jump > 0.5)*(len > tol): x = mean",
    "count = rows(pts)",
    "total = sum(len)",
];

/// The same rules done directly: rows (id, x, y), edges (from, to) in id
/// order, new ids in edge order.
#[allow(clippy::type_complexity)]
fn reference(rounds: usize) -> Vec<(Vec<(u64, f64, f64)>, Vec<(u64, u64)>)> {
    let mut pts: Vec<(u64, f64, f64)> = (0..5)
        .map(|i| (i, i as f64 / 4.0, 7.0 * i as f64))
        .collect();
    let mut segs: Vec<(u64, u64)> = (0..4).map(|i| (i, i + 1)).collect();
    let mut next = 5;
    let mut out = vec![(pts.clone(), segs.clone())];
    for _ in 0..rounds {
        let at = |id: u64| pts.iter().find(|p| p.0 == id).copied().unwrap();
        let f = |x: f64| f64::from(u8::from(x > 0.37));
        let mut new_pts = Vec::new();
        let mut new_segs = segs.clone();
        for (k, &(a, b)) in segs.iter().enumerate() {
            let (pa, pb) = (at(a), at(b));
            if (f(pb.1) - f(pa.1)).abs() > 0.5 && pb.1 - pa.1 > 0.01 {
                let m = next;
                next += 1;
                new_pts.push((m, (pa.1 + pb.1) / 2.0, pa.2));
                new_segs[k] = (a, m);
                new_segs.push((m, b));
            }
        }
        pts.extend(new_pts);
        segs = new_segs;
        out.push((pts.clone(), segs.clone()));
    }
    out
}

#[test]
fn splitting_refines_a_step_as_the_rules_say() {
    let r = registry();
    let y = run(&block(REFINE, 8.0), &r, None, None).unwrap();
    let want = reference(8);
    let (x, yv, len) = (rows_of(&y, "m.x"), rows_of(&y, "m.y"), rows_of(&y, "m.len"));
    for (k, (pts, segs)) in want.iter().enumerate() {
        let (_, ids, xs) = x.rows_at(k).unwrap();
        let ids: Vec<u64> = ids.iter().map(|&i| i as u64).collect();
        assert_eq!(
            ids,
            pts.iter().map(|p| p.0).collect::<Vec<_>>(),
            "round {k}"
        );
        // Bit for bit: mean of the ends, copy of the from end.
        assert_eq!(xs, pts.iter().map(|p| p.1).collect::<Vec<_>>(), "round {k}");
        assert_eq!(
            yv.rows_at(k).unwrap().2,
            pts.iter().map(|p| p.2).collect::<Vec<_>>(),
            "round {k}"
        );
        let (_, eids, lens) = len.rows_at(k).unwrap();
        assert_eq!(eids.len(), segs.len(), "round {k}");
        let at = |id: u64| pts.iter().find(|p| p.0 == id).unwrap().1;
        let want_len: Vec<f64> = segs.iter().map(|&(a, b)| at(b) - at(a)).collect();
        assert_eq!(lens, want_len, "round {k}");
    }
    // Halving is exact: the lengths add to 1 every round, and the step is
    // bracketed by an interval shorter than the tolerance.
    assert!(column(&y, "m.total").iter().all(|&t| t == 1.0));
    let (pts, segs) = want.last().unwrap();
    let at = |id: u64| pts.iter().find(|p| p.0 == id).unwrap().1;
    let bracket = segs
        .iter()
        .find(|&&(a, b)| at(a) <= 0.37 && at(b) > 0.37)
        .unwrap();
    assert!(at(bracket.1) - at(bracket.0) <= 0.01);
    assert_eq!(*column(&y, "m.count").last().unwrap(), pts.len() as f64);
    assert_eq!(pts.len(), 5 + 5, "one new point a round until 1/256");
}

#[test]
fn merging_puts_two_rows_into_one() {
    let r = registry();
    let eqs = [
        "table pts",
        "start pts = 9",
        "state x[pts] = id(pts)",
        "state y[pts] = id(pts)^2",
        "x[n+1] = x",
        "y[n+1] = y",
        "edges segs of pts",
        "start segs = chain",
        "len = to_end(x, segs) - from_end(x, segs)",
        "merge segs where len < 1.5: y = copy",
        "count = rows(pts)",
        "edges = rows(segs)",
    ];
    let y = run(&block(&eqs, 1.0), &r, None, None).unwrap();
    // Edges in order: 0–1 merges, 1–2 waits (1 is taken), 2–3 merges, ….
    let (_, ids, xs) = rows_of(&y, "m.x").rows_at(1).unwrap();
    assert_eq!(ids, &[0.0, 2.0, 4.0, 6.0, 8.0][..]);
    assert_eq!(xs, &[0.5, 2.5, 4.5, 6.5, 8.0][..], "x: mean, the default");
    let (_, _, ys) = rows_of(&y, "m.y").rows_at(1).unwrap();
    assert_eq!(
        ys,
        &[0.0, 4.0, 16.0, 36.0, 64.0][..],
        "y: copy of the from row"
    );
    // The edges left join the rows that stayed, as a chain.
    let (_, eids, lens) = rows_of(&y, "m.len").rows_at(1).unwrap();
    assert_eq!(eids, &[1.0, 3.0, 5.0, 7.0][..]);
    assert_eq!(lens, &[2.0, 2.0, 2.0, 1.5][..]);
}

#[test]
fn removing_rows_removes_their_edges_and_rewind_keeps_them() {
    let r = registry();
    let eqs = [
        "table pts",
        "start pts = 5",
        "state x[pts] = id(pts)",
        "x[n+1] = x",
        "edges segs of pts",
        "start segs = chain",
        "len = to_end(x, segs) - from_end(x, segs)",
        "remove pts where (x > 1.5)*(x < 2.5)",
        "edges = rows(segs)",
        "total = sum(len)",
    ];
    let y = run(&block(&eqs, 2.0), &r, None, None).unwrap();
    assert_eq!(column(&y, "m.edges"), [4.0, 2.0, 2.0]);
    assert_eq!(column(&y, "m.total"), [4.0, 2.0, 2.0]);
    // Snapshots keep the edges: back and on again gives the same.
    let mut m: serde_json::Value = serde_json::from_str(&block(REFINE, 8.0)).unwrap();
    m["controls"] = json!({"recorder": {"every": 1}});
    let mut s = Session::load("m", &m.to_string(), &r).unwrap();
    for _ in 0..6 {
        s.step().unwrap();
    }
    let total = |s: &Session| {
        let h = s.signals();
        *h.values(h.resolve("m.count").unwrap()).last().unwrap()
    };
    let at6 = total(&s);
    s.rewind_to(3.0);
    while s.time() < 5.5 {
        s.step().unwrap();
    }
    assert_eq!(total(&s), at6);
}

#[test]
fn edge_mistakes_are_explained() {
    let r = registry();
    let check = |eqs: &[&str]| -> String {
        let f = json!({"format": 1, "blocks": [{"name": "m", "type": "eq.equations", "equations": eqs}]});
        simsandbox_session::describe::check(&f.to_string(), &r).to_string()
    };
    let base = [
        "table pts",
        "start pts = 3",
        "state x[pts] = id(pts)",
        "x[n+1] = x",
        "edges segs of pts",
        "start segs = chain",
    ];
    let with = |extra: &[&str]| {
        let mut l = base.to_vec();
        l.extend_from_slice(extra);
        check(&l)
    };
    assert!(with(&[]).contains("\"ok\":true"), "{}", with(&[]));
    assert!(check(&["edges segs of pts"]).contains("is not a table declared above"));
    assert!(with(&["state w[segs] = 0", "w[n+1] = w"]).contains("edges hold no values"));
    assert!(with(&["add segs: x = 1"]).contains("not added"));
    assert!(with(&["split pts where x > 1"]).contains("is not an edge table"));
    assert!(with(&["grid k = 2", "d = from_end(k, segs)"]).contains("whose rows segs joins"));
    assert!(with(&["start other = chain"]).contains("is not an edge table"));
    assert!(with(&["split segs where 1: w = 2"]).contains("not a state over pts"));
}
