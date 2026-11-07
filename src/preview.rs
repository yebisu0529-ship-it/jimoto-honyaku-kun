//! Headless previews of the application's actual egui drawing code.
//!
//! This CPU backend needs neither a native window nor a graphics device. Pass a
//! fresh, configured context: its texture deltas are consumed during these frames.
//! GPU paint callbacks are intentionally rejected instead of silently disappearing.

use std::{collections::HashMap, path::Path};

use anyhow::{Context as _, Result, bail, ensure};
use eframe::egui::{self, Color32, Pos2, Rect, TextureId, epaint};

struct Texture {
    size: [usize; 2],
    pixels: Vec<Color32>,
}

const SETTLED_FRAME: u32 = 4;

/// Draw layout/font warm-up frames and save the final frame as a PNG. A scroll
/// area that follows its end needs a few frames to settle there.
pub fn render(
    ctx: &egui::Context,
    path: &Path,
    width: u32,
    height: u32,
    mut draw: impl FnMut(&egui::Context),
) -> Result<()> {
    ensure!(
        width > 0 && height > 0,
        "Preview dimensions must be positive"
    );
    ensure!(
        width <= 4096 && height <= 4096,
        "Preview exceeds 4096 pixels"
    );
    ctx.set_pixels_per_point(1.0);
    let mut textures = HashMap::new();
    let mut image = image::RgbaImage::new(width, height);

    for frame in 0..=SETTLED_FRAME {
        let mut input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(width as f32, height as f32),
            )),
            max_texture_side: Some(8192),
            time: Some(frame as f64 / 60.0),
            ..Default::default()
        };
        input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = Some(1.0);
        let output = ctx.run(input, &mut draw);
        for (id, delta) in &output.textures_delta.set {
            update_texture(&mut textures, *id, delta)?;
        }
        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        if frame == SETTLED_FRAME {
            let background = ctx.style().visuals.panel_fill;
            for pixel in image.pixels_mut() {
                *pixel = image::Rgba(background.to_array());
            }
            for primitive in primitives {
                match primitive.primitive {
                    epaint::Primitive::Mesh(mesh) => {
                        let texture = textures.get(&mesh.texture_id).with_context(|| {
                            format!(
                                "Missing preview texture {:?}; use a fresh egui context",
                                mesh.texture_id
                            )
                        })?;
                        paint_mesh(&mut image, &mesh, primitive.clip_rect, texture)?;
                    }
                    epaint::Primitive::Callback(_) => {
                        bail!("Headless preview does not support GPU paint callbacks")
                    }
                }
            }
        }
        for id in output.textures_delta.free {
            textures.remove(&id);
        }
    }
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    image
        .save_with_format(path, image::ImageFormat::Png)
        .with_context(|| format!("Saving preview {}", path.display()))
}

fn update_texture(
    textures: &mut HashMap<TextureId, Texture>,
    id: TextureId,
    delta: &epaint::ImageDelta,
) -> Result<()> {
    let epaint::ImageData::Color(source) = &delta.image;
    ensure!(
        source.size[0] > 0 && source.size[1] > 0,
        "Empty preview texture"
    );
    if let Some([x, y]) = delta.pos {
        let target = textures
            .get_mut(&id)
            .context("Texture patch has no base image")?;
        ensure!(
            x + source.size[0] <= target.size[0] && y + source.size[1] <= target.size[1],
            "Texture patch exceeds its base image"
        );
        for row in 0..source.size[1] {
            let start = (y + row) * target.size[0] + x;
            target.pixels[start..start + source.size[0]]
                .copy_from_slice(&source.pixels[row * source.size[0]..(row + 1) * source.size[0]]);
        }
    } else {
        textures.insert(
            id,
            Texture {
                size: source.size,
                pixels: source.pixels.clone(),
            },
        );
    }
    Ok(())
}

impl Texture {
    // Bilinear sampling with clamp-to-edge, matching egui's default UI/font sampler.
    fn sample(&self, uv: [f32; 2]) -> [f32; 4] {
        let x = (uv[0] * self.size[0] as f32 - 0.5).clamp(0.0, (self.size[0] - 1) as f32);
        let y = (uv[1] * self.size[1] as f32 - 0.5).clamp(0.0, (self.size[1] - 1) as f32);
        let x0 = x.floor() as usize;
        let y0 = y.floor() as usize;
        let x1 = (x0 + 1).min(self.size[0] - 1);
        let y1 = (y0 + 1).min(self.size[1] - 1);
        let tx = x - x0 as f32;
        let ty = y - y0 as f32;
        let samples = [
            (x0, y0, (1.0 - tx) * (1.0 - ty)),
            (x1, y0, tx * (1.0 - ty)),
            (x0, y1, (1.0 - tx) * ty),
            (x1, y1, tx * ty),
        ];
        let mut result = [0.0; 4];
        for (sx, sy, weight) in samples {
            let color = self.pixels[sy * self.size[0] + sx].to_array();
            for channel in 0..4 {
                result[channel] += color[channel] as f32 / 255.0 * weight;
            }
        }
        result
    }
}

