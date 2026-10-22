//! A small CPU rasteriser for what egui paints, so views can be drawn to an
//! image with no window and no GPU (`simsandbox render`, the Python API;
//! ADR 0039). It does what egui's GPU painters do: triangles with
//! per-vertex colour times a texture, clipped, blended with premultiplied
//! alpha in gamma space. egui already anti-aliases edges by feathering, so
//! one sample per pixel is enough.

use egui::epaint::{ClippedPrimitive, ImageDelta, Primitive, TextureId, Vertex};
use egui::TexturesDelta;
use std::collections::HashMap;

/// An RGBA image, 8 bits per channel, rows top to bottom, not
/// premultiplied (as PNG stores it).
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// A texture, premultiplied, as floats in 0..=1.
struct Texture {
    width: usize,
    height: usize,
    texels: Vec<[f32; 4]>,
    linear: bool,
}

impl Texture {
    fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        let texel = |x: isize, y: isize| {
            let x = x.clamp(0, self.width as isize - 1) as usize;
            let y = y.clamp(0, self.height as isize - 1) as usize;
            self.texels[y * self.width + x]
        };
        let (x, y) = (u * self.width as f32, v * self.height as f32);
        if !self.linear {
            return texel(x.floor() as isize, y.floor() as isize);
        }
        let (x, y) = (x - 0.5, y - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (x0, y0) = (x0 as isize, y0 as isize);
        let [a, b, c, d] = [
            texel(x0, y0),
            texel(x0 + 1, y0),
            texel(x0, y0 + 1),
            texel(x0 + 1, y0 + 1),
        ];
        std::array::from_fn(|i| {
            let top = a[i] + (b[i] - a[i]) * fx;
            let bottom = c[i] + (d[i] - c[i]) * fx;
            top + (bottom - top) * fy
        })
    }
}

/// Textures egui has asked for, kept across frames as a GPU painter keeps
/// them.
#[derive(Default)]
pub struct Textures(HashMap<TextureId, Texture>);

impl Textures {
    /// Apply egui's texture changes for a frame.
    pub fn update(&mut self, mut delta: TexturesDelta) {
        for (id, deltas) in &delta.set {
            for d in deltas {
                self.set(*id, d);
            }
        }
        for id in &delta.free {
            self.0.remove(id);
        }
        delta.clear();
    }

    fn set(&mut self, id: TextureId, d: &ImageDelta) {
        let egui::ImageData::Color(image) = &d.image;
        let [w, h] = image.size;
        let texels = image
            .pixels
            .iter()
            .map(|c| c.to_array().map(|v| v as f32 / 255.0))
            .collect::<Vec<_>>();
        let linear = d.options.magnification == egui::TextureFilter::Linear;
        match d.pos {
            None => {
                self.0.insert(
                    id,
                    Texture {
                        width: w,
                        height: h,
                        texels,
                        linear,
                    },
                );
            }
            Some([x0, y0]) => {
                let Some(t) = self.0.get_mut(&id) else {
                    return;
                };
                for y in 0..h {
                    for x in 0..w {
                        if x0 + x < t.width && y0 + y < t.height {
                            t.texels[(y0 + y) * t.width + x0 + x] = texels[y * w + x];
                        }
                    }
                }
            }
        }
    }
}

/// A canvas being painted, premultiplied, as floats.
pub struct Canvas {
    width: usize,
    height: usize,
    pixels: Vec<[f32; 4]>,
}

impl Canvas {
    /// A canvas of `width` × `height` pixels filled with `background`.
    pub fn new(width: usize, height: usize, background: egui::Color32) -> Self {
        let bg = background.to_array().map(|v| v as f32 / 255.0);
        Self {
            width,
            height,
            pixels: vec![bg; width * height],
        }
    }

    /// Paint tessellated primitives, in order. Coordinates are in points;
    /// one point is `pixels_per_point` pixels.
    pub fn paint(
        &mut self,
        primitives: &[ClippedPrimitive],
        textures: &Textures,
        pixels_per_point: f32,
    ) {
        for p in primitives {
            let Primitive::Mesh(mesh) = &p.primitive else {
                continue; // No view paints with custom callbacks.
            };
            let Some(texture) = textures.0.get(&mesh.texture_id) else {
                continue;
            };
            let clip = p.clip_rect * pixels_per_point;
            let clip = [
                clip.min.x.max(0.0).floor() as usize,
                clip.min.y.max(0.0).floor() as usize,
                (clip.max.x.ceil().max(0.0) as usize).min(self.width),
                (clip.max.y.ceil().max(0.0) as usize).min(self.height),
            ];
            for tri in mesh.indices.chunks_exact(3) {
                let v = [0, 1, 2].map(|k| &mesh.vertices[tri[k] as usize]);
                self.triangle(v, texture, pixels_per_point, clip);
            }
        }
    }

