# valw Phase 5b: editor text, pixelate, numbers, crop Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Text (T), Pixelate (B), Number (N) and Crop (C) in `valw edit`, exported exactly as shown.

**Architecture:** Shapes gain a `text` payload; `Doc` history entries become `Item::{Shape, Crop}`. A new `editor/text.rs` lays out the vendored DejaVu Sans Bold with `ab_glyph` and turns glyph outlines into tiny-skia paths. `export::render` draws text, numbers and pixelation itself (they have no egui-paintable geometry) and `export::crop` cuts the last crop out. The canvas keeps showing the export renderer's output (also for the text being typed); egui only paints in-progress drags, the caret and the crop overlay. `Fit` learns to fit a crop rectangle.

**Tech Stack:** Rust 2024, eframe/egui 0.36.2, tiny-skia 0.12, `ab_glyph` 0.2.32.

**Spec:** `docs/superpowers/specs/2026-10-03-valw-phase-5b-editor-design.md`

## Global Constraints

- jj only; `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo nextest run`, `nix flake check`.
- Sizes per S/M/L (index of `WIDTHS`): text 16/24/36 px, pixel block 8/12/20 px, number diameter 24/32/44 px (number glyphs 0.55 × diameter); line height 1.2 × size; text outline `#1c1c1e` alpha 204, width size/8; crop overlay 50 % black, 1 px white border, 8 screen px edge grab.
- Keys: T text, B pixelate, N number, C crop (Ctrl+C stays Copy); while typing, letters are text and Ctrl shortcuts are ignored; Esc cancels typing, then a pending crop, then starts the close flow; Enter finishes typing or applies a pending crop.
- Tell the user before opening editor windows on the live session.

## Review Focus

1. **Turkish text and kerning** survive export (no notdef boxes). Test: `editor::text::tests::turkish_letters_have_glyphs` (Task 1).
2. **Typing doesn't trigger shortcuts** (T, A, Ctrl+Z, Esc closing the window). Real check in the checklist; key routing in Task 4.
3. **Pixelate covers earlier shapes and stays inside the image** at edges. Tests: `editor::export::tests::pixelate_*` (Task 2).
4. **Crop + undo/redo + dirty** and the export size/offset. Tests: `editor::doc::tests::crops_*`, `editor::export::tests::crop_cuts_out_the_rectangle` (Tasks 2–3).
5. **Drawing after a crop** lands at the right image pixels. Test: `editor::view::tests::round_trip_with_a_crop` (Task 3).

---

## File Structure

| File | Change | Task |
|---|---|---|
| `assets/fonts/DejaVuSans-Bold.ttf`, `assets/fonts/LICENSE-DejaVu` | vendored font | 1 |
| `src/editor/text.rs` | `layout`, `path`, `FONT` | 1 |
| `src/editor/shape.rs` | new tools, `text` field, size tables | 2 |
| `src/editor/export.rs` | text, numbers, pixelate, `crop` | 2 |
| `src/editor/doc.rs` | `Item`, crops, `next_number` | 3 |
| `src/editor/view.rs` | `Fit::with_crop` | 3 |
| `src/editor/mod.rs` | typing, numbers, crop UI, keys, toolbar | 4 |
| `docs/test-checklist.md` | Phase 5b section | 5 |

---

### Task 1: Font and text layout

**Files:** create `assets/fonts/DejaVuSans-Bold.ttf` (from `nixpkgs#dejavu_fonts`, `share/fonts/truetype/DejaVuSans-Bold.ttf`) and `assets/fonts/LICENSE-DejaVu` (upstream `LICENSE`); `Cargo.toml`: `ab_glyph = "0.2.32"`; `src/editor/mod.rs`: `pub mod text;`; create `src/editor/text.rs`.

**Produces:** `text::layout(text: &str, size: f32) -> Layout { glyphs: Vec<(GlyphId, P)>, width: f32, height: f32, lines: Vec<f32> /* width of each line */ }`; `text::path(text: &str, size: f32, origin: P) -> Option<tiny_skia::Path>` (origin = top-left of the first line box); `text::line_height(size) -> f32`.

