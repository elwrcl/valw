# valw polish phase Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Region size label and Shift/Alt/Space modifiers (P1), an optional window shadow (P2), and the synthesised combo shutter sound (P3).

**Architecture:** Each feature is a pure core with tests (`selection::Selection` modifier math, `render::label_origin`/`blend`, `shadow::add`, `sound::synth`, `sound::combo`) plus thin wiring (`region.rs` overlay, `main.rs` capture flow, `toolbar`, Nix wrapper).

**Tech Stack:** Rust 2024, SCTK 0.21, tiny-skia 0.12, `image` 0.25; `pw-play` (PipeWire) at run time.

**Spec:** `docs/superpowers/specs/2026-10-03-valw-polish-design.md`

## Global Constraints

- jj only; `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo nextest run`, `nix flake check`.
- Label: 13 px white on `#1c1c1e` 90 %, 6 px padding, 16 logical px from the pointer, flips at edges. Shadow: 40 px padding, alpha 50 %, blur radius 20 (3 box passes), 12 px down. Sound: 44 100 Hz mono 16-bit; speeds 1.00/1.12/1.25/1.40/1.60, 6–7 at 1.80 + Shepard glide 0→½ and ½→1 octave over 400 ms; combo resets after `combo_reset_secs` (default 5, 1–60); `volume` default 0.6 (0–1).
- No Caps Lock bindings.
- Tell the user before playing sounds or showing the overlay on the live session.

## Review Focus

1. **Modifier changes mid-drag** (Shift/Space pressed and released in any order, Alt toggling) never make the rectangle jump or flip unexpectedly. Tests in `selection::tests` (Task 1).
2. **The label never leaves the output** and never covers the pointer. Test: `render::tests::label_flips_at_edges` (Task 2).
3. **Shadow on images with and without alpha** (a fully opaque shot still gets a rectangular shadow). Tests in `shadow::tests` (Task 3).
4. **No clicks/pops or clipping in the sounds** and a seamless 6↔7 glide. Tests in `sound::synth::tests` (Task 4); listening in the checklist.
5. **A missing `pw-play` or audio server never fails a capture.** Task 5 (warning only).

---

## File Structure

| File | Change | Task |
|---|---|---|
| `src/selection.rs` | `Mods`, modifier-aware `Selection` | 1 |
| `src/region.rs`, `src/wayland.rs` | modifiers and Space to the overlay | 1 |
| `src/render.rs`, `src/region.rs` | size label | 2 |
| `src/shadow.rs`, `src/config.rs`, `src/main.rs` | window shadow | 3 |
| `src/sound/{mod,synth,combo}.rs` | sounds and combo | 4, 5 |
| `src/config.rs`, `src/main.rs`, `src/toolbar/*`, `nix/package.ulu.nix` | playing, settings, toolbar option, `pw-play` | 5 |
| `docs/test-checklist.md` | polish section | 6 |

---

### Task 1: Modifier-aware selection

**Files:** `src/selection.rs`, `src/region.rs`, `src/wayland.rs`.

**Produces:** `selection::Mods { shift, alt, space }`; `Selection::{press(p), motion(p), set_mods(Mods), corners() -> Option<(Point, Point)>, release(p) -> Option<(Point, Point)>, allows_window_switch()}`; `Overlay::{modifiers(Modifiers), space(bool)}`.

- [ ] **Step 1: Failing tests** — add to `selection.rs` tests:

