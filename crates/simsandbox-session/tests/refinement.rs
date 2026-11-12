//! Refinement of meshes (ADR 0085 step 2): `split` and `merge` on cells,
//! simplices by bisection and boxes into 2^d, checked by exact measures,
//! the divergence theorem every step (no hanging point is left open), a
//! shared value that stays equal to each cell's measure, and merging back
//! to the starting mesh.

use serde_json::json;
use simsandbox_session::examples::registry;
use simsandbox_session::run::{run, Results};

fn block(eqs: &[String], t_end: f64) -> String {
    json!({"format": 1, "run": {"dt": 1, "t_end": t_end},
        "blocks": [{"name": "m", "type": "eq.equations", "equations": eqs}]})
    .to_string()
}

fn column(y: &Results, name: &str) -> Vec<f64> {
    y.outputs
        .iter()
        .find(|o| o.name == name)
        .unwrap_or_else(|| panic!("no {name}"))
        .values
        .clone()
}

/// A grid of `shape` over the unit box, split near a point for `rounds`
/// steps, then merged back; `m` is each cell's measure from the first
/// step, and is
/// shared on split and summed on merge.
fn model(shape: &str, boxes: &[usize], rounds: usize) -> Vec<String> {
    let names: Vec<String> = (0..boxes.len()).map(|k| format!("x{k}")).collect();
    let xs = names.join(", ");
    let d = boxes.len();
    let mut eqs = vec!["table pts max 100000".to_string()];
    for n in &names {
        eqs.push(format!("state {n}[pts]"));
        eqs.push(format!("{n}[n+1] = {n}"));
    }
    eqs.push(format!("cells c of pts max 100000: {shape}"));
    let grid: Vec<String> = names
        .iter()
        .zip(boxes)
        .map(|(n, b)| format!("{n} 0 .. 1 by {b}"))
        .collect();
    eqs.push(format!("start c = grid {}", grid.join(", ")));
    eqs.push("sides f of c".into());
    eqs.push("state k = 0".into());
    eqs.push("k[n+1] = k + 1".into());
    // Each cell's measure, from the first step on.
    eqs.push("state m[c] = 0".into());
    eqs.push(format!("m[n+1] = (k < 0.5)*measure(c, {xs}) + (k > 0.5)*m"));
    let dist: Vec<String> = names
        .iter()
        .enumerate()
        .map(|(i, n)| format!("(centre({n}, c) - {})^2", 0.31 - 0.02 * i as f64))
        .collect();
    eqs.push(format!(
        "near = ({}) < 4*measure(c, {xs})^(2/{d})",
        dist.join(" + ")
    ));
    eqs.push(format!("split c where (k <= {rounds})*near: m = share"));
    eqs.push(format!("merge c where k > {rounds}: m = sum"));
    eqs.push(format!("size = sum(measure(c, {xs}))"));
    eqs.push("mass = sum(m)".into());
    eqs.push(format!("off = maxof(abs(m - measure(c, {xs})))"));
    eqs.push("cells = rows(c)".into());
    eqs.push("points = rows(pts)".into());
    eqs.push("sides = rows(f)".into());
    for (k, n) in names.iter().enumerate() {
        eqs.push(format!(
            "div{k} = sum(boundary(f)*centre({n}, f)*normal(f, {k}, {xs})*measure(f, {xs}))"
        ));
    }
    eqs
}

fn check(shape: &str, boxes: &[usize], rounds: usize) {
    let r = registry();
    let steps = rounds + 2 * rounds + 4;
    let y = run(
        &block(&model(shape, boxes, rounds), steps as f64),
        &r,
        None,
        None,
    )
    .unwrap_or_else(|e| panic!("{shape}: {e}"));
    let cells = column(&y, "m.cells");
    let points = column(&y, "m.points");
    // It refines, then merges back to where it started.
    let most = cells.iter().copied().fold(0.0, f64::max);
    assert!(most > 4.0 * cells[0], "{shape}: {cells:?}");
    assert_eq!(cells.last(), cells.first(), "{shape}: {cells:?}");
    assert_eq!(points.last(), points.first(), "{shape}: {points:?}");
    assert_eq!(column(&y, "m.sides").last(), column(&y, "m.sides").first());
    let close = |a: f64, b: f64| (a - b).abs() <= 1e-12;
    for (s, size) in column(&y, "m.size").iter().enumerate() {
        assert!(close(*size, 1.0), "{shape} step {s}: size {size}");
        // Shares are exact halves (or 2^-d), so each cell's share is its
        // measure and the sum is kept (to the rounding of summing values
        // such as 1/6).
        if s > 0 {
            assert!(close(column(&y, "m.mass")[s], 1.0), "{shape} step {s}");
            assert!(column(&y, "m.off")[s] <= 1e-15, "{shape} step {s}");
        }
        // The boundary sum of x_k n_k |side| is the volume: every inner
        // side is two cells', or a small cell's on a large one's.
        for k in 0..boxes.len() {
            let div = column(&y, &format!("m.div{k}"))[s];
            assert!(close(div, 1.0), "{shape} step {s}: div{k} {div}");
        }
    }
}

#[test]
fn triangles_bisect_without_hanging_points_and_merge_back() {
    check("triangle", &[2, 2], 8);
}

#[test]
fn tetrahedra_bisect_without_hanging_points_and_merge_back() {
    check("tetra", &[1, 1, 1], 6);
}

