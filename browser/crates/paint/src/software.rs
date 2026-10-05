//! Scalar fallback. Same rectangles, clips and straight-alpha math as WGSL.

use crate::Quad;
use layout::{RasterImage, Rect};

pub(crate) fn paint(pixels: &mut [u32], width: u32, height: u32, quads: &[(Quad, Rect)]) {
    if pixels.len() < width as usize * height as usize {
        return;
    }
    let canvas = Rect::new(0.0, 0.0, width as f32, height as f32);
    for (quad, clip) in quads {
        let r = quad.rect;
        let bounds = r.intersection(*clip).intersection(canvas);
        let x0 = (bounds.x - 0.5).ceil().max(0.0) as u32;
        let y0 = (bounds.y - 0.5).ceil().max(0.0) as u32;
        let x1 = (bounds.right() - 0.5).ceil().min(width as f32) as u32;
        let y1 = (bounds.bottom() - 0.5).ceil().min(height as f32) as u32;
        if x0 >= x1 || y0 >= y1 {
            continue;
        }
        let c = quad.color;
        if quad.image.is_none() && quad.rounded.is_none() && c.a == 255 {
            let color = (u32::from(c.r) << 16) | (u32::from(c.g) << 8) | u32::from(c.b);
            for y in y0..y1 {
                pixels[(y * width + x0) as usize..(y * width + x1) as usize].fill(color);
            }
            continue;
        }
        for y in y0..y1 {
            for x in x0..x1 {
                if let Some((round, radius)) = quad.rounded {
                    let radius = radius.min(round.width / 2.0).min(round.height / 2.0);
                    let cx = (x as f32 + 0.5).clamp(round.x + radius, round.right() - radius);
                    let cy = (y as f32 + 0.5).clamp(round.y + radius, round.bottom() - radius);
                    if (x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)
                        > radius * radius
                    {
                        continue;
                    }
                }
                let texel = quad
                    .image
                    .as_ref()
                    .map(|image| {
                        sample(
                            image,
                            (x as f32 + 0.5 - r.x) / r.width,
                            (y as f32 + 0.5 - r.y) / r.height,
                        )
                    })
                    .unwrap_or([255.0; 4]);
                let alpha = texel[3] * c.a as f32 / (255.0 * 255.0);
                if alpha <= 0.0 {
                    continue;
                }
                let index = (y * width + x) as usize;
                let dst = pixels[index];
                let blend = |src: f32, tint: u8, shift: u32| {
                    ((src * tint as f32 / 255.0) * alpha
                        + ((dst >> shift) & 255u32) as f32 * (1.0 - alpha))
                        .round()
                        .clamp(0.0, 255.0) as u32
                };
                pixels[index] = (blend(texel[0], c.r, 16) << 16)
                    | (blend(texel[1], c.g, 8) << 8)
                    | blend(texel[2], c.b, 0);
            }
        }
    }
}

fn sample(image: &RasterImage, u: f32, v: f32) -> [f32; 4] {
    let x = (u * image.width as f32 - 0.5).clamp(0.0, image.width.saturating_sub(1) as f32);
    let y = (v * image.height as f32 - 0.5).clamp(0.0, image.height.saturating_sub(1) as f32);
    let (x0, y0) = (x.floor() as u32, y.floor() as u32);
    let (x1, y1) = (
        (x0 + 1).min(image.width - 1),
        (y0 + 1).min(image.height - 1),
    );
    let (dx, dy) = (x - x0 as f32, y - y0 as f32);
    std::array::from_fn(|c| {
        let get = |x, y| image.rgba[((y * image.width + x) * 4) as usize + c] as f32;
        (get(x0, y0) * (1.0 - dx) + get(x1, y0) * dx) * (1.0 - dy)
            + (get(x0, y1) * (1.0 - dx) + get(x1, y1) * dx) * dy
    })
}
