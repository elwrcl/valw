use std::sync::Arc;

use anyhow::{Result, bail, ensure};
use image::{RgbaImage, imageops};
use wayland_client::protocol::{wl_output::Transform, wl_shm::Format};

/// An output's place in the global logical coordinate space.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputGeom {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// A rectangle in a frame's physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Pixels in wl_shm XRGB8888 memory order: B, G, R, X. This is what niri
/// hands out and what the overlay shows, so nothing on the way to the first
/// overlay frame has to reorder bytes. `RgbaImage` is only the container;
/// its channels do not mean R, G, B, A here.
pub type Bgrx = RgbaImage;

/// One captured output: upright pixels at physical resolution.
#[derive(Debug, Clone)]
pub struct Frame {
    pub output: OutputGeom,
    /// Shared with the overlay, which shows it as the undimmed selection.
    pub pixels: Arc<Bgrx>,
}

impl Frame {
    /// The whole frame as a rectangle.
    pub fn full(&self) -> PixelRect {
        PixelRect {
            x: 0,
            y: 0,
            width: self.pixels.width(),
            height: self.pixels.height(),
        }
    }

    /// The `rect` part as opaque RGBA, for saving.
    ///
    /// Alpha is always 255: a screenshot of what's on screen is opaque, even
    /// where the compositor's buffer has alpha 0.
    pub fn to_rgba(&self, rect: PixelRect) -> RgbaImage {
        let stride = self.pixels.width() as usize * 4;
        let raw = self.pixels.as_raw();
        let mut out = RgbaImage::new(rect.width, rect.height);
        for (y, row) in out.rows_mut().enumerate() {
            let start = (rect.y as usize + y) * stride + rect.x as usize * 4;
            let (src, _) = raw[start..start + rect.width as usize * 4].as_chunks::<4>();
            for (px, s) in row.zip(src) {
                px.0 = [s[2], s[1], s[0], 255];
            }
        }
        out
    }
}

/// Shm formats valw can read, in order of preference.
pub const SUPPORTED: [Format; 4] = [
    Format::Xrgb8888,
    Format::Argb8888,
    Format::Xbgr8888,
    Format::Abgr8888,
];

/// Copies a screencopy buffer into B, G, R, X order, dropping stride padding.
pub fn to_bgrx(
    format: Format,
    width: u32,
    height: u32,
    stride: u32,
    data: &[u8],
    y_invert: bool,
) -> Result<Bgrx> {
    let swap = match format {
        Format::Xrgb8888 | Format::Argb8888 => false,
        Format::Xbgr8888 | Format::Abgr8888 => true,
        other => bail!("unsupported buffer format {other:?}"),
    };
    ensure!(
        stride >= width * 4,
        "stride {stride} is too small for width {width}"
    );
    ensure!(
        data.len() >= (stride * height) as usize,
        "buffer is {} bytes, expected at least {}",
        data.len(),
        stride * height
    );

    let row_len = width as usize * 4;
    let mut out = Bgrx::new(width, height);
    for (y, dst) in out.chunks_exact_mut(row_len).enumerate() {
        let src_y = if y_invert { height as usize - 1 - y } else { y };
        let src = &data[src_y * stride as usize..][..row_len];
        dst.copy_from_slice(src);
        if swap {
            for px in dst.as_chunks_mut::<4>().0 {
                px.swap(0, 2);
            }
        }
    }
    Ok(out)
}

