//! Run a model file headless, as `simsandbox run` does, without writing
//! the results, and print the time it took: for timing and profiling
//! (`docs/design/performance.md`).
//!
//! ```sh
//! cargo run --release -p simsandbox-session --example profile_run -- FILE [T_END]
//! valgrind --tool=callgrind target/release/examples/profile_run FILE [T_END]
//! ```

use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        eprintln!("usage: profile_run FILE [T_END]");
        std::process::exit(2);
    };
    let text = std::fs::read_to_string(path).expect("a readable model file");
    let t_end: Option<f64> = args.get(2).map(|a| a.parse().expect("a number"));
    let registry = simsandbox_session::examples::registry();
    let folder = std::path::Path::new(path).parent().map(|p| p.to_path_buf());
    let start = Instant::now();
    let run = simsandbox_core::registry::in_folder(folder.as_deref(), || {
        if simsandbox_file::compose::is_composition(&text) {
            simsandbox_session::compose::run_composition(&text, &registry, t_end).map(|_| ())
        } else {
            simsandbox_session::run::run(&text, &registry, t_end, None).map(|_| ())
        }
    });
    if let Err(e) = run {
        eprintln!("{path}: {e}");
        std::process::exit(1);
    }
    println!("{:.3} s", start.elapsed().as_secs_f64());
}
