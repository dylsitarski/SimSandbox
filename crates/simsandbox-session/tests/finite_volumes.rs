//! Time on meshes (ADR 0085 step 3): continuous states over cells, rates
//! from sums over each cell's sides (`outflow`), integrated with the mesh
//! fixed within a step and refined between steps; checked against exact
//! solutions and against fine fixed grids.

use serde_json::json;
use simsandbox_session::examples::registry;
use simsandbox_session::run::{run, Results};

fn column(y: &Results, name: &str) -> Vec<f64> {
    y.outputs
        .iter()
        .find(|o| o.name == name)
        .unwrap_or_else(|| panic!("no {name}"))
        .values
        .clone()
}

fn last(y: &Results, name: &str) -> f64 {
    *column(y, name).last().unwrap()
}

fn model(eqs: &[String], method: &str, dt: f64, t_end: f64) -> String {
    json!({"format": 1, "run": {"dt": dt, "t_end": t_end},
        "blocks": [{"name": "m", "type": "eq.equations", "method": {"name": method},
                    "equations": eqs}]})
    .to_string()
}

/// A mesh of `shape` over the unit box with coordinates x, y (and z),
/// fixed in time, and the two-point flux of `u` across each side: insulated
/// on the boundary, where `across` is the cell's own value.
fn mesh(shape: &str, boxes: &[usize]) -> Vec<String> {
    let names = ["x", "y", "z"];
    let names = &names[..boxes.len()];
    let xs = names.join(", ");
    let mut eqs = vec!["table pts max 1000000".to_string()];
    for n in names {
        eqs.push(format!("state {n}[pts]"));
        eqs.push(format!("d{n}/dt = 0"));
    }
    eqs.push(format!("cells c of pts max 1000000: {shape}"));
    let grid: Vec<String> = names
        .iter()
        .zip(boxes)
        .map(|(n, b)| format!("{n} 0 .. 1 by {b}"))
        .collect();
    eqs.push(format!("start c = grid {}", grid.join(", ")));
    eqs.push("sides f of c".into());
    for n in names {
        eqs.push(format!("c{n} = centre({n}, c)"));
    }
    let gap: Vec<String> = names
        .iter()
        .map(|n| format!("(across(c{n}, f) - inner(c{n}, f))^2"))
        .collect();
    eqs.push(format!("gap = sqrt({}) + boundary(f)", gap.join(" + ")));
    eqs.push(format!("vol = measure(c, {xs})"));
    eqs.push(format!(
        "flux = (inner(u, f) - across(u, f))*measure(f, {xs})/gap"
    ));
    eqs.push("du/dt = -outflow(flux, f)/vol".into());
    eqs.push("total = sum(u*vol)".into());
    eqs.push("cells = rows(c)".into());
    eqs
}

/// The heat equation on a fixed grid of quads: the slowest cosine mode
/// decays as exp(−2π²t); second order in the cell size, and the total
/// heat kept (insulated walls).
#[test]
fn heat_on_quads_decays_as_the_exact_mode() {
    let r = registry();
    let mut errors = Vec::new();
    for n in [8, 16, 32] {
        let mut eqs = mesh("quad", &[n, n]);
        eqs.push("state u[c] = cos(pi*centre(x, c))*cos(pi*centre(y, c))".into());
        eqs.push("err = maxof(abs(u - exp(-2*pi^2*t)*cos(pi*cx)*cos(pi*cy)))".into());
        let y = run(&model(&eqs, "trbdf2", 0.001, 0.05), &r, None, None).unwrap();
        let total = column(&y, "m.total");
        assert!(total.iter().all(|v| v.abs() < 1e-12), "{total:?}");
        errors.push(last(&y, "m.err"));
    }
    // Halving the cells divides the error by about four.
    for w in errors.windows(2) {
        let order = (w[0] / w[1]).log2();
        assert!((1.8..2.3).contains(&order), "{errors:?}");
    }
}

