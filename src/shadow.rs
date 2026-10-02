//! A macOS-style soft shadow around window shots.

use image::{Rgba, RgbaImage};

pub const PAD: u32 = 40;
pub const OFFSET_Y: u32 = 12;
pub const BLUR: u32 = 20;
pub const OPACITY: f32 = 0.5;

/// `image` on a transparent canvas `PAD` px larger on every side, over a
/// blurred, offset copy of its own alpha in black.
pub fn add(image: &RgbaImage) -> RgbaImage {
    let (w, h) = image.dimensions();
    let (cw, ch) = (w + 2 * PAD, h + 2 * PAD);
    let mut alpha = vec![0f32; (cw * ch) as usize];
    for (x, y, p) in image.enumerate_pixels() {
        let (sx, sy) = (x + PAD, y + PAD + OFFSET_Y);
        if sy < ch {
            alpha[(sy * cw + sx) as usize] = p.0[3] as f32 / 255.0;
        }
    }
    // Three box blurs approximate a Gaussian of about BLUR / 2 sigma.
    let r = (BLUR / 3).max(1) as usize;
    for _ in 0..3 {
        alpha = box_blur(&alpha, cw as usize, ch as usize, r);
    }
    let mut out = RgbaImage::from_fn(cw, ch, |x, y| {
        let a = (alpha[(y * cw + x) as usize] * OPACITY * 255.0).round() as u8;
        Rgba([0, 0, 0, a])
    });
    for (x, y, p) in image.enumerate_pixels() {
        let dst = out.get_pixel_mut(x + PAD, y + PAD);
        *dst = over(*p, *dst);
    }
    out
}

/// Straight-alpha `top` over `bottom`.
fn over(top: Rgba<u8>, bottom: Rgba<u8>) -> Rgba<u8> {
    let ta = top.0[3] as f32 / 255.0;
    let ba = bottom.0[3] as f32 / 255.0;
    let a = ta + ba * (1.0 - ta);
    if a <= 0.0 {
        return Rgba([0, 0, 0, 0]);
    }
    let mix = |t: u8, b: u8| ((t as f32 * ta + b as f32 * ba * (1.0 - ta)) / a).round() as u8;
    Rgba([
        mix(top.0[0], bottom.0[0]),
        mix(top.0[1], bottom.0[1]),
        mix(top.0[2], bottom.0[2]),
        (a * 255.0).round() as u8,
    ])
}

/// A separable box blur of radius `r` (horizontal then vertical).
fn box_blur(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let n = (2 * r + 1) as f32;
    let mut tmp = vec![0f32; src.len()];
    for y in 0..h {
        let row = &src[y * w..(y + 1) * w];
        let mut sum: f32 = (0..=r).map(|i| row.get(i).copied().unwrap_or(0.0)).sum();
        for x in 0..w {
            tmp[y * w + x] = sum / n;
            if let Some(v) = row.get(x + r + 1) {
                sum += v;
            }
            if x >= r {
                sum -= row[x - r];
            }
        }
    }
    let mut out = vec![0f32; src.len()];
    for x in 0..w {
        let at = |y: usize| tmp[y * w + x];
        let mut sum: f32 = (0..=r.min(h - 1)).map(at).sum();
        for y in 0..h {
            out[y * w + x] = sum / n;
            if y + r + 1 < h {
                sum += at(y + r + 1);
            }
            if y >= r {
                sum -= at(y - r);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn window() -> RgbaImage {
        RgbaImage::from_pixel(100, 60, Rgba([200, 100, 50, 255]))
    }

    #[test]
    fn the_image_grows_by_the_padding() {
        let out = add(&window());
        assert_eq!(out.dimensions(), (100 + 2 * PAD, 60 + 2 * PAD));
    }

    #[test]
    fn the_window_is_unchanged_in_the_middle() {
        let out = add(&window());
        assert_eq!(out.get_pixel(PAD + 50, PAD + 30).0, [200, 100, 50, 255]);
    }

    #[test]
    fn the_shadow_is_below_and_fades_out() {
        let out = add(&window());
        let below = out.get_pixel(PAD + 50, PAD + 60 + 8).0;
        let above = out.get_pixel(PAD + 50, PAD - 8).0;
        assert_eq!(&below[..3], &[0, 0, 0]);
        assert!(below[3] > 40, "{below:?}");
        assert!(below[3] > above[3], "offset down: {below:?} vs {above:?}");
        assert_eq!(out.get_pixel(0, 0).0[3], 0, "far corner clear");
    }

    #[test]
    fn rounded_corners_are_followed() {
        let mut w = window();
        for (x, y) in [(0, 0), (1, 0), (0, 1)] {
            w.put_pixel(x, y, Rgba([0, 0, 0, 0]));
        }
        let out = add(&w);
        assert!(
            out.get_pixel(PAD, PAD).0[3] < 255,
            "the transparent corner stays see-through"
        );
    }
}
