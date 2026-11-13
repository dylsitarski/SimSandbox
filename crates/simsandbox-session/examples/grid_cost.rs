//! What a large grid costs a session (ADR 0060, Q38, Q39): time per step
//! stepping alone and with recording and snapshots, and memory.
//! `cargo run --release -p simsandbox-session --example grid_cost [FILE] [STEPS]`

use simsandbox_core::sim::Simulator;
use simsandbox_session::examples::registry;
use simsandbox_session::session::Session;
use std::time::Instant;

/// Resident memory of this process, in MB (Linux only; 0 elsewhere).
fn rss_mb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok())
        })
        .map_or(0.0, |kb| kb / 1024.0)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .map_or("examples/gray-scott.ssbx.json", String::as_str);
    let steps: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let text = std::fs::read_to_string(path).expect("model file");
    let reg = registry();

    let loaded = simsandbox_file::load(&text, &reg).expect("loads");
    let dt = loaded.file.run.dt;
    let mut sim = Simulator::new(loaded.model, 0.0).expect("builds");
    let start = Instant::now();
    for _ in 0..steps {
        sim.step(dt).expect("steps");
    }
    let alone = start.elapsed().as_secs_f64() * 1e3 / steps as f64;
    drop(sim);

    let before = rss_mb();
    let mut s = Session::load(path, &text, &reg).expect("loads");
    let start = Instant::now();
    for k in 0..steps {
        s.step().expect("steps");
        if (k + 1) % (steps / 4).max(1) == 0 {
            println!("  after {:6} steps: {:8.1} MB", k + 1, rss_mb() - before);
        }
    }
    let session = start.elapsed().as_secs_f64() * 1e3 / steps as f64;
    println!("{path}, {steps} steps of {dt} s:");
    println!("  stepping alone        {alone:8.3} ms a step");
    println!("  in a session          {session:8.3} ms a step (recording, snapshots)");
    println!(
        "  memory per step       {:8.3} MB",
        (rss_mb() - before) / steps as f64
    );
}
