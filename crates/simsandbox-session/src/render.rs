//! A model's views, or its diagram, drawn to an image without a window
//! (ADR 0039): the tiles or the node canvas the app would show, painted
//! by egui and rasterised on the CPU ([`crate::raster`]), then encoded as
//! PNG.

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
    /// For the diagram: a subsystem's inside instead of the model
    /// (ADR 0043).
    pub subsystem: Option<String>,
    /// For the model's diagram: draw the data layer, its filters and views
    /// (ADR 0050).
    pub data: bool,
    /// For the diagram: show every node's values, not only those the file
    /// opens (`controls.open`, ADR 0055).
    pub values: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            width: 1200.0,
            height: 800.0,
            scale: 1.0,
            view: None,
            light: false,
            subsystem: None,
            data: true,
            values: false,
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
    check(o)?;
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

    // One view alone, or as the saved arrangement places them.
    let layout = s.file.controls.layout.as_ref().filter(|_| o.view.is_none());
    let image = paint(o, |ui| match layout {
        Some(l) => arranged(ui, &mut views, &h, l),
        None => tiles(ui, &mut views, &h),
    });
    Ok((image, warnings))
}

/// Draw a model file's diagram, as the node canvas shows it, fitted to
/// the image (ADR 0042). Works for files that do not load: blocks that
/// cannot be built show their error.
pub fn render_diagram(
    file: &simsandbox_file::ModelFile,
    registry: &simsandbox_core::registry::Registry,
    o: &RenderOptions,
) -> Result<Image, String> {
    check(o)?;
    let scope = match &o.subsystem {
        None => crate::diagram::Scope::Model,
        Some(s) if file.subsystems.contains_key(s) => crate::diagram::Scope::Subsystem(s.clone()),
        Some(s) => {
            let known: Vec<&str> = file.subsystems.keys().map(String::as_str).collect();
            return Err(format!(
                "no subsystem '{s}' (subsystems: {})",
                if known.is_empty() {
                    "none".into()
                } else {
                    known.join(", ")
                }
            ));
        }
    };
    let mut d = crate::diagram::Diagram::of(file, &scope, registry);
    if o.data {
        d.observe(file, &crate::examples::view_registry());
    }
    // The nodes the file opens, or all.
    let mut canvas = if o.values {
        crate::canvas::Canvas::all_open()
    } else {
        crate::canvas::Canvas::opening(&file.controls.open)
    };
    Ok(paint(o, |ui| {
        canvas.show(ui, &mut d, registry, false);
    }))
}

fn check(o: &RenderOptions) -> Result<(), String> {
    if o.width >= 1.0 && o.height >= 1.0 && o.scale > 0.0 && o.scale <= 8.0 {
        Ok(())
    } else {
        Err("the size must be at least 1 × 1 and the scale in (0, 8]".into())
    }
}

/// Run egui headless for a few frames with `draw` filling a central
/// panel, and rasterise the last frame.
fn paint(o: &RenderOptions, mut draw: impl FnMut(&mut egui::Ui)) -> Image {
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
            egui::Frame::central_panel(ui.style()).show(ui, |ui| draw(ui));
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
    canvas.image()
}

/// The tiles in a grid, each with its type's name as a title, as in the
/// app (without its controls).
fn tiles(ui: &mut egui::Ui, views: &mut Views, history: &simsandbox_views::History) {
    let area = ui.available_rect_before_wrap();
    let all: Vec<usize> = (0..views.tiles.len()).collect();
    grid_in(ui, views, history, &all, area);
    ui.allocate_rect(area, egui::Sense::hover());
}

/// Draw the views as the saved arrangement places them (ADR 0053): each
/// stack shows its front tab, and parts without views (the inspector, the
/// diagram) give their space to the rest.
fn arranged(
    ui: &mut egui::Ui,
    views: &mut Views,
    history: &simsandbox_views::History,
    layout: &simsandbox_file::layout::Layout,
) {
    let area = ui.available_rect_before_wrap();
    // Each saved view's place among the tiles.
    let ids: Vec<u64> = views.tiles.iter().map(|t| t.id).collect();
    let place: Vec<Option<usize>> = views
        .opened
        .iter()
        .map(|id| id.and_then(|id| ids.iter().position(|t| *t == id)))
        .collect();
    for (shown, rect) in view_areas(layout, area) {
        let tiles: Vec<usize> = shown
            .into_iter()
            .filter_map(|k| place.get(k).copied().flatten())
            .collect();
        grid_in(ui, views, history, &tiles, rect);
    }
    ui.allocate_rect(area, egui::Sense::hover());
}

