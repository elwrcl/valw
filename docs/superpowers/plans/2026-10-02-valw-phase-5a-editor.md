# valw Phase 5a: built-in editor basics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `valw edit <file>`: a Markup-like window with arrow/rect/ellipse/line/pen/highlighter, colours, widths, undo/redo, save/copy; the preview opens it instead of Satty.

**Architecture:** Pure modules first (`editor/shape.rs` tools, constraints and shared geometry; `editor/doc.rs` history; `editor/view.rs` fitting; `editor/export.rs` tiny-skia rendering), then the eframe (glow) app in `editor/mod.rs`. Copy goes through a hidden `valw __clipboard` child (no fork inside a multithreaded GUI process). Runtime libraries come from the binary's RUNPATH instead of `LD_LIBRARY_PATH`.

**Tech Stack:** Rust 2024, `eframe`/`egui` 0.36.2 (`glow`, `wayland`, `default_fonts`; no wgpu, no x11), `tiny-skia` 0.12.

**Spec:** `docs/superpowers/specs/2026-10-02-valw-phase-5a-editor-design.md`

## Global Constraints

- jj only; `cargo clippy -- -D warnings`, `cargo fmt --check`, `cargo nextest run`, `nix flake check`.
- No Vulkan, no wgpu. App id `valw-editor`; title `<file name> — valw`.
- Colours `#ff3b30` (default) `#ff9500` `#ffcc00` `#34c759` `#007aff` black white; widths 2/4/8 image px (default 4); highlighter = colour at 40 % opacity (alpha 102), width × 4; arrow head length `max(12, 4 × width)`, 30° half-angle; clicks under 2 image px are discarded except Pen (dot).
- Status messages last 2 s. Ctrl+S overwrites; Ctrl+Shift+S writes `<stem> edited.png` (unique).
- Tell the user before opening editor windows on the live session.

## Review Focus

