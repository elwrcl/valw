//! Text for the editor: the bundled DejaVu Sans Bold, laid out with
//! ab_glyph and turned into tiny-skia paths, so canvas and export draw the
//! same glyphs.

use std::sync::LazyLock;

use ab_glyph::{Font, FontRef, GlyphId, OutlineCurve, PxScale, ScaleFont};
use tiny_skia::{Path, PathBuilder};

use crate::editor::shape::P;

static FONT_DATA: &[u8] = include_bytes!("../../assets/fonts/DejaVuSans-Bold.ttf");

pub static FONT: LazyLock<FontRef<'static>> =
    LazyLock::new(|| FontRef::try_from_slice(FONT_DATA).expect("the bundled font is valid"));

pub fn line_height(size: f32) -> f32 {
    size * 1.2
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// Each glyph and its pen position (baseline) relative to the box's top-left.
    pub glyphs: Vec<(GlyphId, P)>,
    pub width: f32,
    pub height: f32,
    /// The width of every line.
    pub lines: Vec<f32>,
}

pub fn layout(text: &str, size: f32) -> Layout {
    if text.is_empty() {
        return Layout {
            glyphs: Vec::new(),
            width: 0.0,
            height: 0.0,
            lines: Vec::new(),
        };
    }
    let font = FONT.as_scaled(PxScale::from(size));
    let mut glyphs = Vec::new();
    let mut lines = Vec::new();
    for (i, line) in text.split('\n').enumerate() {
        let baseline = i as f32 * line_height(size) + font.ascent();
        let mut x = 0.0;
        let mut previous = None;
        for c in line.chars() {
            let id = font.glyph_id(c);
            if let Some(p) = previous {
                x += font.kern(p, id);
            }
            glyphs.push((id, (x, baseline)));
            x += font.h_advance(id);
            previous = Some(id);
        }
        lines.push(x);
    }
    Layout {
        glyphs,
        width: lines.iter().copied().fold(0.0, f32::max),
        height: lines.len() as f32 * line_height(size),
        lines,
    }
}

/// The outlines of `text` with its line box's top-left at `origin`.
pub fn path(text: &str, size: f32, origin: P) -> Option<Path> {
    let layout = layout(text, size);
    let scale = FONT.as_scaled(PxScale::from(size)).scale_factor();
    let mut pb = PathBuilder::new();
    for (id, (gx, gy)) in layout.glyphs {
        let Some(outline) = FONT.outline(id) else {
            continue;
        };
        // Font units, y up; the pen sits on the baseline.
        let at = |p: ab_glyph::Point| {
            (
                origin.0 + gx + p.x * scale.horizontal,
                origin.1 + gy - p.y * scale.vertical,
            )
        };
        let mut end: Option<ab_glyph::Point> = None;
        for curve in &outline.curves {
            let start = match curve {
                OutlineCurve::Line(a, _)
                | OutlineCurve::Quad(a, _, _)
                | OutlineCurve::Cubic(a, _, _, _) => *a,
            };
            if end != Some(start) {
                if end.is_some() {
                    pb.close();
                }
                let (x, y) = at(start);
                pb.move_to(x, y);
            }
            match curve {
                OutlineCurve::Line(_, b) => {
                    let (x, y) = at(*b);
                    pb.line_to(x, y);
                    end = Some(*b);
                }
                OutlineCurve::Quad(_, c, b) => {
                    let ((cx, cy), (x, y)) = (at(*c), at(*b));
                    pb.quad_to(cx, cy, x, y);
                    end = Some(*b);
                }
                OutlineCurve::Cubic(_, c1, c2, b) => {
                    let ((ax, ay), (bx, by), (x, y)) = (at(*c1), at(*c2), at(*b));
                    pb.cubic_to(ax, ay, bx, by, x, y);
                    end = Some(*b);
                }
            }
        }
        if end.is_some() {
            pb.close();
        }
    }
    pb.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_has_no_size() {
        let l = layout("", 24.0);
        assert_eq!((l.width, l.height), (0.0, 0.0));
        assert!(path("", 24.0, (0.0, 0.0)).is_none());
    }

    #[test]
    fn lines_and_widths() {
        let one = layout("Hello", 24.0);
        let longer = layout("Hello!", 24.0);
        assert!(longer.width > one.width && one.width > 0.0);
        assert_eq!(one.height, line_height(24.0));
        let two = layout("Hi\nthere", 24.0);
        assert_eq!(two.height, 2.0 * line_height(24.0));
        assert_eq!(two.lines.len(), 2);
        assert!(two.lines[1] > two.lines[0]);
        assert_eq!(two.width, two.lines[1]);
    }

    #[test]
    fn kerning_is_applied() {
        let font = FONT.as_scaled(PxScale::from(48.0));
        let (a, v) = (FONT.glyph_id('A'), FONT.glyph_id('V'));
        let plain = font.h_advance(a) + font.h_advance(v);
        assert!(font.kern(a, v) < 0.0, "DejaVu kerns AV");
        assert!(layout("AV", 48.0).width < plain);
    }

    #[test]
    fn turkish_letters_have_glyphs() {
        for c in "ğüşıöçĞÜŞİÖÇ".chars() {
            assert_ne!(FONT.glyph_id(c).0, 0, "{c} has no glyph");
        }
    }

    #[test]
    fn the_path_sits_in_the_line_box() {
        let p = path("Hg", 24.0, (100.0, 50.0)).unwrap();
        let b = p.bounds();
        assert!(b.left() >= 100.0 && b.top() >= 50.0, "{b:?}");
        assert!(b.bottom() <= 50.0 + line_height(24.0), "{b:?}");
    }
}