    fn triangle(&mut self, v: [&Vertex; 3], texture: &Texture, ppp: f32, clip: [usize; 4]) {
        let p = v.map(|v| [v.pos.x * ppp, v.pos.y * ppp]);
        let area = edge(p[0], p[1], p[2]);
        if area == 0.0 || !area.is_finite() {
            return;
        }
        let min_x = p.iter().map(|q| q[0]).fold(f32::INFINITY, f32::min);
        let max_x = p.iter().map(|q| q[0]).fold(f32::NEG_INFINITY, f32::max);
        let min_y = p.iter().map(|q| q[1]).fold(f32::INFINITY, f32::min);
        let max_y = p.iter().map(|q| q[1]).fold(f32::NEG_INFINITY, f32::max);
        let x0 = (min_x.floor().max(clip[0] as f32)) as usize;
        let y0 = (min_y.floor().max(clip[1] as f32)) as usize;
        let x1 = (max_x.ceil().max(0.0) as usize).min(clip[2]);
        let y1 = (max_y.ceil().max(0.0) as usize).min(clip[3]);
        let colour = v.map(|v| v.color.to_array().map(|c| c as f32 / 255.0));
        for y in y0..y1 {
            for x in x0..x1 {
                // Pixel centres, as GPUs sample.
                let q = [x as f32 + 0.5, y as f32 + 0.5];
                let w = [
                    edge(p[1], p[2], q) / area,
                    edge(p[2], p[0], q) / area,
                    edge(p[0], p[1], q) / area,
                ];
                if w.iter().any(|&w| w < 0.0) {
                    continue;
                }
                let u = w[0] * v[0].uv.x + w[1] * v[1].uv.x + w[2] * v[2].uv.x;
                let t = w[0] * v[0].uv.y + w[1] * v[1].uv.y + w[2] * v[2].uv.y;
                let tex = texture.sample(u, t);
                let src: [f32; 4] = std::array::from_fn(|i| {
                    (w[0] * colour[0][i] + w[1] * colour[1][i] + w[2] * colour[2][i]) * tex[i]
                });
                let dst = &mut self.pixels[y * self.width + x];
                for i in 0..4 {
                    dst[i] = src[i] + dst[i] * (1.0 - src[3]);
                }
            }
        }
    }

    /// The painted image, un-premultiplied.
    pub fn image(&self) -> Image {
        let mut rgba = Vec::with_capacity(self.pixels.len() * 4);
        for p in &self.pixels {
            let a = p[3].clamp(0.0, 1.0);
            for c in &p[..3] {
                let c = if a > 0.0 { c / a } else { 0.0 };
                rgba.push((c.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
            rgba.push((a * 255.0).round() as u8);
        }
        Image {
            width: self.width,
            height: self.height,
            rgba,
        }
    }
}

/// Twice the signed area of (a, b, c).
fn edge(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Color32, Pos2, Rect};

    /// Draw `add` with egui and rasterise it on a 100 × 60 canvas.
    fn draw(add: impl FnMut(&mut egui::Ui)) -> Image {
        let ctx = egui::Context::default();
        let mut add = add;
        let mut textures = Textures::default();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(100.0, 60.0))),
            ..Default::default()
        };
        let out = ctx.run_ui(input, |ui| add(ui));
        textures.update(out.textures_delta);
        let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
        let mut canvas = Canvas::new(100, 60, Color32::BLACK);
        canvas.paint(&prims, &textures, out.pixels_per_point);
        canvas.image()
    }

    fn pixel(img: &Image, x: usize, y: usize) -> [u8; 4] {
        let i = 4 * (y * img.width + x);
        img.rgba[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn fills_rectangles_exactly_and_blends_translucent_ones() {
        let img = draw(|ui| {
            let p = ui.painter();
            p.rect_filled(
                Rect::from_min_max(Pos2::new(10.0, 10.0), Pos2::new(30.0, 20.0)),
                0.0,
                Color32::RED,
            );
            // Half-transparent white over the red's right half.
            p.rect_filled(
                Rect::from_min_max(Pos2::new(20.0, 10.0), Pos2::new(40.0, 20.0)),
                0.0,
                Color32::from_white_alpha(128),
            );
        });
        // Inside, away from feathered edges.
        assert_eq!(pixel(&img, 15, 15), [255, 0, 0, 255]);
        let blended = pixel(&img, 25, 15);
        assert_eq!(blended[0], 255);
        assert!((126..=129).contains(&blended[1]), "{blended:?}");
        let over_black = pixel(&img, 35, 15);
        assert!((126..=129).contains(&over_black[0]), "{over_black:?}");
        assert_eq!(pixel(&img, 50, 40), [0, 0, 0, 255]);
    }

    #[test]
    fn draws_text_from_the_font_atlas() {
        let img = draw(|ui| {
            ui.painter().text(
                Pos2::new(5.0, 5.0),
                egui::Align2::LEFT_TOP,
                "Hello",
                egui::FontId::proportional(20.0),
                Color32::WHITE,
            );
        });
        let lit = (0..img.width * img.height)
            .filter(|&i| img.rgba[4 * i] > 128)
            .count();
        // Glyphs cover part of their box: some pixels, not all.
        assert!((40..1000).contains(&lit), "{lit} lit pixels");
        // Nothing outside the text's box.
        assert!((0..100).all(|x| pixel(&img, x, 50)[0] == 0));
    }
}