/// A bump of heat, held as one value per cell of the coarse grid at the
/// start, spreading for 0.01: on the coarse grid (`n` boxes a side), on a
/// grid 2^levels times as fine, and on the coarse grid refined `levels`
/// times where the heat is and merged where it has gone.
fn bump(shape: &str, d: usize, n: usize, boxes: usize, finest: Option<(usize, f64)>) -> Results {
    let r = registry();
    let names = &["x", "y", "z"][..d];
    let mut eqs = mesh(shape, &vec![boxes; d]);
    let centre = [0.47, 0.52, 0.49];
    // The starting value of the coarse cell holding the centre.
    let dist: Vec<String> = names
        .iter()
        .zip(centre)
        .map(|(x, c)| format!("((floor({n}*centre({x}, c)) + 0.5)/{n} - {c})^2"))
        .collect();
    eqs.push(format!(
        "state u[c] = exp(-({})/(2*0.12^2))",
        dist.join(" + ")
    ));
    eqs.push("energy = sum(u^2*vol)".into());
    let r2: Vec<String> = names.iter().map(|x| format!("(c{x} - 0.5)^2")).collect();
    eqs.push(format!("moment = sum(u*vol*({}))", r2.join(" + ")));
    // Refined where the heat is above a part of its peak, merged below
    // two fifths of that.
    if let Some((m, part)) = finest {
        eqs.push("peak = maxof(u)".into());
        eqs.push(format!(
            "split c where (u > {part}*peak)*(vol > 1.5/{m}^{d})"
        ));
        eqs.push(format!("merge c where u < {}*peak", 0.4 * part));
    }
    run(&model(&eqs, "trbdf2", 0.001, 0.01), &r, None, None).unwrap()
}

/// The refined run's `measures` much closer to the fine run's than the
/// coarse run's are, with fewer cells than the fine grid.
fn closer(what: &str, coarse: &Results, fine: &Results, amr: &Results, measures: &[&str]) {
    let fc = last(fine, "m.cells");
    let most = column(amr, "m.cells").iter().copied().fold(0.0, f64::max);
    assert!(
        most < 0.6 * fc && most > last(coarse, "m.cells"),
        "{what}: {most} of {fc}"
    );
    for name in measures {
        let (f, a, c) = (last(fine, name), last(amr, name), last(coarse, name));
        assert!(
            (a - f).abs() < 0.3 * (c - f).abs(),
            "{what} {name}: {c} {a} {f}"
        );
    }
}

/// The fluxes keep heat, and so do the transfers (a copy to children that
/// share their parent's measure; the mean of equal children).
fn refined_matches_fine(shape: &str, d: usize, n: usize, levels: u32, part: f64) {
    let fine = n * 2usize.pow(levels);
    let runs = [
        bump(shape, d, n, n, None),
        bump(shape, d, n, fine, None),
        bump(shape, d, n, n, Some((fine, part))),
    ];
    for y in &runs {
        let total = column(y, "m.total");
        for v in &total {
            assert!(
                (v - total[0]).abs() < 1e-13 * total[0],
                "{shape}: {total:?}"
            );
        }
    }
    closer(
        shape,
        &runs[0],
        &runs[1],
        &runs[2],
        &["m.energy", "m.moment"],
    );
}

#[test]
fn refining_quads_where_the_heat_is_matches_a_fine_grid() {
    refined_matches_fine("quad", 2, 8, 2, 0.1);
}

#[test]
fn refining_hexahedra_where_the_heat_is_matches_a_fine_grid() {
    refined_matches_fine("hexa", 3, 4, 1, 0.05);
}

/// The heat equation on hexahedra: second order in three dimensions.
#[test]
fn heat_on_hexahedra_decays_as_the_exact_mode() {
    let r = registry();
    let mut errors = Vec::new();
    for n in [4, 8] {
        let mut eqs = mesh("hexa", &[n, n, n]);
        eqs.push(
            "state u[c] = cos(pi*centre(x, c))*cos(pi*centre(y, c))*cos(pi*centre(z, c))".into(),
        );
        eqs.push("err = maxof(abs(u - exp(-3*pi^2*t)*cos(pi*cx)*cos(pi*cy)*cos(pi*cz)))".into());
        let y = run(&model(&eqs, "trbdf2", 0.001, 0.02), &r, None, None).unwrap();
        assert!(column(&y, "m.total").iter().all(|v| v.abs() < 1e-12));
        errors.push(last(&y, "m.err"));
    }
    let order = (errors[0] / errors[1]).log2();
    assert!((1.7..2.4).contains(&order), "{errors:?}");
}

