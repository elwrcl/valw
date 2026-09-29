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
}