1. **Export ≠ screen:** shapes must land at the same image pixels in the saved PNG as on the canvas, at any window size. Tests: `editor::view::tests::round_trip`, `editor::export::tests::line_pixels` (Tasks 2–3).
2. **Images with alpha** (window shots from Phase 3): export must keep transparency, not turn it black or premultiply twice. Test: `editor::export::tests::keeps_transparency` (Task 2).
3. **Unsaved changes on close** (Esc or the compositor's close): never lose edits silently. Test: `editor::doc::tests::*dirty*` (Task 1); real check in the checklist.
4. **Undo/redo after save:** dirty must track content, not a counter. Test: `editor::doc::tests::undo_past_a_save_is_dirty` (Task 1).
5. **The editor closes while the clipboard must live on:** Ctrl+C then Esc must keep the copied image available. Covered by the `__clipboard` child (Task 4), checked in the checklist.

---

## File Structure

| File | Change | Task |
|---|---|---|
| `src/editor/shape.rs` | `Tool`, `Style`, `Shape`, `Drag`, `Prim`, `constrain`, `geometry`, palette/widths | 1 |
| `src/editor/doc.rs` | `Doc` | 1 |
| `src/editor/view.rs` | `Fit` | 2 |
| `src/editor/export.rs` | `render`, `edited_path` | 2 |
| `src/config.rs` | `[editor] backend` | 3 |
| `src/host.rs` | `editor_command`, `open_editor` | 3 |
| `src/output.rs`, `src/main.rs` | `serve_clipboard`, `__clipboard` | 4 |
| `src/editor/mod.rs` | eframe app | 4 |
| `Cargo.toml`, `nix/package.ulu.nix`, `nix/devshell.ulu.nix` | deps, RUNPATH, dev `LD_LIBRARY_PATH` | 4 |
| `docs/test-checklist.md` | Phase 5a section | 5 |

---

### Task 1: Shapes, geometry and history

**Files:** create `src/editor/mod.rs` (`pub mod doc; pub mod shape;` only), `src/editor/shape.rs`, `src/editor/doc.rs`; `src/main.rs`: `mod editor;` + `// The editor lands in pieces; Task 4 removes this.` `#![allow(dead_code)]`.

**Produces:** `shape::{Tool, Style, Shape, Drag, Prim, PALETTE, WIDTHS, constrain, geometry}`, `Shape::is_click`, `doc::Doc::{new, add, undo, redo, can_undo, can_redo, is_dirty, mark_saved, shapes}`.

- [ ] **Step 1: Failing tests** — `src/editor/shape.rs` (tests only):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const RED: Style = Style { color: [255, 59, 48], width: 4.0 };

    fn shape(tool: Tool, points: &[(f32, f32)]) -> Shape {
        Shape { tool, style: RED, points: points.to_vec() }
    }

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    #[test]
    fn shift_snaps_lines_to_45_degrees() {
        let end = constrain(Tool::Line, (0.0, 0.0), (100.0, 10.0), true);
        assert!(close(end, ((100f32.hypot(10.0)), 0.0)), "{end:?}");
        let end = constrain(Tool::Arrow, (0.0, 0.0), (50.0, 45.0), true);
        let d = 50f32.hypot(45.0) / 2f32.sqrt();
        assert!(close(end, (d, d)), "{end:?}");
        assert_eq!(constrain(Tool::Line, (0.0, 0.0), (100.0, 10.0), false), (100.0, 10.0));
    }

    #[test]
    fn shift_makes_squares_and_circles() {
        assert_eq!(constrain(Tool::Rectangle, (10.0, 10.0), (40.0, 20.0), true), (40.0, 40.0));
        assert_eq!(constrain(Tool::Ellipse, (10.0, 10.0), (0.0, 60.0), true), (-40.0, 60.0));
    }

    #[test]
    fn clicks_are_discarded_but_a_pen_dot_is_kept() {
        assert!(shape(Tool::Arrow, &[(5.0, 5.0), (6.0, 6.0)]).is_click());
        assert!(!shape(Tool::Arrow, &[(5.0, 5.0), (8.0, 5.0)]).is_click());
        assert!(shape(Tool::Highlighter, &[(5.0, 5.0), (5.5, 5.0)]).is_click());
        assert!(!shape(Tool::Pen, &[(5.0, 5.0)]).is_click());
    }

    #[test]
    fn arrow_has_a_line_and_a_head_at_the_end() {
        let g = geometry(&shape(Tool::Arrow, &[(0.0, 0.0), (100.0, 0.0)]));
        assert_eq!(g.len(), 2);
        let Prim::Stroke { points, width, .. } = &g[0] else { panic!("{g:?}") };
        assert_eq!(*width, 4.0);
        assert!(close(points[0], (0.0, 0.0)) && close(points[1], (84.0, 0.0)), "{points:?}");
        let Prim::Fill { points, .. } = &g[1] else { panic!("{g:?}") };
        // Head length max(12, 4 × 4) = 16, half-width 16 × tan 30°.
        let half = 16.0 * (30f32).to_radians().tan();
        assert!(close(points[0], (100.0, 0.0)), "{points:?}");
        assert!(close(points[1], (84.0, half)) && close(points[2], (84.0, -half)), "{points:?}");
    }

    #[test]
    fn highlighter_is_wide_and_translucent() {
        let g = geometry(&shape(Tool::Highlighter, &[(0.0, 0.0), (50.0, 0.0)]));
        let Prim::Stroke { width, color, round, .. } = &g[0] else { panic!("{g:?}") };
        assert_eq!(*width, 16.0);
        assert_eq!(*color, [255, 59, 48, 102]);
        assert!(!round, "butt caps: overlapping caps would darken");
    }

    #[test]
    fn rectangle_and_ellipse_are_closed_outlines() {
        let g = geometry(&shape(Tool::Rectangle, &[(10.0, 10.0), (0.0, 20.0)]));
        let Prim::Stroke { points, closed, .. } = &g[0] else { panic!("{g:?}") };
        assert!(*closed);
        assert_eq!(points, &[(10.0, 10.0), (0.0, 10.0), (0.0, 20.0), (10.0, 20.0)]);
        let g = geometry(&shape(Tool::Ellipse, &[(0.0, 0.0), (20.0, 10.0)]));
        let Prim::Stroke { points, closed, .. } = &g[0] else { panic!("{g:?}") };
        assert!(*closed && points.len() == 72);
        assert!(points.iter().all(|p| ((p.0 - 10.0) / 10.0).powi(2) + ((p.1 - 5.0) / 5.0).powi(2) - 1.0 < 1e-3));
    }

    #[test]
    fn a_pen_dot_is_a_filled_circle() {
        let g = geometry(&shape(Tool::Pen, &[(5.0, 5.0)]));
        let Prim::Fill { points, color } = &g[0] else { panic!("{g:?}") };
        assert_eq!(*color, [255, 59, 48, 255]);
        assert!(points.iter().all(|p| ((p.0 - 5.0).hypot(p.1 - 5.0) - 2.0).abs() < 1e-3));
    }

    #[test]
    fn drag_collects_points_per_tool() {
        let mut d = Drag::new(Tool::Pen, RED, (0.0, 0.0));
        d.move_to((1.0, 1.0));
        d.move_to((2.0, 3.0));
        assert_eq!(d.shape(false).points, vec![(0.0, 0.0), (1.0, 1.0), (2.0, 3.0)]);
        let mut d = Drag::new(Tool::Rectangle, RED, (0.0, 0.0));
        d.move_to((5.0, 9.0));
        d.move_to((30.0, 10.0));
        assert_eq!(d.shape(false).points, vec![(0.0, 0.0), (30.0, 10.0)]);
        assert_eq!(d.shape(true).points, vec![(0.0, 0.0), (30.0, 30.0)]);
    }
}
```

`src/editor/doc.rs` (tests only):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::shape::{Style, Tool};

    fn line(x: f32) -> Shape {
        Shape { tool: Tool::Line, style: Style { color: [0, 0, 0], width: 2.0 }, points: vec![(0.0, 0.0), (x, 0.0)] }
    }

    #[test]
    fn undo_and_redo_move_shapes() {
        let mut d = Doc::new();
        assert!(!d.can_undo() && !d.can_redo());
        d.add(line(1.0));
        d.add(line(2.0));
        assert!(d.undo());
        assert_eq!(d.shapes(), &[line(1.0)]);
        assert!(d.redo());
        assert_eq!(d.shapes(), &[line(1.0), line(2.0)]);
        assert!(!d.redo());
    }

    #[test]
    fn a_new_shape_clears_redo() {
        let mut d = Doc::new();
        d.add(line(1.0));
        d.undo();
        d.add(line(3.0));
        assert!(!d.can_redo());
        assert_eq!(d.shapes(), &[line(3.0)]);
    }

    #[test]
    fn dirty_until_saved() {
        let mut d = Doc::new();
        assert!(!d.is_dirty());
        d.add(line(1.0));
        assert!(d.is_dirty());
        d.mark_saved();
        assert!(!d.is_dirty());
    }

    #[test]
    fn undo_past_a_save_is_dirty() {
        let mut d = Doc::new();
        d.add(line(1.0));
        d.mark_saved();
        d.undo();
        assert!(d.is_dirty());
        d.redo();
        assert!(!d.is_dirty(), "back to the saved content");
    }
}
```

Run: `cargo nextest run editor::` → fails to compile (types missing).

- [ ] **Step 2: Implement `shape.rs`** (above its tests)

