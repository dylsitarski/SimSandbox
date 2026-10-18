//! Every built-in view draws without panicking, including on signals that
//! reach infinity or NaN (e.g. a resonance curve swept through zero damping).

use simsandbox_views::{
    Available, Bindings, Coordinate, FieldShape, History, ViewRegistry, ViewState,
};

/// Draw `state` for a few frames in a headless egui context and tessellate
/// the result, as the app would.
fn render(state: &mut ViewState, registry: &ViewRegistry, history: &History) {
    let ctx = egui::Context::default();
    let t = registry.get(&state.type_name).unwrap();
    for _ in 0..3 {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        let out = ctx.run_ui(input, |ui| {
            state.show(ui, t, history);
        });
        ctx.tessellate(out.shapes, out.pixels_per_point);
        let mut delta = out.textures_delta;
        delta.clear();
    }
}

fn bind(pairs: &[(&str, &[&str])]) -> Bindings {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect()))
        .collect()
}

#[test]
fn views_draw_signals_with_infinities_and_nans() {
    let registry = ViewRegistry::builtin();
    let names = vec!["a".to_string(), "b".to_string(), "c".to_string()];
    let mut h = History::with_fields(
        "spring.c",
        names.clone(),
        vec![
            ("g".into(), FieldShape::Grid { nx: 3, ny: 1 }),
            ("p".into(), FieldShape::Points { count: 2, dim: 2 }),
            ("gx".into(), FieldShape::Grid { nx: 3, ny: 1 }),
            ("s".into(), FieldShape::Grid { nx: 2, ny: 2 }),
        ],
    );
    let coordinates = vec![Coordinate {
        name: "gx".into(),
        of: "g".into(),
        axis: 0,
    }];
    h.set_coordinates(coordinates.clone());
    let bad = [f64::INFINITY, f64::NAN, f64::NEG_INFINITY];
    for k in 0..50 {
        let v = k as f64;
        let odd = if k % 7 == 0 { bad[k % 3] } else { v };
        h.push_sample(
            v * 0.1,
            &[1.0 / (v - 10.0).abs().min(1.0) * odd, odd.sin(), v],
            &[
                &[odd, 1.0, 2.0],
                &[odd, 0.0, 1.0, odd],
                &[0.0, if k % 5 == 0 { odd } else { 1.0 }, 2.0],
                &[odd, 1.0, odd, 2.0],
            ],
        );
    }
    let grids = vec!["g".to_string(), "gx".to_string(), "s".to_string()];
    let lines = vec!["g".to_string(), "gx".to_string()];
    let points = vec!["p".to_string()];
    let available = Available {
        scalars: &names,
        grids: &grids,
        points: &points,
        lines: &lines,
        coordinates: &coordinates,
        ..Default::default()
    };
    let cases = [
        ("plot.xy", bind(&[("x", &["t"]), ("y", &["a", "b"])])),
        ("plot.xy", bind(&[("x", &["a"]), ("y", &["b"])])),
        (
            "plot.xy",
            bind(&[("x", &["a"]), ("y", &["b"]), ("colour", &["t"])]),
        ),
        (
            "plot.xy",
            bind(&[("x", &["a"]), ("y", &["b"]), ("colour", &["c"])]),
        ),
        (
            "plot.3d",
            bind(&[("x", &["a"]), ("y", &["b"]), ("z", &["c"])]),
        ),
        ("plot.heatmap", bind(&[("field", &["g"])])),
        (
            "plot.heatmap",
            bind(&[("field", &["g"]), ("x", &["t"]), ("y", &["gx"])]),
        ),
        (
            "plot.heatmap",
            bind(&[("field", &["g"]), ("x", &["gx"]), ("y", &["t"])]),
        ),
        ("plot.heatmap", bind(&[("field", &["g"]), ("x", &["gx"])])),
        ("plot.heatmap", bind(&[("field", &["s"])])),
        ("plot.profile", bind(&[("y", &["g"])])),
        ("plot.profile", bind(&[("x", &["gx"]), ("y", &["g"])])),
        ("plot.profile", bind(&[("y", &["s"])])),
        ("plot.scatter", bind(&[("points", &["p"])])),
    ];
    for (name, b) in cases {
        let t = registry.get(name).unwrap();
        let mut state = ViewState::restore(t, b, Default::default(), None, &available).unwrap();
        render(&mut state, &registry, &h);
        // And transposed, where the view can swap that way.
        if t.swap == Some(simsandbox_views::Swap::Transpose) {
            state.swap(t);
            render(&mut state, &registry, &h);
        }
    }
}
