//! Meshes as tables (ADR 0085 step 1): cells of any listed shape over a
//! table of points, sides kept with stable ids, geometry from the corners,
//! and starting grids, checked against exact areas, volumes and the
//! divergence theorem.

use serde_json::json;
use simsandbox_session::examples::registry;
use simsandbox_session::run::{run, Results};

fn block(eqs: &[String]) -> String {
    json!({"format": 1, "run": {"dt": 1, "t_end": 1},
        "blocks": [{"name": "m", "type": "eq.equations", "equations": eqs}]})
    .to_string()
}

fn last(y: &Results, name: &str) -> f64 {
    *y.outputs
        .iter()
        .find(|o| o.name == name)
        .unwrap_or_else(|| panic!("no {name}"))
        .values
        .last()
        .unwrap()
}

/// A grid of `shape` over the box [0, 2] × [0, 1] (× [0, 3]), its
/// measure, sides and, for each coordinate k, the boundary sum of
/// x_k n_k |side|: by the divergence theorem, the measure.
fn mesh(shape: &str, coords: &[(&str, f64, usize)]) -> Vec<String> {
    let names: Vec<&str> = coords.iter().map(|c| c.0).collect();
    let mut eqs = vec!["table pts".to_string()];
    for n in &names {
        eqs.push(format!("state {n}[pts]"));
        eqs.push(format!("{n}[n+1] = {n}"));
    }
    eqs.push(format!("cells c of pts: {shape}"));
    let grid: Vec<String> = coords
        .iter()
        .map(|(n, hi, boxes)| format!("{n} 0 .. {hi} by {boxes}"))
        .collect();
    eqs.push(format!("start c = grid {}", grid.join(", ")));
    eqs.push("sides f of c".into());
    let xs = names.join(", ");
    eqs.push(format!("size = sum(measure(c, {xs}))"));
    eqs.push("cells = rows(c)".into());
    eqs.push("sides = rows(f)".into());
    eqs.push("outside = sum(boundary(f))".into());
    for (k, n) in names.iter().enumerate() {
        eqs.push(format!(
            "div{k} = sum(boundary(f)*centre({n}, f)*normal(f, {k}, {xs})*measure(f, {xs}))"
        ));
    }
    // The boundary is closed: its outward normals times sizes add to 0.
    eqs.push(format!(
        "closed = sum(boundary(f)*measure(f, {xs})*normal(f, 0, {xs}))"
    ));
    eqs
}

fn check(shape: &str, coords: &[(&str, f64, usize)], cells: f64, sides: f64, outside: f64) {
    let r = registry();
    let y = run(&block(&mesh(shape, coords)), &r, None, None)
        .unwrap_or_else(|e| panic!("{shape}: {e}"));
    let size: f64 = coords.iter().map(|c| c.1).product();
    let close = |a: f64, b: f64| (a - b).abs() <= 1e-12 * b.abs().max(1.0);
    assert!(
        close(last(&y, "m.size"), size),
        "{shape}: size {}",
        last(&y, "m.size")
    );
    assert_eq!(last(&y, "m.cells"), cells, "{shape}");
    assert_eq!(last(&y, "m.sides"), sides, "{shape}");
    assert_eq!(last(&y, "m.outside"), outside, "{shape}");
    for k in 0..coords.len() {
        let d = last(&y, &format!("m.div{k}"));
        assert!(
            close(d, size),
            "{shape}: divergence along {k}: {d} vs {size}"
        );
    }
    assert!(last(&y, "m.closed").abs() < 1e-12, "{shape}");
}

#[test]
fn every_shape_has_its_measure_sides_and_outward_normals() {
    // 1D: 4 segments, 5 points as sides, 2 on the boundary.
    check("segment", &[("x", 2.0, 4)], 4.0, 5.0, 2.0);
    // 2D, 3 × 2 boxes: 12 triangles; sides: 3·3 + 4·2 edges + 6 diagonals.
    check(
        "triangle",
        &[("x", 2.0, 3), ("y", 1.0, 2)],
        12.0,
        23.0,
        10.0,
    );
    check("quad", &[("x", 2.0, 3), ("y", 1.0, 2)], 6.0, 17.0, 10.0);
    // 3D, 2 × 1 × 1 boxes: 6 tetrahedra a box.
    // 48 faces of 12 tetrahedra: 20 on the boundary, 14 inside.
    check(
        "tetra",
        &[("x", 2.0, 2), ("y", 1.0, 1), ("z", 3.0, 1)],
        12.0,
        34.0,
        20.0,
    );
    check(
        "hexa",
        &[("x", 2.0, 2), ("y", 1.0, 1), ("z", 3.0, 1)],
        2.0,
        11.0,
        10.0,
    );
    // Parameter spaces: simplices and boxes of named dimension.
    check("simplex 2", &[("a", 2.0, 2), ("b", 1.0, 2)], 8.0, 16.0, 8.0);
    check(
        "box 3",
        &[("u", 2.0, 1), ("v", 1.0, 2), ("w", 3.0, 1)],
        2.0,
        11.0,
        10.0,
    );
}