```rust
//! Markup tools and the geometry both painters share: the canvas (egui)
//! and the export (tiny-skia) draw the same primitives, in image pixels.

pub type P = (f32, f32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Arrow,
    Rectangle,
    Ellipse,
    Line,
    Pen,
    Highlighter,
}

impl Tool {
    pub const ALL: [Tool; 6] = [
        Tool::Arrow,
        Tool::Rectangle,
        Tool::Ellipse,
        Tool::Line,
        Tool::Pen,
        Tool::Highlighter,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Arrow => "Arrow",
            Tool::Rectangle => "Rectangle",
            Tool::Ellipse => "Ellipse",
            Tool::Line => "Line",
            Tool::Pen => "Pen",
            Tool::Highlighter => "Highlighter",
        }
    }

    fn freehand(self) -> bool {
        matches!(self, Tool::Pen | Tool::Highlighter)
    }
}

/// The toolbar's colours; the first is the default.
pub const PALETTE: [[u8; 3]; 7] = [
    [0xff, 0x3b, 0x30],
    [0xff, 0x95, 0x00],
    [0xff, 0xcc, 0x00],
    [0x34, 0xc7, 0x59],
    [0x00, 0x7a, 0xff],
    [0x00, 0x00, 0x00],
    [0xff, 0xff, 0xff],
];

/// S, M, L in image pixels; M is the default.
pub const WIDTHS: [(&str, f32); 3] = [("S", 2.0), ("M", 4.0), ("L", 8.0)];

const HIGHLIGHT_ALPHA: u8 = 102;
const CLICK: f32 = 2.0;
const ELLIPSE_POINTS: usize = 72;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    pub color: [u8; 3],
    pub width: f32,
}

/// One finished mark. Two-point tools hold [start, end]; Pen and
/// Highlighter hold every point.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub tool: Tool,
    pub style: Style,
    pub points: Vec<P>,
}

impl Shape {
    /// A press without a real drag. A Pen click still leaves a dot.
    pub fn is_click(&self) -> bool {
        if self.tool == Tool::Pen {
            return false;
        }
        let first = self.points[0];
        self.points
            .iter()
            .all(|p| (p.0 - first.0).hypot(p.1 - first.1) < CLICK)
    }
}

/// A shape being drawn.
#[derive(Debug, Clone)]
pub struct Drag {
    tool: Tool,
    style: Style,
    points: Vec<P>,
}

impl Drag {
    pub fn new(tool: Tool, style: Style, start: P) -> Drag {
        Drag { tool, style, points: vec![start] }
    }

    pub fn move_to(&mut self, p: P) {
        if self.tool.freehand() || self.points.len() == 1 {
            self.points.push(p);
        } else {
            self.points[1] = p;
        }
    }

    /// The shape so far; `shift` applies the tool's constraint.
    pub fn shape(&self, shift: bool) -> Shape {
        let mut points = self.points.clone();
        if !self.tool.freehand() && points.len() == 2 {
            points[1] = constrain(self.tool, points[0], points[1], shift);
        }
        Shape { tool: self.tool, style: self.style, points }
    }
}

/// Shift: lines and arrows snap to 45° steps, rectangles and ellipses
/// become squares and circles.
pub fn constrain(tool: Tool, start: P, end: P, shift: bool) -> P {
    if !shift {
        return end;
    }
    let (dx, dy) = (end.0 - start.0, end.1 - start.1);
    match tool {
        Tool::Line | Tool::Arrow => {
            let step = std::f32::consts::FRAC_PI_4;
            let angle = (dy.atan2(dx) / step).round() * step;
            let len = dx.hypot(dy);
            (start.0 + len * angle.cos(), start.1 + len * angle.sin())
        }
        Tool::Rectangle | Tool::Ellipse => {
            let side = dx.abs().max(dy.abs());
            (start.0 + side.copysign(dx), start.1 + side.copysign(dy))
        }
        Tool::Pen | Tool::Highlighter => end,
    }
}

/// A primitive in image pixels. Colours are straight (not premultiplied) RGBA.
#[derive(Debug, Clone, PartialEq)]
pub enum Prim {
    Stroke {
        points: Vec<P>,
        width: f32,
        color: [u8; 4],
        closed: bool,
        /// Round caps and joins; false means butt caps.
        round: bool,
    },
    /// A convex polygon.
    Fill { points: Vec<P>, color: [u8; 4] },
}

pub fn geometry(shape: &Shape) -> Vec<Prim> {
    let [r, g, b] = shape.style.color;
    let opaque = [r, g, b, 255];
    let w = shape.style.width;
    let pts = &shape.points;
    let stroke = |points: Vec<P>, closed: bool| Prim::Stroke {
        points,
        width: w,
        color: opaque,
        closed,
        round: true,
    };
    match shape.tool {
        Tool::Line => vec![stroke(vec![pts[0], pts[1]], false)],
        Tool::Arrow => arrow(pts[0], pts[1], w, opaque),
        Tool::Rectangle => {
            let ((x0, y0), (x1, y1)) = (pts[0], pts[1]);
            vec![stroke(vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)], true)]
        }
        Tool::Ellipse => {
            let ((x0, y0), (x1, y1)) = (pts[0], pts[1]);
            let (cx, cy, rx, ry) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0, (x1 - x0).abs() / 2.0, (y1 - y0).abs() / 2.0);
            vec![stroke(polygon(cx, cy, rx, ry, ELLIPSE_POINTS), true)]
        }
        Tool::Pen if pts.len() == 1 => {
            let (x, y) = pts[0];
            vec![Prim::Fill { points: polygon(x, y, w / 2.0, w / 2.0, 24), color: opaque }]
        }
        Tool::Pen => vec![stroke(pts.clone(), false)],
        Tool::Highlighter => vec![Prim::Stroke {
            points: pts.clone(),
            width: w * 4.0,
            color: [r, g, b, HIGHLIGHT_ALPHA],
            closed: false,
            round: false,
        }],
    }
}

/// A line to the base of a filled head whose tip is at `end`.
fn arrow(start: P, end: P, width: f32, color: [u8; 4]) -> Vec<Prim> {
    let (dx, dy) = (end.0 - start.0, end.1 - start.1);
    let len = dx.hypot(dy).max(f32::EPSILON);
    let (ux, uy) = (dx / len, dy / len);
    let head = (4.0 * width).max(12.0).min(len);
    let half = head * 30f32.to_radians().tan();
    let base = (end.0 - ux * head, end.1 - uy * head);
    vec![
        Prim::Stroke { points: vec![start, base], width, color, closed: false, round: true },
        Prim::Fill {
            points: vec![end, (base.0 - uy * half, base.1 + ux * half), (base.0 + uy * half, base.1 - ux * half)],
            color,
        },
    ]
}

/// `n` points around an ellipse, starting at angle 0.
fn polygon(cx: f32, cy: f32, rx: f32, ry: f32, n: usize) -> Vec<P> {
    (0..n)
        .map(|i| {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            (cx + rx * a.cos(), cy + ry * a.sin())
        })
        .collect()
}
```

