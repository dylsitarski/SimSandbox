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

/// A model parameter `size` (in `size_unit`) setting a typed block's
/// length `L [m]` by `formula`, inside a subsystem `pond` when `nested`.
fn sized(size_unit: &str, formula: &str) -> String {
    format!(
        r#"{{"format": 1, "run": {{"dt": 0.1}},
          "params": {{"size": {{"value": 2.0, "unit": "{size_unit}"}}}},
          "blocks": [
            {{"name": "tank", "type": "eq.equations",
             "equations": ["param L [m] = 1", "area = L^2 [m^2]"],
             "params": {{"L": "{formula}"}}}}
          ]}}"#
    )
}

fn param(text: &str, block: &str, name: &str) -> f64 {
    let loaded = load(text, &registry()).unwrap();
    loaded.model.param(loaded.ids[block], name).unwrap()
}

#[test]
fn parameter_formulas_are_checked_and_computed_in_si() {
    // 2 km is 2000 m; the same in metres stays 2.
    assert_eq!(param(&sized("km", "size"), "tank", "L"), 2000.0);
    assert_eq!(param(&sized("m", "size"), "tank", "L"), 2.0);
    assert_eq!(param(&sized("km", "size/4"), "tank", "L"), 500.0);
    // Changing it later keeps the conversion.
    let mut loaded = load(&sized("km", "size"), &registry()).unwrap();
    let set = loaded.params.set("size", 3.0).unwrap();
    assert_eq!(set, vec![("tank".to_string(), "L".to_string(), 3000.0)]);
    // A formula giving another kind of quantity is refused where it is
    // written.
    let (path, message) = error(&sized("kg", "size"));
    assert_eq!(path, "blocks[0].params.L");
    assert_eq!(message, "tank.L is in m but 'size' is in kg");
    let (_, message) = error(&sized("m", "size^2"));
    assert!(message.contains("is in m^2"), "{message}");
    let (_, message) = error(&sized("m", "size + 1/size"));
    assert!(message.contains("cannot add m and m^-1"), "{message}");
    // A unit that is not one.
    let (path, _) = error(&sized("paces", "size"));
    assert_eq!(path, "params.size.unit");
    // Without units on its names a formula is not checked (as before
    // ADR 0045): 2 is taken as metres.
    assert_eq!(param(&sized("", "size"), "tank", "L"), 2.0);
}

/// The model's `size` passed to a subsystem's parameter `side` (in
/// `side_unit`), which sets the length of the block inside.
fn nested(side_unit: &str, inner: &str) -> String {
    format!(
        r#"{{"format": 1, "run": {{"dt": 0.1}},
          "params": {{"size": {{"value": 2.0, "unit": "km"}}}},
          "subsystems": {{"pond": {{
            "params": {{"side": {{"value": 1.0, "unit": "{side_unit}"}}}},
            "blocks": [{{"name": "tank", "type": "eq.equations",
              "equations": ["param L [m] = 1", "area = L^2 [m^2]"],
              "params": {{"L": "{inner}"}}}}]
          }}}},
          "blocks": [{{"name": "p", "type": "pond", "params": {{"side": "size"}}}}]}}"#
    )
}

#[test]
fn subsystem_parameters_are_checked_on_the_way_in_and_out() {
    // km → the subsystem's cm → the block's m.
    let v = param(&nested("cm", "side"), "p/tank", "L");
    assert!((v - 2000.0).abs() < 1e-9, "{v}");
    // Into a subsystem parameter of another kind.
    let (path, message) = error(&nested("s", "side"));
    assert_eq!(path, "blocks[0].params.side");
    assert_eq!(message, "p.side is in s but 'size' is in m");
    // From a subsystem parameter into a block of another kind, also when
    // the subsystem is checked on its own.
    let (path, message) = error(&nested("m", "side*side"));
    assert_eq!(path, "subsystems.pond.blocks[0].params.L");
    assert!(message.contains("tank.L is in m"), "{message}");
    let unused = nested("m", "side*side").replace(
        r#""blocks": [{"name": "p", "type": "pond", "params": {"side": "size"}}]"#,
        r#""blocks": []"#,
    );
    let (path, _) = error(&unused);
    assert_eq!(path, "subsystems.pond.blocks[0].params.L");
    let (path, _) = error(&nested("acres", "side"));
    assert_eq!(path, "subsystems.pond.params.side.unit");
}