#[test]
fn sides_keep_their_ids_when_cells_go() {
    // Remove the cells right of x = 1 after the first step: sides still
    // there keep their ids (the new boundary sides included); the rest go.
    let mut eqs = mesh("triangle", &[("x", 2.0, 2), ("y", 1.0, 1)]);
    eqs.push("cx = centre(x, c)".into());
    eqs.push("sideid = id(f)".into());
    eqs.push("sidemid = centre(x, f)".into());
    eqs.push("remove c where cx > 1".into());
    let r = registry();
    let y = run(&block(&eqs), &r, None, None).unwrap();
    assert_eq!(last(&y, "m.cells"), 2.0);
    assert!((last(&y, "m.size") - 1.0).abs() < 1e-15);
    assert_eq!(last(&y, "m.outside"), 4.0);
    let rows = |n: &str| y.populations.iter().find(|p| p.name == n).unwrap();
    let (ids, mids) = (rows("m.sideid"), rows("m.sidemid"));
    let before: Vec<(u64, f64)> = {
        let (_, i, m) = mids.rows_at(0).unwrap();
        i.iter().map(|&i| i as u64).zip(m.iter().copied()).collect()
    };
    let (_, after_ids, after_mid) = mids.rows_at(1).unwrap();
    assert_eq!(ids.rows_at(1).unwrap().1, after_ids);
    // Each side left is one from before, at the same place, by id.
    for (&id, &m) in after_ids.iter().zip(after_mid) {
        let was = before.iter().find(|b| b.0 == id as u64).unwrap();
        assert_eq!(was.1, m, "side {id}");
        assert!(m <= 1.0);
    }
    // Every side left of x = 1 stayed: 5 of them (3 edges and the
    // diagonal of the left box, and the shared edge at x = 1).
    assert_eq!(after_ids.len(), 5);
    assert!(before.len() > after_ids.len());
}

#[test]
fn cells_hold_values_and_go_with_their_points() {
    // A value per cell and per side, stepped as maps; removing a point
    // removes the cells that use it, and their sides.
    let mut eqs = mesh("quad", &[("x", 2.0, 2), ("y", 1.0, 1)]);
    eqs.push("state u[c] = 1 + id(c)".into());
    eqs.push("u[n+1] = 2*u".into());
    eqs.push("state g[f] = 0".into());
    eqs.push("g[n+1] = g + boundary(f)".into());
    eqs.push("total = sum(u)".into());
    eqs.push("marks = sum(g)".into());
    eqs.push("remove pts where (x > 1.5)*(y > 0.5)".into());
    let r = registry();
    let y = run(&block(&eqs), &r, None, None).unwrap();
    // Two quads (u 1 and 2) doubled; the right one used the corner (2, 1)
    // and went: 2 left. Its 3 outer sides went too; the shared one is now
    // on the boundary with its mark from before (0, inside then).
    assert_eq!(last(&y, "m.cells"), 1.0);
    assert_eq!(last(&y, "m.total"), 2.0);
    assert_eq!(last(&y, "m.sides"), 4.0);
    assert_eq!(last(&y, "m.outside"), 4.0);
    assert_eq!(last(&y, "m.marks"), 3.0);
}

#[test]
fn mesh_mistakes_are_explained() {
    let r = registry();
    let check = |extra: &[&str]| -> String {
        let mut eqs = mesh("triangle", &[("x", 1.0, 1), ("y", 1.0, 1)]);
        eqs.extend(extra.iter().map(|s| s.to_string()));
        let f = json!({"format": 1, "blocks": [{"name": "m", "type": "eq.equations", "equations": eqs}]});
        simsandbox_session::describe::check(&f.to_string(), &r).to_string()
    };
    assert!(check(&[]).contains("\"ok\":true"), "{}", check(&[]));
    assert!(check(&["cells d of pts: hexagon"]).contains("is not a shape"));
    assert!(check(&["cells d of pts: prism", "sides h of d"]).contains("two shapes"));
    assert!(check(&["k = corner(x, c, 3)"]).contains("numbered 0 to 2"));
    assert!(check(&["k = inner(x, f)"]).contains("the cells f lie between"));
    assert!(check(&["k = normal(c, 0, x, y)"]).contains("takes sides"));
    assert!(check(&["split c where 1"]).contains("ADR 0085 step 2"));
    assert!(check(&["add c: x = 1"]).contains("not added"));
    assert!(check(&["remove f where 1"]).contains("kept by SimSandbox"));
    let no_y = json!({"format": 1, "blocks": [{"name": "m", "type": "eq.equations", "equations": [
        "table pts", "state x[pts]", "x[n+1] = x", "cells c of pts: triangle",
        "start c = grid x 0 .. 1 by 1, y 0 .. 1 by 1"]}]});
    let e = simsandbox_session::describe::check(&no_y.to_string(), &r).to_string();
    assert!(e.contains("declare 'state y[pts]'"), "{e}");
}