(Check `arrow_has_a_line_and_a_head_at_the_end`: direction (1,0) → base (84,0); head points (84, +half) and (84, −half) — the second point is `base − u⊥·half` with u⊥ = (−uy, ux) = (0, 1)… compute: `(base.0 - uy*half, base.1 + ux*half)` = (84, +half) ✓, third = (84, −half) ✓.)

- [ ] **Step 3: Implement `doc.rs`** (above its tests)

```rust
//! The edit history: the shapes drawn so far, what undo took away, and
//! what was last saved.

use crate::editor::shape::Shape;

#[derive(Debug, Default)]
pub struct Doc {
    shapes: Vec<Shape>,
    redo: Vec<Shape>,
    saved: Vec<Shape>,
}

impl Doc {
    pub fn new() -> Doc {
        Doc::default()
    }

    pub fn shapes(&self) -> &[Shape] {
        &self.shapes
    }

    pub fn add(&mut self, shape: Shape) {
        self.shapes.push(shape);
        self.redo.clear();
    }

    pub fn undo(&mut self) -> bool {
        let Some(shape) = self.shapes.pop() else { return false };
        self.redo.push(shape);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(shape) = self.redo.pop() else { return false };
        self.shapes.push(shape);
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.shapes.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Whether the shapes differ from what was last saved.
    pub fn is_dirty(&self) -> bool {
        self.shapes != self.saved
    }

    pub fn mark_saved(&mut self) {
        self.saved = self.shapes.clone();
    }
}
```

- [ ] **Step 4: Run** — `cargo nextest run` (+12), clippy, fmt. **Step 5: Commit** `feat: editor shapes, geometry and history`.

---

### Task 2: Fitting and export

**Files:** `src/editor/view.rs`, `src/editor/export.rs`; `Cargo.toml`: `tiny-skia = "0.12.0"`; `src/editor/mod.rs`: `pub mod export; pub mod view;`.

**Produces:** `view::Fit { scale: f32, offset: P }`, `Fit::new(image: (u32, u32), area: (f32, f32, f32, f32) /* x, y, w, h */, pixels_per_point: f32)`, `to_screen(P) -> P`, `to_image(P) -> P`, `clamp(P, image) -> P`; `export::render(base: &RgbaImage, shapes: &[Shape]) -> RgbaImage`, `export::edited_path(&Path) -> PathBuf`.

- [ ] **Step 1: Failing tests**

`src/editor/view.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_images_are_not_upscaled() {
        let f = Fit::new((400, 300), (0.0, 50.0, 1000.0, 800.0), 1.0);
        assert_eq!(f.scale, 1.0);
        assert_eq!(f.offset, (300.0, 300.0), "centred");
    }

    #[test]
    fn large_images_fit_the_area() {
        let f = Fit::new((1920, 1080), (0.0, 40.0, 960.0, 1000.0), 1.0);
        assert_eq!(f.scale, 0.5);
        assert_eq!(f.offset, (0.0, 40.0 + (1000.0 - 540.0) / 2.0));
    }

    #[test]
    fn hundred_percent_means_one_image_pixel_per_physical_pixel() {
        let f = Fit::new((400, 300), (0.0, 0.0, 1000.0, 800.0), 2.0);
        assert_eq!(f.scale, 0.5, "0.5 points per image px at 2× = 1 physical px");
    }

    #[test]
    fn round_trip() {
        let f = Fit::new((1920, 1080), (10.0, 40.0, 1000.0, 700.0), 1.25);
        for p in [(0.0, 0.0), (1919.0, 1079.0), (333.5, 777.25)] {
            let q = f.to_image(f.to_screen(p));
            assert!((q.0 - p.0).abs() < 1e-3 && (q.1 - p.1).abs() < 1e-3, "{p:?} {q:?}");
        }
    }

    #[test]
    fn clamp_keeps_points_on_the_image() {
        assert_eq!(clamp((-5.0, 50.0), (100, 80)), (0.0, 50.0));
        assert_eq!(clamp((150.0, 90.0), (100, 80)), (100.0, 80.0));
    }
}
```

`src/editor/export.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::shape::{Style, Tool};
    use image::Rgba;

    fn line(tool: Tool, color: [u8; 3]) -> Shape {
        Shape { tool, style: Style { color, width: 4.0 }, points: vec![(10.0, 20.0), (90.0, 20.0)] }
    }

    #[test]
    fn line_pixels() {
        let base = RgbaImage::from_pixel(100, 40, Rgba([255, 255, 255, 255]));
        let out = render(&base, &[line(Tool::Line, [255, 0, 0])]);
        assert_eq!(out.dimensions(), (100, 40));
        assert_eq!(out.get_pixel(50, 20).0, [255, 0, 0, 255], "on the line");
        assert_eq!(out.get_pixel(50, 5).0, [255, 255, 255, 255], "away from it");
        assert_eq!(out.get_pixel(5, 20).0, [255, 255, 255, 255], "before its start cap");
    }

    #[test]
    fn highlighter_blends_at_40_percent() {
        let base = RgbaImage::from_pixel(100, 40, Rgba([0, 0, 255, 255]));
        let out = render(&base, &[line(Tool::Highlighter, [255, 255, 0])]);
        let [r, g, b, a] = out.get_pixel(50, 20).0;
        assert_eq!(a, 255);
        for (got, want) in [(r, 102), (g, 102), (b, 153)] {
            assert!((got as i32 - want).abs() <= 2, "{:?}", out.get_pixel(50, 20));
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
        assert!((r as i32 - 200).abs() <= 2 && (g as i32 - 100).abs() <= 2 && (b as i32 - 50).abs() <= 2);
        assert_eq!(out.get_pixel(50, 20).0, [255, 0, 0, 255]);
    }

    #[test]
    fn edited_path_sits_next_to_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Screenshot at 10.00.png");
        assert_eq!(edited_path(&file), dir.path().join("Screenshot at 10.00 edited.png"));
        std::fs::write(dir.path().join("Screenshot at 10.00 edited.png"), b"x").unwrap();
        assert_ne!(edited_path(&file), dir.path().join("Screenshot at 10.00 edited.png"));
    }
}
```

