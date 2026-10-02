//! The edited image at full resolution: the base plus every shape,
//! drawn with tiny-skia from the same geometry as the canvas.

use std::path::{Path, PathBuf};

use image::RgbaImage;
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};

use crate::editor::shape::{Prim, Shape, geometry};

pub fn render(base: &RgbaImage, shapes: &[Shape]) -> RgbaImage {
    let (w, h) = base.dimensions();
    let mut pixmap = Pixmap::new(w, h).expect("the image has a size");
    for (dst, src) in pixmap.pixels_mut().iter_mut().zip(base.pixels()) {
        let [r, g, b, a] = src.0;
        *dst = tiny_skia::ColorU8::from_rgba(r, g, b, a).premultiply();
    }
    for prim in shapes.iter().flat_map(geometry) {
        draw(&mut pixmap, &prim);
    }
    let mut out = RgbaImage::new(w, h);
    for (dst, src) in out.pixels_mut().zip(pixmap.pixels()) {
        let c = src.demultiply();
        dst.0 = [c.red(), c.green(), c.blue(), c.alpha()];
    }
    out
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
    let mut paint = Paint::default();
    paint.set_color_rgba8(color[0], color[1], color[2], color[3]);
    paint.anti_alias = true;
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
    use image::Rgba;

    fn line(tool: Tool, color: [u8; 3]) -> Shape {
        Shape {
            tool,
            style: Style { color, width: 4.0 },
            points: vec![(10.0, 20.0), (90.0, 20.0)],
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
}