```rust
    fn pt(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    const SHIFT: Mods = Mods { shift: true, alt: false, space: false };
    const ALT: Mods = Mods { shift: false, alt: true, space: false };
    const SPACE: Mods = Mods { shift: false, alt: false, space: true };

    #[test]
    fn plain_drag_is_press_to_pointer() {
        let mut s = Selection::default();
        s.press(pt(10.0, 10.0));
        s.motion(pt(50.0, 30.0));
        assert_eq!(s.corners(), Some((pt(10.0, 10.0), pt(50.0, 30.0))));
    }

    #[test]
    fn shift_locks_the_dimension_that_moves_first() {
        let mut s = Selection::default();
        s.press(pt(0.0, 0.0));
        s.motion(pt(40.0, 30.0));
        s.set_mods(SHIFT);
        s.motion(pt(60.0, 32.0)); // mostly horizontal: width follows
        assert_eq!(s.corners(), Some((pt(0.0, 0.0), pt(60.0, 30.0))));
        s.motion(pt(70.0, 90.0)); // the axis stays decided
        assert_eq!(s.corners(), Some((pt(0.0, 0.0), pt(70.0, 30.0))));
        s.set_mods(Mods::default());
        s.motion(pt(70.0, 90.0));
        assert_eq!(s.corners(), Some((pt(0.0, 0.0), pt(70.0, 90.0))), "plain again");

        let mut v = Selection::default();
        v.press(pt(0.0, 0.0));
        v.motion(pt(40.0, 30.0));
        v.set_mods(SHIFT);
        v.motion(pt(41.0, 80.0)); // mostly vertical: height follows
        assert_eq!(v.corners(), Some((pt(0.0, 0.0), pt(40.0, 80.0))));
    }

    #[test]
    fn alt_grows_from_the_centre() {
        let mut s = Selection::default();
        s.press(pt(100.0, 100.0));
        s.set_mods(ALT);
        s.motion(pt(130.0, 120.0));
        assert_eq!(s.corners(), Some((pt(70.0, 80.0), pt(130.0, 120.0))));
    }

    #[test]
    fn shift_and_alt_combine() {
        let mut s = Selection::default();
        s.press(pt(100.0, 100.0));
        s.motion(pt(120.0, 110.0));
        s.set_mods(Mods { shift: true, alt: true, space: false });
        s.motion(pt(150.0, 112.0));
        assert_eq!(s.corners(), Some((pt(50.0, 90.0), pt(150.0, 110.0))));
    }

    #[test]
    fn space_moves_then_resizing_continues_without_a_jump() {
        let mut s = Selection::default();
        s.press(pt(0.0, 0.0));
        s.motion(pt(40.0, 30.0));
        s.set_mods(SPACE);
        s.motion(pt(50.0, 40.0));
        assert_eq!(s.corners(), Some((pt(10.0, 10.0), pt(50.0, 40.0))));
        s.motion(pt(60.0, 40.0));
        assert_eq!(s.corners(), Some((pt(20.0, 10.0), pt(60.0, 40.0))));
        s.set_mods(Mods::default());
        s.motion(pt(60.0, 40.0));
        assert_eq!(s.corners(), Some((pt(20.0, 10.0), pt(60.0, 40.0))), "no jump on release");
        s.motion(pt(70.0, 45.0));
        assert_eq!(s.corners(), Some((pt(20.0, 10.0), pt(70.0, 45.0))));
    }

    #[test]
    fn space_with_a_pointer_away_from_the_corner_keeps_the_offset() {
        let mut s = Selection::default();
        s.press(pt(0.0, 0.0));
        s.motion(pt(40.0, 30.0));
        s.set_mods(SPACE);
        s.motion(pt(45.0, 30.0));
        s.set_mods(Mods::default());
        s.motion(pt(55.0, 40.0));
        assert_eq!(s.corners(), Some((pt(5.0, 0.0), pt(55.0, 40.0))));
    }

    #[test]
    fn release_applies_the_held_modifiers() {
        let mut s = Selection::default();
        s.press(pt(100.0, 100.0));
        s.set_mods(ALT);
        assert_eq!(s.release(pt(110.0, 105.0)), Some((pt(90.0, 95.0), pt(110.0, 105.0))));
        assert_eq!(s, Selection::Idle);
    }
```

Run `cargo nextest run selection::` → compile errors.

- [ ] **Step 2: Implement** — replace `Selection` (keep `Point`, `clip`, `resolve`, `MIN_PIXELS`):

```rust
/// Modifier keys that shape a selection while dragging.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub alt: bool,
    pub space: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Axis {
    X,
    Y,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drag {
    /// The press point (the centre with Alt); Space moves it.
    start: Point,
    /// The far corner before Alt is applied.
    end: Point,
    /// The last pointer position.
    last: Point,
    /// Added to the pointer to get `end` (non-zero after a Space move).
    offset: (f64, f64),
    mods: Mods,
    /// Shift: `end` when Shift went down, and the axis that follows.
    lock: Option<(Point, Option<Axis>)>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum Selection {
    #[default]
    Idle,
    Dragging(Drag),
}

impl Selection {
    pub fn press(&mut self, p: Point) {
        let mods = match self {
            Selection::Dragging(d) => d.mods,
            Selection::Idle => Mods::default(),
        };
        *self = Selection::Dragging(Drag { start: p, end: p, last: p, offset: (0.0, 0.0), mods, lock: None });
    }

    pub fn motion(&mut self, p: Point) {
        let Selection::Dragging(d) = self else { return };
        if d.mods.space {
            let (dx, dy) = (p.x - d.last.x, p.y - d.last.y);
            d.start = Point { x: d.start.x + dx, y: d.start.y + dy };
            d.end = Point { x: d.end.x + dx, y: d.end.y + dy };
        } else {
            let target = Point { x: p.x + d.offset.0, y: p.y + d.offset.1 };
            d.end = match &mut d.lock {
                Some((frozen, axis)) => {
                    let axis = *axis.get_or_insert_with(|| {
                        if (target.x - frozen.x).abs() >= (target.y - frozen.y).abs() { Axis::X } else { Axis::Y }
                    });
                    match axis {
                        Axis::X => Point { x: target.x, y: frozen.y },
                        Axis::Y => Point { x: frozen.x, y: target.y },
                    }
                }
                None => target,
            };
        }
        d.last = p;
    }

    /// The keyboard's modifiers changed (also while idle, for the next drag).
    pub fn set_mods(&mut self, mods: Mods) {
        let Selection::Dragging(d) = self else { return };
        if mods.shift && !d.mods.shift {
            d.lock = Some((d.end, None));
        } else if !mods.shift {
            d.lock = None;
        }
        if d.mods.space && !mods.space {
            // Resize again from where the corner is, not where the pointer is.
            d.offset = (d.end.x - d.last.x, d.end.y - d.last.y);
        }
        d.mods = mods;
    }

    /// The selection's corners in global logical coordinates.
    pub fn corners(&self) -> Option<(Point, Point)> {
        let Selection::Dragging(d) = self else { return None };
        if d.mods.alt {
            let opposite = Point { x: 2.0 * d.start.x - d.end.x, y: 2.0 * d.start.y - d.end.y };
            Some((opposite, d.end))
        } else {
            Some((d.start, d.end))
        }
    }

    /// Ends the drag at `p` and returns its corners. Back to `Idle` either way.
    pub fn release(&mut self, p: Point) -> Option<(Point, Point)> {
        self.motion(p);
        let corners = self.corners();
        *self = Selection::Idle;
        corners
    }

    /// Space switches to window mode only before a drag starts.
    pub fn allows_window_switch(&self) -> bool {
        *self == Selection::Idle
    }
}
```