Run: `cargo nextest run editor::` → compile errors (missing items).

- [ ] **Step 2: Implement `view.rs`**

```rust
//! Fitting the image into the canvas: image pixels ↔ egui points.

use crate::editor::shape::P;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    /// Points per image pixel.
    pub scale: f32,
    /// Where the image's top-left corner is, in points.
    pub offset: P,
}

impl Fit {
    /// Fits `image` into `area` (x, y, w, h in points), centred, never
    /// above 100 % (one image pixel per physical pixel).
    pub fn new(image: (u32, u32), area: (f32, f32, f32, f32), pixels_per_point: f32) -> Fit {
        let (iw, ih) = (image.0 as f32, image.1 as f32);
        let (x, y, w, h) = area;
        let scale = (w / iw).min(h / ih).min(1.0 / pixels_per_point);
        Fit { scale, offset: (x + (w - iw * scale) / 2.0, y + (h - ih * scale) / 2.0) }
    }

    pub fn to_screen(&self, p: P) -> P {
        (self.offset.0 + p.0 * self.scale, self.offset.1 + p.1 * self.scale)
    }

    pub fn to_image(&self, p: P) -> P {
        ((p.0 - self.offset.0) / self.scale, (p.1 - self.offset.1) / self.scale)
    }
}

/// `p` moved onto the image's area.
pub fn clamp(p: P, image: (u32, u32)) -> P {
    (p.0.clamp(0.0, image.0 as f32), p.1.clamp(0.0, image.1 as f32))
}
```

(`large_images_fit_the_area`: scale = min(0.5, 0.926, 1) = 0.5; offset x = 0 + (960 − 960)/2 = 0 ✓.)

- [ ] **Step 3: Implement `export.rs`**

```rust
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
        Prim::Fill { .. } => pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None),
        Prim::Stroke { width, round, .. } => {
            let stroke = Stroke {
                width: *width,
                line_cap: if *round { LineCap::Round } else { LineCap::Butt },
                line_join: if *round { LineJoin::Round } else { LineJoin::Miter },
                ..Stroke::default()
            };
            pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
        }
    }
}

/// `<stem> edited.png` next to `path`, numbered if taken.
pub fn edited_path(path: &Path) -> PathBuf {
    let dir = path.parent().unwrap_or(Path::new("."));
    let stem = path.file_stem().map(|s| s.to_string_lossy()).unwrap_or_default();
    crate::output::unique_path(dir, &format!("{stem} edited.png"))
}
```

(If `pixels_mut`, `premultiply`, `demultiply` or `PathBuilder::finish` differ in tiny-skia 0.12, adapt to its API.)

- [ ] **Step 4: Run** (+9), clippy, fmt. **Step 5: Commit** `feat: editor fitting and export`.

---

### Task 3: Editor backend config and the preview's command

**Files:** `src/config.rs`, `src/host.rs`.

**Produces:** `config::{Editor, Backend}` (`Backend::{Builtin, Satty}`, serde lowercase, default `Builtin`), `Config::editor`; `host::editor_command(backend: Backend, path: &Path, exe: &Path) -> std::process::Command`.

- [ ] **Step 1: Failing tests**

`src/config.rs` tests (the unknown-keys test uses `[editor]`: change its table to `[toolbar]\nposition = "top"` so it stays unknown → `"toolbar"`):

```rust
    #[test]
    fn editor_backend_default_and_values() {
        assert_eq!(Config::default().editor.backend, Backend::Builtin);
        let (config, _) = parse("[editor]\nbackend = \"satty\"\n").unwrap();
        assert_eq!(config.editor.backend, Backend::Satty);
        assert!(parse("[editor]\nbackend = \"gimp\"\n").is_err());
    }
```

`src/host.rs` tests:

```rust
    #[test]
    fn editor_command_per_backend() {
        let path = Path::new("/p/Shot 1.png");
        let exe = Path::new("/nix/store/x/bin/valw");
        let builtin = editor_command(Backend::Builtin, path, exe);
        assert_eq!(builtin.get_program(), exe);
        assert_eq!(builtin.get_args().collect::<Vec<_>>(), ["edit", "/p/Shot 1.png"]);
        let satty = editor_command(Backend::Satty, path, exe);
        assert_eq!(satty.get_program(), "satty");
        assert_eq!(
            satty.get_args().collect::<Vec<_>>(),
            ["--filename", "/p/Shot 1.png", "--output-filename", "/p/Shot 1.png"]
        );
    }
```

Run → compile errors.

- [ ] **Step 2: Implement**

`config.rs`: `pub editor: Editor,` in `Config`, and

```rust
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(default)]
pub struct Editor {
    pub backend: Backend,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    /// `valw edit`.
    #[default]
    Builtin,
    Satty,
}
```

`host.rs`: replace `open_editor` with

