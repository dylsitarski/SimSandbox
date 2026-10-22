//! A model's views drawn to an image without a window (ADR 0039): the
//! tiles the app would show, laid out the same way, painted by egui and
//! rasterised on the CPU ([`crate::raster`]), then encoded as PNG.

use crate::raster::{Canvas, Image, Textures};
use crate::views::{grid, Views};
use crate::Session;
use simsandbox_views::ViewRegistry;

/// How to render.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderOptions {
    /// Image size in points.
    pub width: f32,
    pub height: f32,
    /// Pixels per point (2 for a sharper, larger image).
    pub scale: f32,
    /// Only this tile (0-based, in `controls.views` order); all if `None`.
    pub view: Option<usize>,
    pub light: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            width: 1200.0,
            height: 800.0,
            scale: 1.0,
            view: None,
            light: false,
        }
    }
}

/// Draw the session's views as they are now. Returns the image and
/// warnings about saved views that cannot be shown.
pub fn render(
    s: &Session,
    registry: ViewRegistry,
    o: &RenderOptions,
) -> Result<(Image, Vec<String>), String> {
    if !(o.width >= 1.0 && o.height >= 1.0 && o.scale > 0.0 && o.scale <= 8.0) {
        return Err("the size must be at least 1 × 1 and the scale in (0, 8]".into());
    }
    let h = s.signals();
    let available = s.available(&h);
    let mut views = Views::new(registry);
    let warnings = views.open(&available, &s.file.controls.views);
    if let Some(k) = o.view {
        let n = views.tiles.len();
        if k >= n {
            return Err(format!(
                "there is no view {k}: the model has {n} (0 to {})",
                n.max(1) - 1
            ));
        }
        let keep = views.tiles.remove(k);
        views.tiles = vec![keep];
    }

    let ctx = egui::Context::default();
    let theme = if o.light {
        egui::Theme::Light
    } else {
        egui::Theme::Dark
    };
    ctx.set_theme(theme);
    let mut textures = Textures::default();
    let size = egui::vec2(o.width, o.height);
    let mut last = None;
    // Plots settle their bounds and layouts over a few frames.
    for _ in 0..4 {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            viewport_id: egui::ViewportId::ROOT,
            viewports: std::iter::once((
                egui::ViewportId::ROOT,
                egui::ViewportInfo {
                    native_pixels_per_point: Some(o.scale),
                    ..Default::default()
                },
            ))
            .collect(),
            ..Default::default()
        };
        let out = ctx.run_ui(input, |ui| {
            egui::Frame::central_panel(ui.style()).show(ui, |ui| {
                tiles(ui, &mut views, &h);
            });
        });
        textures.update(out.textures_delta);
        last = Some((out.shapes, out.pixels_per_point));
    }
    let (shapes, ppp) = last.expect("ran at least one frame");
    let primitives = ctx.tessellate(shapes, ppp);
    let (w, hgt) = (
        (o.width * ppp).round() as usize,
        (o.height * ppp).round() as usize,
    );
    let mut canvas = Canvas::new(w, hgt, ctx.style_of(theme).visuals.panel_fill);
    canvas.paint(&primitives, &textures, ppp);
    Ok((canvas.image(), warnings))
}

/// The tiles in a grid, each with its type's name as a title, as in the
/// app (without its controls).
fn tiles(ui: &mut egui::Ui, views: &mut Views, history: &simsandbox_views::History) {
    let area = ui.available_rect_before_wrap();
    let (cols, rows) = grid(views.tiles.len());
    for (k, tile) in views.tiles.iter_mut().enumerate() {
        let Some(t) = views.registry.get(&tile.state.type_name) else {
            continue;
        };
        let (c, r) = (k % cols, k / cols);
        let w = area.width() / cols as f32;
        let h = area.height() / rows as f32;
        let cell = egui::Rect::from_min_size(
            area.min + egui::vec2(c as f32 * w, r as f32 * h),
            egui::vec2(w, h),
        )
        .shrink(3.0);
        let builder = egui::UiBuilder::new()
            .id_salt(("tile", tile.id))
            .max_rect(cell);
        ui.scope_builder(builder, |ui| {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                ui.label(egui::RichText::new(t.label).strong());
                tile.state.show(ui, t, history);
            });
        });
    }
    ui.allocate_rect(area, egui::Sense::hover());
}

/// The image as PNG.
pub fn png(image: &Image) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    {
        let mut e = png::Encoder::new(&mut bytes, image.width as u32, image.height as u32);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        let mut w = e.write_header().map_err(|e| e.to_string())?;
        w.write_image_data(&image.rgba).map_err(|e| e.to_string())?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::examples::{registry, view_registry, EXAMPLES};

    #[test]
    fn every_example_renders_its_views() {
        for e in EXAMPLES {
            let mut s = Session::load(e.name, e.text, &registry()).unwrap();
            s.run_until(s.dt * 50.0).unwrap();
            let o = RenderOptions {
                width: 400.0,
                height: 300.0,
                ..Default::default()
            };
            let (img, warnings) = render(&s, view_registry(), &o).unwrap();
            assert!(warnings.is_empty(), "{}: {warnings:?}", e.name);
            assert_eq!((img.width, img.height), (400, 300));
            // Something besides the background was drawn: titles, axes,
            // data.
            let first = &img.rgba[..4];
            let differing = img.rgba.chunks(4).filter(|p| p != &first).count();
            assert!(differing > 2000, "{}: {differing} pixels drawn", e.name);
            let bytes = png(&img).unwrap();
            assert_eq!(&bytes[1..4], b"PNG");
        }
    }

    #[test]
    fn one_view_at_a_scale_and_bad_choices_are_errors() {
        let e = &EXAMPLES[0];
        let mut s = Session::load(e.name, e.text, &registry()).unwrap();
        s.run_until(1.0).unwrap();
        let o = RenderOptions {
            width: 200.0,
            height: 100.0,
            scale: 2.0,
            view: Some(0),
            light: true,
        };
        let (img, _) = render(&s, view_registry(), &o).unwrap();
        assert_eq!((img.width, img.height), (400, 200));
        // A light background.
        assert!(img.rgba[0] > 200);
        let o = RenderOptions {
            view: Some(99),
            ..Default::default()
        };
        assert!(render(&s, view_registry(), &o)
            .unwrap_err()
            .contains("no view 99"));
    }
}