Note: with Shift held while idle, the first drag starts without a lock; `set_mods` before the press stores nothing (the overlay passes the current modifiers to `set_mods` right after `press`, see below). Update the older selection tests that used `Selection::Dragging { .. }` patterns to use `corners()`.

- [ ] **Step 3: Overlay wiring** — `region.rs`: add `mods: Mods` to `Overlay` (default); `pub fn modifiers(&mut self, m: Modifiers)` → `self.mods.shift = m.shift; self.mods.alt = m.alt;` then `self.selection.set_mods(self.mods)` and redraw the anchor; `pub fn space(&mut self, down: bool)` → if `down && self.selection.allows_window_switch()` → `switch_to_window()`, else `self.mods.space = down; self.selection.set_mods(self.mods)`; after `self.selection.press(p)` call `self.selection.set_mods(self.mods)` so modifiers already held apply; `selection_on` uses `self.selection.corners()`.

`wayland.rs`: `press_key` Space → `overlay.space(true)` (replacing `switch_to_window`); `release_key` (name its `KeyEvent` argument) → `if event.keysym == Keysym::space && let Some(o) = &mut self.overlay { o.space(false) }`; `update_modifiers` → also `if let Some(o) = &mut self.overlay { o.modifiers(modifiers) }`.

- [ ] **Step 4: Run** (+7), clippy, fmt. **Step 5: Commit** `feat: Shift, Alt and Space while selecting a region`.

---

### Task 2: The size label

**Files:** `src/render.rs`, `src/region.rs`.

**Produces:** `render::label_origin(pointer: (f64, f64), label: (u32, u32), surface: (u32, u32), gap: f64) -> (u32, u32)` (physical px); `render::blend(dst: &mut [u8], dst_width: u32, src: &tiny_skia::Pixmap, at: (u32, u32))` (premultiplied over XRGB, clipped); `render::size_text(rect: PixelRect) -> String` (`"640 × 480"`).

- [ ] **Step 1: Failing tests** — `render.rs` tests:

```rust
    #[test]
    fn label_sits_below_right_of_the_pointer() {
        assert_eq!(label_origin((100.0, 100.0), (80, 24), (1920, 1080), 16.0), (116, 116));
    }

    #[test]
    fn label_flips_at_edges() {
        assert_eq!(label_origin((1900.0, 100.0), (80, 24), (1920, 1080), 16.0), (1900 - 16 - 80, 116));
        assert_eq!(label_origin((100.0, 1070.0), (80, 24), (1920, 1080), 16.0), (116, 1070 - 16 - 24));
        assert_eq!(label_origin((1900.0, 1070.0), (80, 24), (1920, 1080), 16.0), (1804, 1030));
        // Tiny outputs: never negative.
        assert_eq!(label_origin((5.0, 5.0), (80, 24), (60, 20), 16.0), (0, 0));
    }

    #[test]
    fn size_text_uses_the_multiplication_sign() {
        let r = PixelRect { x: 3, y: 4, width: 640, height: 480 };
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
        assert_eq!(px(&dst, w, 2, 0)[..3], [100, 100, 100], "left of it untouched");
        let mut half = tiny_skia::Pixmap::new(1, 1).unwrap();
        half.fill(tiny_skia::Color::from_rgba8(0, 0, 0, 128));
        blend(&mut dst, w, &half, (0, 1));
        assert!((px(&dst, w, 0, 1)[0] as i32 - 50).abs() <= 1, "half black over 100");
    }
```

(`PixelRect` is available in tests through the existing `#[cfg(test)] use`.) Run → compile errors.

- [ ] **Step 2: Implement** (in `render.rs`, non-test):