```rust
/// The command that opens `path` in the configured editor; both save over
/// the file.
fn editor_command(backend: Backend, path: &Path, exe: &Path) -> std::process::Command {
    let mut command = match backend {
        Backend::Builtin => {
            let mut c = std::process::Command::new(exe);
            c.arg("edit").arg(path);
            c
        }
        Backend::Satty => {
            let mut c = std::process::Command::new("satty");
            c.arg("--filename").arg(path).arg("--output-filename").arg(path);
            c
        }
    };
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command
}

/// Opens the screenshot in the configured editor.
fn open_editor(path: &Path) {
    use std::os::unix::process::CommandExt;

    let backend = match config::load(&config::default_path()) {
        Ok(c) => c.editor.backend,
        Err(e) => {
            tracing::warn!("using the built-in editor: {e:#}");
            Backend::Builtin
        }
    };
    let exe = std::env::current_exe().unwrap_or_else(|_| "valw".into());
    let mut command = editor_command(backend, path, &exe);
    // SAFETY: setsid is async-signal-safe.
    unsafe {
        command.pre_exec(|| rustix::process::setsid().map(drop).map_err(Into::into));
    }
    if let Err(e) = command.spawn() {
        tracing::warn!("could not start the editor ({backend:?}): {e}");
    }
}
```

with `use crate::config::{self, Backend};` (merge with existing imports). `get_args` yields `&OsStr`; compare via `.collect::<Vec<_>>()` against `&str` arrays works through `PartialEq<str> for OsStr`; if not, map with `to_str().unwrap()`.

- [ ] **Step 3: Run** (+2), clippy, fmt. **Step 4: Commit** `feat: preview opens the configured editor`.

---

### Task 4: The editor window, `__clipboard`, packaging

**Files:** `src/editor/mod.rs`, `src/output.rs`, `src/main.rs`, `Cargo.toml`, `nix/package.ulu.nix`, `nix/devshell.ulu.nix`.

**Consumes:** Tasks 1–2. **Produces:** `editor::run(path: &Path) -> Result<()>`; `output::serve_clipboard(png: Vec<u8>) -> Result<()>` (serves in the foreground until replaced); `Command::Edit { file: PathBuf }`; hidden `Command::Clipboard` (`__clipboard`, PNG on stdin).

- [ ] **Step 1: Failing tests** — `src/main.rs` `flag_conflicts` gains:

```rust
        assert!(parses(&["edit", "a.png"]));
        assert!(!parses(&["edit"]));
        assert!(parses(&["__clipboard"]));
```

and `preview_host_is_hidden_from_help` gains `assert!(!help.contains("__clipboard"), "{help}");`. Run → fails.

- [ ] **Step 2: Dependencies and packaging**

`Cargo.toml`: `eframe = { version = "0.36.2", default-features = false, features = ["default_fonts", "glow", "wayland"] }`.

winit (through eframe) loads libwayland-client, libxkbcommon and libEGL with `dlopen`, and Cargo's feature unification switches valw's own libwayland to `dlopen` too. Give the binary a RUNPATH instead of an inherited `LD_LIBRARY_PATH` (this also fixes the Phase 4 minor about the variable leaking into Satty and the clipboard child). `nix/package.ulu.nix`:

```nix
        # Clicking a preview opens Satty if configured. --suffix keeps a user's own Satty first.
        postInstall = ''
          wrapProgram $out/bin/valw --suffix PATH : ${lib.makeBinPath [ pkgs.satty ]}
        '';

        # dlopen()ed at run time: libwayland (winit, valw), libxkbcommon (winit),
        # libEGL (zoom, editor; Mesa's drivers come from the system).
        postFixup = ''
          patchelf --add-rpath ${lib.makeLibraryPath [ pkgs.wayland pkgs.libxkbcommon pkgs.libglvnd ]} $out/bin/.valw-wrapped
        '';
```

(If the wrapped file is named differently, `ls $out/bin` in the build log and adjust.) `nix/devshell.ulu.nix`: inside `mkShell`, `LD_LIBRARY_PATH = lib.makeLibraryPath [ pkgs.wayland pkgs.libxkbcommon pkgs.libglvnd ];` (take `lib` from the module arguments).

- [ ] **Step 3: `output::serve_clipboard` and `__clipboard`**

In `src/output.rs`, split `copy_to_clipboard`:

```rust
fn prepare_clipboard(png: Vec<u8>) -> Result<PreparedCopy> {
    let mut options = Options::new();
    options.foreground(true);
    options
        .prepare_copy(Source::Bytes(png.into()), MimeType::Specific("image/png".into()))
        .context("could not copy to the clipboard")
        .hint("the compositor needs ext-data-control or wlr-data-control; see `valw doctor`")
}

pub fn copy_to_clipboard(png: Vec<u8>, lock: Lock) -> Result<()> {
    let prepared = prepare_clipboard(png)?;
    crate::detach::spawn(lock, move || {
        let _ = prepared.serve();
    })
}

/// Serves `png` as the clipboard from this process until something else
/// takes the clipboard.
pub fn serve_clipboard(png: Vec<u8>) -> Result<()> {
    prepare_clipboard(png)?.serve().context("the clipboard server failed")
}
```

(`PreparedCopy` is `wl_clipboard_rs::copy::PreparedCopy`; import it.)

In `src/main.rs`: `Command::Edit { /// The image to edit. file: PathBuf }` documented "Mark up an image (opened by clicking a preview).", and

```rust
    /// Internal: serve a PNG from stdin as the clipboard (the editor's copy).
    #[command(name = "__clipboard", hide = true)]
    Clipboard,
```

`run` arms: `Command::Edit { file } => editor::run(&file),` and

```rust
        Command::Clipboard => {
            let mut png = Vec::new();
            std::io::Read::read_to_end(&mut std::io::stdin(), &mut png).context("could not read the image")?;
            output::serve_clipboard(png)
        }
```

- [ ] **Step 4: `src/editor/mod.rs`** (replace the Task 1/2 module list; keep the four `pub mod` lines)