fn edge(a: Pos2, b: Pos2, p: Pos2) -> f32 {
    (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
}

fn top_left(a: Pos2, b: Pos2) -> bool {
    b.y < a.y || (b.y == a.y && b.x > a.x)
}

fn paint_mesh(
    image: &mut image::RgbaImage,
    mesh: &epaint::Mesh,
    clip: Rect,
    texture: &Texture,
) -> Result<()> {
    ensure!(mesh.is_valid(), "Invalid preview mesh");
    for indices in mesh.indices.as_chunks::<3>().0 {
        let mut vertices = [
            mesh.vertices[indices[0] as usize],
            mesh.vertices[indices[1] as usize],
            mesh.vertices[indices[2] as usize],
        ];
        let mut area = edge(vertices[0].pos, vertices[1].pos, vertices[2].pos);
        if !area.is_finite() || area.abs() < 1e-6 {
            continue;
        }
        if area < 0.0 {
            vertices.swap(1, 2);
            area = -area;
        }
        let [a, b, c] = vertices;
        let min_x = a
            .pos
            .x
            .min(b.pos.x)
            .min(c.pos.x)
            .max(clip.min.x)
            .max(0.0)
            .floor() as u32;
        let min_y = a
            .pos
            .y
            .min(b.pos.y)
            .min(c.pos.y)
            .max(clip.min.y)
            .max(0.0)
            .floor() as u32;
        let max_x = a
            .pos
            .x
            .max(b.pos.x)
            .max(c.pos.x)
            .min(clip.max.x)
            .min(image.width() as f32)
            .ceil()
            .max(0.0) as u32;
        let max_y = a
            .pos
            .y
            .max(b.pos.y)
            .max(c.pos.y)
            .min(clip.max.y)
            .min(image.height() as f32)
            .ceil()
            .max(0.0) as u32;
        let colors = [a.color.to_array(), b.color.to_array(), c.color.to_array()];
        for y in min_y..max_y {
            for x in min_x..max_x {
                let p = egui::pos2(x as f32 + 0.5, y as f32 + 0.5);
                if p.x < clip.min.x || p.x >= clip.max.x || p.y < clip.min.y || p.y >= clip.max.y {
                    continue;
                }
                let edges = [
                    edge(b.pos, c.pos, p),
                    edge(c.pos, a.pos, p),
                    edge(a.pos, b.pos, p),
                ];
                let inclusive = [
                    top_left(b.pos, c.pos),
                    top_left(c.pos, a.pos),
                    top_left(a.pos, b.pos),
                ];
                if (0..3).any(|i| edges[i] < 0.0 || (edges[i] == 0.0 && !inclusive[i])) {
                    continue;
                }
                let weights = edges.map(|value| value / area);
                let uv = [
                    a.uv.x * weights[0] + b.uv.x * weights[1] + c.uv.x * weights[2],
                    a.uv.y * weights[0] + b.uv.y * weights[1] + c.uv.y * weights[2],
                ];
                let sample = texture.sample(uv);
                let mut source = [0.0; 4];
                for channel in 0..4 {
                    source[channel] = sample[channel]
                        * (0..3)
                            .map(|i| colors[i][channel] as f32 / 255.0 * weights[i])
                            .sum::<f32>();
                }
                let destination = image.get_pixel_mut(x, y);
                for channel in 0..4 {
                    destination[channel] = ((source[channel]
                        + destination[channel] as f32 / 255.0 * (1.0 - source[3]))
                        * 255.0)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_triangle_edge_is_blended_once_and_clip_is_respected() {
        let mut image = image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 255]));
        let texture = Texture {
            size: [1, 1],
            pixels: vec![Color32::WHITE],
        };
        let mut mesh = epaint::Mesh::default();
        mesh.add_colored_rect(
            Rect::from_min_max(Pos2::ZERO, egui::pos2(4.0, 4.0)),
            Color32::from_rgba_premultiplied(128, 0, 0, 128),
        );
        paint_mesh(
            &mut image,
            &mesh,
            Rect::from_min_max(Pos2::ZERO, egui::pos2(2.0, 4.0)),
            &texture,
        )
        .unwrap();
        for y in 0..4 {
            for x in 0..4 {
                assert_eq!(
                    image.get_pixel(x, y).0,
                    if x < 2 {
                        [128, 0, 0, 255]
                    } else {
                        [0, 0, 0, 255]
                    }
                );
            }
        }
    }
}