```rust
/// Where a `label`-sized box goes next to the pointer: `gap` px right of and
/// below it, or on the other side where it would leave the surface.
pub fn label_origin(pointer: (f64, f64), label: (u32, u32), surface: (u32, u32), gap: f64) -> (u32, u32) {
    let place = |p: f64, len: u32, max: u32| {
        let after = p + gap;
        let v = if after + len as f64 <= max as f64 { after } else { p - gap - len as f64 };
        v.clamp(0.0, max.saturating_sub(len) as f64) as u32
    };
    (place(pointer.0, label.0, surface.0), place(pointer.1, label.1, surface.1))
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
```

`region.rs`: `Overlay` gains `pointer: Option<(usize, (f64, f64))>` (surface index and surface-local logical position) updated on Enter/Motion. In `Surface::draw` (the caller passes the label): after `render::compose`, if there is a selection on this surface and the pointer is on it, build `let scale = s.width as f64 / s.geom.width as f64;` `let text = render::size_text(rect);` `let size = ((crate::toolbar::draw::measure(&text) + 12.0) * scale).ceil() as u32` wide and `((crate::toolbar::draw::LABEL * 1.2 + 12.0) * scale).ceil() as u32` tall; `let pill = crate::toolbar::draw::pill(size, scale as f32, &text);` then `render::blend(canvas, width, &pill, render::label_origin((px * scale, py * scale), (pill.width(), pill.height()), (width, height), 16.0 * scale))`. Pass the label into `draw` as an `Option<(String, (f64, f64))>` computed by `redraw`.

- [ ] **Step 3: Run** (+4), clippy, fmt. **Step 4: Smoke** (tell the user): not possible without dragging; covered by the checklist. **Step 5: Commit** `feat: size label while selecting a region`.

---

### Task 3: Window shadow

**Files:** create `src/shadow.rs`; `src/config.rs` (`Capture.window_shadow: bool`, default false); `src/main.rs` (`mod shadow;`, apply in `window_shot`).

**Produces:** `shadow::add(image: &RgbaImage) -> RgbaImage`; `PAD = 40`, `OFFSET_Y = 12`, `BLUR = 20`, `OPACITY = 0.5`.

- [ ] **Step 1: Failing tests** (`shadow.rs`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn window() -> RgbaImage {
        RgbaImage::from_pixel(100, 60, Rgba([200, 100, 50, 255]))
    }

    #[test]
    fn the_image_grows_by_the_padding() {
        let out = add(&window());
        assert_eq!(out.dimensions(), (100 + 2 * PAD, 60 + 2 * PAD));
    }

    #[test]
    fn the_window_is_unchanged_in_the_middle() {
        let out = add(&window());
        assert_eq!(out.get_pixel(PAD + 50, PAD + 30).0, [200, 100, 50, 255]);
    }

    #[test]
    fn the_shadow_is_below_and_fades_out() {
        let out = add(&window());
        let below = out.get_pixel(PAD + 50, PAD + 60 + 8).0;
        let above = out.get_pixel(PAD + 50, PAD - 8).0;
        assert_eq!(&below[..3], &[0, 0, 0]);
        assert!(below[3] > 40, "{below:?}");
        assert!(below[3] > above[3], "offset down: {below:?} vs {above:?}");
        assert_eq!(out.get_pixel(0, 0).0[3], 0, "far corner clear");
    }

    #[test]
    fn rounded_corners_are_followed() {
        let mut w = window();
        for (x, y) in [(0, 0), (1, 0), (0, 1)] {
            w.put_pixel(x, y, Rgba([0, 0, 0, 0]));
        }
        let out = add(&w);
        assert_eq!(out.get_pixel(PAD, PAD).0[3] < 255, true, "the transparent corner stays see-through");
    }
}
```

Run → compile errors.

- [ ] **Step 2: Implement**

```rust
//! A macOS-style soft shadow around window shots.

use image::{Rgba, RgbaImage};

pub const PAD: u32 = 40;
pub const OFFSET_Y: u32 = 12;
pub const BLUR: u32 = 20;
pub const OPACITY: f32 = 0.5;

/// `image` on a transparent canvas `PAD` px larger on every side, over a
/// blurred, offset copy of its own alpha in black.
pub fn add(image: &RgbaImage) -> RgbaImage {
    let (w, h) = image.dimensions();
    let (cw, ch) = (w + 2 * PAD, h + 2 * PAD);
    let mut alpha = vec![0f32; (cw * ch) as usize];
    for (x, y, p) in image.enumerate_pixels() {
        let (sx, sy) = (x + PAD, y + PAD + OFFSET_Y);
        if sy < ch {
            alpha[(sy * cw + sx) as usize] = p.0[3] as f32 / 255.0;
        }
    }
    // Three box blurs approximate a Gaussian of about BLUR / 2 sigma.
    let r = (BLUR / 3).max(1) as usize;
    for _ in 0..3 {
        alpha = box_blur(&alpha, cw as usize, ch as usize, r);
    }
    let mut out = RgbaImage::from_fn(cw, ch, |x, y| {
        let a = (alpha[(y * cw + x) as usize] * OPACITY * 255.0).round() as u8;
        Rgba([0, 0, 0, a])
    });
    for (x, y, p) in image.enumerate_pixels() {
        let dst = out.get_pixel_mut(x + PAD, y + PAD);
        *dst = over(*p, *dst);
    }
    out
}

