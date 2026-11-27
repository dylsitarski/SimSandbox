//! The Structure lens (ADR 0108): the wires between two nodes as one
//! bundle, named by its ports' group (ADR 0109 point 3).

use simsandbox_session::diagram::{bundles, Diagram};
use simsandbox_session::examples::{registry, EXAMPLES};

fn diagram(name: &str) -> Diagram {
    let e = EXAMPLES.iter().find(|e| e.name.starts_with(name)).unwrap();
    let (file, _) = simsandbox_file::ModelFile::from_json(e.text).unwrap();
    Diagram::new(&file, &registry())
}

#[test]
fn wires_between_two_blocks_are_one_bundle_named_by_their_group() {
    let d = diagram("Cart-pole");
    let named = |b: &simsandbox_session::diagram::Bundle| {
        (
            d.nodes[b.from].name.clone(),
            d.nodes[b.to].name.clone(),
            b.label.clone(),
            b.wires.len(),
        )
    };
    let all: Vec<_> = bundles(&d).iter().map(named).collect();
    let find = |from: &str, to: &str| {
        all.iter()
            .find(|b| b.0 == from && b.1 == to)
            .unwrap_or_else(|| panic!("no bundle {from} to {to} in {all:?}"))
            .clone()
    };
    // The pointer's four outputs share a group.
    assert_eq!(find("hand", "control").2, "pointer");
    assert_eq!(find("hand", "control").3, 4);
    // The controller's forces drive the mechanism's parameters.
    assert_eq!(find("control", "cartpole").2, "3 drives");
    // The joints come back to the controller: a loop, one bundle.
    let back = find("cartpole", "control");
    assert_eq!(back.2, "joints");
    // Every wire is in exactly one bundle.
    let mut held: Vec<usize> = bundles(&d).iter().flat_map(|b| b.wires.clone()).collect();
    held.sort_unstable();
    assert_eq!(held, (0..d.wires.len()).collect::<Vec<_>>());
}

#[test]
fn a_bundle_without_a_shared_group_says_its_wire_or_its_count() {
    // The forced spring: one wire, named by its ports.
    let d = diagram("Forced spring");
    let b = bundles(&d);
    assert!(!b.is_empty());
    for b in &b {
        if b.wires.len() == 1 {
            let w = &d.wires[b.wires[0]];
            let port = |n: &str| n.rsplit(['.', '/']).next().unwrap().to_string();
            let (a, z) = (port(&w.from_name), port(&w.to_name));
            assert!(
                b.label == a || b.label == format!("{a} to {z}"),
                "{}",
                b.label
            );
        }
    }
}
