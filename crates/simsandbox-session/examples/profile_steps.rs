//! Step a model file through a `Session`, as the app's simulation thread
//! does (history, probes, filters), and print the time per step: for
//! timing the app's engine without the window (`docs/design/performance.md`).
//!
//! ```sh
//! cargo run --release -p simsandbox-session --example profile_steps -- FILE [STEPS]
//! ```

use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("usage: profile_steps FILE [STEPS]");
        std::process::exit(2);
    };
    let steps: usize = args.get(1).map_or(200, |s| s.parse().expect("a number"));
    let text = std::fs::read_to_string(path).expect("a readable model file");
    let registry = simsandbox_session::examples::registry();
    let folder = std::path::Path::new(path).parent();
    let mut s = simsandbox_session::Session::load_in(path, &text, &registry, folder)
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    s.flush(true);
    let start = Instant::now();
    for _ in 0..steps {
        s.step().expect("a step");
        s.flush(false);
    }
    let took = start.elapsed().as_secs_f64();
    println!(
        "{steps} steps: {:.3} s, {:.3} ms a step, {:.0} steps a second",
        took,
        took / steps as f64 * 1e3,
        steps as f64 / took
    );
}
