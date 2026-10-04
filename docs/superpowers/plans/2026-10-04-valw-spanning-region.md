# valw spanning region Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A region drag can cross outputs; each output shows its part, and the shot is the whole rectangle stitched from every output it covers, transparent where no output is.

**Architecture:** `selection::span` turns the drag's corners (global logical coordinates, as today) into per-output parts with their place in the whole image; `stitch::stitch` assembles the RGBA image from the frozen frames; `region` draws every output's part, puts the whole-size label on the pointer's output, and returns `Choice::Region` (one output, unchanged) or the new `Choice::Span`.

**Tech Stack:** Rust 2024, `image` (`RgbaImage`, `imageops::{replace, resize}`), SCTK layer-shell (unchanged).

**Spec:** `docs/superpowers/specs/2026-10-04-valw-spanning-region-design.md`

## Global Constraints

- `jj` only (`jj commit -m "<msg>" <paths>`), never `git`; `cargo clippy --all-targets -- -D warnings`, `cargo nextest run`, `cargo fmt --check`; nix3 commands only.
- English code and comments, matching the surrounding density and idiom.
- A selection inside one output must produce exactly today's result (`Choice::Region(output, rect)`, opaque, same pixels).
- Areas of the rectangle that no output covers are alpha 0; covered pixels alpha 255.
- The whole image is at the largest scale among the outputs the rectangle covers.
- No live-session runs without telling the user first; copland edits need approval.

## Review Focus

