//! `simsandbox-app` — the SimSandbox desktop app (ADR 0008).
//!
//! ```text
//! simsandbox-app [FILE] [--view time|phase|3d|diagram]   (shows only that view)
//! simsandbox-app [FILE] --window signals|equations (opens that window)
//! simsandbox-app [FILE] --screenshot OUT.ppm [--after SECONDS]
//! ```
//!
//! Without a file it opens the built-in Lorenz example. `--screenshot`
//! saves the window after `--after` seconds (default 3) and exits; it is
//! for documentation and visual checks.

mod app;
mod diagram;
mod dock;
mod inspector;
mod live;
mod log;
mod pacing;
mod probe_editor;
mod recorder_editor;

use app::{ScreenshotRequest, SimApp};
use eframe::egui;
use std::process::ExitCode;

struct Args {
    path: Option<String>,
    view: Option<String>,
    window: Option<String>,
    screenshot: Option<ScreenshotRequest>,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut path = None;
    let mut view = None;
    let mut window = None;
    let mut shot: Option<std::path::PathBuf> = None;
    let mut after = 3.0;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--screenshot" => {
                shot = Some(it.next().ok_or("missing value for --screenshot")?.into())
            }
            "--view" => {
                let v = it.next().ok_or("missing value for --view")?;
                if !["time", "phase", "3d", "diagram"].contains(&v.as_str()) {
                    return Err(format!(
                        "unknown view '{v}' (expected time, phase, 3d or diagram)"
                    ));
                }
                view = Some(v.clone());
            }
            "--window" => {
                let v = it.next().ok_or("missing value for --window")?;
                if !["signals", "equations"].contains(&v.as_str()) {
                    return Err(format!(
                        "unknown window '{v}' (expected signals or equations)"
                    ));
                }
                window = Some(v.clone());
            }
            "--after" => {
                let v = it.next().ok_or("missing value for --after")?;
                after = v
                    .parse()
                    .map_err(|_| format!("invalid number '{v}' for --after"))?;
            }
            "--version" | "-V" => {
                println!("simsandbox-app {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            flag if flag.starts_with("--") => return Err(format!("unknown flag '{flag}'")),
            file => path = Some(file.to_string()),
        }
    }
    Ok(Args {
        path,
        view,
        window,
        screenshot: shot.map(|path| ScreenshotRequest { path, after }),
    })
}

/// The app, shared with the windows of popped-out tabs, which draw it
/// on their own (ADR 0054).
struct Shared(std::sync::Arc<std::sync::Mutex<SimApp>>);

impl eframe::App for Shared {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let mut app = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        eframe::App::ui(&mut *app, ui, frame);
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse(&args) {
        Ok(v) => v,
        Err(e) => {
            eprintln!(
                "error: {e}\nusage: simsandbox-app [FILE] [--view time|phase|3d|diagram] [--window signals|equations] [--screenshot OUT.ppm [--after SECONDS]]"
            );
            return ExitCode::FAILURE;
        }
    };
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title("SimSandbox")
            .with_inner_size([1100.0, 700.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    let result = eframe::run_native(
        "SimSandbox",
        options,
        Box::new(move |_cc| {
            let app = SimApp::new(
                args.path,
                args.view.as_deref(),
                args.window.as_deref(),
                args.screenshot,
            );
            Ok(Box::new(Shared(app.shared())))
        }),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