/// The views each stack shows at the front, and where, in `area`.
pub fn view_areas(
    layout: &simsandbox_file::layout::Layout,
    area: egui::Rect,
) -> Vec<(Vec<usize>, egui::Rect)> {
    use simsandbox_file::layout::{Layout, SplitKind};
    fn front(l: &Layout) -> Vec<usize> {
        match l {
            Layout::Tabs { tabs, active } => {
                tabs.get(*active).map(|p| p.views()).unwrap_or_default()
            }
            Layout::Split { first, second, .. } => {
                let mut v = front(first);
                v.extend(front(second));
                v
            }
        }
    }
    match layout {
        Layout::Tabs { .. } => {
            let v = front(layout);
            if v.is_empty() {
                Vec::new()
            } else {
                vec![(v, area)]
            }
        }
        Layout::Split {
            split,
            fraction,
            first,
            second,
        } => match (front(first).is_empty(), front(second).is_empty()) {
            (true, true) => Vec::new(),
            (true, false) => view_areas(second, area),
            (false, true) => view_areas(first, area),
            (false, false) => {
                let f = *fraction as f32;
                let (a, b) = match split {
                    SplitKind::LeftRight => {
                        let x = area.min.x + area.width() * f;
                        (
                            egui::Rect::from_min_max(area.min, egui::pos2(x, area.max.y)),
                            egui::Rect::from_min_max(egui::pos2(x, area.min.y), area.max),
                        )
                    }
                    SplitKind::TopBottom => {
                        let y = area.min.y + area.height() * f;
                        (
                            egui::Rect::from_min_max(area.min, egui::pos2(area.max.x, y)),
                            egui::Rect::from_min_max(egui::pos2(area.min.x, y), area.max),
                        )
                    }
                };
                let mut out = view_areas(first, a);
                out.extend(view_areas(second, b));
                out
            }
        },
    }
}

/// Some tiles (by place) in an automatic grid filling `area`.
fn grid_in(
    ui: &mut egui::Ui,
    views: &mut Views,
    history: &simsandbox_views::History,
    shown: &[usize],
    area: egui::Rect,
) {
    if shown.is_empty() {
        return;
    }
    let (cols, rows) = grid(shown.len());
    for (k, &i) in shown.iter().enumerate() {
        let tile = &mut views.tiles[i];
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
    fn every_example_renders_its_diagram() {
        for e in EXAMPLES {
            let (f, _) = simsandbox_file::ModelFile::from_json(e.text).unwrap();
            let o = RenderOptions {
                width: 500.0,
                height: 300.0,
                ..Default::default()
            };
            let img = render_diagram(&f, &registry(), &o).unwrap();
            let first = &img.rgba[..4];
            let differing = img.rgba.chunks(4).filter(|p| p != &first).count();
            assert!(differing > 3000, "{}: {differing} pixels drawn", e.name);
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
            subsystem: None,
            data: false,
            values: false,
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

    #[test]
    fn the_arrangement_places_the_views_and_drops_the_rest() {
        let l: simsandbox_file::layout::Layout = serde_json::from_str(
            r#"{"split": "left_right", "fraction": 0.25,
                "first": {"tabs": ["inspector"]},
                "second": {"split": "top_bottom", "fraction": 0.5,
                    "first": {"tabs": [{"views": [0, 2]}, "diagram"]},
                    "second": {"tabs": ["diagram", "view:1"], "active": 1}}}"#,
        )
        .unwrap();
        let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 200.0));
        let a = view_areas(&l, area);
        // The inspector gives its quarter to the views.
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].0, [0, 2]);
        assert_eq!(
            a[0].1,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(400.0, 100.0))
        );
        assert_eq!(a[1].0, [1]);
        assert_eq!(a[1].1.min, egui::pos2(0.0, 100.0));
    }
}