1. A box that touches one output but also covers a gap (a corner beyond the output): it becomes a `Span` with a transparent strip, never a silently clipped `Region`. — `choose` test in Task 3.
2. Seams: two parts meet without a gap or overlap row/column for integer layouts (the user's). — `span` test in Task 1 checks offsets and sizes add up.
3. The label: the whole shot's size, on the output the pointer is over, also during a drag whose events still arrive on the starting surface. — `locate` test in Task 1, `label` logic in Task 3 uses the global pointer.
4. Mixed scales: the lower-scale part is resized to its place, not pasted at its native size. — `span` and `stitch` tests (Tasks 1, 2).
5. A click (under `MIN_PIXELS` in the whole rectangle) still clears and keeps waiting. — `choose` test in Task 3.

---

### Task 1: `selection::span` and `selection::locate`

**Files:**
- Modify: `src/selection.rs` (new types and functions after `resolve`; tests at the bottom)

**Interfaces:**
- Consumes: `Point`, `clip`, `MIN_PIXELS`, `OutputGeom`, `PixelRect`.
- Produces:
  ```rust
  pub struct Part { pub output: usize, pub rect: PixelRect, pub at: (u32, u32), pub size: (u32, u32) }
  pub struct Span { pub parts: Vec<Part>, pub size: (u32, u32) }
  impl Span { pub fn single(&self) -> Option<(usize, PixelRect)> }
  pub fn span(a: Point, b: Point, outputs: &[(&OutputGeom, (u32, u32))]) -> Option<Span>
  pub fn locate(p: Point, outputs: &[(&OutputGeom, (u32, u32))]) -> usize
  ```
  (`outputs[i]` = output `i`'s geometry and frame size in physical pixels; both derive `Debug, Clone, PartialEq`.)

- [ ] **Step 1: Write the failing tests** (append inside `mod tests`, which already has `out`, `p` and `rect` helpers)

```rust
    /// The user's layout: the laptop at the origin, the monitor above it,
    /// wider and 277 px to the left.
    fn layout() -> (OutputGeom, OutputGeom) {
        (out("LVDS-1", 0, 0, 1366, 768), out("HDMI-A-1", -277, -1080, 1920, 1080))
    }

    #[test]
    fn a_span_inside_one_output_is_todays_clip() {
        let (laptop, monitor) = layout();
        let outputs = [(&laptop, (1366, 768)), (&monitor, (1920, 1080))];
        let s = span(p(100.0, 100.0), p(300.0, 250.0), &outputs).unwrap();
        assert_eq!(s.size, (200, 150));
        assert_eq!(s.single(), Some((0, rect(100, 100, 200, 150))));
    }

    #[test]
    fn a_span_from_the_laptop_up_onto_the_monitor() {
        let (laptop, monitor) = layout();
        let outputs = [(&laptop, (1366, 768)), (&monitor, (1920, 1080))];
        let s = span(p(100.0, 500.0), p(600.0, -300.0), &outputs).unwrap();
        assert_eq!(s.size, (500, 800));
        assert_eq!(
            s.parts,
            vec![
                Part { output: 0, rect: rect(100, 0, 500, 500), at: (0, 300), size: (500, 500) },
                Part { output: 1, rect: rect(377, 780, 500, 300), at: (0, 0), size: (500, 300) },
            ]
        );
        assert_eq!(s.single(), None);
        // The parts meet without a gap or an overlap.
        assert_eq!(s.parts[1].at.1 + s.parts[1].size.1, s.parts[0].at.1);
    }

    #[test]
    fn a_span_over_a_gap_keeps_the_whole_box() {
        let (laptop, monitor) = layout();
        let outputs = [(&laptop, (1366, 768)), (&monitor, (1920, 1080))];
        let s = span(p(-200.0, -200.0), p(300.0, 300.0), &outputs).unwrap();
        assert_eq!(s.size, (500, 500));
        assert_eq!(
            s.parts,
            vec![
                Part { output: 0, rect: rect(0, 0, 300, 300), at: (200, 200), size: (300, 300) },
                Part { output: 1, rect: rect(77, 880, 500, 200), at: (0, 0), size: (500, 200) },
            ]
        );
        // One output touched, but the box reaches past it: not a plain crop.
        let corner = span(p(-50.0, 100.0), p(200.0, 300.0), &outputs).unwrap();
        assert_eq!(corner.parts.len(), 1);
        assert_eq!(corner.single(), None);
    }

    #[test]
    fn a_span_takes_the_largest_scale() {
        let a = out("A", 0, 0, 100, 100);
        let b = out("B", 100, 0, 100, 100);
        let outputs = [(&a, (200, 200)), (&b, (100, 100))];
        let s = span(p(50.0, 10.0), p(150.0, 60.0), &outputs).unwrap();
        assert_eq!(s.size, (200, 100));
        assert_eq!(
            s.parts,
            vec![
                Part { output: 0, rect: rect(100, 20, 100, 100), at: (0, 0), size: (100, 100) },
                Part { output: 1, rect: rect(0, 10, 50, 50), at: (100, 0), size: (100, 100) },
            ]
        );
    }

    #[test]
    fn a_tiny_span_is_a_click() {
        let (laptop, monitor) = layout();
        let outputs = [(&laptop, (1366, 768)), (&monitor, (1920, 1080))];
        assert_eq!(span(p(10.0, 10.0), p(11.0, 30.0), &outputs), None);
    }

    #[test]
    fn locate_finds_the_output_or_the_nearest() {
        let (laptop, monitor) = layout();
        let outputs = [(&laptop, (1366, 768)), (&monitor, (1920, 1080))];
        assert_eq!(locate(p(500.0, 500.0), &outputs), 0);
        assert_eq!(locate(p(500.0, -500.0), &outputs), 1);
        // Left of the laptop, under the monitor: the laptop is 100 px away.
        assert_eq!(locate(p(-100.0, 300.0), &outputs), 0);
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo nextest run -E 'test(span) | test(locate)'`
Expected: compile errors, `cannot find function span` / `locate`, `cannot find struct Part`.

- [ ] **Step 3: Implement** (after `pub fn resolve`)

```rust
/// One output's share of a selection and where it goes in the whole image.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub output: usize,
    /// In that output's frame, physical pixels.
    pub rect: PixelRect,
    /// Its top-left corner and size in the whole image.
    pub at: (u32, u32),
    pub size: (u32, u32),
}

/// A selection over one or more outputs: its parts and the whole image size.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub parts: Vec<Part>,
    pub size: (u32, u32),
}

impl Span {
    /// The output and rectangle when the selection is a plain crop of one
    /// output (today's region shot); `None` when it needs stitching.
    pub fn single(&self) -> Option<(usize, PixelRect)> {
        match self.parts.as_slice() {
            [p] if p.at == (0, 0) && p.size == self.size && (p.rect.width, p.rect.height) == self.size => {
                Some((p.output, p.rect))
            }
            _ => None,
        }
    }
}

/// The drag from `a` to `b` over `outputs` (each output's geometry and frame
/// size), at the largest scale among the outputs it covers. `None` for a
/// click (under [`MIN_PIXELS`]) or a rectangle over no output.
pub fn span(a: Point, b: Point, outputs: &[(&OutputGeom, (u32, u32))]) -> Option<Span> {
    let (x0, y0) = (a.x.min(b.x), a.y.min(b.y));
    let (x1, y1) = (a.x.max(b.x), a.y.max(b.y));
    let covered: Vec<(usize, PixelRect, f64)> = outputs
        .iter()
        .enumerate()
        .filter_map(|(i, (g, (w, h)))| {
            let r = clip(a, b, g, *w, *h)?;
            Some((i, r, *w as f64 / g.width as f64))
        })
        .collect();
    let scale = covered.iter().map(|c| c.2).fold(0.0, f64::max);
    if scale == 0.0 {
        return None;
    }
    let size = (((x1 - x0) * scale).ceil() as u32, ((y1 - y0) * scale).ceil() as u32);
    if size.0 < MIN_PIXELS || size.1 < MIN_PIXELS {
        return None;
    }
    let parts = covered
        .into_iter()
        .map(|(i, rect, own)| {
            let g = outputs[i].0;
            let at = (
                ((x0.max(g.x as f64) - x0) * scale).floor() as u32,
                ((y0.max(g.y as f64) - y0) * scale).floor() as u32,
            );
            let k = scale / own;
            let part = (
                ((rect.width as f64 * k).round() as u32).min(size.0 - at.0.min(size.0)),
                ((rect.height as f64 * k).round() as u32).min(size.1 - at.1.min(size.1)),
            );
            Part { output: i, rect, at, size: part }
        })
        .collect();
    Some(Span { parts, size })
}

/// The output a global point is over, or the nearest one (`outputs` is never
/// empty: there is one surface per output).
pub fn locate(p: Point, outputs: &[(&OutputGeom, (u32, u32))]) -> usize {
    let distance = |g: &OutputGeom| {
        let dx = (g.x as f64 - p.x).max(p.x - (g.x + g.width) as f64).max(0.0);
        let dy = (g.y as f64 - p.y).max(p.y - (g.y + g.height) as f64).max(0.0);
        dx * dx + dy * dy
    };
    (0..outputs.len())
        .min_by(|&i, &j| distance(outputs[i].0).total_cmp(&distance(outputs[j].0)))
        .unwrap_or(0)
}
```

- [ ] **Step 4: Run them to verify they pass**

Run: `cargo fmt && cargo nextest run -E 'test(selection)'`
Expected: all selection tests pass (the existing ones and the 6 new ones).

- [ ] **Step 5: Commit**

```bash
jj commit -m "selection: span a rectangle over several outputs, locate a point" src/selection.rs
```

(`span`/`locate` are unused until Task 3: clippy's dead-code lint fails on this commit alone, like Task 1 of the plugin plan; Task 3 makes it clean.)

---

### Task 2: `stitch`

**Files:**
- Create: `src/stitch.rs`
- Modify: `src/main.rs` (`mod stitch;` in the module list, alphabetical)

**Interfaces:**
- Consumes: Task 1's `Span`, `Part`; `frame::{Frame, OutputGeom}`, `Frame::to_rgba`.
- Produces: `pub fn stitch(frames: &[Frame], span: &Span) -> RgbaImage`.

- [ ] **Step 1: Write the failing tests** (`src/stitch.rs`, tests first; the function body comes in Step 3)

```rust
//! A selection over several outputs as one image.

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
            output: OutputGeom { name: "X".into(), x: 0, y: 0, width: w as i32, height: h as i32 },
            pixels: Arc::new(RgbaImage::from_pixel(w, h, image::Rgba([rgb[2], rgb[1], rgb[0], 0]))),
        }
    }

    fn rect(x: u32, y: u32, width: u32, height: u32) -> PixelRect {
        PixelRect { x, y, width, height }
    }

    #[test]
    fn parts_land_in_place_and_gaps_stay_clear() {
        let frames = [frame(10, 10, [255, 0, 0]), frame(10, 10, [0, 0, 255])];
        let span = Span {
            size: (8, 6),
            parts: vec![
                Part { output: 0, rect: rect(0, 0, 4, 3), at: (4, 3), size: (4, 3) },
                Part { output: 1, rect: rect(2, 2, 8, 3), at: (0, 0), size: (8, 3) },
            ],
        };
        let img = stitch(&frames, &span);
        assert_eq!(img.dimensions(), (8, 6));
        assert_eq!(img.get_pixel(0, 0).0, [0, 0, 255, 255], "the monitor's part, opaque");
        assert_eq!(img.get_pixel(7, 2).0, [0, 0, 255, 255]);
        assert_eq!(img.get_pixel(5, 4).0, [255, 0, 0, 255], "the laptop's part");
        assert_eq!(img.get_pixel(1, 4).0[3], 0, "no output there: transparent");
    }

    #[test]
    fn a_smaller_scale_part_is_resized_to_its_place() {
        let frames = [frame(4, 4, [0, 255, 0])];
        let span = Span {
            size: (8, 8),
            parts: vec![Part { output: 0, rect: rect(0, 0, 4, 4), at: (0, 0), size: (8, 8) }],
        };
        let img = stitch(&frames, &span);
        assert_eq!(img.get_pixel(7, 7).0, [0, 255, 0, 255], "filled to the corner");
    }
}
```

and add `mod stitch;` to `src/main.rs` between `mod stack;` and `mod theme;`.

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo nextest run -E 'test(stitch)'`
Expected: compile error, `cannot find function stitch`.

- [ ] **Step 3: Implement** (above `#[cfg(test)]` in `src/stitch.rs`)

```rust
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
            crop = imageops::resize(&crop, part.size.0, part.size.1, imageops::FilterType::Triangle);
        }
        imageops::replace(&mut image, &crop, part.at.0 as i64, part.at.1 as i64);
    }
    image
}
```

- [ ] **Step 4: Run them to verify they pass**

Run: `cargo fmt && cargo nextest run -E 'test(stitch)'`
Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
jj commit -m "stitch: a selection over several outputs as one image" src/stitch.rs src/main.rs
```

---

### Task 3: The overlay across outputs, `Choice::Span`, the capture

**Files:**
- Modify: `src/region.rs` (`Choice`, `Overlay` fields, `Surface` fields, `selection_on`, new `label_on`/`outputs`/`redraw_changed`/`choose`, `pointer`, `apply_mods`, `redraw`; a `tests` module)
- Modify: `src/main.rs` (the `Mode::Region` arm)
- Modify: `docs/test-checklist.md` (a "Region across monitors" section)

**Interfaces:**
- Consumes: Task 1's `span`, `locate`, `Span::single`; Task 2's `stitch`.
- Produces: `Choice::Span(selection::Span, usize)` (the span and the output the pointer was over at release, for the preview).

- [ ] **Step 1: Write the failing tests** (append to `src/region.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn out(name: &str, x: i32, y: i32, width: i32, height: i32) -> OutputGeom {
        OutputGeom { name: name.into(), x, y, width, height }
    }

    fn p(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    #[test]
    fn one_output_is_todays_region_and_several_a_span() {
        let laptop = out("LVDS-1", 0, 0, 1366, 768);
        let monitor = out("HDMI-A-1", -277, -1080, 1920, 1080);
        let outputs = [(&laptop, (1366, 768)), (&monitor, (1920, 1080))];
        assert_eq!(
            choose(p(100.0, 100.0), p(300.0, 250.0), &outputs, p(300.0, 250.0)),
            Some(Choice::Region(
                0,
                PixelRect { x: 100, y: 100, width: 200, height: 150 }
            ))
        );
        let across = choose(p(100.0, 500.0), p(600.0, -300.0), &outputs, p(600.0, -300.0));
        let Some(Choice::Span(span, on)) = across else {
            panic!("expected a span, got {across:?}");
        };
        assert_eq!((span.size, span.parts.len(), on), ((500, 800), 2, 1), "preview on the monitor");
        // A box touching only the laptop but reaching past its left edge.
        assert!(matches!(
            choose(p(-50.0, 100.0), p(200.0, 300.0), &outputs, p(200.0, 300.0)),
            Some(Choice::Span(_, 0))
        ));
        assert_eq!(choose(p(10.0, 10.0), p(11.0, 30.0), &outputs, p(11.0, 30.0)), None, "a click");
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo nextest run -E 'test(one_output_is)'`
Expected: compile errors, `cannot find function choose`, no variant `Choice::Span`.

- [ ] **Step 3: Implement**

In `src/region.rs`:

1. Imports: `use crate::frame::{OutputGeom, PixelRect};` (add `OutputGeom` to the existing `crate::frame` import).
2. `Choice` gains:
   ```rust
       /// A rectangle over several outputs (or past one's edge), with the
       /// output the pointer was over at release (for the preview).
       Span(selection::Span, usize),
   ```
3. `Overlay`: remove `anchor`; replace `pointer: Option<(usize, (f64, f64))>` with
   ```rust
       /// The pointer in global logical coordinates. A drag's events keep
       /// arriving on the surface it started on, with positions past its
       /// edges, so this stays right across outputs.
       pointer: Option<Point>,
   ```
   and drop `anchor: None,` from the constructor in `select`.
4. `Surface` gains (and `Surface::new` sets them to `None` / `false`):
   ```rust
       /// The selection part and whether the label were last drawn here.
       shown: Option<PixelRect>,
       labelled: bool,
   ```
5. Replace `selection_on`, `redraw` and `apply_mods`'s body, and add the helpers:
   ```rust
       fn outputs(&self) -> Vec<(&OutputGeom, (u32, u32))> {
           self.surfaces
               .iter()
               .map(|s| (&s.geom, (s.width, s.height)))
               .collect()
       }

       /// The selection's part on surface `i`.
       fn selection_on(&self, i: usize) -> Option<PixelRect> {
           let (start, end) = self.selection.corners()?;
           let s = &self.surfaces[i];
           selection::clip(start, end, &s.geom, s.width, s.height)
       }

       /// The size label on surface `i`: the whole shot's size next to the
       /// pointer, on the output the pointer is over.
       fn label_on(&self, i: usize) -> Option<(PixelRect, (f64, f64))> {
           let (start, end) = self.selection.corners()?;
           let p = self.pointer?;
           let outputs = self.outputs();
           if selection::locate(p, &outputs) != i {
               return None;
           }
           let (width, height) = selection::span(start, end, &outputs)?.size;
           let g = &self.surfaces[i].geom;
           Some((
               PixelRect { x: 0, y: 0, width, height },
               (p.x - g.x as f64, p.y - g.y as f64),
           ))
       }

       fn redraw(&mut self, i: usize, qh: &QueueHandle<State>) {
           let sel = self.selection_on(i);
           let label = self.label_on(i);
           let s = &mut self.surfaces[i];
           s.shown = sel;
           s.labelled = label.is_some();
           s.dirty = true;
           if s.configured && !s.frame_pending {
               s.draw(sel, label, qh);
           }
       }

       /// Redraws the surfaces whose part of the selection or label changed.
       fn redraw_changed(&mut self, qh: &QueueHandle<State>) {
           for i in 0..self.surfaces.len() {
               let s = &self.surfaces[i];
               if self.selection_on(i) != s.shown || s.labelled || self.label_on(i).is_some() {
                   self.redraw(i, qh);
               }
           }
       }
   ```
   `apply_mods` becomes `self.selection.set_mods(self.mods); self.redraw_changed(qh);`.
6. Add the pure decision (a free function above `impl Surface`):
   ```rust
   /// What a finished drag from `a` to `b` captures: one output's crop as
   /// before, or the whole rectangle to stitch; `None` for a click.
   fn choose(
       a: Point,
       b: Point,
       outputs: &[(&OutputGeom, (u32, u32))],
       pointer: Point,
   ) -> Option<Choice> {
       let span = selection::span(a, b, outputs)?;
       Some(match span.single() {
           Some((i, rect)) => Choice::Region(i, rect),
           None => Choice::Span(span, selection::locate(pointer, outputs)),
       })
   }
   ```
7. `pointer()`: after computing `p`, set `self.pointer = Some(p);` (instead of the old `(i, position)`). `Press(BTN_LEFT)`: `self.selection.press_with(p, self.mods); self.redraw_changed(qh);`. `Motion`: `self.selection.motion(p); self.redraw_changed(qh);`. `Release(BTN_LEFT)`:
   ```rust
                   let Some((start, end)) = self.selection.release(p) else {
                       continue;
                   };
                   let choice = choose(start, end, &self.outputs(), p);
                   match choice {
                       Some(choice) => {
                           tracing::info!("selected {choice:?} (logical {start:?} to {end:?})");
                           self.outcome = Some(Some(choice));
                       }
                       // A click, not a drag: clear it and keep waiting.
                       None => self.redraw_changed(qh),
                   }
   ```
   (`i` from `index_of` is still needed for the geometry of the event's surface.)

8. In `src/selection.rs`, remove `resolve` and its test (its only caller was the old release code; the click threshold now lives in `span`, tested by `a_tiny_span_is_a_click`). Update the `MIN_PIXELS` doc comment if it names `resolve`.

In `src/main.rs`, the `Mode::Region` arm gets:
```rust
                region::Choice::Span(span, on) => {
                    let source = frames[on].output.name.clone();
                    (vec![(stitch::stitch(&frames, &span), None)], 0, source)
                }
```

In `docs/test-checklist.md`, add after the Polish section's items (before its "Known minors"):
```markdown
- [ ] Region across monitors: drag from the laptop up onto the monitor and back; each screen shows its part, the label shows the whole size on the screen under the pointer; Shift, Alt and Space work across the edge; the PNG is the whole rectangle, seams exact, parts over no screen transparent; the preview appears on the screen where you let go; a box on one screen is exactly as before.
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo fmt && cargo nextest run && cargo clippy --all-targets -- -D warnings`
Expected: the whole suite passes (the new region test included); clippy clean (Tasks 1–2's functions are now used).

- [ ] **Step 5: Commit**

```bash
jj commit -m "region: a selection across monitors, stitched into one image" src/region.rs src/main.rs docs/test-checklist.md
```
