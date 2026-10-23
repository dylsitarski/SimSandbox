//! Units between blocks (ADR 0045): typed equations declare them,
//! connections need the same unit, and `units.convert` is the one way to
//! change it.

use simsandbox_core::registry::Registry;
use simsandbox_core::sim::Simulator;
use simsandbox_file::{load, FileError};

fn registry() -> Registry {
    let mut r = Registry::new();
    simsandbox_eq::register(&mut r).unwrap();
    simsandbox_agents::register(&mut r).unwrap();
    r
}

/// A typed driver `y = A*sin(w*t)` (its output in `unit`) pushing a
/// coupled oscillator's neighbour input (in m), optionally through a
/// conversion.
fn driven(unit: &str, convert: Option<(&str, &str)>) -> String {
    let (extra, wire) = match convert {
        Some((from, to)) => (
            format!(
                r#"{{"name": "c", "type": "units.convert", "settings": {{"from": "{from}", "to": "{to}"}}}},"#
            ),
            r#"{"from": "drive.y", "to": "c.u"}, {"from": "c.y", "to": "osc.neighbor_x"}"#
                .to_string(),
        ),
        None => (
            String::new(),
            r#"{"from": "drive.y", "to": "osc.neighbor_x"}"#.to_string(),
        ),
    };
    format!(
        r#"{{"format": 1, "run": {{"dt": 0.01}}, "blocks": [
          {{"name": "drive", "type": "eq.equations", "method": {{"name": "evaluate"}},
           "equations": ["param A [{unit}] = 0.1", "param w [1/s] = 2", "y = A*sin(w*t) [{unit}]"]}},
          {extra}
          {{"name": "osc", "type": "eq.coupled_oscillator", "method": {{"name": "rk4"}}}}
        ], "connections": [{wire}]}}"#
    )
}

fn error(text: &str) -> (String, String) {
    match load(text, &registry()) {
        Err(FileError::Invalid { path, message }) => (path, message),
        Err(e) => panic!("{e}"),
        Ok(_) => panic!("expected an error"),
    }
}

fn final_x(text: &str) -> f64 {
    let loaded = load(text, &registry()).unwrap();
    let id = loaded.ids["osc"];
    let mut sim = Simulator::new(loaded.model, 0.0).unwrap();
    for _ in 0..200 {
        sim.step(0.01).unwrap();
    }
    sim.output(id, "x").unwrap()[0]
}

#[test]
fn a_typed_output_in_metres_drives_a_block_in_metres() {
    let x = final_x(&driven("m", None));
    assert!(x.is_finite() && x != 0.0);
    // Dimensionless or in kilometres, the wire is refused, saying how to
    // convert.
    let (path, message) = error(&driven("1", None));
    assert_eq!(path, "connections[0]");
    assert!(message.contains("units.convert"), "{message}");
    let (_, message) = error(&driven("km", None));
    assert!(
        message.contains("[km]") && message.contains("exactly"),
        "{message}"
    );
    // Converted explicitly: 0.1 km is 100 m, so the drive is 1000 times
    // larger than 0.1 m (the oscillator is linear).
    let km = final_x(&driven("km", Some(("km", "m"))));
    assert!((km / x - 1000.0).abs() < 1e-9, "{km} vs {x}");
}

#[test]
fn conversions_keep_the_kind_of_quantity() {
    let (path, message) = error(&driven("km", Some(("km", "s"))));
    assert_eq!(path, "blocks[1].settings.to");
    assert!(message.contains("different things"), "{message}");
    let (path, _) = error(&driven("km", Some(("km", "furlong"))));
    assert_eq!(path, "blocks[1].settings.to");
    let bad = driven("m", None).replace(
        r#""method": {"name": "rk4"}}"#,
        r#""method": {"name": "rk4"}, "settings": {"colour": "red"}}"#,
    );
    assert_eq!(error(&bad).0, "blocks[1].settings.colour");
}
