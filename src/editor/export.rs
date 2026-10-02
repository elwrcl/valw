//! The edited image at full resolution: the base plus every shape,
//! drawn with tiny-skia from the same geometry as the canvas.

use std::path::{Path, PathBuf};

use image::RgbaImage;
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};

use crate::editor::shape::{
    P, Prim, Shape, Tool, block_size, geometry, number_diameter, text_size,
};
use crate::editor::text;
use crate::frame::PixelRect;

pub fn render<'a>(base: &RgbaImage, shapes: impl IntoIterator<Item = &'a Shape>) -> RgbaImage {
    let (w, h) = base.dimensions();
    let mut pixmap = Pixmap::new(w, h).expect("the image has a size");
    for (dst, src) in pixmap.pixels_mut().iter_mut().zip(base.pixels()) {
        let [r, g, b, a] = src.0;
        *dst = tiny_skia::ColorU8::from_rgba(r, g, b, a).premultiply();
    }
    for shape in shapes {
        draw_shape(&mut pixmap, shape);
    }
    let mut out = RgbaImage::new(w, h);
    for (dst, src) in out.pixels_mut().zip(pixmap.pixels()) {
        let c = src.demultiply();
        dst.0 = [c.red(), c.green(), c.blue(), c.alpha()];
    }
    out
}

/// The part of `image` inside the last crop, or all of it.
// The editor calls this from Task 3 of the 5b plan on; that task removes this.
#[allow(dead_code)]
pub fn crop(image: RgbaImage, rect: Option<PixelRect>) -> RgbaImage {
    match rect {
        Some(r) => image::imageops::crop_imm(&image, r.x, r.y, r.width, r.height).to_image(),
        None => image,
    }
}

fn draw_shape(pixmap: &mut Pixmap, shape: &Shape) {
    let [r, g, b] = shape.style.color;
    match shape.tool {
        Tool::Text => draw_text(
            pixmap,
            &shape.text,
            text_size(shape.style.width),
            shape.points[0],
            [r, g, b, 255],
        ),
        Tool::Number => draw_number(pixmap, shape, [r, g, b, 255]),
        Tool::Pixelate => pixelate(
            pixmap,
            shape.points[0],
            shape.points[1],
            block_size(shape.style.width),
        ),
        _ => {
            for prim in geometry(shape) {
                draw(pixmap, &prim);
            }
        }
    }
}

fn paint(c: [u8; 4]) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(c[0], c[1], c[2], c[3]);
    paint.anti_alias = true;
    paint
}

/// Near black at 80 %: keeps text readable on any background.
const OUTLINE: [u8; 4] = [0x1c, 0x1c, 0x1e, 204];

