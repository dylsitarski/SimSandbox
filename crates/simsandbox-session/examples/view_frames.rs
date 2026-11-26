//! Draw a model's views frame after frame without a window, as the app
//! does each frame (egui's frame and its tessellation, no GPU), and print
//! the time per frame and the texture data each frame uploads: for timing
//! the views (`docs/design/performance.md`).
//!
//! ```sh
//! cargo run --release -p simsandbox-session --example view_frames -- FILE \
//!     [--t-end T] [--frames N] [--view K] [--steps S] [--size WxH]
//! ```
//!
//! The model runs to `T` first (its recording is what the views draw);
//! `--steps S` steps it `S` times between frames, as while it runs, else
//! it is still, as while paused.

use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!(
            "usage: view_frames FILE [--t-end T] [--frames N] [--view K] [--steps S] [--size WxH]"
        );
        std::process::exit(2);
    };
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
    };
    let t_end: f64 = flag("--t-end").map_or(10.0, |s| s.parse().expect("a number"));
    let frames: usize = flag("--frames").map_or(100, |s| s.parse().expect("a number"));
    let steps: usize = flag("--steps").map_or(0, |s| s.parse().expect("a number"));
    let view: Option<usize> = flag("--view").map(|s| s.parse().expect("a number"));
    let (w, h) = flag("--size").map_or((1200.0, 800.0), |s| {
        let (w, h) = s.split_once('x').expect("WxH");
        (w.parse().expect("a width"), h.parse().expect("a height"))
    });

    let text = std::fs::read_to_string(path).expect("a readable model file");
    let registry = simsandbox_session::examples::registry();
    let folder = std::path::Path::new(path).parent();
    let mut s = simsandbox_session::Session::load_in(path, &text, &registry, folder)
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    s.run_until(t_end).expect("a run");
    s.flush(true);

    let mut views =
        simsandbox_session::views::Views::new(simsandbox_session::examples::view_registry());
    views.templates.clone_from(&s.file.templates);
    {
        let h = s.signals();
        let available = s.available(&h);
        views.open(&available, &s.file.controls.views);
    }
    if let Some(k) = view {
        let keep = views.tiles.remove(k);
        views.tiles = vec![keep];
    }
    println!("{} samples, {} views", s.signals().len(), views.tiles.len());

    let ctx = egui::Context::default();
    let size = egui::vec2(w, h);
    let mut uploaded = 0usize;
    let mut vertices = 0usize;
    let mut drawing = 0.0;
    for frame in 0..frames + 2 {
        for _ in 0..steps {
            s.step().expect("a step");
        }
        s.flush(true);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            ..Default::default()
        };
        let start = Instant::now();
        let history = s.signals();
        let recorded = simsandbox_views::Recorded::single(&history);
        let out = ctx.run_ui(input, |ui| {
            egui::Frame::central_panel(ui.style()).show(ui, |ui| {
                let area = ui.available_rect_before_wrap();
                let n = views.tiles.len().max(1);
                for (k, tile) in views.tiles.iter_mut().enumerate() {
                    let Some(t) = views.registry.get(&tile.state.type_name) else {
                        continue;
                    };
                    let hgt = area.height() / n as f32;
                    let cell = egui::Rect::from_min_size(
                        area.min + egui::vec2(0.0, k as f32 * hgt),
                        egui::vec2(area.width(), hgt),
                    );
                    let builder = egui::UiBuilder::new()
                        .id_salt(("tile", tile.id))
                        .max_rect(cell);
                    ui.scope_builder(builder, |ui| tile.state.show(ui, t, recorded));
                }
            });
        });
        let primitives = ctx.tessellate(out.shapes, out.pixels_per_point);
        let took = start.elapsed().as_secs_f64();
        drop(history);
        // The first frames settle the plots' bounds and fonts.
        if frame >= 2 {
            drawing += took;
            uploaded += out
                .textures_delta
                .set
                .values()
                .flatten()
                .map(|d| d.image.width() * d.image.height() * 4)
                .sum::<usize>();
            vertices += primitives
                .iter()
                .map(|p| match &p.primitive {
                    egui::epaint::Primitive::Mesh(m) => m.vertices.len(),
                    egui::epaint::Primitive::Callback(_) => 0,
                })
                .sum::<usize>();
        }
    }
    let n = frames.max(1) as f64;
    println!(
        "{frames} frames: {:.3} ms a frame, {:.0} vertices and {:.0} bytes of textures uploaded a frame",
        drawing / n * 1e3,
        vertices as f64 / n,
        uploaded as f64 / n
    );
}