/// Turns a buffer in the output's native orientation upright, the same way
/// grim does: rotate clockwise by the transform's angle, then mirror.
pub fn apply_transform(img: Bgrx, transform: Transform) -> Bgrx {
    let rotated = match transform {
        Transform::_90 | Transform::Flipped90 => imageops::rotate90(&img),
        Transform::_180 | Transform::Flipped180 => imageops::rotate180(&img),
        Transform::_270 | Transform::Flipped270 => imageops::rotate270(&img),
        _ => img,
    };
    match transform {
        Transform::Flipped
        | Transform::Flipped90
        | Transform::Flipped180
        | Transform::Flipped270 => imageops::flip_horizontal(&rotated),
        _ => rotated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2x1 buffer: a red pixel then a blue one, in the given byte order.
    fn two_pixels(format: Format) -> Vec<u8> {
        match format {
            Format::Xrgb8888 | Format::Argb8888 => vec![0, 0, 255, 0, 255, 0, 0, 0],
            _ => vec![255, 0, 0, 0, 0, 0, 255, 0],
        }
    }

    fn bgr(img: &Bgrx, x: u32, y: u32) -> [u8; 3] {
        let p = img.get_pixel(x, y).0;
        [p[0], p[1], p[2]]
    }

    #[test]
    fn reads_all_supported_formats_as_bgrx() {
        for format in SUPPORTED {
            let img = to_bgrx(format, 2, 1, 8, &two_pixels(format), false).unwrap();
            assert_eq!(bgr(&img, 0, 0), [0, 0, 255], "red, {format:?}");
            assert_eq!(bgr(&img, 1, 0), [255, 0, 0], "blue, {format:?}");
        }
    }

    #[test]
    fn honours_stride_padding() {
        // One pixel per row, 8-byte stride.
        let data = [0, 0, 255, 0, 9, 9, 9, 9, 255, 0, 0, 0, 9, 9, 9, 9];
        let img = to_bgrx(Format::Xrgb8888, 1, 2, 8, &data, false).unwrap();
        assert_eq!(img.as_raw().len(), 8);
        assert_eq!(bgr(&img, 0, 0), [0, 0, 255]);
        assert_eq!(bgr(&img, 0, 1), [255, 0, 0]);
    }

    #[test]
    fn y_invert_flips_rows() {
        let data = [0, 0, 255, 0, 255, 0, 0, 0];
        let img = to_bgrx(Format::Xrgb8888, 1, 2, 4, &data, true).unwrap();
        assert_eq!(bgr(&img, 0, 0), [255, 0, 0]);
        assert_eq!(bgr(&img, 0, 1), [0, 0, 255]);
    }

    #[test]
    fn rejects_unknown_format_by_name() {
        let err = to_bgrx(Format::Xbgr2101010, 1, 1, 4, &[0; 4], false).unwrap_err();
        assert_eq!(err.to_string(), "unsupported buffer format Xbgr2101010");
    }

    #[test]
    fn rejects_short_buffer() {
        assert!(to_bgrx(Format::Xrgb8888, 2, 2, 8, &[0; 8], false).is_err());
    }

    fn frame(width: u32, height: u32, data: Vec<u8>) -> Frame {
        Frame {
            output: OutputGeom {
                name: "A".into(),
                x: 0,
                y: 0,
                width: width as i32,
                height: height as i32,
            },
            pixels: Arc::new(Bgrx::from_raw(width, height, data).unwrap()),
        }
    }

    #[test]
    fn to_rgba_swaps_channels_and_makes_opaque() {
        // Red with alpha 0, as an ARGB8888 buffer may deliver it.
        let f = frame(1, 1, vec![0, 0, 255, 0]);
        assert_eq!(f.to_rgba(f.full()).get_pixel(0, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn to_rgba_crops() {
        // 3x2: pixel value = its index, in the blue byte.
        let data = (0u8..6).flat_map(|i| [i, 0, 0, 0]).collect();
        let f = frame(3, 2, data);
        let rect = PixelRect {
            x: 1,
            y: 1,
            width: 2,
            height: 1,
        };
        let out = f.to_rgba(rect);
        assert_eq!(out.dimensions(), (2, 1));
        assert_eq!(out.get_pixel(0, 0).0, [0, 0, 4, 255]);
        assert_eq!(out.get_pixel(1, 0).0, [0, 0, 5, 255]);
    }

    fn marked() -> Bgrx {
        // 3x2, top-left pixel marked.
        let mut img = Bgrx::new(3, 2);
        img.put_pixel(0, 0, image::Rgba([255, 255, 255, 255]));
        img
    }

    fn marked_at(img: &Bgrx) -> (u32, u32, u32, u32) {
        let (x, y, _) = img
            .enumerate_pixels()
            .find(|(_, _, p)| p.0[0] == 255)
            .unwrap();
        (img.width(), img.height(), x, y)
    }

    #[test]
    fn transforms_match_grim() {
        use Transform::*;
        let cases = [
            (Normal, (3, 2, 0, 0)),
            (_90, (2, 3, 1, 0)),
            (_180, (3, 2, 2, 1)),
            (_270, (2, 3, 0, 2)),
            (Flipped, (3, 2, 2, 0)),
            (Flipped90, (2, 3, 0, 0)),
            (Flipped180, (3, 2, 0, 1)),
            (Flipped270, (2, 3, 1, 2)),
        ];
        for (t, want) in cases {
            assert_eq!(marked_at(&apply_transform(marked(), t)), want, "{t:?}");
        }
    }
}