/// Straight-alpha `top` over `bottom`.
fn over(top: Rgba<u8>, bottom: Rgba<u8>) -> Rgba<u8> {
    let ta = top.0[3] as f32 / 255.0;
    let ba = bottom.0[3] as f32 / 255.0;
    let a = ta + ba * (1.0 - ta);
    if a <= 0.0 {
        return Rgba([0, 0, 0, 0]);
    }
    let mix = |t: u8, b: u8| ((t as f32 * ta + b as f32 * ba * (1.0 - ta)) / a).round() as u8;
    Rgba([mix(top.0[0], bottom.0[0]), mix(top.0[1], bottom.0[1]), mix(top.0[2], bottom.0[2]), (a * 255.0).round() as u8])
}

/// A separable box blur of radius `r` (horizontal then vertical).
fn box_blur(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let n = (2 * r + 1) as f32;
    let mut tmp = vec![0f32; src.len()];
    for y in 0..h {
        let row = &src[y * w..(y + 1) * w];
        let mut sum: f32 = (0..=r).map(|i| row.get(i).copied().unwrap_or(0.0)).sum();
        for x in 0..w {
            tmp[y * w + x] = sum / n;
            if let Some(v) = row.get(x + r + 1) { sum += v; }
            if x >= r { sum -= row[x - r]; }
        }
    }
    let mut out = vec![0f32; src.len()];
    for x in 0..w {
        let at = |y: usize| tmp[y * w + x];
        let mut sum: f32 = (0..=r.min(h - 1)).map(at).sum();
        for y in 0..h {
            out[y * w + x] = sum / n;
            if y + r + 1 < h { sum += at(y + r + 1); }
            if y >= r { sum -= at(y - r); }
        }
    }
    out
}
```

`config.rs`: `Capture { show_cursor, window_shadow }` (default false); `main.rs`: `window_shot(outputs, cursor, shadow: bool)` applies `shadow::add(&image)` when `shadow`; both call sites pass `config.capture.window_shadow`.

- [ ] **Step 3: Run** (+4 and a config default check added to `partial_file_keeps_other_defaults`: `assert!(!config.capture.window_shadow)`), clippy, fmt. **Step 4: Commit** `feat: optional shadow around window shots`.

---

### Task 4: Synthesised sounds and the combo

**Files:** create `src/sound/mod.rs` (`pub mod combo; pub mod synth;`), `src/sound/synth.rs`, `src/sound/combo.rs`; `src/main.rs`: `mod sound;` (+ a temporary dead-code allowance until Task 5).

**Produces:** `synth::{RATE, shutter() -> Vec<f32>, speed(&[f32], f32) -> Vec<f32>, partials(position: f32) -> Vec<(f32, f32)>, glide(from: f32, to: f32, secs: f32) -> Vec<f32>, sound(n: u8) -> Vec<f32>, wav(&[f32]) -> Vec<u8>}`; `combo::{next(prev: Option<(u8, u64)>, now_ms: u64, reset_ms: u64) -> u8, load(&Path) -> Option<(u8, u64)>, save(&Path, u8, u64) -> Result<()>}`.

- [ ] **Step 1: Failing tests**

`synth.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn peak(s: &[f32]) -> f32 {
        s.iter().fold(0.0, |m, v| m.max(v.abs()))
    }

    #[test]
    fn the_shutter_is_short_and_never_clips() {
        let s = shutter();
        let ms = s.len() as f32 * 1000.0 / RATE as f32;
        assert!((140.0..=160.0).contains(&ms), "{ms} ms");
        assert!(peak(&s) <= 0.95 && peak(&s) > 0.3);
        assert!(s.last().unwrap().abs() < 0.01, "ends quietly (no pop)");
    }

    #[test]
    fn faster_is_shorter() {
        let s = shutter();
        assert_eq!(speed(&s, 1.6).len(), (s.len() as f32 / 1.6).ceil() as usize);
    }

    #[test]
    fn sounds_escalate_then_loop() {
        let lens: Vec<usize> = (1..=5).map(|n| sound(n).len()).collect();
        assert!(lens.windows(2).all(|w| w[1] < w[0]), "{lens:?}");
        assert!(sound(6).len() > sound(5).len(), "6 adds the glide");
        assert_eq!(sound(6).len(), sound(7).len());
        for n in 1..=7 {
            assert!(peak(&sound(n)) <= 0.95, "sound {n} clips");
        }
    }

    #[test]
    fn the_glide_returns_to_its_start_an_octave_up() {
        let start = partials(0.0);
        let end = partials(1.0);
        for (f, a) in start.iter().filter(|p| p.1 > 0.01) {
            let twin = end.iter().find(|p| (p.0 - f).abs() < 0.01).expect("same frequency an octave later");
            assert!((twin.1 - a).abs() < 1e-3, "{f} Hz: {a} vs {}", twin.1);
        }
    }

    #[test]
    fn wav_header() {
        let w = wav(&[0.0, 0.5, -0.5]);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(&w[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(w[24..28].try_into().unwrap()), RATE);
        assert_eq!(w.len(), 44 + 3 * 2);
        assert_eq!(i16::from_le_bytes(w[46..48].try_into().unwrap()), 16383);
    }
}
```

`combo.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_up_then_alternates_six_and_seven() {
        let mut prev = None;
        let mut seen = Vec::new();
        for i in 0..11u64 {
            let n = next(prev, i * 1000, 5000);
            seen.push(n);
            prev = Some((n, i * 1000));
        }
        assert_eq!(seen, [1, 2, 3, 4, 5, 6, 7, 6, 7, 6, 7]);
    }

    #[test]
    fn a_pause_resets() {
        assert_eq!(next(Some((4, 1000)), 7000, 5000), 1);
        assert_eq!(next(Some((4, 1000)), 5999, 5000), 5);
        assert_eq!(next(None, 0, 5000), 1);
        assert_eq!(next(Some((3, 9000)), 1000, 5000), 1, "a clock going backwards");
    }

    #[test]
    fn state_file_round_trip_and_broken_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("valw-combo");
        assert_eq!(load(&path), None);
        save(&path, 6, 123456).unwrap();
        assert_eq!(load(&path), Some((6, 123456)));
        std::fs::write(&path, "garbage").unwrap();
        assert_eq!(load(&path), None);
        std::fs::write(&path, "9 5").unwrap();
        assert_eq!(load(&path), None, "out of range");
    }
}
```

Run → compile errors.

- [ ] **Step 2: Implement `synth.rs`**

```rust
//! The shutter sounds, synthesised: no samples, no licences.

