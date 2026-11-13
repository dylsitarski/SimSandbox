//! The memory budget (ADR 0062 step 5): one setting shared by snapshots,
//! the history, probes and filters, a quarter each by default; what they
//! hold is reported against it.

use serde_json::json;
use simsandbox_session::examples::registry;
use simsandbox_session::run::run_probes;
use simsandbox_session::session::{Session, DEFAULT_MEMORY_MB};

/// A 50-cell rod recorded by a probe that keeps what it records by
/// default.
fn rod() -> String {
    json!({"format": 1, "run": {"dt": 0.001, "t_end": 1},
        "blocks": [{"name": "m", "type": "eq.equations", "method": {"name": "rk4"},
            "equations": ["grid x = 0 .. 1, 50", "state T[x] = x", "dT/dt = 0.01*lap(T)"]}],
        "controls": {"probes": [{"name": "rod", "record": ["m.T"]}]}})
    .to_string()
}

#[test]
fn probes_thin_past_their_share_of_the_budget() {
    let r = registry();
    // At the default budget a quarter (256 MB) holds every sample.
    let all = run_probes(&rod(), &r, None, None, Vec::new(), &[], None, None).unwrap();
    assert_eq!(all.probes[0].1.read().len(), 1001);
    assert_eq!(all.memory.budget, (DEFAULT_MEMORY_MB * 1e6) as usize);
    assert!(!all.memory.over());
    // 0.4 MB: a quarter is 12 500 values, 250 samples of 50 cells.
    let small = run_probes(&rod(), &r, None, None, Vec::new(), &[], None, Some(0.4)).unwrap();
    let n = small.probes[0].1.read().len();
    assert!((125..=250).contains(&n), "{n}");
    // Its samples are the full run's, every other one dropped as it went.
    let (a, b) = (all.probes[0].1.read(), small.probes[0].1.read());
    let last = b.times()[n - 1];
    assert_eq!(last, *a.times().last().unwrap());
    assert!(b.times().iter().all(|t| a.times().contains(t)));
    // What it holds is reported, the probe by name.
    let parts = &small.memory.parts;
    assert!(parts
        .iter()
        .any(|(p, bytes)| p == "probe rod" && *bytes >= 8 * 50 * n));
    assert!(run_probes(&rod(), &r, None, None, Vec::new(), &[], None, Some(-1.0)).is_err());
}

#[test]
fn snapshots_take_their_share_unless_the_recorder_sets_its_own() {
    let r = registry();
    let mut v: serde_json::Value = serde_json::from_str(&rod()).unwrap();
    v["controls"]["recorder"] = json!({"every": 1});
    let mut s = Session::load("rod", &v.to_string(), &r).unwrap();
    // 0.2 MB: snapshots get 50 kB, about 6250 values; each holds the 50
    // states and outputs, so they thin to a hundred or so.
    s.set_memory_budget(0.2);
    for _ in 0..1000 {
        s.step().unwrap();
    }
    let mem = s.memory_use();
    let snaps = mem.parts.iter().find(|p| p.0 == "snapshots").unwrap().1;
    assert!(snaps <= 50_000 + 8 * 200, "{snaps}");
    assert!(!mem.over(), "{mem:?}");
    // 0.05 MB is less than the hundred samples the history always keeps
    // of a field: over the budget, and the largest part is named.
    s.set_memory_budget(0.05);
    s.step().unwrap();
    let mem = s.memory_use();
    assert!(mem.over(), "{mem:?}");
    assert_eq!(mem.largest().unwrap().0, "history");
    // A recorder's own size wins over the share.
    v["controls"]["recorder"] = json!({"every": 1, "mb": 10});
    let mut s = Session::load("rod", &v.to_string(), &r).unwrap();
    s.set_memory_budget(0.2);
    for _ in 0..1000 {
        s.step().unwrap();
    }
    let snaps = s.memory_use().parts[0].1;
    assert!(snaps > 50_000 * 4, "{snaps}");
}
