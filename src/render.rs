use crate::frame::PixelRect;

/// Overlay brightness outside the selection (60%, i.e. darkened by 40%).
pub const DIM: u8 = 153;

/// B, G, R, X pixels with every byte scaled by about `brightness / 255`.
///
/// This runs on every output before the overlay can appear, so it is one
/// branch-free pass the compiler can vectorise. Scaling X too is harmless:
/// the overlay buffer is XRGB8888, which ignores it.
pub fn dim(pixels: &[u8], brightness: u8) -> Vec<u8> {
    let factor = brightness as u16 + 1;
    pixels
        .iter()
        .map(|&b| ((b as u16 * factor) >> 8) as u8)
        .collect()
}

/// Draws one overlay frame into `dst`: `dark` everywhere, `bright` inside
/// `sel`, and a 1 px white outline on the edge of `sel`.
pub fn compose(dst: &mut [u8], dark: &[u8], bright: &[u8], width: u32, sel: Option<PixelRect>) {
    dst.copy_from_slice(dark);
    let Some(r) = sel else { return };
    let stride = width as usize * 4;
    let (x0, x1) = (r.x as usize * 4, (r.x + r.width) as usize * 4);

    for y in r.y..r.y + r.height {
        let row = y as usize * stride;
        dst[row + x0..row + x1].copy_from_slice(&bright[row + x0..row + x1]);
        if y == r.y || y == r.y + r.height - 1 {
            dst[row + x0..row + x1].fill(255);
        } else {
            dst[row + x0..row + x0 + 4].fill(255);
            dst[row + x1 - 4..row + x1].fill(255);
        }
    }
}

/// Where a `label`-sized box goes next to the pointer: `gap` px right of and
/// below it, or on the other side where it would leave the surface.
pub fn label_origin(
    pointer: (f64, f64),
    label: (u32, u32),
    surface: (u32, u32),
    gap: f64,
) -> (u32, u32) {
    let place = |p: f64, len: u32, max: u32| {
        let after = p + gap;
        let v = if after + len as f64 <= max as f64 {
            after
        } else {
            p - gap - len as f64
        };
        v.clamp(0.0, max.saturating_sub(len) as f64) as u32
    };
    (
        place(pointer.0, label.0, surface.0),
        place(pointer.1, label.1, surface.1),
    )
}

/// `W × H` of a selection in physical pixels.
pub fn size_text(r: crate::frame::PixelRect) -> String {
    format!("{} × {}", r.width, r.height)
}

/// Draws premultiplied `src` over the XRGB8888 `dst` at `at`, clipped.
pub fn blend(dst: &mut [u8], dst_width: u32, src: &tiny_skia::Pixmap, at: (u32, u32)) {
    let dst_height = (dst.len() / 4) as u32 / dst_width.max(1);
    for sy in 0..src.height() {
        let y = at.1 + sy;
        if y >= dst_height {
            break;
        }
        for sx in 0..src.width() {
            let x = at.0 + sx;
            if x >= dst_width {
                break;
            }
            let s = src.pixel(sx, sy).expect("in bounds");
            let i = ((y * dst_width + x) * 4) as usize;
            let inv = 255 - s.alpha() as u16;
            let over = |d: u8, c: u8| (c as u16 + (d as u16 * inv + 127) / 255) as u8;
            dst[i] = over(dst[i], s.blue());
            dst[i + 1] = over(dst[i + 1], s.green());
            dst[i + 2] = over(dst[i + 2], s.red());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dims_every_byte() {
        assert_eq!(dim(&[200, 100, 50, 0], 255), vec![200, 100, 50, 0]);
        assert_eq!(dim(&[200, 100, 50, 255], DIM), vec![120, 60, 30, 153]);
        assert_eq!(dim(&[255, 0, 1, 2], 0), vec![0, 0, 0, 0]);
    }

    fn px(buf: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * width + x) * 4) as usize;
        buf[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn compose_outline_and_inside() {
        let (w, h) = (6, 6);
        let dark = vec![1u8; (w * h * 4) as usize];
        let bright = vec![2u8; (w * h * 4) as usize];
        let mut dst = vec![0u8; dark.len()];
        let sel = PixelRect {
            x: 1,
            y: 1,
            width: 4,
            height: 4,
        };

        compose(&mut dst, &dark, &bright, w, Some(sel));

        assert_eq!(px(&dst, w, 0, 0), [1; 4], "outside");
        assert_eq!(px(&dst, w, 1, 1), [255; 4], "corner");
        assert_eq!(px(&dst, w, 4, 2), [255; 4], "right edge");
        assert_eq!(px(&dst, w, 2, 4), [255; 4], "bottom edge");
        assert_eq!(px(&dst, w, 2, 2), [2; 4], "inside");
        assert_eq!(px(&dst, w, 5, 5), [1; 4], "outside");
    }

    #[test]
    fn compose_without_selection_is_all_dark() {
        let dark = vec![1u8; 16];
        let mut dst = vec![0u8; 16];
        compose(&mut dst, &dark, &[2u8; 16], 2, None);
        assert_eq!(dst, dark);
    }

    #[test]
    fn compose_one_pixel_selection() {
        let dark = vec![1u8; 16];
        let mut dst = vec![0u8; 16];
        let sel = PixelRect {
            x: 1,
            y: 1,
            width: 1,
            height: 1,
        };
        compose(&mut dst, &dark, &[2u8; 16], 2, Some(sel));
        assert_eq!(px(&dst, 2, 1, 1), [255; 4]);
    }

    #[test]
    fn label_sits_below_right_of_the_pointer() {
        assert_eq!(
            label_origin((100.0, 100.0), (80, 24), (1920, 1080), 16.0),
            (116, 116)
        );
    }

    #[test]
    fn label_flips_at_edges() {
        assert_eq!(
            label_origin((1900.0, 100.0), (80, 24), (1920, 1080), 16.0),
            (1900 - 16 - 80, 116)
        );
        assert_eq!(
            label_origin((100.0, 1070.0), (80, 24), (1920, 1080), 16.0),
            (116, 1070 - 16 - 24)
        );
        assert_eq!(
            label_origin((1900.0, 1070.0), (80, 24), (1920, 1080), 16.0),
            (1804, 1030)
        );
        // Tiny outputs: never negative.
        assert_eq!(label_origin((5.0, 5.0), (80, 24), (60, 20), 16.0), (0, 0));
    }

    #[test]
    fn size_text_uses_the_multiplication_sign() {
        let r = PixelRect {
            x: 3,
            y: 4,
            width: 640,
            height: 480,
        };
        assert_eq!(size_text(r), "640 × 480");
    }

    #[test]
    fn blend_draws_over_and_clips() {
        let (w, h) = (4u32, 2u32);
        let mut dst = vec![100u8; (w * h * 4) as usize];
        let mut src = tiny_skia::Pixmap::new(2, 2).unwrap();
        src.fill(tiny_skia::Color::from_rgba8(255, 0, 0, 255));
        blend(&mut dst, w, &src, (3, 0));
        assert_eq!(px(&dst, w, 3, 0)[..3], [0, 0, 255], "BGR red, opaque");
        assert_eq!(
            px(&dst, w, 2, 0)[..3],
            [100, 100, 100],
            "left of it untouched"
        );
        let mut half = tiny_skia::Pixmap::new(1, 1).unwrap();
        half.fill(tiny_skia::Color::from_rgba8(0, 0, 0, 128));
        blend(&mut dst, w, &half, (0, 1));
        assert!(
            (px(&dst, w, 0, 1)[0] as i32 - 50).abs() <= 1,
            "half black over 100"
        );
    }
}