use std::f32::consts::TAU;

pub const RATE: u32 = 44_100;
const SPEEDS: [f32; 5] = [1.00, 1.12, 1.25, 1.40, 1.60];
const LOOP_SPEED: f32 = 1.80;
const GLIDE_SECS: f32 = 0.4;
/// The Shepard tone: octave-spaced partials from BASE Hz under a bell
/// centred on CENTRE Hz (log scale).
const BASE: f32 = 55.0;
const PARTIALS: i32 = 9;
const CENTRE: f32 = 660.0;
const WIDTH_OCTAVES: f32 = 1.4;

/// A deterministic noise source (xorshift), so every build sounds the same.
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as f32 / u32::MAX as f32 * 2.0 - 1.0
    }
}

fn secs(n: usize) -> f32 {
    n as f32 / RATE as f32
}

/// A two-part mechanical "tak-shk", about 150 ms.
pub fn shutter() -> Vec<f32> {
    let len = (0.150 * RATE as f32) as usize;
    let mut noise = Noise(0x1234_5678);
    let (mut low, mut high_prev, mut high) = (0.0f32, 0.0f32, 0.0f32);
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let t = secs(i);
        let n = noise.next();
        // Click: bright, 0.5 ms attack, ~6 ms decay.
        let click = n * (t / 0.0005).min(1.0) * (-t / 0.006).exp();
        // "shk": band-passed noise from 60 ms, ~30 ms decay, softer.
        low += 0.25 * (n - low);
        high = 0.9 * (high + low - high_prev);
        high_prev = low;
        let u = t - 0.060;
        let shk = if u >= 0.0 { high * 0.9 * (u / 0.002).min(1.0) * (-u / 0.03).exp() } else { 0.0 };
        out.push(click * 0.8 + shk);
    }
    fade_out(&mut out, 0.010);
    normalise(&mut out, 0.9);
    out
}

/// `samples` played `factor` times faster (shorter and higher).
pub fn speed(samples: &[f32], factor: f32) -> Vec<f32> {
    let len = (samples.len() as f32 / factor).ceil() as usize;
    (0..len)
        .map(|i| {
            let pos = i as f32 * factor;
            let j = pos as usize;
            let frac = pos - j as f32;
            let a = samples.get(j).copied().unwrap_or(0.0);
            let b = samples.get(j + 1).copied().unwrap_or(0.0);
            a + (b - a) * frac
        })
        .collect()
}

/// The Shepard tone's partials `(frequency, amplitude)` at `position`
/// octaves (0..1) along the glide. Amplitude depends only on frequency, so
/// position 1 sounds like position 0.
pub fn partials(position: f32) -> Vec<(f32, f32)> {
    (-1..PARTIALS)
        .map(|k| {
            let f = BASE * 2f32.powf(k as f32 + position);
            let d = (f / CENTRE).log2() / WIDTH_OCTAVES;
            (f, (-0.5 * d * d).exp())
        })
        .collect()
}

/// A Shepard glide from `from` to `to` octaves over `secs` seconds.
pub fn glide(from: f32, to: f32, secs_long: f32) -> Vec<f32> {
    let len = (secs_long * RATE as f32) as usize;
    let count = partials(0.0).len();
    let mut phases = vec![0f32; count];
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let position = from + (to - from) * i as f32 / len as f32;
        let mut v = 0.0;
        for (p, (f, a)) in phases.iter_mut().zip(partials(position)) {
            *p = (*p + TAU * f / RATE as f32) % TAU;
            v += a * p.sin();
        }
        out.push(v);
    }
    fade_in(&mut out, 0.010);
    fade_out(&mut out, 0.040);
    normalise(&mut out, 0.5);
    out
}

