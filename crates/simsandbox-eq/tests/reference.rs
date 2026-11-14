//! The typed-equations reference (`docs/reference/typed-equations.md`) is
//! held to the code: every block of lines it marks `ssbx` parses, and
//! every function the parser knows is in it. A function added without
//! its entry, or an example that no longer parses, fails here.

use simsandbox_eq::symbolic::EquationSet;
use simsandbox_eq::OdeSystem;

const REFERENCE: &str = include_str!("../../../docs/reference/typed-equations.md");

/// The fenced blocks marked `ssbx`, each as its lines.
fn examples() -> Vec<Vec<String>> {
    let mut all = Vec::new();
    let mut open: Option<Vec<String>> = None;
    for line in REFERENCE.lines() {
        match (&mut open, line.trim()) {
            (None, "```ssbx") => open = Some(Vec::new()),
            (Some(_), "```") => all.extend(open.take()),
            (Some(lines), _) => lines.push(line.to_string()),
            _ => {}
        }
    }
    assert!(open.is_none(), "a block is not closed");
    all
}

#[test]
fn every_example_in_the_reference_parses() {
    let examples = examples();
    assert!(
        examples.len() >= 8,
        "only {} examples found",
        examples.len()
    );
    for (k, lines) in examples.iter().enumerate() {
        if let Err(e) = EquationSet::parse("example", lines) {
            panic!(
                "example {} does not parse: {e}\n{}",
                k + 1,
                lines.join("\n")
            );
        }
    }
}

#[test]
fn the_first_example_gives_what_the_reference_says() {
    let set = EquationSet::parse("osc", &examples()[0]).unwrap();
    let spec = set.spec();
    assert_eq!(spec.states, ["x", "v"]);
    let params: Vec<&str> = spec.params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(params, ["k", "c", "x0", "v0"]);
    let ports: Vec<&str> = spec.ports.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(ports, ["x", "v", "E"]);
}

#[test]
fn every_function_is_in_the_reference() {
    let listed = |name: &str| {
        REFERENCE.contains(&format!("`{name}`")) || REFERENCE.contains(&format!("`{name}("))
    };
    let mut missing: Vec<&str> = simsandbox_expr::Func::ALL
        .iter()
        .map(|(name, _)| *name)
        .chain(simsandbox_expr::ARRAY_OPS.iter().copied())
        .filter(|name| !listed(name))
        .collect();
    missing.sort_unstable();
    assert!(
        missing.is_empty(),
        "not in docs/reference/typed-equations.md: {missing:?}"
    );
    assert!(REFERENCE.contains(&format!("at most {} axes", simsandbox_expr::MAX_AXES)));
    assert!(REFERENCE.contains(&format!("at most {} levels", simsandbox_expr::MAX_DEPTH)));
}