```rust
//! `valw edit`: a Markup-like editor window (eframe, OpenGL ES).

pub mod doc;
pub mod export;
pub mod shape;
pub mod view;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use eframe::egui::{self, Color32, Key, Modifiers, Pos2, Rect, Sense, Stroke, StrokeKind, ViewportCommand};
use image::RgbaImage;

use self::doc::Doc;
use self::shape::{Drag, PALETTE, Prim, Style, Tool, WIDTHS, geometry};
use self::view::Fit;

const APP_ID: &str = "valw-editor";
const STATUS_FOR: Duration = Duration::from_secs(2);

pub fn run(path: &Path) -> Result<()> {
    let image = crate::thumbnail::load(path)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (w, h) = image.dimensions();
    let viewport = egui::ViewportBuilder::default()
        .with_title(format!("{name} — valw"))
        .with_app_id(APP_ID)
        .with_inner_size([(w as f32).clamp(480.0, 1600.0), (h as f32 + 48.0).clamp(320.0, 1000.0)]);
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    let path = path.to_path_buf();
    eframe::run_native(
        APP_ID,
        options,
        Box::new(move |cc| Ok(Box::new(Editor::new(&cc.egui_ctx, path, image)))),
    )
    .map_err(|e| anyhow!("the editor window failed: {e}"))
}

struct Editor {
    path: PathBuf,
    base: RgbaImage,
    texture: egui::TextureHandle,
    doc: Doc,
    tool: Tool,
    color: usize,
    width: usize,
    drag: Option<Drag>,
    status: Option<(String, Instant)>,
    /// The unsaved-changes bar is showing.
    confirm_close: bool,
    /// The user chose to close; let the close through.
    closing: bool,
}

impl Editor {
    fn new(ctx: &egui::Context, path: PathBuf, base: RgbaImage) -> Editor {
        let size = [base.width() as usize, base.height() as usize];
        let pixels = egui::ColorImage::from_rgba_unmultiplied(size, base.as_raw());
        let texture = ctx.load_texture("image", pixels, egui::TextureOptions::LINEAR);
        Editor {
            path,
            base,
            texture,
            doc: Doc::new(),
            tool: Tool::Arrow,
            color: 0,
            width: 1,
            drag: None,
            status: None,
            confirm_close: false,
            closing: false,
        }
    }

    fn style(&self) -> Style {
        Style { color: PALETTE[self.color], width: WIDTHS[self.width].1 }
    }

    fn say(&mut self, text: impl Into<String>) {
        self.status = Some((text.into(), Instant::now()));
    }

    fn png(&self) -> Result<Vec<u8>> {
        crate::output::encode_png(&export::render(&self.base, self.doc.shapes()))
    }

    fn save(&mut self, copy: bool) -> bool {
        let target = if copy { export::edited_path(&self.path) } else { self.path.clone() };
        match self.png().and_then(|png| crate::output::write_atomic(&target, &png)) {
            Ok(()) => {
                if !copy {
                    self.doc.mark_saved();
                }
                tracing::info!("saved {}", target.display());
                self.say(if copy { "Saved a copy" } else { "Saved" });
                true
            }
            Err(e) => {
                tracing::error!("save failed: {e:#}");
                self.say(format!("Could not save: {e}"));
                false
            }
        }
    }

    fn copy(&mut self) {
        let result = self.png().and_then(|png| {
            use std::os::unix::process::CommandExt;
            let exe = std::env::current_exe().context("no path to valw")?;
            let mut command = std::process::Command::new(exe);
            command
                .arg("__clipboard")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            // SAFETY: setsid is async-signal-safe.
            unsafe {
                command.pre_exec(|| rustix::process::setsid().map(drop).map_err(Into::into));
            }
            let mut child = command.spawn().context("could not start the clipboard server")?;
            child.stdin.take().context("no stdin")?.write_all(&png).context("could not hand over the image")?;
            Ok(())
        });
        match result {
            Ok(()) => self.say("Copied"),
            Err(e) => {
                tracing::error!("copy failed: {e:#}");
                self.say(format!("Could not copy: {e}"));
            }
        }
    }

    fn close(&mut self, ctx: &egui::Context) {
        self.closing = true;
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }

    fn keys(&mut self, ctx: &egui::Context) {
        let shift_cmd = Modifiers::COMMAND | Modifiers::SHIFT;
        let pressed = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
        // Most specific first: consume_key ignores extra Shift.
        if pressed(shift_cmd, Key::S) {
            self.save(true);
        } else if pressed(Modifiers::COMMAND, Key::S) {
            if self.save(false) && self.confirm_close {
                self.close(ctx);
            }
        }
        if pressed(shift_cmd, Key::Z) || pressed(Modifiers::COMMAND, Key::Y) {
            self.doc.redo();
        } else if pressed(Modifiers::COMMAND, Key::Z) {
            self.doc.undo();
        }
        if pressed(Modifiers::COMMAND, Key::C) {
            self.copy();
        }
        if pressed(Modifiers::NONE, Key::Escape) {
            if self.doc.is_dirty() && !self.confirm_close {
                self.confirm_close = true;
            } else {
                self.close(ctx);
            }
        }
        for (key, tool) in [
            (Key::A, Tool::Arrow),
            (Key::R, Tool::Rectangle),
            (Key::O, Tool::Ellipse),
            (Key::L, Tool::Line),
            (Key::P, Tool::Pen),
            (Key::H, Tool::Highlighter),
        ] {
            if pressed(Modifiers::NONE, key) {
                self.tool = tool;
            }
        }
    }

    /// The compositor asked to close: with unsaved changes, show the bar instead.
    fn guard_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && self.doc.is_dirty() && !self.closing {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.confirm_close = true;
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for tool in Tool::ALL {
                if ui.selectable_label(self.tool == tool, tool.label()).clicked() {
                    self.tool = tool;
                }
            }
            ui.separator();
            for (i, [r, g, b]) in PALETTE.into_iter().enumerate() {
                let (rect, response) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), Sense::click());
                ui.painter().rect_filled(rect, 4.0, Color32::from_rgb(r, g, b));
                let ring = if i == self.color { ui.visuals().strong_text_color() } else { ui.visuals().weak_text_color() };
                ui.painter().rect_stroke(rect, 4.0, Stroke::new(if i == self.color { 2.0 } else { 1.0 }, ring), StrokeKind::Outside);
                if response.clicked() {
                    self.color = i;
                }
            }
            ui.separator();
            for (i, (label, _)) in WIDTHS.into_iter().enumerate() {
                if ui.selectable_label(self.width == i, label).clicked() {
                    self.width = i;
                }
            }
            ui.separator();
            if ui.add_enabled(self.doc.can_undo(), egui::Button::new("Undo")).clicked() {
                self.doc.undo();
            }
            if ui.add_enabled(self.doc.can_redo(), egui::Button::new("Redo")).clicked() {
                self.doc.redo();
            }
            ui.separator();
            if ui.button("Copy").clicked() {
                self.copy();
            }
            if ui.button("Save").clicked() {
                self.save(false);
            }
            if let Some((text, at)) = &self.status {
                if at.elapsed() < STATUS_FOR {
                    ui.label(text.as_str());
                    ui.ctx().request_repaint_after(STATUS_FOR);
                }
            }
        });
    }

    fn canvas(&mut self, ui: &mut egui::Ui) {
        let area = ui.max_rect();
        let (response, painter) = ui.allocate_painter(area.size(), Sense::drag());
        let size = self.base.dimensions();
        let fit = Fit::new(size, (area.min.x, area.min.y, area.width(), area.height()), ui.ctx().pixels_per_point());
        let to_pos = |p: shape::P| {
            let (x, y) = fit.to_screen(p);
            Pos2::new(x, y)
        };
        let image_rect = Rect::from_min_max(to_pos((0.0, 0.0)), to_pos((size.0 as f32, size.1 as f32)));
        painter.image(self.texture.id(), image_rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);

        let shift = ui.input(|i| i.modifiers.shift);
        let pointer = response.interact_pointer_pos().map(|p| fit.to_image((p.x, p.y)));
        if response.drag_started()
            && let Some(p) = pointer
            && image_rect.contains(response.interact_pointer_pos().unwrap_or_default())
        {
            self.drag = Some(Drag::new(self.tool, self.style(), p));
        }
        if let (Some(drag), Some(p)) = (&mut self.drag, pointer) {
            drag.move_to(view::clamp(p, size));
        }
        if response.drag_stopped()
            && let Some(drag) = self.drag.take()
        {
            let shape = drag.shape(shift);
            if !shape.is_click() {
                self.doc.add(shape);
            }
        }

        let live = self.drag.as_ref().map(|d| d.shape(shift));
        for shape in self.doc.shapes().iter().chain(live.as_ref()) {
            for prim in geometry(shape) {
                paint(&painter, &prim, fit.scale, to_pos);
            }
        }
    }
}

/// Draws one primitive on the canvas.
fn paint(painter: &egui::Painter, prim: &Prim, scale: f32, to_pos: impl Fn(shape::P) -> Pos2) {
    let color = |c: &[u8; 4]| Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
    match prim {
        Prim::Fill { points, color: c } => {
            let points = points.iter().map(|&p| to_pos(p)).collect();
            painter.add(egui::Shape::convex_polygon(points, color(c), Stroke::NONE));
        }
        Prim::Stroke { points, width, color: c, closed, round } => {
            let points: Vec<Pos2> = points.iter().map(|&p| to_pos(p)).collect();
            let stroke = Stroke::new(width * scale, color(c));
            if *closed {
                painter.add(egui::Shape::closed_line(points, stroke));
            } else {
                // egui lines have butt caps; add the round caps the export draws.
                if *round {
                    for end in [points[0], points[points.len() - 1]] {
                        painter.circle_filled(end, width * scale / 2.0, color(c));
                    }
                }
                painter.add(egui::Shape::line(points, stroke));
            }
        }
    }
}

impl eframe::App for Editor {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.keys(&ctx);
        self.guard_close(&ctx);
        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        if self.confirm_close && self.doc.is_dirty() {
            egui::Panel::bottom("unsaved").show(ui, |ui| {
                ui.label("Unsaved changes — Esc to discard, Ctrl+S to save");
            });
        }
        egui::CentralPanel::no_frame().show(ui, |ui| self.canvas(ui));
    }
}
```

