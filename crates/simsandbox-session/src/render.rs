//! A model's views, or its diagram, drawn to an image without a window
//! (ADR 0039): the tiles or the node canvas the app would show, painted
//! by egui and rasterised on the CPU ([`crate::raster`]), then encoded as
//! PNG.

use crate::raster::{Canvas, Image, Textures};
use crate::views::{grid, Views};
use crate::Session;
use simsandbox_views::Recorded;
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
    /// For the diagram: the inside of the use at this path (`plant`,
    /// `vehicle/plant`) instead of the model (ADR 0068).
    pub inside: Option<String>,
    /// For the diagram: uses expanded in place, by path, besides those
    /// the file lists in `controls.expand` (ADR 0066 step 4d).
    pub expand: Vec<String>,
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
            inside: None,
            expand: Vec::new(),
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

    // Bindings from a probe draw what it recorded (ADR 0063).
    let shared: Vec<(&str, crate::session::SharedHistory)> = s
        .probe_names()
        .into_iter()
        .filter_map(|n| s.probe(n).map(|h| (n, h)))
        .collect();
    let guards: Vec<_> = shared.iter().map(|(n, h)| (*n, h.read())).collect();
    let probes: Vec<(&str, &simsandbox_views::History)> =
        guards.iter().map(|(n, g)| (*n, &**g)).collect();
    // Outputs of filters with clocks of their own, from their recordings.
    let derived_shared = s.derived_histories();
    let derived_guards: Vec<_> = derived_shared.iter().map(|(_, h)| h.read()).collect();
    let derived: Vec<&simsandbox_views::History> = derived_guards.iter().map(|g| &**g).collect();
    let recorded = Recorded {
        all: &h,
        probes: &probes,
        derived: &derived,
    };
    // One view alone, or as the saved arrangement places them.
    let layout = s.file.controls.layout.as_ref().filter(|_| o.view.is_none());
    let image = paint(o, |ui| match layout {
        Some(l) => arranged(ui, &mut views, &recorded, l),
        None => tiles(ui, &mut views, &recorded),
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
    // The inside of a use, by its path (ADR 0068), or the model.
    let inner;
    let file = match &o.inside {
        None => file,
        Some(path) => {
            let names: Vec<String> = path.split('/').map(str::to_string).collect();
            let folder = file
                .folder
                .clone()
                .or_else(simsandbox_core::registry::current_folder);
            inner = crate::edit_use::model_at(file, folder.as_deref(), &names)?;
            &inner
        }
    };
    let mut d = crate::diagram::Diagram::new(file, registry);
    // Inside a used model its ports and parameters can be exposed
    // (ADR 0074).
    d.inside = o.inside.is_some();
    let expanded: Vec<String> = file
        .controls
        .expand
        .iter()
        .chain(&o.expand)
        .cloned()
        .collect();
    // Without the data layer, probes feeding blocks still show: they are
    // part of the model (ADR 0063).
    let observed = if o.data {
        file.clone()
    } else {
        crate::observers::model_part(file)
    };
    d.observe(&observed, &crate::examples::view_registry());
    if !expanded.is_empty() {
        d.expand(file, registry, &expanded);
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
pub fn paint(o: &RenderOptions, mut draw: impl FnMut(&mut egui::Ui)) -> Image {
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
fn tiles(ui: &mut egui::Ui, views: &mut Views, history: &Recorded<'_>) {
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
    history: &Recorded<'_>,
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
    history: &Recorded<'_>,
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
                tile.state.show(ui, t, *history);
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
            let mut s =
                Session::load_in(e.name, e.text, &registry(), e.folder().as_deref()).unwrap();
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
            let img = simsandbox_core::registry::in_folder(e.folder().as_deref(), || {
                render_diagram(&f, &registry(), &o)
            })
            .unwrap();
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
            inside: None,
            expand: Vec::new(),
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

    /// A field over three axes drawn by a heat map, a surface and a
    /// contour (two kept, the third picked, reduced or faceted), and a
    /// profile of a 2D grid along one axis (ADR 0086).
    #[test]
    fn fields_of_any_rank_are_drawn_with_axes_left_over() {
        let model = |views: serde_json::Value| {
            serde_json::json!({"format": 1, "run": {"dt": 1, "t_end": 2},
                "blocks": [{"name": "m", "type": "eq.equations", "equations": [
                    "grid i = 4", "grid j = 3", "grid k = 5", "state c = 0", "c[n+1] = c + 1",
                    "q = i + 10*j + 100*k + c", "p = i + 10*j"]}],
                "controls": {"views": views}})
            .to_string()
        };
        let draw = |views: serde_json::Value| {
            let mut s = Session::load("m", &model(views), &registry()).unwrap();
            s.run_until(2.0).unwrap();
            let o = RenderOptions {
                width: 400.0,
                height: 300.0,
                ..Default::default()
            };
            let (img, warnings) = render(&s, view_registry(), &o).unwrap();
            assert!(warnings.is_empty(), "{warnings:?}");
            img.rgba
        };
        let heat = |sel: serde_json::Value| {
            draw(
                serde_json::json!([{"type": "plot.heatmap", "bind": {"field": "m.q"}, "selection": sel}]),
            )
        };
        let first = heat(serde_json::json!({}));
        let picked = heat(serde_json::json!({"pick": {"k": 3}}));
        let reduced = heat(serde_json::json!({"reduce": {"k": "mean"}}));
        let faceted = heat(serde_json::json!({"facet": "k"}));
        let differ =
            |a: &[u8], b: &[u8]| a.chunks(4).zip(b.chunks(4)).filter(|(p, q)| p != q).count();
        let drawn = |a: &[u8]| a.chunks(4).filter(|p| p != &&a[..4]).count();
        eprintln!(
            "drawn {} {} {} {}",
            drawn(&first),
            drawn(&picked),
            drawn(&reduced),
            drawn(&faceted)
        );
        assert!(differ(&first, &picked) > 0, "picked");
        assert!(differ(&first, &faceted) > 1000, "faceted");
        assert!(differ(&picked, &reduced) > 0, "reduced");
        for t in ["plot.surface", "plot.contour"] {
            draw(
                serde_json::json!([{"type": t, "bind": {"field": "m.q"}, "selection": {"pick": {"k": 4}}}]),
            );
        }
        let profile = |sel: serde_json::Value| {
            draw(
                serde_json::json!([{"type": "plot.profile", "bind": {"y": ["m.p"]}, "selection": sel}]),
            )
        };
        let plain = profile(serde_json::json!({}));
        assert!(
            differ(&plain, &profile(serde_json::json!({"pick": {"j": 2}}))) > 0,
            "profile pick"
        );
        assert!(
            differ(&plain, &profile(serde_json::json!({"facet": "j"}))) > 0,
            "profile facets"
        );
    }

    /// A quantity over a table's rows drawn by a profile against another
    /// over the same table, paired by id (ADR 0086 step 4b).
    #[test]
    fn rows_are_drawn_against_rows_of_the_same_table() {
        let model = serde_json::json!({"format": 1, "run": {"dt": 1, "t_end": 6},
            "blocks": [{"name": "m", "type": "eq.equations", "equations": [
                "table pts max 200", "start pts = 5", "state x[pts] = id(pts)/4",
                "state y[pts] = (id(pts)/4)^2", "x[n+1] = x", "y[n+1] = x^2",
                "edges segs of pts", "start segs = chain",
                "len = to_end(x, segs) - from_end(x, segs)",
                "split segs where len > 0.05: x = mean"]}],
            "controls": {"views": [{"type": "plot.profile", "bind": {"y": ["m.y"], "x": "m.x"}}]}})
        .to_string();
        let mut s = Session::load("m", &model, &registry()).unwrap();
        let h = s.signals();
        let a = s.available(&h);
        assert!(a.rows.contains(&"m.y".to_string()));
        assert_eq!(a.axis_of("m.y", "m.x"), Ok(Some(0)));
        drop(h);
        s.run_until(6.0).unwrap();
        let o = RenderOptions {
            width: 400.0,
            height: 300.0,
            ..Default::default()
        };
        let (img, warnings) = render(&s, view_registry(), &o).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let drawn = img.rgba.chunks(4).filter(|p| p != &&img.rgba[..4]).count();
        assert!(drawn > 2000, "{drawn}");
    }

    /// Meshes reach views through their ports (ADR 0086 step 4c): a
    /// refining 2D mesh and a 3D mesh, each opened on a mesh view of the
    /// value over its cells, which reads each cell's own state.
    #[test]
    fn meshes_are_drawn_from_their_ports() {
        for (shape, grid, coords) in [
            ("triangle", "x 0 .. 1 by 4, y 0 .. 1 by 4", "x, y"),
            (
                "hexa",
                "x 0 .. 1 by 2, y 0 .. 1 by 2, z 0 .. 1 by 2",
                "x, y, z",
            ),
        ] {
            let names: Vec<&str> = coords.split(", ").collect();
            let mut eqs = vec!["table pts max 10000".to_string()];
            for n in &names {
                eqs.push(format!("state {n}[pts]"));
                eqs.push(format!("{n}[n+1] = {n}"));
            }
            eqs.extend([
                format!("cells c of pts max 10000: {shape}"),
                format!("start c = grid {grid}"),
                "state u[c] = 1 + id(c)".into(),
                "u[n+1] = u".into(),
                "split c where id(c) < 1".into(),
            ]);
            let model = serde_json::json!({"format": 1, "run": {"dt": 1, "t_end": 2},
                "blocks": [{"name": "m", "type": "eq.equations", "equations": eqs}]})
            .to_string();
            let mut s = Session::load("m", &model, &registry()).unwrap();
            let h = s.signals();
            let a = s.available(&h);
            assert_eq!(a.meshes.len(), 1, "{shape}");
            let m = &a.meshes[0];
            assert_eq!(m.cells, "m.c");
            assert_eq!(m.over_cells, ["m.u"]);
            let want: Vec<String> = names.iter().map(|n| format!("m.{n}")).collect();
            assert_eq!(m.coordinates, want);
            drop(h);
            s.run_until(2.0).unwrap();
            // The cells' own values, not their corners (a fix with ADR
            // 0086: rows outputs of states over cells read past the
            // corners).
            let h = s.signals();
            let u = h.resolve("m.u");
            let Some(simsandbox_views::SignalRef::Field(f)) = u else {
                panic!("m.u recorded")
            };
            let (_, ids, values) = h.fields()[f].rows(h.len() - 1).unwrap();
            for (i, v) in ids.iter().zip(values) {
                assert!(*v == 1.0 + i || *i >= 8.0, "{shape}: cell {i} holds {v}");
            }
            drop(h);
            let o = RenderOptions {
                width: 400.0,
                height: 300.0,
                ..Default::default()
            };
            let (img, warnings) = render(&s, view_registry(), &o).unwrap();
            assert!(warnings.is_empty(), "{warnings:?}");
            let drawn = img.rgba.chunks(4).filter(|p| p != &&img.rgba[..4]).count();
            assert!(drawn > 10000, "{shape}: {drawn}");
        }
    }

    /// A cut through a 3D mesh (its section drawn flat) and a scatter of
    /// rows against rows by id, saved with their views (ADR 0086).
    #[test]
    fn cuts_and_rows_scatters_render_from_their_selections() {
        let model = |views: serde_json::Value| {
            serde_json::json!({"format": 1, "run": {"dt": 1, "t_end": 2},
                "blocks": [{"name": "m", "type": "eq.equations", "equations": [
                    "table pts max 10000", "state x[pts]", "state y[pts]", "state z[pts]",
                    "x[n+1] = x", "y[n+1] = y", "z[n+1] = z",
                    "cells c of pts max 10000: hexa",
                    "start c = grid x 0 .. 1 by 2, y 0 .. 1 by 2, z 0 .. 1 by 2",
                    "state u[c] = (id(c) - 2.5)^2", "u[n+1] = u", "w = x*y"]}],
                "controls": {"views": views}})
            .to_string()
        };
        let draw = |views: serde_json::Value| {
            let mut s = Session::load("m", &model(views), &registry()).unwrap();
            s.run_until(2.0).unwrap();
            let o = RenderOptions {
                width: 400.0,
                height: 300.0,
                ..Default::default()
            };
            let (img, warnings) = render(&s, view_registry(), &o).unwrap();
            assert!(warnings.is_empty(), "{warnings:?}");
            img.rgba
        };
        let mesh = |sel: serde_json::Value| {
            draw(serde_json::json!([{"type": "plot.mesh", "selection": sel,
                "bind": {"value": "m.u", "mesh": "m.c", "x": "m.x", "y": "m.y", "z": "m.z"}}]))
        };
        let differ =
            |a: &[u8], b: &[u8]| a.chunks(4).zip(b.chunks(4)).filter(|(p, q)| p != q).count();
        let whole = mesh(serde_json::json!({}));
        let low = mesh(serde_json::json!({"cut": {"normal": [0, 0, 1], "at": 0.25}}));
        let high = mesh(serde_json::json!({"cut": {"normal": [0, 0, 1], "at": 0.75}}));
        assert!(differ(&whole, &low) > 1000 && differ(&low, &high) > 1000);
        let scatter =
            draw(serde_json::json!([{"type": "plot.xy", "bind": {"x": "m.x", "y": ["m.w"]}}]));
        let drawn = scatter.chunks(4).filter(|p| p != &&scatter[..4]).count();
        assert!(drawn > 2000, "{drawn}");
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
