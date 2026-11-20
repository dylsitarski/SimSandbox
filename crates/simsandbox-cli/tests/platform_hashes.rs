//! Mechanisms' results pinned per platform (ADR 0103 point 7): each case
//! is run through the `simsandbox` binary and its printed results hashed.
//! The CSV writes every number exactly (the shortest text that reads back
//! the same bits), so equal text is equal bits. A platform's values are in
//! `platform-hashes.json`, keyed `os-arch`; a change of results on any
//! platform fails here.
//!
//! Pinning: `SIMSANDBOX_PIN_HASHES=1 cargo test -p simsandbox-cli --test
//! platform_hashes` writes this platform's values; a platform with none
//! fails with the line to add (CI prints it), see the verification policy.

use serde_json::{json, Map, Value};
use std::path::PathBuf;
use std::process::Command;

fn here(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(name)
}

/// FNV-1a, 64 bits: small, the same everywhere, no dependency.
fn fnv(bytes: &[u8]) -> String {
    let h = bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    format!("{h:016x}")
}

fn run(args: &[&str]) -> Vec<u8> {
    let out = Command::new(env!("CARGO_BIN_EXE_simsandbox"))
        .args(args)
        .env("SIMSANDBOX_NATIVE", "off")
        .output()
        .expect("the binary runs");
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // Line ends as written on every platform.
    String::from_utf8(out.stdout)
        .expect("text")
        .replace("\r\n", "\n")
        .into_bytes()
}

fn scratch(name: &str, text: &str) -> String {
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::write(&path, text).unwrap();
    path.to_string_lossy().into_owned()
}

/// A chain of `links` rods from a hinge at the origin, from horizontal at
/// rest, with `method`.
fn chain(links: usize, method: &str) -> String {
    let bodies: Vec<Value> = (0..links)
        .map(|i| {
            json!({
                "name": format!("l{i}"),
                "parent": if i == 0 { "ground".to_string() } else { format!("l{}", i - 1) },
                "joint": format!("j{i}"),
                "kind": "revolute",
                "at": if i == 0 { [0.0, 0.0] } else { [0.1, 0.0] },
                "mass": 0.1,
                "centre": [0.05, 0.0],
                "inertia": 0.1 * 0.01 / 12.0,
                "damping": 0.0001
            })
        })
        .collect();
    json!({
        "format": 1,
        "run": {"dt": 0.001},
        "blocks": [{"name": "chain", "type": "mech.planar", "method": {"name": method}, "bodies": bodies}]
    })
    .to_string()
}

/// Each case's name and its results' hash on this platform.
fn hashes() -> Map<String, Value> {
    let model = here("../../examples/cartpole.ssbx.json");
    let model = model.to_string_lossy();
    let grab = scratch(
        "pinned-grab.run.json",
        &json!({"format": 1, "model": model, "model_hash": "made by hand", "track": [
            [0.5, "hand.x", 0.6], [0.5, "hand.y", 0.6], [0.5, "hand.down", 1.0],
            [0.5, "hand.picked", 0.0], [1.0, "hand.down", 0.0], [1.0, "hand.picked", -1.0]]})
        .to_string(),
    );
    let dopri = scratch("pinned-chain-dopri5.json", &chain(10, "dopri5"));
    let trbdf2 = scratch("pinned-chain-trbdf2.json", &chain(10, "trbdf2"));
    let cases: [(&str, Vec<&str>); 4] = [
        ("cartpole", vec!["run", &model, "--t-end", "6"]),
        (
            "cartpole grabbed",
            vec!["run", &model, "--t-end", "6", "--inputs", &grab],
        ),
        ("chain of 10, dopri5", vec!["run", &dopri, "--t-end", "2"]),
        ("chain of 10, trbdf2", vec!["run", &trbdf2, "--t-end", "1"]),
    ];
    cases
        .iter()
        .map(|(name, args)| (name.to_string(), Value::String(fnv(&run(args)))))
        .collect()
}

#[test]
fn mechanisms_give_the_pinned_bits_on_this_platform() {
    let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    let file = here("tests/platform-hashes.json");
    let mut pinned: Map<String, Value> =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    let now = hashes();
    if std::env::var_os("SIMSANDBOX_PIN_HASHES").is_some() {
        pinned.insert(platform, Value::Object(now));
        let text = serde_json::to_string_pretty(&pinned).unwrap() + "\n";
        std::fs::write(&file, text).unwrap();
        return;
    }
    let line = serde_json::to_string(&now).unwrap();
    let Some(Value::Object(want)) = pinned.get(&platform) else {
        panic!(
            "no results pinned for {platform}; add to crates/simsandbox-cli/tests/platform-hashes.json:\n\"{platform}\": {line}"
        );
    };
    let differ: Vec<&String> = now.keys().filter(|k| want.get(*k) != now.get(*k)).collect();
    assert!(
        differ.is_empty(),
        "results changed on {platform} in {differ:?}; if meant, pin again (verification policy):\n\"{platform}\": {line}"
    );
}
