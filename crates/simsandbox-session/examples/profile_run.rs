//! Run a model file headless, as `simsandbox run` does, without writing
//! the results, and print the time it took: for timing and profiling
//! (`docs/design/performance.md`).
//!
//! ```sh
//! cargo run --release -p simsandbox-session --example profile_run -- FILE [T_END] [--signals A,B] [--every N]
//! valgrind --tool=callgrind target/release/examples/profile_run FILE [T_END]
//! ```

use simsandbox_session::run::Keep;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("usage: profile_run FILE [T_END] [--signals A,B] [--every N]");
        std::process::exit(2);
    };
    let mut t_end: Option<f64> = None;
    let mut keep = Keep::default();
    let mut rest = args[1..].iter();
    while let Some(a) = rest.next() {
        match a.as_str() {
            "--signals" => {
                let names = rest.next().expect("names");
                keep.signals = Some(names.split(',').map(String::from).collect());
            }
            "--every" => keep.every = rest.next().expect("N").parse().expect("a whole number"),
            t => t_end = Some(t.parse().expect("a number")),
        }
    }
    let text = std::fs::read_to_string(path).expect("a readable model file");
    let registry = simsandbox_session::examples::registry();
    let folder = std::path::Path::new(path).parent().map(|p| p.to_path_buf());
    let start = Instant::now();
    let run = simsandbox_core::registry::in_folder(folder.as_deref(), || {
        if simsandbox_file::compose::is_composition(&text) {
            simsandbox_session::compose::run_composition_keeping(
                &text,
                &registry,
                t_end,
                simsandbox_core::parallel::Parallel::Auto,
                &keep,
            )
            .map(|_| ())
        } else {
            simsandbox_session::run::run_keeping(&text, &registry, t_end, None, Vec::new(), &keep)
                .map(|_| ())
        }
    });
    if let Err(e) = run {
        eprintln!("{path}: {e}");
        std::process::exit(1);
    }
    println!("{:.3} s", start.elapsed().as_secs_f64());
}
