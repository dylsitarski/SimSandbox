//! The Stage's headless half (ADR 0108 point 4): a signal dropped on a
//! view is bound where it fits.

use simsandbox_session::examples::{registry, view_registry, EXAMPLES};
use simsandbox_session::session::Session;
use simsandbox_session::views::Views;

#[test]
fn a_signal_dropped_on_a_view_is_bound_where_it_fits() {
    let r = registry();
    let e = EXAMPLES
        .iter()
        .find(|e| e.name.starts_with("Cart-pole"))
        .unwrap();
    let s = Session::load("cartpole", e.text, &r).unwrap();
    let h = s.signals();
    let available = s.available(&h);
    let mut views = Views::new(view_registry());
    views.templates = s.templates();
    views.open(&available, &s.file.controls.views);
    // A scene drawing a template binds in its form.
    let scene = views.tiles[0].id;
    let refused = views.bind_dropped(scene, "cartpole.hinge_angle", &available);
    assert!(refused.unwrap_err().contains("binds in its form"));
    // A plot against time takes it as one more curve, once.
    let bind = [
        ("x".to_string(), vec!["t".to_string()]),
        (
            "y".to_string(),
            vec!["cartpole.slider_position".to_string()],
        ),
    ]
    .into();
    assert!(views.add_bound("plot.xy", bind, &available));
    let plot = views.selected.unwrap();
    for _ in 0..2 {
        assert_eq!(
            views.bind_dropped(plot, "cartpole.hinge_angle", &available),
            Ok("y")
        );
    }
    let tile = views.tiles.iter().find(|t| t.id == plot).unwrap();
    assert_eq!(
        tile.state.bind["y"],
        vec!["cartpole.slider_position", "cartpole.hinge_angle"]
    );
    // Something it cannot show is refused, the view unchanged.
    assert!(views
        .bind_dropped(plot, "nothing.x", &available)
        .unwrap_err()
        .contains("cannot show"));
}
