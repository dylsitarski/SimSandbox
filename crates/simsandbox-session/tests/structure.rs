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

fn at(d: &Diagram, moves: &[simsandbox_session::diagram::Edit], name: &str) -> [f64; 2] {
    moves
        .iter()
        .find_map(|e| match e {
            simsandbox_session::diagram::Edit::Move { block, at } if block == name => Some(*at),
            _ => None,
        })
        .unwrap_or_else(|| {
            panic!(
                "{name} not placed; nodes {:?}",
                d.nodes.iter().map(|n| &n.name).collect::<Vec<_>>()
            )
        })
}

fn arranged(name: &str) -> (Diagram, Vec<simsandbox_session::diagram::Edit>) {
    let e = EXAMPLES.iter().find(|e| e.name.starts_with(name)).unwrap();
    let (file, _) = simsandbox_file::ModelFile::from_json(e.text).unwrap();
    let r = registry();
    let d = Diagram::new(&file, &r);
    let roles = simsandbox_session::diagram::roles(&file, &r);
    let moves = simsandbox_session::diagram::arrange(&d, &roles);
    assert_eq!(moves.len(), d.nodes.len(), "every node placed once");
    (d, moves)
}

#[test]
fn arranged_by_role_interaction_is_left_the_controller_below_the_plant() {
    // ADR 0108 point 3: a suggestion a person applies.
    let (d, m) = arranged("Cart-pole");
    let (hand, control, cartpole) = (
        at(&d, &m, "hand"),
        at(&d, &m, "control"),
        at(&d, &m, "cartpole"),
    );
    assert!(
        hand[0] < control[0] && control[0] < cartpole[0],
        "{hand:?} {control:?} {cartpole:?}"
    );
    assert!(
        control[1] > cartpole[1],
        "the controller is below the plant"
    );
    // The card's roles, from the file alone.
    let e = EXAMPLES
        .iter()
        .find(|e| e.name.starts_with("Cart-pole"))
        .unwrap();
    let (file, _) = simsandbox_file::ModelFile::from_json(e.text).unwrap();
    let roles = simsandbox_session::diagram::roles(&file, &registry());
    assert_eq!(roles["hand"], "interaction");
    assert_eq!(roles["control"], "controller");
    assert_eq!(roles["cartpole"], "system");
}

#[test]
fn a_plant_in_a_chain_is_arranged_left_to_right() {
    let (d, m) = arranged("Platoon");
    let xs: Vec<f64> = ["leader", "car1", "car2", "car3"]
        .iter()
        .map(|n| at(&d, &m, n)[0])
        .collect();
    assert!(xs.windows(2).all(|w| w[0] < w[1]), "{xs:?}");
}

#[test]
fn without_a_plant_nodes_are_arranged_by_signal_flow() {
    let (d, m) = arranged("Tuning");
    assert!(at(&d, &m, "tuning")[0] < at(&d, &m, "best")[0]);
}

#[test]
fn arranging_a_file_writes_places_and_a_second_time_moves_nothing() {
    let e = EXAMPLES
        .iter()
        .find(|e| e.name.starts_with("Cart-pole"))
        .unwrap();
    let (mut file, _) = simsandbox_file::ModelFile::from_json(e.text).unwrap();
    let r = registry();
    assert!(simsandbox_session::diagram::arrange_file(&mut file, &r).unwrap() > 0);
    assert!(file.blocks.iter().all(|b| b.canvas.is_some()));
    assert_eq!(
        simsandbox_session::diagram::arrange_file(&mut file, &r).unwrap(),
        0
    );
}

#[test]
fn a_folded_block_draws_its_wired_ports_and_counts_the_rest() {
    // The Structure lens, step 3 (ADR 0108): ports only when wired, the
    // rest on demand; each block says what it does.
    use simsandbox_core::block::Causality;
    let mut d = diagram("Cart-pole");
    let i = d.node("cartpole").unwrap();
    let whole = d.nodes[i].height();
    d.fold(|_| false);
    let n = &d.nodes[i];
    assert!(n.folded);
    let shown: Vec<&str> = (0..n.outputs.len())
        .filter(|&k| n.shows(Causality::Output, k))
        .map(|k| n.outputs[k].name.as_str())
        .collect();
    assert_eq!(
        shown,
        [
            "slider_position",
            "slider_speed",
            "hinge_angle",
            "hinge_rate"
        ]
    );
    assert_eq!(
        n.hidden(),
        n.inputs.len() + n.outputs.len() - n.rows_on(Causality::Input) - shown.len()
    );
    assert!(n.height() < whole);
    // Every wire still ends at a port that is drawn.
    for w in &d.wires {
        assert!(
            d.nodes[w.from.0].shows(Causality::Output, w.from.1),
            "{}",
            w.from_name
        );
        if !w.param {
            assert!(
                d.nodes[w.to.0].shows(Causality::Input, w.to.1),
                "{}",
                w.to_name
            );
        }
    }
    // A port folded away attaches at the header's side; rows close up.
    let energy = n.outputs.iter().position(|p| p.name == "energy").unwrap();
    assert_eq!(n.row_of(Causality::Output, energy), None);
    assert_eq!(n.row_of(Causality::Output, 3), Some(3));
    // Unfolded on demand.
    d.fold(|name| name == "cartpole");
    assert!(!d.nodes[i].folded && d.nodes[d.node("control").unwrap()].folded);
    // Roles on the nodes, as the card names them.
    let role = |name: &str| d.nodes[d.node(name).unwrap()].role;
    assert_eq!(
        (role("hand"), role("control"), role("cartpole")),
        ("interaction", "controller", "system")
    );
}

#[test]
fn what_fits_an_input_or_parameter_is_offered_by_unit_and_name() {
    // Connecting by intent (ADR 0108 step 4): same unit and shape.
    use simsandbox_session::wiring::{fits, pairs, sources};
    let d = diagram("Cart-pole");
    let from: Vec<String> = sources(&d, "cartpole.slider_force")
        .unwrap()
        .into_iter()
        .map(|s| s.signal)
        .collect();
    assert_eq!(from, ["control.F", "control.fx", "control.fy"]);
    assert!(fits(&d, "control.F", "cartpole.slider_force"));
    assert!(
        !fits(&d, "cartpole.slider_position", "cartpole.slider_force"),
        "not its own"
    );
    assert!(!fits(&d, "hand.x", "cartpole.slider_force"), "m is not N");
    assert!(sources(&d, "cartpole.nothing").is_err());
    // From the controller into the mechanism: not what is wired already;
    // what nothing sets first.
    let p = pairs(&d, "control", "cartpole");
    assert!(!p
        .iter()
        .any(|p| p.from == "control.F" && p.to == "cartpole.slider_force"));
    assert!(p
        .iter()
        .all(|p| p.from.starts_with("control.") && p.to.starts_with("cartpole.")));
    let first_replacing = p.iter().position(|p| p.replaces).unwrap_or(p.len());
    assert!(p[first_replacing..].iter().all(|p| p.replaces));
    assert!(
        p.iter().any(|p| p.drives && !p.replaces),
        "an idle force can be driven"
    );
    assert!(pairs(&d, "control", "control").is_empty());
}