/// Gray–Scott: two species reacting and diffusing from a square seeded on
/// the coarse grid; refined where the second species is.
fn gray_scott(boxes: usize, finest: Option<usize>) -> Results {
    let r = registry();
    let mut eqs = mesh("quad", &[boxes, boxes]);
    eqs.retain(|e| !e.starts_with("du/dt") && !e.starts_with("flux") && !e.starts_with("total"));
    let seed = "(abs(centre(x, c) - 0.5) < 0.125)*(abs(centre(y, c) - 0.5) < 0.125)";
    eqs.extend([
        format!("state u[c] = 1 - 0.5*{seed}"),
        format!("state v[c] = 0.25*{seed}"),
        "param F = 0.04".into(),
        "param k = 0.06".into(),
        "fu = 2e-5*(inner(u, f) - across(u, f))*measure(f, x, y)/gap".into(),
        "fv = 1e-5*(inner(v, f) - across(v, f))*measure(f, x, y)/gap".into(),
        "du/dt = -outflow(fu, f)/vol - u*v^2 + F*(1 - u)".into(),
        "dv/dt = -outflow(fv, f)/vol + u*v^2 - (F + k)*v".into(),
        "mass = sum(v*vol)".into(),
        "energy = sum(v^2*vol)".into(),
    ]);
    if let Some(m) = finest {
        eqs.push(format!("split c where (v > 0.001)*(vol > 1.5/{m}^2)"));
        eqs.push("merge c where v < 0.0002".into());
    }
    run(&model(&eqs, "rk4", 1.0, 200.0), &r, None, None).unwrap()
}

#[test]
fn gray_scott_refined_where_it_reacts_matches_a_fine_grid() {
    let (coarse, fine, amr) = (
        gray_scott(16, None),
        gray_scott(32, None),
        gray_scott(16, Some(32)),
    );
    closer("gray-scott", &coarse, &fine, &amr, &["m.mass", "m.energy"]);
}

#[test]
fn mesh_rate_mistakes_are_explained() {
    let r = registry();
    let check = |extra: &[&str]| -> String {
        let mut eqs = mesh("quad", &[2, 2]);
        eqs.retain(|e| {
            !e.starts_with("du/dt") && !e.starts_with("flux") && !e.starts_with("total")
        });
        eqs.push("state u[c] = 1".into());
        eqs.extend(extra.iter().map(|s| s.to_string()));
        let f = json!({"format": 1, "blocks": [{"name": "m", "type": "eq.equations", "equations": eqs}]});
        simsandbox_session::describe::check(&f.to_string(), &r).to_string()
    };
    assert!(check(&["du/dt = -outflow(across(u, f) - inner(u, f), f)"]).contains("\"ok\":true"));
    assert!(check(&["du/dt = outflow(u, f)"]).contains("over the sides f"));
    assert!(check(&["du/dt = outflow(u, c)"]).contains("'outflow' takes"));
    assert!(check(&["du/dt = 0", "g = across(u, c)"]).contains("'across' takes"));
}

/// Continuous states on a mesh that refines: back to a time and on again
/// gives the same bits.
#[test]
fn rewinding_a_refining_heat_mesh_gives_the_same_values() {
    let r = registry();
    let mut eqs = mesh("quad", &[4, 4]);
    eqs.push("state u[c] = exp(-((centre(x, c) - 0.4)^2 + (centre(y, c) - 0.6)^2)/0.02)".into());
    eqs.push("split c where (u > 0.3)*(vol > 1/300)".into());
    eqs.push("merge c where u < 0.1".into());
    eqs.push("energy = sum(u^2*vol)".into());
    let mut m: serde_json::Value =
        serde_json::from_str(&model(&eqs, "trbdf2", 0.002, 0.02)).unwrap();
    m["controls"] = json!({"recorder": {"every": 1}});
    let mut s = simsandbox_session::session::Session::load("m", &m.to_string(), &r).unwrap();
    let read = |s: &simsandbox_session::session::Session| {
        let h = s.signals();
        let at = |n: &str| *h.values(h.resolve(n).unwrap()).last().unwrap();
        (at("m.energy"), at("m.cells"))
    };
    let mut forward = Vec::new();
    for _ in 0..8 {
        s.step().unwrap();
        forward.push(read(&s));
    }
    assert!(forward.iter().any(|f| f.1 != 16.0), "{forward:?}");
    s.rewind_to(0.006);
    let mut again = Vec::new();
    while s.time() < 0.0159 {
        s.step().unwrap();
        again.push(read(&s));
    }
    assert_eq!(again, forward[3..]);
}