- [ ] **Step 1: Failing tests** — `src/editor/text.rs`:

```rust
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
```

Run `cargo nextest run editor::text` → compile errors.

- [ ] **Step 2: Implement** (above the tests)

```rust
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
        return Layout { glyphs: Vec::new(), width: 0.0, height: 0.0, lines: Vec::new() };
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
        let Some(outline) = FONT.outline(id) else { continue };
        // Font units, y up; the pen sits on the baseline.
        let at = |p: ab_glyph::Point| {
            (origin.0 + gx + p.x * scale.horizontal, origin.1 + gy - p.y * scale.vertical)
        };
        let mut end: Option<ab_glyph::Point> = None;
        for curve in &outline.curves {
            let start = match curve {
                OutlineCurve::Line(a, _) | OutlineCurve::Quad(a, _, _) | OutlineCurve::Cubic(a, _, _, _) => *a,
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
```

(If ab_glyph's outline y axis turns out to point down, `the_path_sits_in_the_line_box` fails: flip the sign and ledger it.)

- [ ] **Step 3: Run** (+5), clippy, fmt. **Step 4: Commit** `feat: bundled font and text layout for the editor`.

---

### Task 2: New tools in shapes and export

**Files:** `src/editor/shape.rs`, `src/editor/export.rs`.

**Produces:** `Tool::{Text, Pixelate, Number, Crop}` (in `Tool::ALL` after Highlighter; labels "Text", "Pixelate", "Number", "Crop"); `Shape { tool, style, points, text: String }`; `shape::{size_index, text_size, block_size, number_diameter}` (all from `Style::width`); `export::render<'a>(base, shapes: impl IntoIterator<Item = &'a Shape>) -> RgbaImage`; `export::crop(image: RgbaImage, rect: Option<PixelRect>) -> RgbaImage`.

Shape semantics: Text: `points = [top-left]`, `text` = the text. Number: `points = [centre]`, `text` = its number. Pixelate: `points = [start, end]` (Shift: square, like Rectangle). Crop never becomes a `Shape` (it is a `Doc` item, Task 3); `Drag` is never created for Text, Number or Crop. `geometry()` returns nothing for Text and Number, and for Pixelate a 2 px closed white outline (alpha 200, butt) used only while dragging. `is_click()` is false for Text and Number.

- [ ] **Step 1: Failing tests**

Add `text: String::new()` to every `Shape { .. }` literal in existing tests (shape, doc, export tests). Then in `shape.rs` tests:

```rust
    #[test]
    fn sizes_follow_s_m_l() {
        let at = |w: f32| Style { color: [0, 0, 0], width: w };
        assert_eq!([2.0, 4.0, 8.0].map(|w| text_size(at(w).width)), [16.0, 24.0, 36.0]);
        assert_eq!([2.0, 4.0, 8.0].map(|w| block_size(at(w).width)), [8, 12, 20]);
        assert_eq!([2.0, 4.0, 8.0].map(|w| number_diameter(at(w).width)), [24.0, 32.0, 44.0]);
    }

    #[test]
    fn pixelate_drags_like_a_rectangle() {
        let mut d = Drag::new(Tool::Pixelate, RED, (0.0, 0.0));
        d.move_to((30.0, 10.0));
        assert_eq!(d.shape(true).points, vec![(0.0, 0.0), (30.0, 30.0)]);
        let g = geometry(&d.shape(false));
        let Prim::Stroke { color, closed, .. } = &g[0] else { panic!("{g:?}") };
        assert!(*closed && *color == [255, 255, 255, 200]);
    }

    #[test]
    fn text_and_numbers_are_never_clicks_and_have_no_egui_geometry() {
        for tool in [Tool::Text, Tool::Number] {
            let s = Shape { tool, style: RED, points: vec![(5.0, 5.0)], text: "1".into() };
            assert!(!s.is_click());
            assert!(geometry(&s).is_empty());
        }
    }
```

`export.rs` tests:

```rust
    fn white(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba([255, 255, 255, 255]))
    }

    #[test]
    fn text_is_filled_with_the_colour_and_outlined() {
        let t = Shape {
            tool: Tool::Text,
            style: Style { color: [0, 122, 255], width: 8.0 },
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
            style: Style { color: [255, 0, 0], width: 8.0 },
            points: vec![(30.0, 30.0)],
            text: "1".into(),
        };
        let out = render(&white(60, 60), &[n]);
        assert_eq!(out.get_pixel(30, 10).0, [255, 0, 0, 255], "inside the 44 px circle, above the digit");
        assert_eq!(out.get_pixel(3, 3).0, [255, 255, 255, 255], "outside it");
        let whites_inside = (20..40).flat_map(|x| (20..40).map(move |y| (x, y)))
            .filter(|&(x, y)| out.get_pixel(x, y).0 == [255, 255, 255, 255])
            .count();
        assert!(whites_inside > 10, "the digit: {whites_inside}");
    }

    #[test]
    fn pixelate_averages_blocks_including_earlier_shapes() {
        let base = RgbaImage::from_fn(24, 12, |x, _| if x % 2 == 0 { Rgba([0, 0, 0, 255]) } else { Rgba([200, 100, 50, 255]) });
        let px = Shape {
            tool: Tool::Pixelate,
            style: Style { color: [0, 0, 0], width: 4.0 },
            points: vec![(0.0, 0.0), (24.0, 12.0)],
            text: String::new(),
        };
        let out = render(&base, &[px.clone()]);
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
            style: Style { color: [255, 0, 0], width: 8.0 },
            points: vec![(0.0, 6.0), (24.0, 6.0)],
            text: String::new(),
        };
        let both = render(&base, &[red, px]);
        assert!(both.get_pixel(0, 0).0[0] > first[0], "the line is pixelated in");
    }

    #[test]
    fn pixelate_stays_inside_the_image() {
        let px = Shape {
            tool: Tool::Pixelate,
            style: Style { color: [0, 0, 0], width: 4.0 },
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
        let rect = PixelRect { x: 2, y: 3, width: 4, height: 2 };
        let out = crop(img.clone(), Some(rect));
        assert_eq!(out.dimensions(), (4, 2));
        assert_eq!(out.get_pixel(0, 0).0, [2, 3, 0, 255]);
        assert_eq!(crop(img.clone(), None), img);
    }
```

(Add `use crate::frame::PixelRect;` in the test module if not covered by `super::*`.)

Run → compile errors.

- [ ] **Step 2: Implement `shape.rs` changes**

- `Tool` gains `Text, Pixelate, Number, Crop`; `ALL` becomes 10 entries; labels as above.
- `Shape` gains `pub text: String`; `Drag::shape` sets `text: String::new()`.
- `constrain`: `Tool::Rectangle | Tool::Ellipse | Tool::Pixelate` share the square branch; `Tool::Pen | Tool::Highlighter | Tool::Text | Tool::Number | Tool::Crop => end`.
- `is_click`: `if matches!(self.tool, Tool::Pen | Tool::Text | Tool::Number) { return false; }`.
- Size tables:

```rust
/// 0, 1, 2 for S, M, L.
pub fn size_index(width: f32) -> usize {
    WIDTHS.iter().position(|w| w.1 == width).unwrap_or(1)
}

pub fn text_size(width: f32) -> f32 {
    [16.0, 24.0, 36.0][size_index(width)]
}

pub fn block_size(width: f32) -> u32 {
    [8, 12, 20][size_index(width)]
}

pub fn number_diameter(width: f32) -> f32 {
    [24.0, 32.0, 44.0][size_index(width)]
}
```

- `geometry`: `Tool::Text | Tool::Number | Tool::Crop => Vec::new()`, and

```rust
        Tool::Pixelate => {
            let ((x0, y0), (x1, y1)) = (pts[0], pts[1]);
            vec![Prim::Stroke {
                points: vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)],
                width: 2.0,
                color: [255, 255, 255, 200],
                closed: true,
                round: false,
            }]
        }
```

- [ ] **Step 3: Implement `export.rs` changes**

`render` takes `shapes: impl IntoIterator<Item = &'a Shape>` and draws each shape with:

```rust
fn draw_shape(pixmap: &mut Pixmap, shape: &Shape) {
    let [r, g, b] = shape.style.color;
    match shape.tool {
        Tool::Text => draw_text(pixmap, &shape.text, text_size(shape.style.width), shape.points[0], [r, g, b, 255], true),
        Tool::Number => {
            let d = number_diameter(shape.style.width);
            let (cx, cy) = shape.points[0];
            if let Some(circle) = PathBuilder::from_circle(cx, cy, d / 2.0) {
                pixmap.fill_path(&circle, &paint([r, g, b, 255]), FillRule::Winding, Transform::identity(), None);
            }
            let size = d * 0.55;
            if let Some(glyphs) = text::path(&shape.text, size, (0.0, 0.0)) {
                // Centre the digits' ink, not their line box.
                let b = glyphs.bounds();
                let shift = Transform::from_translate(cx - (b.left() + b.right()) / 2.0, cy - (b.top() + b.bottom()) / 2.0);
                if let Some(glyphs) = glyphs.transform(shift) {
                    pixmap.fill_path(&glyphs, &paint([255, 255, 255, 255]), FillRule::Winding, Transform::identity(), None);
                }
            }
        }
        Tool::Pixelate => pixelate(pixmap, shape.points[0], shape.points[1], block_size(shape.style.width)),
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

const OUTLINE: [u8; 4] = [0x1c, 0x1c, 0x1e, 204];

/// `text` with its line box at `origin`; `outlined` adds the dark outline
/// that keeps it readable on any background.
fn draw_text(pixmap: &mut Pixmap, text: &str, size: f32, origin: P, color: [u8; 4], outlined: bool) {
    let Some(path) = text::path(text, size, origin) else { return };
    if outlined {
        let stroke = Stroke { width: size / 8.0, line_join: LineJoin::Round, ..Stroke::default() };
        pixmap.stroke_path(&path, &paint(OUTLINE), &stroke, Transform::identity(), None);
    }
    pixmap.fill_path(&path, &paint(color), FillRule::Winding, Transform::identity(), None);
}

/// Replaces each `block`-sized square of the rectangle (aligned to its
/// top-left, clipped to the image) with its average colour.
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
                    for (s, v) in sum.iter_mut().zip([p.red(), p.green(), p.blue(), p.alpha()]) {
                        *s += v as u32;
                    }
                }
            }
            let n = (ex - bx) * (ey - by);
            let [r, g, bl, al] = sum.map(|s| ((s + n / 2) / n) as u8);
            // Premultiplied channels average to valid premultiplied values.
            let avg = tiny_skia::PremultipliedColorU8::from_rgba(r.min(al), g.min(al), bl.min(al), al)
                .expect("channels never exceed alpha");
            for y in by..ey {
                for x in bx..ex {
                    pixels[(y * w + x) as usize] = avg;
                }
            }
        }
    }
}

/// The part of `image` inside the last crop, or all of it.
pub fn crop(image: RgbaImage, rect: Option<PixelRect>) -> RgbaImage {
    match rect {
        Some(r) => image::imageops::crop_imm(&image, r.x, r.y, r.width, r.height).to_image(),
        None => image,
    }
}
```

`draw` (the existing primitive painter) keeps using its own `Paint`; switch it to `paint(*color)` to share code. Imports: `crate::editor::shape::{P, Tool, block_size, number_diameter, text_size}`, `crate::editor::text`, `crate::frame::PixelRect`.

- [ ] **Step 4: Run** (all pass, +8), clippy, fmt. **Step 5: Commit** `feat: text, numbers and pixelate in the editor export`.

---

### Task 3: History items, crops and fitting

**Files:** `src/editor/doc.rs`, `src/editor/view.rs`, `src/editor/mod.rs` (call sites only).

**Produces:** `doc::Item::{Shape(Shape), Crop(PixelRect)}`; `Doc::{add_crop(PixelRect), crop() -> Option<PixelRect>, shapes() -> impl Iterator<Item = &Shape>, next_number() -> u32}` (the rest unchanged); `view::Fit::with_crop(crop: PixelRect, area, pixels_per_point) -> Fit` (and `Fit::new` = `with_crop` over the whole image); `Fit` gains `origin: P` (the crop's top-left in image px), used by `to_screen`/`to_image`.

- [ ] **Step 1: Failing tests**

`doc.rs` (the existing tests use `d.shapes()` as a slice: change those asserts to `d.shapes().cloned().collect::<Vec<_>>()`):

```rust
    fn number(n: &str) -> Shape {
        Shape { tool: Tool::Number, style: Style { color: [0, 0, 0], width: 4.0 }, points: vec![(1.0, 1.0)], text: n.into() }
    }

    const R: PixelRect = PixelRect { x: 1, y: 2, width: 30, height: 20 };

    #[test]
    fn numbers_count_only_numbers_and_follow_undo() {
        let mut d = Doc::new();
        assert_eq!(d.next_number(), 1);
        d.add(number("1"));
        d.add(line(5.0));
        d.add(number("2"));
        assert_eq!(d.next_number(), 3);
        d.undo();
        assert_eq!(d.next_number(), 2);
    }

    #[test]
    fn crops_are_undoable_and_nest() {
        let mut d = Doc::new();
        assert_eq!(d.crop(), None);
        d.add_crop(R);
        let inner = PixelRect { x: 5, y: 5, width: 10, height: 10 };
        d.add_crop(inner);
        assert_eq!(d.crop(), Some(inner));
        d.undo();
        assert_eq!(d.crop(), Some(R));
        d.undo();
        assert_eq!(d.crop(), None);
        d.redo();
        assert_eq!(d.crop(), Some(R));
    }

    #[test]
    fn crops_make_the_doc_dirty_and_skip_shapes() {
        let mut d = Doc::new();
        d.add(line(1.0));
        d.mark_saved();
        d.add_crop(R);
        assert!(d.is_dirty());
        assert_eq!(d.shapes().count(), 1);
    }
```

`view.rs`:

```rust
    #[test]
    fn a_crop_fills_the_area() {
        let crop = PixelRect { x: 100, y: 50, width: 200, height: 100 };
        let f = Fit::with_crop(crop, (0.0, 0.0, 400.0, 400.0), 1.0);
        assert_eq!(f.scale, 1.0);
        assert_eq!(f.to_screen((100.0, 50.0)), (100.0, 150.0), "the crop's corner is centred");
    }

    #[test]
    fn round_trip_with_a_crop() {
        let crop = PixelRect { x: 640, y: 360, width: 640, height: 360 };
        let f = Fit::with_crop(crop, (0.0, 30.0, 500.0, 400.0), 1.0);
        for p in [(640.0, 360.0), (1279.0, 719.0), (800.5, 400.25)] {
            let q = f.to_image(f.to_screen(p));
            assert!((q.0 - p.0).abs() < 1e-3 && (q.1 - p.1).abs() < 1e-3, "{p:?} {q:?}");
        }
    }
```

Run → compile errors.

- [ ] **Step 2: Implement `doc.rs`**

Replace the three `Vec<Shape>` fields with `Vec<Item>`:

```rust
use crate::editor::shape::{Shape, Tool};
use crate::frame::PixelRect;

/// One step of the history.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Shape(Shape),
    /// Show and export only this part of the image (image pixels).
    Crop(PixelRect),
}

#[derive(Debug, Default)]
pub struct Doc {
    items: Vec<Item>,
    redo: Vec<Item>,
    saved: Vec<Item>,
    revision: u64,
}
```

`add(shape)` pushes `Item::Shape`; `add_crop(rect)` pushes `Item::Crop` (both through a private `push(item)` that clears redo and bumps the revision); `undo`/`redo` move `Item`s; `can_undo` = items non-empty; `is_dirty` = `items != saved`; `mark_saved` clones items; and

```rust
    pub fn shapes(&self) -> impl Iterator<Item = &Shape> {
        self.items.iter().filter_map(|i| match i {
            Item::Shape(s) => Some(s),
            Item::Crop(_) => None,
        })
    }

    /// The last crop, if any.
    pub fn crop(&self) -> Option<PixelRect> {
        self.items.iter().rev().find_map(|i| match i {
            Item::Crop(r) => Some(*r),
            Item::Shape(_) => None,
        })
    }

    pub fn next_number(&self) -> u32 {
        1 + self.shapes().filter(|s| s.tool == Tool::Number).count() as u32
    }
```

- [ ] **Step 3: Implement `view.rs`**

```rust
use crate::frame::PixelRect;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    /// Points per image pixel.
    pub scale: f32,
    /// Where `origin` is drawn, in points.
    pub offset: P,
    /// The image pixel at the top-left of what is shown (the crop's corner).
    pub origin: P,
}

impl Fit {
    pub fn new(image: (u32, u32), area: (f32, f32, f32, f32), pixels_per_point: f32) -> Fit {
        Fit::with_crop(PixelRect { x: 0, y: 0, width: image.0, height: image.1 }, area, pixels_per_point)
    }

    /// Fits `crop` into `area` (x, y, w, h in points), centred, never above
    /// 100 % (one image pixel per physical pixel).
    pub fn with_crop(crop: PixelRect, area: (f32, f32, f32, f32), pixels_per_point: f32) -> Fit {
        // ... the existing body with iw/ih = crop.width/height ...
        Fit { scale, offset: (snap(..), snap(..)), origin: (crop.x as f32, crop.y as f32) }
    }

    pub fn to_screen(self, p: P) -> P {
        (self.offset.0 + (p.0 - self.origin.0) * self.scale, self.offset.1 + (p.1 - self.origin.1) * self.scale)
    }

    pub fn to_image(self, p: P) -> P {
        ((p.0 - self.offset.0) / self.scale + self.origin.0, (p.1 - self.offset.1) / self.scale + self.origin.1)
    }
}

/// `p` moved into `rect`.
pub fn clamp(p: P, rect: PixelRect) -> P {
    (
        p.0.clamp(rect.x as f32, (rect.x + rect.width) as f32),
        p.1.clamp(rect.y as f32, (rect.y + rect.height) as f32),
    )
}
```

Update `clamp_keeps_points_on_the_image` to pass a `PixelRect { x: 0, y: 0, width: 100, height: 80 }`.

- [ ] **Step 4: Call sites in `mod.rs`** (minimal, so it compiles; the UI comes in Task 4): `self.doc.shapes()` is now an iterator (`export::render` takes iterators); `png()` = `export::crop(export::render(&self.base, self.doc.shapes()), self.doc.crop())`; the canvas uses `let shown = self.doc.crop().unwrap_or(full)`, `Fit::with_crop(shown, …)`, the image rect from `to_pos` of the crop's corners with UVs `crop / image size`, and clamps the drag to `shown`.

- [ ] **Step 5: Run** (all pass, +5), clippy, fmt. **Step 6: Commit** `feat: crop history and fitting in the editor`.

---

### Task 4: The editor UI for the new tools

**Files:** `src/editor/mod.rs`.

**Produces:** private `Typing { at: P, text: String }`, `CropEdit { rect: (f32, f32, f32, f32), grab: Option<Edges> }`, `Edges { left, right, top, bottom: bool }`, `fn crop_edges(rect_screen: (f32, f32, f32, f32), p: P, tolerance: f32) -> Option<Edges>`.

- [ ] **Step 1: Failing test** — in `mod.rs` tests:

```rust
    #[test]
    fn crop_edges_near_edges_and_corners() {
        let r = (100.0, 100.0, 300.0, 200.0);
        let e = |l, r_, t, b| Some(Edges { left: l, right: r_, top: t, bottom: b });
        assert_eq!(crop_edges(r, (102.0, 150.0), 8.0), e(true, false, false, false));
        assert_eq!(crop_edges(r, (295.0, 150.0), 8.0), e(false, true, false, false));
        assert_eq!(crop_edges(r, (200.0, 205.0), 8.0), e(false, false, false, true));
        assert_eq!(crop_edges(r, (99.0, 101.0), 8.0), e(true, false, true, false));
        assert_eq!(crop_edges(r, (200.0, 150.0), 8.0), None, "inside");
        assert_eq!(crop_edges(r, (50.0, 150.0), 8.0), None, "outside");
    }
```

Run → compile error.

- [ ] **Step 2: Implement**

```rust
/// Which edges of the crop rectangle a press grabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Edges {
    left: bool,
    right: bool,
    top: bool,
    bottom: bool,
}

/// The edges of `r` (x0, y0, x1, y1 on screen) within `tolerance` of `p`;
/// `None` inside, outside, or far from every edge.
fn crop_edges(r: (f32, f32, f32, f32), p: P, tolerance: f32) -> Option<Edges> {
    let (x0, y0, x1, y1) = r;
    let within_x = p.0 > x0 - tolerance && p.0 < x1 + tolerance;
    let within_y = p.1 > y0 - tolerance && p.1 < y1 + tolerance;
    let e = Edges {
        left: within_y && (p.0 - x0).abs() <= tolerance,
        right: within_y && (p.0 - x1).abs() <= tolerance,
        top: within_x && (p.1 - y0).abs() <= tolerance,
        bottom: within_x && (p.1 - y1).abs() <= tolerance,
    };
    (e.left || e.right || e.top || e.bottom).then_some(e)
}
```

Editor fields: `typing: Option<Typing>`, `cropping: Option<CropEdit>` (rect in image px as x0, y0, x1, y1, unnormalised while dragging), plus the composite key becomes `shown: (u64, Option<String>)` — the revision and the typed text — so the texture re-renders while typing.

Behaviour, in `keys()` before everything else:

```rust
        if self.typing.is_some() {
            self.type_keys(ctx);
            return;
        }
        if pressed(Modifiers::NONE, Key::Enter) && self.cropping.is_some() {
            self.apply_crop();
        }
        // Esc: a pending crop first, then the close flow.
        if self.cropping.is_some() && pressed(Modifiers::NONE, Key::Escape) {
            self.cropping = None;
        }
```

and the tool-key table gains `(Key::T, Tool::Text), (Key::B, Tool::Pixelate), (Key::N, Tool::Number), (Key::C, Tool::Crop)` (after the Ctrl+C check, which consumes Ctrl+C first). Changing tool (keys or toolbar) goes through `fn set_tool(&mut self, tool)` which finishes typing and drops a pending crop.

```rust
    /// While typing every key edits the text; nothing else reacts.
    fn type_keys(&mut self, ctx: &egui::Context) {
        let events = ctx.input_mut(|i| std::mem::take(&mut i.events));
        for event in events {
            match event {
                egui::Event::Text(s) => {
                    if let Some(t) = &mut self.typing {
                        t.text.extend(s.chars().filter(|c| !c.is_control()));
                    }
                }
                egui::Event::Key { key, pressed: true, modifiers, .. } => match key {
                    Key::Backspace => {
                        if let Some(t) = &mut self.typing {
                            t.text.pop();
                        }
                    }
                    Key::Enter if modifiers.shift => {
                        if let Some(t) = &mut self.typing {
                            t.text.push('\n');
                        }
                    }
                    Key::Enter => self.finish_typing(),
                    Key::Escape => self.typing = None,
                    _ => {}
                },
                _ => {}
            }
        }
    }

    fn finish_typing(&mut self) {
        if let Some(t) = self.typing.take()
            && !t.text.trim().is_empty()
        {
            self.doc.add(Shape { tool: Tool::Text, style: self.style(), points: vec![t.at], text: t.text });
        }
    }

    fn apply_crop(&mut self) {
        if let Some(c) = self.cropping.take() {
            let (x0, y0, x1, y1) = c.rect;
            let (x0, x1, y0, y1) = (x0.min(x1).floor(), x0.max(x1).ceil(), y0.min(y1).floor(), y0.max(y1).ceil());
            if x1 - x0 >= 2.0 && y1 - y0 >= 2.0 {
                self.doc.add_crop(PixelRect { x: x0 as u32, y: y0 as u32, width: (x1 - x0) as u32, height: (y1 - y0) as u32 });
            }
        }
    }
```

In `canvas()`, after the pointer is mapped (`p` = image point clamped to the shown rect):

- On `drag_started`: if `typing` is some → `finish_typing()` and stop (the press only finishes the text). Otherwise by tool: Text → `typing = Some(Typing { at: p, text: String::new() })`; Number → `doc.add(Shape { tool: Number, style, points: vec![p], text: doc.next_number().to_string() })`; Crop → if `cropping` is some and `crop_edges(screen rect, pointer, 8.0)` hits → set its `grab`; else `cropping = Some(CropEdit { rect: (p.0, p.1, p.0, p.1), grab: None })` (a new rectangle, its far corner follows the pointer); everything else → `Drag::new` as in 5a.
- While dragging with Crop: with `grab` move the grabbed edges to `p`; without, move the far corner (x1, y1). On `drag_stopped` with Crop: clear `grab`; a rectangle under 2 px is dropped.
- The Esc-during-drag minor from 5a stays deferred.
- Texture key: `(revision, typing text)`; when it changes, render `doc.shapes()` plus the typed text as a temporary `Tool::Text` shape into the texture.
- Overlays painted with egui on top of the texture: the caret while typing (a 1.5 px line in the text colour at `at + (last line width, (lines − 1) × line height)`, `line height` tall; visible when `(time × 2) as i64 % 2 == 0`, `request_repaint_after(500 ms)`), and with a pending crop the darkened outside (four `rect_filled` with `Color32::from_black_alpha(128)`) plus a 1 px white `rect_stroke`. Hovering a crop edge sets the cursor icon (`ResizeHorizontal`, `ResizeVertical`, `ResizeNwSe`, `ResizeNeSw`).

- [ ] **Step 3: Run** (all pass, +1), clippy, fmt.

- [ ] **Step 4: Smoke run** (tell the user first): open `valw edit` on a generated image for a few seconds; the toolbar shows the 10 tools; no errors in the log.

- [ ] **Step 5: Commit** `feat: text, numbers, pixelate and crop in the editor`.

---

### Task 5: Checks and checklist

- [ ] `nix flake check -L --keep-going` passes.
- [ ] Append to `docs/test-checklist.md`:

```markdown

## Editor tools (Phase 5b)

- [ ] T: click, type (Turkish: ğüşıöçİ), Shift+Enter for a new line, Enter finishes; a click elsewhere also finishes; Esc drops it; while typing, letters don't switch tools and Ctrl+Z/Esc don't undo or close.
- [ ] Text is bold in the chosen colour with a dark outline, readable on light and dark backgrounds; S/M/L sizes; the saved PNG matches the canvas.
- [ ] B: dragging pixelates the area (also earlier arrows inside it); S/M/L block sizes; edges of the image are fine.
- [ ] N: clicks place 1, 2, 3 …; undo then a new click reuses the freed number.
- [ ] C: drag a crop, adjust its edges and corners (the cursor changes), Enter applies, Esc cancels; the canvas then shows only the crop; drawing after a crop lands where you click; undo restores the full image; the saved PNG has the crop's size.
- [ ] The toolbar shows all 10 tools and still wraps in a narrow window.
```

- [ ] Commit `docs: editor tools checklist`.