#[test]
fn quads_split_in_four_with_hanging_sides_and_merge_back() {
    check("quad", &[2, 2], 5);
}

#[test]
fn boxes_in_three_dimensions_split_in_eight_and_merge_back() {
    check("hexa", &[2, 2, 2], 3);
    check("box 3", &[1, 1, 1], 3);
}

#[test]
fn refinement_mistakes_are_explained() {
    let r = registry();
    let check = |extra: &[&str], shape: &str| -> String {
        let mut eqs = vec![
            "table pts".to_string(),
            "state x[pts]".into(),
            "state y[pts]".into(),
            "x[n+1] = x".into(),
            "y[n+1] = y".into(),
            format!("cells c of pts: {shape}"),
            "state u[c] = 1".into(),
            "u[n+1] = u".into(),
        ];
        if shape != "prism" {
            eqs.push("start c = grid x 0 .. 1 by 1, y 0 .. 1 by 1".into());
        }
        eqs.extend(extra.iter().map(|s| s.to_string()));
        let f = json!({"format": 1, "blocks": [{"name": "m", "type": "eq.equations", "equations": eqs}]});
        simsandbox_session::describe::check(&f.to_string(), &r).to_string()
    };
    let ok = check(
        &[
            "split c where 1: u = share, x = mean",
            "merge c where 1: u = sum",
        ],
        "triangle",
    );
    assert!(ok.contains("\"ok\":true"), "{ok}");
    assert!(check(&["split c where 1"], "prism").contains("not refined yet"));
    assert!(check(&["split c where 1: u = sum"], "quad").contains("copy, share or a value"));
    assert!(check(&["split c where 1: x = share"], "quad").contains("is mean or a value"));
    assert!(check(&["merge c where 1: x = mean"], "quad").contains("leaves the points' values"));
    assert!(check(&["merge c where 1: u = share"], "quad").contains("mean, sum, copy"));
    assert!(check(&["split c where 1: w = 1"], "quad").contains("not a state over pts or c"));
    assert!(check(&["sides f of c", "split f where 1"], "quad").contains("kept by SimSandbox"));
    assert!(check(&["split c where x > 1"], "quad").contains("the condition is over pts"));
}

#[test]
fn rewinding_a_refined_mesh_gives_the_same_mesh_again() {
    let r = registry();
    let mut m: serde_json::Value =
        serde_json::from_str(&block(&model("quad", &[2, 2], 4), 12.0)).unwrap();
    m["controls"] = json!({"recorder": {"every": 1}});
    let mut s = simsandbox_session::session::Session::load("m", &m.to_string(), &r).unwrap();
    let cells = |s: &simsandbox_session::session::Session| {
        let h = s.signals();
        *h.values(h.resolve("m.cells").unwrap()).last().unwrap()
    };
    let mut forward = Vec::new();
    for _ in 0..8 {
        s.step().unwrap();
        forward.push(cells(&s));
    }
    s.rewind_to(3.0);
    let mut again = Vec::new();
    while s.time() < 7.5 {
        s.step().unwrap();
        again.push(cells(&s));
    }
    assert_eq!(again, forward[3..]);
}

/// Triangles from a file are labelled by their longest side, and bisect
/// without hanging points.
#[test]
fn triangles_from_a_file_bisect_too() {
    // [0, 2] × [0, 1] as four triangles around its centre.
    const MSH: &str = "$MeshFormat\n2.2 0 8\n$EndMeshFormat\n$Nodes\n5\n1 0 0 0\n2 2 0 0\n3 2 1 0\n4 0 1 0\n5 1 0.5 0\n$EndNodes\n$Elements\n4\n1 2 2 1 1 1 2 5\n2 2 2 1 1 2 3 5\n3 2 2 1 1 3 4 5\n4 2 2 1 1 4 1 5\n$EndElements\n";
    let dir = std::env::temp_dir().join(format!("simsandbox-refine-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("four.msh"), MSH).unwrap();
    let eqs = [
        "table pts max 10000",
        "state x[pts]",
        "state y[pts]",
        "x[n+1] = x",
        "y[n+1] = y",
        "cells c of pts max 10000: triangle",
        "start c = file \"four.msh\" at x, y",
        "sides f of c",
        "split c where ((centre(x, c) - 0.3)^2 + (centre(y, c) - 0.2)^2) < 4*measure(c, x, y)",
        "size = sum(measure(c, x, y))",
        "cells = rows(c)",
        "div = sum(boundary(f)*centre(x, f)*normal(f, 0, x, y)*measure(f, x, y))",
    ];
    let text = json!({"format": 1, "run": {"dt": 1, "t_end": 10},
        "blocks": [{"name": "m", "type": "eq.equations", "equations": eqs}]})
    .to_string();
    let s = simsandbox_session::session::Session::load_in("four", &text, &registry(), Some(&dir))
        .unwrap();
    let mut s = s;
    while s.time() < 9.5 {
        s.step().unwrap();
    }
    let h = s.signals();
    let at = |n: &str| h.values(h.resolve(n).unwrap()).to_vec();
    assert!(*at("m.cells").last().unwrap() > 40.0);
    for (size, div) in at("m.size").iter().zip(at("m.div")) {
        assert!(
            (size - 2.0).abs() < 1e-12 && (div - 2.0).abs() < 1e-12,
            "{size} {div}"
        );
    }
    drop(h);
    std::fs::remove_dir_all(&dir).unwrap();
}