/// The sound for shot `n` of a combo (1–7).
pub fn sound(n: u8) -> Vec<f32> {
    match n {
        1..=5 => speed(&shutter(), SPEEDS[n as usize - 1]),
        6 | 7 => {
            let mut s = speed(&shutter(), LOOP_SPEED);
            let half = if n == 6 { (0.0, 0.5) } else { (0.5, 1.0) };
            s.extend(glide(half.0, half.1, GLIDE_SECS));
            s
        }
        _ => sound(1),
    }
}

/// 16-bit mono PCM WAV.
pub fn wav(samples: &[f32]) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + data as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&RATE.to_le_bytes());
    w.extend_from_slice(&(RATE * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        w.extend_from_slice(&v.to_le_bytes());
    }
    w
}

fn fade_in(s: &mut [f32], secs_long: f32) {
    let n = ((secs_long * RATE as f32) as usize).min(s.len());
    for (i, v) in s[..n].iter_mut().enumerate() {
        *v *= i as f32 / n as f32;
    }
}

fn fade_out(s: &mut [f32], secs_long: f32) {
    let n = ((secs_long * RATE as f32) as usize).min(s.len());
    let len = s.len();
    for (i, v) in s[len - n..].iter_mut().enumerate() {
        *v *= 1.0 - (i + 1) as f32 / n as f32;
    }
}

fn normalise(s: &mut [f32], peak: f32) {
    let max = s.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if max > 0.0 {
        for v in s.iter_mut() {
            *v *= peak / max;
        }
    }
}
```

(The wav test expects `0.5 → 16383`: `(0.5 * 32767).round() = 16384`; adjust the test to 16384 if that is what it yields — the rounding rule is the implementation's — and ledger it.)

- [ ] **Step 3: Implement `combo.rs`**

```rust
//! Which shot of a combo this is, shared between `valw` runs.

use std::path::Path;

use anyhow::Result;

/// The number for a shot at `now_ms`, given the previous shot.
pub fn next(prev: Option<(u8, u64)>, now_ms: u64, reset_ms: u64) -> u8 {
    match prev {
        Some((n, at)) if now_ms >= at && now_ms - at < reset_ms => match n {
            7 => 6,
            n => n + 1,
        },
        _ => 1,
    }
}

/// `(number, unix ms)` of the last shot, if the file makes sense.
pub fn load(path: &Path) -> Option<(u8, u64)> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut parts = text.split_whitespace();
    let n: u8 = parts.next()?.parse().ok()?;
    let at: u64 = parts.next()?.parse().ok()?;
    (1..=7).contains(&n).then_some((n, at))
}

pub fn save(path: &Path, n: u8, at_ms: u64) -> Result<()> {
    crate::output::write_atomic(path, format!("{n} {at_ms}\n").as_bytes())
}
```

- [ ] **Step 4: Run** (+8), clippy, fmt. **Step 5: Commit** `feat: synthesised shutter sounds and the combo count`.

---

### Task 5: Playing, settings, toolbar option, packaging

**Files:** `src/sound/mod.rs`, `src/config.rs`, `src/main.rs`, `src/toolbar/{state,layout,draw,mod}.rs`, `nix/package.ulu.nix`.

**Produces:** `config::Sound { enabled: bool, volume: f32, combo_reset_secs: u64 }` (defaults true, 0.6, 5; validation `volume` in 0–1 and finite, `combo_reset_secs` in 1–60); `sound::play(config: &config::Sound, enabled: bool)`; `Common.toolbar_sound: Option<bool>` + `wants_sound(&Common, &Config) -> bool`; toolbar `ToolbarState.sound`, `MenuItem::Sound` ("Play sound"), `Picked.sound`; hidden `valw __combo-demo` (plays 1…7, 6, 7 with 0.5 s gaps, for tuning).

- [ ] **Step 1: Failing tests**

- `config.rs`: `sound_defaults_and_bounds` — defaults (true, 0.6, 5); `volume = 1.5` → error "sound.volume must be between 0 and 1"; `volume = nan` → same error; `combo_reset_secs = 0` → "sound.combo_reset_secs must be between 1 and 60".
- `main.rs`: in `toolbar_options_win_over_the_config_both_ways` add `sound` to the pick and assert `wants_sound` both ways; `flag_conflicts`: `parses(&["__combo-demo"])`; help hides `__combo-demo`.
- `toolbar::state` tests: `ToolbarState` gains `sound` (defaults from `sound.enabled`; round trip includes it); `toolbar::layout` menu rows now end with `MenuItem::Sound`.
- `sound/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cache_holds_seven_wavs() {
        let dir = tempfile::tempdir().unwrap();
        for n in 1..=7 {
            let path = cached(dir.path(), n).unwrap();
            assert!(path.ends_with(format!("{n}.wav")));
            assert_eq!(&std::fs::read(&path).unwrap()[0..4], b"RIFF");
        }
    }
}
```

Run → compile errors.

- [ ] **Step 2: Implement**

`sound/mod.rs`:

```rust
//! The combo shutter sound, played with PipeWire's `pw-play`.