Remove the Task 1 dead-code allowance from `src/main.rs`. API drift (egui 0.36: `selectable_label`, `rect_stroke`'s `StrokeKind`, `Panel::top/bottom(..).show(ui, ..)`, `CentralPanel::no_frame().show(ui, ..)`): follow the compiler, keep the behaviour, and ledger any change.

- [ ] **Step 5: Run** — `cargo nextest run` (all pass), clippy, fmt.

- [ ] **Step 6: Smoke run** (tell the user first: a window opens for a few seconds) — `cargo run -q -- edit <a recent screenshot>` in the dev shell under `timeout -s INT 4`; read the newest log: no errors. Optionally a niri screenshot shows the window.

- [ ] **Step 7: Commit** — `jj commit -m "feat: valw edit"`

---

### Task 5: Checks and checklist

- [ ] `nix flake check -L --keep-going`: all pass (the headless sway check exercises the dlopen'ed libwayland through RUNPATH).
- [ ] Append to `docs/test-checklist.md`:

```markdown

## Editor (Phase 5a)

- [ ] Clicking a preview opens `valw edit` (title `<name> — valw`); `[editor] backend = "satty"` still opens Satty.
- [ ] Each tool draws (arrow head at the release point, outline rectangle/ellipse, line, pen dot on a click, translucent highlighter); Shift snaps lines to 45° and makes squares/circles.
- [ ] Colours and S/M/L change new shapes only; A/R/O/L/P/H switch tools.
- [ ] Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y and the toolbar buttons undo and redo.
- [ ] Ctrl+S overwrites the file (the preview's file shows the edits); Ctrl+Shift+S writes `… edited.png`; "Saved" appears for 2 s.
- [ ] The saved PNG matches the canvas (same positions and widths), also for a window shot with transparent corners.
- [ ] Ctrl+C then Esc: the edited image is still pasteable afterwards.
- [ ] Esc with unsaved changes shows the bar; Esc again discards; Ctrl+S in the bar saves and closes. Closing the window with Mod+Q behaves the same.
- [ ] A large (1920×1080) shot fits the window; a small one isn't blown up; the window can be resized.
```

- [ ] Commit `docs: editor checklist`.