/// `text` with its line box at `origin`, over a dark outline.
fn draw_text(pixmap: &mut Pixmap, text: &str, size: f32, origin: P, color: [u8; 4]) {
    let Some(path) = text::path(text, size, origin) else {
        return;
    };
    let stroke = Stroke {
        width: size / 8.0,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    pixmap.stroke_path(&path, &paint(OUTLINE), &stroke, Transform::identity(), None);
    pixmap.fill_path(
        &path,
        &paint(color),
        FillRule::Winding,
        Transform::identity(),
        None,
    );
}

/// A filled circle with the number in white, centred on its ink.
fn draw_number(pixmap: &mut Pixmap, shape: &Shape, color: [u8; 4]) {
    let d = number_diameter(shape.style.width);
    let (cx, cy) = shape.points[0];
    if let Some(circle) = PathBuilder::from_circle(cx, cy, d / 2.0) {
        pixmap.fill_path(
            &circle,
            &paint(color),
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
    let Some(glyphs) = text::path(&shape.text, d * 0.55, (0.0, 0.0)) else {
        return;
    };
    let b = glyphs.bounds();
    let centre = Transform::from_translate(
        cx - (b.left() + b.right()) / 2.0,
        cy - (b.top() + b.bottom()) / 2.0,
    );
    if let Some(glyphs) = glyphs.transform(centre) {
        pixmap.fill_path(
            &glyphs,
            &paint([255, 255, 255, 255]),
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

/// Replaces each `block`-sized square of the rectangle from `a` to `b`
/// (aligned to its top-left, clipped to the image) with its average.
fn pixelate(pixmap: &mut Pixmap, a: P, b: P, block: u32) {
    let (w, h) = (pixmap.width(), pixmap.height());
    let clamp = |v: f32, max: u32| v.round().clamp(0.0, max as f32) as u32;
    let (x0, x1) = (clamp(a.0.min(b.0), w), clamp(a.0.max(b.0), w));
    let (y0, y1) = (clamp(a.1.min(b.1), h), clamp(a.1.max(b.1), h));
    let pixels = pixmap.pixels_mut();
    for by in (y0..y1).step_by(block as usize) {
        for bx in (x0..x1).step_by(block as usize) {
            let (ex, ey) = ((bx + block).min(x1), (by + block).min(y1));
            let mut sum = [0u32; 4];
            for y in by..ey {
                for x in bx..ex {
                    let p = pixels[(y * w + x) as usize];
                    for (s, v) in sum
                        .iter_mut()
                        .zip([p.red(), p.green(), p.blue(), p.alpha()])
                    {
                        *s += u32::from(v);
                    }
                }
            }
            let n = (ex - bx) * (ey - by);
            let [r, g, bl, al] = sum.map(|s| ((s + n / 2) / n) as u8);
            // Averages of premultiplied channels stay at or below alpha.
            let avg =
                tiny_skia::PremultipliedColorU8::from_rgba(r.min(al), g.min(al), bl.min(al), al)
                    .expect("channels are clamped to alpha");
            for y in by..ey {
                for x in bx..ex {
                    pixels[(y * w + x) as usize] = avg;
                }
            }
        }
    }
}

fn draw(pixmap: &mut Pixmap, prim: &Prim) {
    let (points, color) = match prim {
        Prim::Stroke { points, color, .. } | Prim::Fill { points, color } => (points, color),
    };
    let mut pb = PathBuilder::new();
    pb.move_to(points[0].0, points[0].1);
    for p in &points[1..] {
        pb.line_to(p.0, p.1);
    }
    let closed = matches!(prim, Prim::Fill { .. } | Prim::Stroke { closed: true, .. });
    if closed {
        pb.close();
    }
    let Some(path) = pb.finish() else { return };
    let paint = paint(*color);
    match prim {
        Prim::Fill { .. } => pixmap.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        ),
        Prim::Stroke { width, round, .. } => {
            let stroke = Stroke {
                width: *width,
                line_cap: if *round {
                    LineCap::Round
                } else {
                    LineCap::Butt
                },
                line_join: if *round {
                    LineJoin::Round
                } else {
                    LineJoin::Miter
                },
                ..Stroke::default()
            };
            pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    }
}

/// `<stem> edited.png` next to `path`, numbered if taken.
pub fn edited_path(path: &Path) -> PathBuf {
    let dir = path.parent().unwrap_or(Path::new("."));
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy())
        .unwrap_or_default();
    crate::output::unique_path(dir, &format!("{stem} edited.png"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::shape::{Style, Tool};
    use crate::frame::PixelRect;
    use image::Rgba;

    fn line(tool: Tool, color: [u8; 3]) -> Shape {
        Shape {
            tool,
            style: Style { color, width: 4.0 },
            points: vec![(10.0, 20.0), (90.0, 20.0)],
            text: String::new(),
        }
    }

    #[test]
    fn line_pixels() {
        let base = RgbaImage::from_pixel(100, 40, Rgba([255, 255, 255, 255]));
        let out = render(&base, &[line(Tool::Line, [255, 0, 0])]);
        assert_eq!(out.dimensions(), (100, 40));
        assert_eq!(out.get_pixel(50, 20).0, [255, 0, 0, 255], "on the line");
        assert_eq!(out.get_pixel(50, 5).0, [255, 255, 255, 255], "away from it");
        assert_eq!(
            out.get_pixel(5, 20).0,
            [255, 255, 255, 255],
            "before its start cap"
        );
    }

    #[test]
    fn rectangles_have_sharp_corners() {
        let base = RgbaImage::from_pixel(60, 40, Rgba([255, 255, 255, 255]));
        let rect = Shape {
            tool: Tool::Rectangle,
            style: Style {
                color: [255, 0, 0],
                width: 8.0,
            },
            points: vec![(10.0, 10.0), (50.0, 30.0)],
            text: String::new(),
        };
        let out = render(&base, &[rect]);
        // A round join would leave the outer corner (6..7, 6..7) white.
        assert_eq!(out.get_pixel(6, 6).0, [255, 0, 0, 255]);
    }

    #[test]
    fn highlighter_blends_at_40_percent() {
        let base = RgbaImage::from_pixel(100, 40, Rgba([0, 0, 255, 255]));
        let out = render(&base, &[line(Tool::Highlighter, [255, 255, 0])]);
        let [r, g, b, a] = out.get_pixel(50, 20).0;
        assert_eq!(a, 255);
        for (got, want) in [(r, 102), (g, 102), (b, 153)] {
            assert!(
                (got as i32 - want).abs() <= 2,
                "{:?}",
                out.get_pixel(50, 20)
            );
        }
    }

    #[test]
    fn keeps_transparency() {
        let base = RgbaImage::from_pixel(100, 40, Rgba([10, 20, 30, 0]));
        let mut half = base.clone();
        half.put_pixel(50, 5, Rgba([200, 100, 50, 128]));
        let out = render(&half, &[line(Tool::Line, [255, 0, 0])]);
        assert_eq!(out.get_pixel(0, 0).0[3], 0, "transparent stays transparent");
        let [r, g, b, a] = out.get_pixel(50, 5).0;
        assert_eq!(a, 128);
        assert!(
            (r as i32 - 200).abs() <= 2
                && (g as i32 - 100).abs() <= 2
                && (b as i32 - 50).abs() <= 2
        );
        assert_eq!(out.get_pixel(50, 20).0, [255, 0, 0, 255]);
    }

    #[test]
    fn edited_path_sits_next_to_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Screenshot at 10.00.png");
        assert_eq!(
            edited_path(&file),
            dir.path().join("Screenshot at 10.00 edited.png")
        );
        std::fs::write(dir.path().join("Screenshot at 10.00 edited.png"), b"x").unwrap();
        assert_ne!(
            edited_path(&file),
            dir.path().join("Screenshot at 10.00 edited.png")
        );
    }

    fn white(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba([255, 255, 255, 255]))
    }

    #[test]
    fn text_is_filled_with_the_colour_and_outlined() {
        let t = Shape {
            tool: Tool::Text,
            style: Style {
                color: [0, 122, 255],
                width: 8.0,
            },
            points: vec![(10.0, 10.0)],
            text: "I".into(),
        };
        let out = render(&white(80, 60), &[t]);
        // The middle of the I's stem: x ≈ 10 + stem centre, y in the cap height.
        let blue = out.pixels().filter(|p| p.0 == [0, 122, 255, 255]).count();
        assert!(blue > 40, "{blue} fully blue pixels");
        let dark = out.pixels().filter(|p| p.0[0] < 80 && p.0[2] < 80).count();
        assert!(dark > 10, "the outline: {dark}");
        assert_eq!(out.get_pixel(70, 55).0, [255, 255, 255, 255]);
    }

    #[test]
    fn a_number_is_a_filled_circle_with_a_white_digit() {
        let n = Shape {
            tool: Tool::Number,
            style: Style {
                color: [255, 0, 0],
                width: 8.0,
            },
            points: vec![(30.0, 30.0)],
            text: "1".into(),
        };
        let out = render(&white(60, 60), &[n]);
        assert_eq!(
            out.get_pixel(30, 10).0,
            [255, 0, 0, 255],
            "inside the 44 px circle, above the digit"
        );
        assert_eq!(out.get_pixel(3, 3).0, [255, 255, 255, 255], "outside it");
        let whites_inside = (20..40)
            .flat_map(|x| (20..40).map(move |y| (x, y)))
            .filter(|&(x, y)| out.get_pixel(x, y).0 == [255, 255, 255, 255])
            .count();
        assert!(whites_inside > 10, "the digit: {whites_inside}");
    }

    #[test]
    fn pixelate_averages_blocks_including_earlier_shapes() {
        let base = RgbaImage::from_fn(24, 12, |x, _| {
            if x % 2 == 0 {
                Rgba([0, 0, 0, 255])
            } else {
                Rgba([200, 100, 50, 255])
            }
        });
        let px = Shape {
            tool: Tool::Pixelate,
            style: Style {
                color: [0, 0, 0],
                width: 4.0,
            },
            points: vec![(0.0, 0.0), (24.0, 12.0)],
            text: String::new(),
        };
        let out = render(&base, std::slice::from_ref(&px));
        let first = out.get_pixel(0, 0).0;
        for x in 0..12 {
            for y in 0..12 {
                assert_eq!(out.get_pixel(x, y).0, first);
            }
        }
        for (got, want) in first.iter().zip([100, 50, 25, 255]) {
            assert!((*got as i32 - want).abs() <= 1, "{first:?}");
        }
        let red = Shape {
            tool: Tool::Line,
            style: Style {
                color: [255, 0, 0],
                width: 8.0,
            },
            points: vec![(0.0, 6.0), (24.0, 6.0)],
            text: String::new(),
        };
        let both = render(&base, &[red, px]);
        assert!(
            both.get_pixel(0, 0).0[0] > first[0],
            "the line is pixelated in"
        );
    }

    #[test]
    fn pixelate_stays_inside_the_image() {
        let px = Shape {
            tool: Tool::Pixelate,
            style: Style {
                color: [0, 0, 0],
                width: 4.0,
            },
            points: vec![(-10.0, -10.0), (500.0, 500.0)],
            text: String::new(),
        };
        let out = render(&white(30, 20), &[px]);
        assert_eq!(out.dimensions(), (30, 20));
        assert_eq!(out.get_pixel(29, 19).0, [255, 255, 255, 255]);
    }

    #[test]
    fn crop_cuts_out_the_rectangle() {
        let img = RgbaImage::from_fn(10, 8, |x, y| Rgba([x as u8, y as u8, 0, 255]));
        let rect = PixelRect {
            x: 2,
            y: 3,
            width: 4,
            height: 2,
        };
        let out = crop(img.clone(), Some(rect));
        assert_eq!(out.dimensions(), (4, 2));
        assert_eq!(out.get_pixel(0, 0).0, [2, 3, 0, 255]);
        assert_eq!(crop(img.clone(), None), img);
    }
}
