//! A selection over several outputs as one image.

use image::{RgbaImage, imageops};

use crate::frame::Frame;
use crate::selection::Span;

/// The whole selection: each output's part from its frozen frame at its
/// place (resized when its output's scale is smaller), transparent where
/// no output is.
pub fn stitch(frames: &[Frame], span: &Span) -> RgbaImage {
    let mut image = RgbaImage::new(span.size.0, span.size.1);
    for part in &span.parts {
        let mut crop = frames[part.output].to_rgba(part.rect);
        if crop.dimensions() != part.size {
            crop = imageops::resize(
                &crop,
                part.size.0,
                part.size.1,
                imageops::FilterType::Triangle,
            );
        }
        imageops::replace(&mut image, &crop, part.at.0 as i64, part.at.1 as i64);
    }
    image
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use image::RgbaImage;

    use super::*;
    use crate::frame::{Frame, OutputGeom, PixelRect};
    use crate::selection::{Part, Span};

    /// A frame of one colour, given as R, G, B (stored as B, G, R, X).
    fn frame(w: u32, h: u32, rgb: [u8; 3]) -> Frame {
        Frame {
            output: OutputGeom {
                name: "X".into(),
                x: 0,
                y: 0,
                width: w as i32,
                height: h as i32,
            },
            pixels: Arc::new(RgbaImage::from_pixel(
                w,
                h,
                image::Rgba([rgb[2], rgb[1], rgb[0], 0]),
            )),
        }
    }

    fn rect(x: u32, y: u32, width: u32, height: u32) -> PixelRect {
        PixelRect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn parts_land_in_place_and_gaps_stay_clear() {
        let frames = [frame(10, 10, [255, 0, 0]), frame(10, 10, [0, 0, 255])];
        let span = Span {
            size: (8, 6),
            parts: vec![
                Part {
                    output: 0,
                    rect: rect(0, 0, 4, 3),
                    at: (4, 3),
                    size: (4, 3),
                },
                Part {
                    output: 1,
                    rect: rect(2, 2, 8, 3),
                    at: (0, 0),
                    size: (8, 3),
                },
            ],
        };
        let img = stitch(&frames, &span);
        assert_eq!(img.dimensions(), (8, 6));
        assert_eq!(
            img.get_pixel(0, 0).0,
            [0, 0, 255, 255],
            "the monitor's part, opaque"
        );
        assert_eq!(img.get_pixel(7, 2).0, [0, 0, 255, 255]);
        assert_eq!(img.get_pixel(5, 4).0, [255, 0, 0, 255], "the laptop's part");
        assert_eq!(img.get_pixel(1, 4).0[3], 0, "no output there: transparent");
    }

    #[test]
    fn a_smaller_scale_part_is_resized_to_its_place() {
        let frames = [frame(4, 4, [0, 255, 0])];
        let span = Span {
            size: (8, 8),
            parts: vec![Part {
                output: 0,
                rect: rect(0, 0, 4, 4),
                at: (0, 0),
                size: (8, 8),
            }],
        };
        let img = stitch(&frames, &span);
        assert_eq!(
            img.get_pixel(7, 7).0,
            [0, 255, 0, 255],
            "filled to the corner"
        );
    }
}