pub mod combo;
pub mod synth;

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::config;

fn runtime_dir() -> PathBuf {
    crate::lock::default_path().parent().map(Path::to_path_buf).unwrap_or_else(std::env::temp_dir)
}

/// The WAV for shot `n`, written into `dir` the first time.
fn cached(dir: &Path, n: u8) -> Result<PathBuf> {
    let path = dir.join(format!("{n}.wav"));
    if !path.exists() {
        std::fs::create_dir_all(dir).with_context(|| format!("could not create {}", dir.display()))?;
        crate::output::write_atomic(&path, &synth::wav(&synth::sound(n)))?;
    }
    Ok(path)
}

/// Plays the next sound of the combo, unless sounds are off. Never fails the
/// capture: problems are logged.
pub fn play(config: &config::Sound, enabled: bool) {
    if !enabled {
        return;
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let state = runtime_dir().join("valw-combo");
    let n = combo::next(combo::load(&state), now, config.combo_reset_secs * 1000);
    if let Err(e) = combo::save(&state, n, now) {
        tracing::warn!("could not remember the combo: {e:#}");
    }
    if let Err(e) = spawn(n, config.volume) {
        tracing::warn!("no shutter sound: {e:#}");
    }
}

fn spawn(n: u8, volume: f32) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let path = cached(&runtime_dir().join("valw-sound"), n)?;
    let mut command = std::process::Command::new("pw-play");
    command
        .arg(format!("--volume={volume}"))
        .arg(&path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // SAFETY: setsid is async-signal-safe.
    unsafe {
        command.pre_exec(|| rustix::process::setsid().map(drop).map_err(Into::into));
    }
    command.spawn().context("could not start pw-play")?;
    Ok(())
}

/// `valw __combo-demo`: the whole combo, for tuning by ear.
pub fn demo(config: &config::Sound) -> Result<()> {
    for n in [1, 2, 3, 4, 5, 6, 7, 6, 7] {
        spawn(n, config.volume)?;
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    Ok(())
}
```

`main.rs`: `Common.toolbar_sound: Option<bool>` (`#[arg(skip)]`); `fn wants_sound(common, config) -> bool { common.toolbar_sound.unwrap_or(config.sound.enabled) }`; in `capture`, right after the `let (shots, clip, source) = match mode { … };` block: `sound::play(&config.sound, wants_sound(&common, &config));` (compute `let sound_on = wants_sound(&common, &config);` early next to `cursor`/`preview`, since `common.output` moves later); `from_toolbar` sets `toolbar_sound: Some(pick.sound)`; hidden `#[command(name = "__combo-demo", hide = true)] ComboDemo` → `sound::demo(&config::load(&config::default_path())?.sound)`.

Toolbar: `ToolbarState.sound: bool` (`File.sound: Option<bool>`, default `config.sound.enabled`); `MenuItem::Sound` row "Play sound" appended to `MENU`; `draw::bar` treats `MenuItem::Sound` like Cursor (`state.sound`); `Toolbar::pointer` toggles it; `Picked.sound`; `run` returns it.

`nix/package.ulu.nix`: `--suffix PATH : ${lib.makeBinPath [ pkgs.satty pkgs.pipewire ]}`.

- [ ] **Step 3: Run** (all pass), clippy, fmt. Remove the Task 4 allowance.

- [ ] **Step 4: Listen** (tell the user first: it plays sounds): `valw __combo-demo` once in the dev shell (`pw-play` is on the system), and read the log for warnings. Record what was heard in the ledger; tuning by ear is the user's in the checklist.

- [ ] **Step 5: Commit** `feat: combo shutter sound`.

---

### Task 6: Checks and checklist

- [ ] `nix flake check -L --keep-going` passes.
- [ ] Append to `docs/test-checklist.md`:

```markdown

## Polish

- [ ] Region: while dragging, a `W × H` label follows the pointer (physical pixels, same as the saved PNG), flipping near the edges.
- [ ] Shift locks the dimension you move first; Alt grows from the centre; Space held moves the selection, and after letting go resizing continues without a jump; Space before a drag still switches to window mode.
- [ ] `[capture] window_shadow = true`: window shots get a soft shadow following the rounded corners; preview, clipboard and file all have it.
- [ ] Shots in quick succession play sounds 1 → 5 getting faster and brighter, then 6, 7, 6, 7 rising endlessly; after a 5 s pause it starts at 1 again. `valw __combo-demo` plays the whole sequence for tuning.
- [ ] `[sound] enabled = false`, `volume`, `combo_reset_secs` take effect; the toolbar's Play sound option overrides the config; no `pw-play` → no sound, capture still works (warning in the log).
```

- [ ] Commit `docs: polish checklist`.
