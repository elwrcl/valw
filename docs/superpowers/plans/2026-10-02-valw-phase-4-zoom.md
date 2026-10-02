# valw Phase 4: zoom mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `valw zoom` freezes the focused output and shows it zoomable (wheel), pannable (drag), with a flashlight (`f`), and `c` captures the visible area.

**Architecture:** Pure view math in `zoom/view.rs`; an EGL + OpenGL ES 3 renderer in `zoom/gl.rs` (one fullscreen triangle, `texelFetch` from the frozen frame, flashlight in the fragment shader); `zoom/mod.rs` owns the layer surface, input and the frame-callback-driven draw loop. The whole program moves to `wayland-backend`'s `client_system` so EGL can use the C `wl_display*`.

**Tech Stack:** Rust 2024, SCTK 0.21, `wayland-backend` (`client_system`), `khronos-egl` 6 (`dynamic`), `libloading` 0.8, `wayland-egl` 0.32, `glow` 0.18.

**Spec:** `docs/superpowers/specs/2026-10-02-valw-phase-4-zoom-design.md`

## Global Constraints

- jj only (`jj commit -m`), never git. `cargo clippy -- -D warnings`, `cargo fmt --check`, `cargo nextest run`, `nix flake check`.
- No Vulkan, no wgpu. OpenGL ES 3.0, `#version 300 es`.
- Scale range 1–32; flashlight radius 20–2000 logical px, default 180; `scroll_step` default 1.15; touchpad: one step per 15 logical px; flashlight outside = 25 % brightness; animation ≈ 90 % in 120 ms.
- Namespace `valw-zoom`; Esc/`q` exit 0 with nothing saved.
- Tell the user before running zoom against the live session (it takes the keyboard).

## Review Focus

1. **Zoom keeps the point under the pointer**, at every scale and near the edges where clamping kicks in. Tests: `view::tests::zoom_keeps_the_point_under_the_pointer`, `zoom_out_near_an_edge_stays_inside` (Task 1).
2. **`c` after zooming** must crop exactly the visible pixels, never outside the frame, never empty. Test: `view::tests::visible_rect_*` (Task 1).
3. **Touchpad scrolling** (no `value120`) must zoom smoothly, not in jumps or not at all. Test: `view::tests::scroll_steps_from_wheel_and_touchpad` (Task 1).
4. **The switch to libwayland-client** must not change region/screen/preview behaviour. Covered by the existing suite and the headless sway check (Task 4).
5. **No EGL / no GLES 3** (other machines, the headless check): `valw zoom` fails clearly, `valw doctor` warns, other modes unaffected. Test: `doctor::tests::egl_check_levels` (Task 2).

---

## File Structure

| File | Change | Task |
|---|---|---|
| `src/zoom/view.rs` (new) | `View`, `Animated`, `scroll_steps`, `radius` | 1 |
| `src/zoom/mod.rs` (new) | Task 1: `pub mod view;` only. Task 3: `Zoom`, `Outcome`, `Command`, `command()`, `run()` | 1, 3 |
| `src/config.rs` | `[zoom]` | 1 |
| `src/zoom/gl.rs` (new) | `probe`, `Renderer` | 2 |
| `src/doctor.rs` | `check_egl` | 2 |
| `Cargo.toml`, `nix/package.ulu.nix` | deps, `client_system`, `LD_LIBRARY_PATH` | 2 |
| `src/wayland.rs` | `State::zoom` and event routing | 3 |
| `src/main.rs` | `Command::Zoom`, `Mode::Zoom` | 3 |
| `docs/test-checklist.md` | Phase 4 section | 4 |

---

### Task 1: View math and config

**Files:** Create `src/zoom/mod.rs`, `src/zoom/view.rs`; modify `src/main.rs` (`mod zoom;` + allowance), `src/config.rs`.

**Interfaces — Produces:**
- `zoom::view::{View, Animated, scroll_steps, radius, MIN_SCALE, MAX_SCALE, MIN_RADIUS, MAX_RADIUS}`
- `View { scale: f64, origin: (f64, f64) }`, `View::IDENTITY`, `to_frame(p)`, `zoom_at(p, factor, size) -> View`, `pan(delta, size) -> View`, `clamped(size) -> View`, `visible_rect((u32, u32)) -> PixelRect`
- `Animated { shown: View, target: View }`, `Animated::new()`, `set(View)`, `step(dt) -> bool` (true while moving)
- `scroll_steps(value120: i32, absolute: f64) -> f64` (positive = zoom in), `radius(r, steps, step) -> f64`
- `config::Zoom { scroll_step: f64, flashlight_radius: f64 }`, `Config::zoom`

- [ ] **Step 1: Failing tests**

`src/zoom/mod.rs`:

```rust
//! Zoom mode: the frozen focused output, zoomed and panned on the GPU.

pub mod view;
```

At the top of `src/main.rs`: `// Zoom lands in pieces; Task 3 removes this.` + `#![allow(dead_code)]` + empty line; add `mod zoom;` after `mod window;`.

`src/zoom/view.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: (f64, f64) = (1920.0, 1080.0);

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6
    }

    #[test]
    fn zoom_keeps_the_point_under_the_pointer() {
        for p in [(960.0, 540.0), (100.0, 900.0), (1500.0, 200.0)] {
            let v = View::IDENTITY.zoom_at(p, 2.0, SIZE);
            assert_eq!(v.scale, 2.0);
            assert!(close(v.to_frame(p), p), "{p:?} -> {v:?}");
            let w = v.zoom_at(p, 1.15, SIZE);
            assert!(close(w.to_frame(p), v.to_frame(p)), "{p:?} -> {w:?}");
        }
    }

    #[test]
    fn scale_is_clamped() {
        let v = View::IDENTITY.zoom_at((10.0, 10.0), 0.5, SIZE);
        assert_eq!(v, View::IDENTITY);
        let v = View::IDENTITY.zoom_at((10.0, 10.0), 1000.0, SIZE);
        assert_eq!(v.scale, MAX_SCALE);
    }

    #[test]
    fn zoom_out_near_an_edge_stays_inside() {
        let v = View::IDENTITY.zoom_at((1900.0, 1070.0), 4.0, SIZE);
        let out = v.zoom_at((0.0, 0.0), 0.5, SIZE);
        assert_eq!(out.scale, 2.0);
        assert!(out.origin.0 >= 0.0 && out.origin.0 <= 1920.0 - 960.0, "{out:?}");
        assert!(out.origin.1 >= 0.0 && out.origin.1 <= 1080.0 - 540.0, "{out:?}");
    }

    #[test]
    fn pan_moves_with_the_pointer_and_is_clamped() {
        let v = View::IDENTITY.zoom_at((960.0, 540.0), 2.0, SIZE);
        let p = v.pan((100.0, 0.0), SIZE);
        assert!(close(p.origin, (v.origin.0 - 50.0, v.origin.1)), "{p:?}");
        let far = v.pan((-1e6, 1e6), SIZE);
        assert!(close(far.origin, (960.0, 0.0)), "{far:?}");
        assert_eq!(View::IDENTITY.pan((50.0, 50.0), SIZE), View::IDENTITY);
    }

    #[test]
    fn visible_rect_at_identity_is_the_frame() {
        let r = View::IDENTITY.visible_rect((1920, 1080));
        assert_eq!((r.x, r.y, r.width, r.height), (0, 0, 1920, 1080));
    }

    #[test]
    fn visible_rect_rounds_outwards_inside_the_frame() {
        let v = View { scale: 4.0, origin: (100.5, 200.25) };
        let r = v.visible_rect((1920, 1080));
        assert_eq!((r.x, r.y, r.width, r.height), (100, 200, 481, 271));
        let edge = View { scale: 4.0, origin: (1440.0, 810.0) };
        let r = edge.visible_rect((1920, 1080));
        assert_eq!((r.x, r.y, r.width, r.height), (1440, 810, 480, 270));
    }

    #[test]
    fn animation_reaches_the_target_and_stops() {
        let mut a = Animated::new();
        a.target = View::IDENTITY.zoom_at((960.0, 540.0), 4.0, SIZE);
        let mut frames = 0;
        while a.step(1.0 / 60.0) {
            frames += 1;
            assert!(frames < 120, "never settles");
        }
        assert_eq!(a.shown, a.target);
        // About 90 % of the way after 120 ms.
        let mut b = Animated::new();
        b.target = View { scale: 11.0, origin: (0.0, 0.0) };
        b.step(0.12);
        assert!((b.shown.scale - 10.0).abs() < 0.2, "{:?}", b.shown);
    }

    #[test]
    fn set_jumps() {
        let mut a = Animated::new();
        let v = View { scale: 2.0, origin: (5.0, 5.0) };
        a.set(v);
        assert_eq!((a.shown, a.target), (v, v));
        assert!(!a.step(0.016));
    }

    #[test]
    fn scroll_steps_from_wheel_and_touchpad() {
        assert_eq!(scroll_steps(-120, -15.0), 1.0, "one notch up zooms in");
        assert_eq!(scroll_steps(240, 30.0), -2.0, "two notches down");
        assert_eq!(scroll_steps(0, -7.5), 0.5, "touchpad: 15 px per step");
        assert_eq!(scroll_steps(0, 0.0), 0.0);
    }

    #[test]
    fn radius_steps_and_bounds() {
        assert!((radius(180.0, 1.0, 1.15) - 207.0).abs() < 1e-9);
        assert_eq!(radius(25.0, -10.0, 1.15), MIN_RADIUS);
        assert_eq!(radius(1900.0, 10.0, 1.15), MAX_RADIUS);
    }
}
```

In `src/config.rs` tests, change `unknown_keys_are_reported_not_rejected` to use an `[editor]` table (zoom becomes known):

```rust
        let text = "[editor]\nbackend = \"satty\"\n\n[save]\nfolder = \"x\"\n";
        let (config, unknown) = parse(text).unwrap();
        assert_eq!(config, Config::default());
        assert_eq!(unknown, vec!["editor".to_string(), "save.folder".to_string()]);
```

and add:

```rust
    #[test]
    fn zoom_defaults_overrides_and_bounds() {
        assert_eq!(
            Config::default().zoom,
            Zoom {
                scroll_step: 1.15,
                flashlight_radius: 180.0
            }
        );
        let (config, unknown) = parse("[zoom]\nscroll_step = 1.3\n").unwrap();
        assert!(unknown.is_empty());
        assert_eq!(config.zoom.scroll_step, 1.3);
        assert_eq!(config.zoom.flashlight_radius, 180.0);
        let err = parse("[zoom]\nscroll_step = 1.0\n").unwrap_err();
        assert_eq!(err.to_string(), "zoom.scroll_step must be greater than 1");
        let err = parse("[zoom]\nflashlight_radius = 5.0\n").unwrap_err();
        assert_eq!(
            err.to_string(),
            "zoom.flashlight_radius must be between 20 and 2000"
        );
    }
```

- [ ] **Step 2: Run, watch them fail**

Run: `cargo nextest run zoom:: config::`
Expected: compilation fails (`View`, `Animated`, `scroll_steps`, `radius`, `Zoom` missing).

- [ ] **Step 3: Implement `view.rs`** (above the tests)

```rust
//! The zoom view: which part of the frozen frame fills the screen.
//!
//! Everything is in the frame's physical pixels. The surface's buffer has
//! the frame's size, so at 1× one screen pixel is one frame pixel.

use crate::frame::PixelRect;

pub const MIN_SCALE: f64 = 1.0;
pub const MAX_SCALE: f64 = 32.0;
pub const MIN_RADIUS: f64 = 20.0;
pub const MAX_RADIUS: f64 = 2000.0;
/// Logical px of touchpad scrolling per zoom step.
const TOUCHPAD_PX_PER_STEP: f64 = 15.0;
/// Per second: 1 - e^(-RATE * 0.12) ≈ 0.9, 90 % of the way in 120 ms.
const RATE: f64 = 19.2;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub scale: f64,
    /// The frame pixel at the screen's top-left corner.
    pub origin: (f64, f64),
}

impl View {
    pub const IDENTITY: View = View {
        scale: 1.0,
        origin: (0.0, 0.0),
    };

    /// The frame point under screen point `p`.
    pub fn to_frame(&self, p: (f64, f64)) -> (f64, f64) {
        (
            self.origin.0 + p.0 / self.scale,
            self.origin.1 + p.1 / self.scale,
        )
    }

    /// Zooms by `factor`, keeping the frame point under `p` in place.
    pub fn zoom_at(&self, p: (f64, f64), factor: f64, size: (f64, f64)) -> View {
        let f = self.to_frame(p);
        let scale = (self.scale * factor).clamp(MIN_SCALE, MAX_SCALE);
        View {
            scale,
            origin: (f.0 - p.0 / scale, f.1 - p.1 / scale),
        }
        .clamped(size)
    }

    /// Moves the image along with a pointer that moved `delta` screen px.
    pub fn pan(&self, delta: (f64, f64), size: (f64, f64)) -> View {
        View {
            scale: self.scale,
            origin: (
                self.origin.0 - delta.0 / self.scale,
                self.origin.1 - delta.1 / self.scale,
            ),
        }
        .clamped(size)
    }

    /// Keeps the view inside a frame of `size` px.
    pub fn clamped(self, size: (f64, f64)) -> View {
        let max = |len: f64| len - len / self.scale;
        View {
            scale: self.scale,
            origin: (
                self.origin.0.clamp(0.0, max(size.0)),
                self.origin.1.clamp(0.0, max(size.1)),
            ),
        }
    }

    /// The frame pixels on screen, rounded outwards and inside the frame.
    pub fn visible_rect(&self, size: (u32, u32)) -> PixelRect {
        let span = |origin: f64, len: u32| {
            let len = len as f64;
            let start = origin.floor().clamp(0.0, len - 1.0);
            let end = (origin + len / self.scale).ceil().clamp(start + 1.0, len);
            (start as u32, (end - start) as u32)
        };
        let (x, width) = span(self.origin.0, size.0);
        let (y, height) = span(self.origin.1, size.1);
        PixelRect {
            x,
            y,
            width,
            height,
        }
    }
}

/// The view on screen, easing towards the one asked for.
#[derive(Debug, Clone, Copy)]
pub struct Animated {
    pub shown: View,
    pub target: View,
}

impl Animated {
    pub fn new() -> Self {
        Animated {
            shown: View::IDENTITY,
            target: View::IDENTITY,
        }
    }

    /// Jumps to `view` without easing (panning).
    pub fn set(&mut self, view: View) {
        self.shown = view;
        self.target = view;
    }

    /// Advances by `dt` seconds. True while still moving.
    pub fn step(&mut self, dt: f64) -> bool {
        let k = 1.0 - (-RATE * dt).exp();
        let (s, t) = (&mut self.shown, self.target);
        let lerp = |a: f64, b: f64| a + (b - a) * k;
        s.scale = lerp(s.scale, t.scale);
        s.origin = (lerp(s.origin.0, t.origin.0), lerp(s.origin.1, t.origin.1));
        let settled = (s.scale - t.scale).abs() < 1e-3
            && (s.origin.0 - t.origin.0).abs() < 0.05
            && (s.origin.1 - t.origin.1).abs() < 0.05;
        if settled {
            self.shown = t;
        }
        !settled
    }
}

/// Zoom steps in one axis event: wheel notches (`value120`), or touchpad
/// pixels when there are none. Positive zooms in (scrolling up).
pub fn scroll_steps(value120: i32, absolute: f64) -> f64 {
    if value120 != 0 {
        -(value120 as f64) / 120.0
    } else {
        -absolute / TOUCHPAD_PX_PER_STEP
    }
}

/// The flashlight radius after `steps` scroll steps of `step` each.
pub fn radius(r: f64, steps: f64, step: f64) -> f64 {
    (r * step.powf(steps)).clamp(MIN_RADIUS, MAX_RADIUS)
}
```

(Check `visible_rect_rounds_outwards_inside_the_frame`: origin 100.5, width 1920/4 = 480 → start 100, end ceil(580.5) = 581 → 481; y: 200.25 + 270 = 470.25 → end 471, start 200 → 271. Edge: 1440 + 480 = 1920 → 480; 810 + 270 = 1080 → 270.)

- [ ] **Step 4: Implement `config::Zoom`**

In `src/config.rs`: add `pub zoom: Zoom,` to `Config`; add

```rust
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Zoom {
    pub scroll_step: f64,
    pub flashlight_radius: f64,
}

impl Default for Zoom {
    fn default() -> Self {
        Self {
            scroll_step: 1.15,
            flashlight_radius: 180.0,
        }
    }
}
```

and in `parse`, after the timeout check:

```rust
    if config.zoom.scroll_step <= 1.0 {
        bail!("zoom.scroll_step must be greater than 1");
    }
    if !(20.0..=2000.0).contains(&config.zoom.flashlight_radius) {
        bail!("zoom.flashlight_radius must be between 20 and 2000");
    }
```

- [ ] **Step 5: Run**

Run: `cargo nextest run` → all pass (previous 121 + 10 view + 1 config). `cargo clippy -- -D warnings`, `cargo fmt --check` clean. (`Animated::new` without `Default` may trip `clippy::new_without_default`; if so add `impl Default for Animated` delegating to `new` and ledger it.)

- [ ] **Step 6: Commit** — `jj commit -m "feat: zoom view math and config"`

---

### Task 2: EGL/GLES renderer, backend switch, doctor, packaging

**Files:** Modify `Cargo.toml`, `nix/package.ulu.nix`, `src/doctor.rs`, `src/zoom/mod.rs`; create `src/zoom/gl.rs`.

**Interfaces — Produces:**
- `zoom::gl::probe(conn: &Connection) -> Result<String>` (EGL vendor when a GLES 3 window config exists)
- `zoom::gl::Renderer::new(conn: &Connection, surface: &WlSurface, size: (u32, u32), frame: &Bgrx) -> Result<Renderer>`; `Renderer::draw(&self, view: &View, flashlight: Option<(f64, f64, f64)>) -> Result<()>` (centre and radius in buffer px); `Drop` releases EGL.
- `doctor::check_egl(result: Result<String>) -> Check`

- [ ] **Step 1: Failing test** — in `src/doctor.rs` tests:

```rust
    #[test]
    fn egl_check_levels() {
        let ok = check_egl(Ok("Mesa Project".into()));
        assert_eq!(ok.level, Level::Ok);
        assert!(ok.text.contains("Mesa Project"), "{}", ok.text);
        let missing = check_egl(Err(anyhow::anyhow!("could not load libEGL.so.1")));
        assert_eq!(missing.level, Level::Warn, "only zoom needs it");
        assert!(missing.text.contains("zoom"), "{}", missing.text);
    }
```

(Check `Check`'s field names in `src/doctor.rs` and use them; the test above assumes `level` and `text`.)

Run: `cargo nextest run doctor::` → fails, `check_egl` missing.

- [ ] **Step 2: Dependencies** — in `Cargo.toml` `[dependencies]` (alphabetical):

```toml
glow = "0.18.0"
khronos-egl = { version = "6.0.0", features = ["dynamic"] }
libloading = "0.8"
wayland-backend = { version = "0.3.17", features = ["client_system"] }
wayland-egl = "0.32.11"
```

- [ ] **Step 3: `check_egl` and the doctor line**

```rust
/// EGL with an OpenGL ES 3 config; only zoom needs it.
pub fn check_egl(result: Result<String>) -> Check {
    match result {
        Ok(vendor) => check(Level::Ok, format!("EGL ({vendor}) with OpenGL ES 3 (zoom)")),
        Err(e) => check(
            Level::Warn,
            format!("no OpenGL ES 3 through EGL: {} (zoom won't work)", one_line(&e)),
        ),
    }
}
```

In `run`, inside `Ok(wl) =>` after the outputs loop: `checks.push(check_egl(crate::zoom::gl::probe(&wl.conn)));`

- [ ] **Step 4: `src/zoom/gl.rs`** — add `pub mod gl;` to `src/zoom/mod.rs`.

```rust
//! OpenGL ES 3 through EGL on a Wayland surface: just enough to show the
//! frozen frame zoomed in.

use anyhow::{Context, Result, anyhow};
use glow::HasContext;
use khronos_egl as egl;
use wayland_client::{Connection, Proxy, protocol::wl_surface::WlSurface};

use super::view::View;
use crate::frame::Bgrx;

type Egl = egl::DynamicInstance<egl::EGL1_5>;

/// `EGL_PLATFORM_WAYLAND_KHR`.
const PLATFORM_WAYLAND: egl::Enum = 0x31D8;

const VERTEX: &str = "#version 300 es
// One triangle that covers the screen.
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
";

const FRAGMENT: &str = "#version 300 es
precision highp float;
uniform sampler2D frame;
uniform float scale;
// The frame pixel at the screen's top-left.
uniform vec2 origin;
// Centre and radius in screen px; radius 0 means off.
uniform vec3 flashlight;
out vec4 color;
void main() {
    ivec2 size = textureSize(frame, 0);
    // gl_FragCoord starts at the bottom-left; the frame at the top-left.
    vec2 screen = vec2(gl_FragCoord.x, float(size.y) - gl_FragCoord.y);
    ivec2 texel = clamp(ivec2(floor(origin + screen / scale)), ivec2(0), size - 1);
    // The texture holds the frame's B, G, R, X bytes.
    vec3 rgb = texelFetch(frame, texel, 0).bgr;
    if (flashlight.z > 0.0) {
        float d = distance(screen, flashlight.xy);
        rgb *= mix(1.0, 0.25, smoothstep(flashlight.z - 1.5, flashlight.z + 1.5, d));
    }
    color = vec4(rgb, 1.0);
}
";

/// Loads libEGL and opens the compositor's EGL display.
fn open(conn: &Connection) -> Result<(Egl, egl::Display)> {
    let lib = unsafe { libloading::Library::new("libEGL.so.1") }
        .context("could not load libEGL.so.1")?;
    let egl = unsafe { Egl::load_required_from(lib) }
        .map_err(|e| anyhow!("libEGL.so.1 has no EGL 1.5: {e}"))?;
    let display = unsafe {
        egl.get_platform_display(
            PLATFORM_WAYLAND,
            conn.backend().display_ptr().cast(),
            &[egl::ATTRIB_NONE],
        )
    }
    .context("no EGL display for the Wayland connection")?;
    egl.initialize(display).context("could not initialise EGL")?;
    Ok((egl, display))
}

fn config(egl: &Egl, display: egl::Display) -> Result<egl::Config> {
    #[rustfmt::skip]
    let attributes = [
        egl::SURFACE_TYPE, egl::WINDOW_BIT,
        egl::RENDERABLE_TYPE, egl::OPENGL_ES3_BIT,
        egl::RED_SIZE, 8, egl::GREEN_SIZE, 8, egl::BLUE_SIZE, 8,
        egl::NONE,
    ];
    egl.choose_first_config(display, &attributes)
        .context("eglChooseConfig failed")?
        .context("no EGL config for OpenGL ES 3 windows")
}

/// The EGL vendor, if a window config for OpenGL ES 3 exists.
pub fn probe(conn: &Connection) -> Result<String> {
    let (egl, display) = open(conn)?;
    let result = config(&egl, display).map(|_| {
        egl.query_string(Some(display), egl::VENDOR)
            .map(|v| v.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "unknown vendor".into())
    });
    let _ = egl.terminate(display);
    result
}

/// Draws the frozen frame on one surface.
pub struct Renderer {
    gl: glow::Context,
    scale: glow::UniformLocation,
    origin: glow::UniformLocation,
    flashlight: glow::UniformLocation,
    egl: Egl,
    display: egl::Display,
    context: egl::Context,
    surface: egl::Surface,
    // Destroyed after the EGL surface that draws into it.
    _window: wayland_egl::WlEglSurface,
}

impl Renderer {
    /// A GL surface of `size` buffer px on `surface`, showing `frame`.
    pub fn new(
        conn: &Connection,
        surface: &WlSurface,
        size: (u32, u32),
        frame: &Bgrx,
    ) -> Result<Renderer> {
        let (egl, display) = open(conn)?;
        egl.bind_api(egl::OPENGL_ES_API)
            .context("no OpenGL ES in EGL")?;
        let config = config(&egl, display)?;
        let context = egl
            .create_context(display, config, None, &[egl::CONTEXT_MAJOR_VERSION, 3, egl::NONE])
            .context("could not create an OpenGL ES 3 context")?;
        let window = wayland_egl::WlEglSurface::new(surface.id(), size.0 as i32, size.1 as i32)
            .context("could not create a wl_egl_window")?;
        let egl_surface = unsafe {
            egl.create_window_surface(display, config, window.ptr() as egl::NativeWindowType, None)
        }
        .context("could not create the EGL window surface")?;
        egl.make_current(display, Some(egl_surface), Some(egl_surface), Some(context))
            .context("eglMakeCurrent failed")?;
        // valw paces drawing with frame callbacks; swaps must not block.
        egl.swap_interval(display, 0)
            .context("eglSwapInterval failed")?;

        let gl = unsafe {
            glow::Context::from_loader_function(|name| {
                egl.get_proc_address(name)
                    .map_or(std::ptr::null(), |f| f as *const _)
            })
        };
        let (scale, origin, flashlight) = unsafe { setup(&gl, size, frame)? };
        Ok(Renderer {
            gl,
            scale,
            origin,
            flashlight,
            egl,
            display,
            context,
            surface: egl_surface,
            _window: window,
        })
    }

    /// Draws `view` and swaps (which commits the surface).
    pub fn draw(&self, view: &View, flashlight: Option<(f64, f64, f64)>) -> Result<()> {
        let (x, y, r) = flashlight.unwrap_or_default();
        unsafe {
            self.gl.uniform_1_f32(Some(&self.scale), view.scale as f32);
            self.gl
                .uniform_2_f32(Some(&self.origin), view.origin.0 as f32, view.origin.1 as f32);
            self.gl
                .uniform_3_f32(Some(&self.flashlight), x as f32, y as f32, r as f32);
            self.gl.draw_arrays(glow::TRIANGLES, 0, 3);
        }
        self.egl
            .swap_buffers(self.display, self.surface)
            .context("eglSwapBuffers failed")
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        let _ = self.egl.make_current(self.display, None, None, None);
        let _ = self.egl.destroy_surface(self.display, self.surface);
        let _ = self.egl.destroy_context(self.display, self.context);
        let _ = self.egl.terminate(self.display);
    }
}

/// Compiles the program, uploads the frame and returns the uniforms.
unsafe fn setup(
    gl: &glow::Context,
    size: (u32, u32),
    frame: &Bgrx,
) -> Result<(glow::UniformLocation, glow::UniformLocation, glow::UniformLocation)> {
    unsafe {
        tracing::info!("zoom renderer: {}", gl.get_parameter_string(glow::RENDERER));
        let program = gl.create_program().map_err(|e| anyhow!(e))?;
        for (kind, source) in [(glow::VERTEX_SHADER, VERTEX), (glow::FRAGMENT_SHADER, FRAGMENT)] {
            let shader = gl.create_shader(kind).map_err(|e| anyhow!(e))?;
            gl.shader_source(shader, source);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                anyhow::bail!("shader: {}", gl.get_shader_info_log(shader));
            }
            gl.attach_shader(program, shader);
        }
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            anyhow::bail!("program: {}", gl.get_program_info_log(program));
        }
        gl.use_program(Some(program));
        // GLES 3 needs a bound vertex array even without attributes.
        let vao = gl.create_vertex_array().map_err(|e| anyhow!(e))?;
        gl.bind_vertex_array(Some(vao));

        let texture = gl.create_texture().map_err(|e| anyhow!(e))?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::NEAREST as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::NEAREST as i32);
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            frame.width() as i32,
            frame.height() as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(frame.as_raw())),
        );
        gl.viewport(0, 0, size.0 as i32, size.1 as i32);

        let uniform = |name: &str| {
            gl.get_uniform_location(program, name)
                .with_context(|| format!("no uniform {name}"))
        };
        gl.uniform_1_i32(uniform("frame").ok().as_ref(), 0);
        Ok((uniform("scale")?, uniform("origin")?, uniform("flashlight")?))
    }
}
```

(Exact `glow` 0.18 / `khronos-egl` 6 signatures may differ slightly, e.g. `tex_image_2d`'s pixel argument or `swap_interval`'s; adapt to what the compiler says and keep the behaviour.)

- [ ] **Step 5: Packaging** — in `nix/package.ulu.nix`, `postInstall` becomes:

```nix
        # Clicking a preview opens Satty. --suffix keeps a user's own Satty first.
        # Zoom loads libEGL.so.1 (libglvnd); Mesa's drivers come from the system.
        postInstall = ''
          wrapProgram $out/bin/valw \
            --suffix PATH : ${lib.makeBinPath [ pkgs.satty ]} \
            --suffix LD_LIBRARY_PATH : ${lib.makeLibraryPath [ pkgs.libglvnd ]}
        '';
```

- [ ] **Step 6: Run** — `cargo nextest run` (all pass, +1), clippy and fmt clean. Then `cargo run -q -- doctor 2>&1 | grep -i egl` with `LD_LIBRARY_PATH` set to libglvnd's lib dir (`nix build --no-link --print-out-paths nixpkgs#libglvnd`): expect `ok   EGL (Mesa Project) with OpenGL ES 3 (zoom)`. doctor only reads; it's safe on the live session.

- [ ] **Step 7: Commit** — `jj commit -m "feat: EGL/GLES 3 renderer for zoom"`

---

### Task 3: The zoom mode

**Files:** Modify `src/zoom/mod.rs`, `src/wayland.rs`, `src/main.rs`.

**Interfaces:**
- Consumes: Task 1 `view::*`, `config::Zoom`; Task 2 `gl::Renderer`.
- Produces: `zoom::run(wl: &mut Wayland, output: &Output, frame: &Frame, config: &config::Zoom) -> Result<Outcome>`; `enum Outcome { Capture(PixelRect), Leave }`; `enum Command { Leave, Capture, Flashlight, Reset }`, `fn command(Keysym) -> Option<Command>`; `Zoom::{configure, frame_done, closed, pointer, key, modifiers}`; `State::zoom: Option<Zoom>`.

- [ ] **Step 1: Failing tests** — append to `src/zoom/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        assert_eq!(command(Keysym::Escape), Some(Command::Leave));
        assert_eq!(command(Keysym::q), Some(Command::Leave));
        assert_eq!(command(Keysym::c), Some(Command::Capture));
        assert_eq!(command(Keysym::f), Some(Command::Flashlight));
        assert_eq!(command(Keysym::_0), Some(Command::Reset));
        assert_eq!(command(Keysym::a), None);
    }
}
```

and to `flag_conflicts` in `src/main.rs`:

```rust
        assert!(parses(&["zoom", "--cursor", "--no-preview"]));
        assert!(!parses(&["zoom", "--clipboard-only", "-o", "a.png"]));
```

Run: `cargo nextest run zoom::tests flag_conflicts` → fails to compile (`command` missing).

- [ ] **Step 2: Implement `src/zoom/mod.rs`**

```rust
//! Zoom mode: the frozen focused output, zoomed and panned on the GPU.

pub mod gl;
pub mod view;

use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use smithay_client_toolkit::{
    compositor::FrameCallbackData,
    seat::{
        keyboard::{Keysym, Modifiers},
        pointer::{BTN_LEFT, PointerEvent, PointerEventKind},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerSurface},
    },
};
use wayland_client::{QueueHandle, protocol::wl_surface::WlSurface};
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::{
    Shape, WpCursorShapeDeviceV1,
};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;

use self::view::{Animated, View};
use crate::config;
use crate::error::HintExt;
use crate::frame::{Frame, PixelRect};
use crate::wayland::{Output, State, Wayland};

pub const NAMESPACE: &str = "valw-zoom";

/// How a zoom session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// `c`: this part of the frame.
    Capture(PixelRect),
    /// Esc or `q`: nothing to save.
    Leave,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Leave,
    Capture,
    Flashlight,
    Reset,
}

pub fn command(key: Keysym) -> Option<Command> {
    match key {
        Keysym::Escape | Keysym::q => Some(Command::Leave),
        Keysym::c => Some(Command::Capture),
        Keysym::f => Some(Command::Flashlight),
        Keysym::_0 => Some(Command::Reset),
        _ => None,
    }
}

pub struct Zoom {
    // Dropped first: the EGL surface goes before the wl_surface under it.
    renderer: gl::Renderer,
    layer: LayerSurface,
    viewport: WpViewport,
    /// Frame size in physical px.
    size: (u32, u32),
    /// Buffer px per surface-local (logical) px.
    ratio: f64,
    view: Animated,
    step: f64,
    /// Pointer position in buffer px.
    pointer: (f64, f64),
    /// Last pointer position while the left button pans.
    panning: Option<(f64, f64)>,
    flashlight: bool,
    /// Flashlight radius in logical px.
    radius: f64,
    ctrl: bool,
    configured: bool,
    frame_pending: bool,
    dirty: bool,
    last_draw: Option<Instant>,
    outcome: Option<Outcome>,
    error: Option<anyhow::Error>,
}

/// Shows `frame` (the frozen `output`) until the user leaves or captures.
pub fn run(
    wl: &mut Wayland,
    output: &Output,
    frame: &Frame,
    config: &config::Zoom,
) -> Result<Outcome> {
    let qh = wl.queue.handle();
    let s = &wl.state;
    let layer_shell = s
        .layer_shell
        .as_ref()
        .context("compositor does not support wlr-layer-shell")
        .hint("run `valw doctor` to see what the compositor supports")?;
    let viewporter = s
        .viewporter
        .as_ref()
        .context("compositor does not support wp-viewporter")
        .hint("run `valw doctor` to see what the compositor supports")?;
    let surface = s.compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(
        &qh,
        surface,
        Layer::Overlay,
        Some(NAMESPACE),
        Some(&output.wl),
    );
    layer.set_anchor(Anchor::all());
    layer.set_exclusive_zone(-1);
    layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
    layer.commit();
    let viewport = viewporter.get_viewport(layer.wl_surface(), &qh, ());
    let pixels: &Arc<_> = &frame.pixels;
    let size = pixels.dimensions();
    let renderer = gl::Renderer::new(&wl.conn, layer.wl_surface(), size, pixels)
        .context("could not start OpenGL ES for zoom")
        .hint("run `valw doctor` to check EGL")?;

    wl.state.zoom = Some(Zoom {
        renderer,
        layer,
        viewport,
        size,
        ratio: 1.0,
        view: Animated::new(),
        step: config.scroll_step,
        pointer: (0.0, 0.0),
        panning: None,
        flashlight: false,
        radius: config.flashlight_radius,
        ctrl: false,
        configured: false,
        frame_pending: false,
        dirty: true,
        last_draw: None,
        outcome: None,
        error: None,
    });
    let result = wl.dispatch_blocking(|s| {
        s.zoom
            .as_ref()
            .is_some_and(|z| z.outcome.is_some() || z.error.is_some())
    });
    let ended = wl.state.zoom.take().map(|z| (z.outcome, z.error));
    // Dropping the zoom destroys its surface; make that visible right away.
    let _ = wl.conn.flush();
    result?;
    match ended {
        Some((_, Some(e))) => Err(e),
        Some((Some(outcome), None)) => Ok(outcome),
        _ => Ok(Outcome::Leave),
    }
}

impl Zoom {
    fn is(&self, surface: &WlSurface) -> bool {
        self.layer.wl_surface() == surface
    }

    fn sizef(&self) -> (f64, f64) {
        (self.size.0 as f64, self.size.1 as f64)
    }

    pub fn configure(&mut self, layer: &LayerSurface, (w, h): (u32, u32), qh: &QueueHandle<State>) {
        if !self.is(layer.wl_surface()) {
            return;
        }
        if w > 0 {
            self.ratio = self.size.0 as f64 / w as f64;
        }
        // Show the physical-size buffer 1:1 on the logical-size surface.
        self.viewport.set_destination(w as i32, h as i32);
        self.configured = true;
        self.request_draw(qh);
    }

    pub fn closed(&mut self, layer: &LayerSurface) {
        if self.is(layer.wl_surface()) {
            self.outcome.get_or_insert(Outcome::Leave);
        }
    }

    pub fn frame_done(&mut self, surface: &WlSurface, qh: &QueueHandle<State>) {
        if !self.is(surface) {
            return;
        }
        self.frame_pending = false;
        if self.dirty {
            self.draw(qh);
        }
    }

    fn request_draw(&mut self, qh: &QueueHandle<State>) {
        self.dirty = true;
        if self.configured && !self.frame_pending {
            self.draw(qh);
        }
    }

    fn draw(&mut self, qh: &QueueHandle<State>) {
        let now = Instant::now();
        let dt = self
            .last_draw
            .map_or(0.0, |t| now.duration_since(t).as_secs_f64().min(0.1));
        let moving = self.view.step(dt);
        self.last_draw = moving.then_some(now);
        let surface = self.layer.wl_surface();
        surface.frame(qh, FrameCallbackData(surface.clone()));
        let flashlight = self
            .flashlight
            .then(|| (self.pointer.0, self.pointer.1, self.radius * self.ratio));
        if let Err(e) = self.renderer.draw(&self.view.shown, flashlight) {
            self.error = Some(e);
            return;
        }
        self.frame_pending = true;
        self.dirty = moving;
    }

    pub fn pointer(
        &mut self,
        events: &[PointerEvent],
        cursor: Option<&WpCursorShapeDeviceV1>,
        qh: &QueueHandle<State>,
    ) {
        for event in events {
            if !self.is(&event.surface) {
                continue;
            }
            let p = (event.position.0 * self.ratio, event.position.1 * self.ratio);
            match event.kind {
                PointerEventKind::Enter { serial } => {
                    if let Some(cursor) = cursor {
                        cursor.set_shape(serial, Shape::Default);
                    }
                    self.pointer = p;
                }
                PointerEventKind::Press {
                    button: BTN_LEFT,
                    serial,
                    ..
                } => {
                    self.panning = Some(p);
                    if let Some(cursor) = cursor {
                        cursor.set_shape(serial, Shape::Grabbing);
                    }
                }
                PointerEventKind::Release {
                    button: BTN_LEFT,
                    serial,
                    ..
                } => {
                    self.panning = None;
                    if let Some(cursor) = cursor {
                        cursor.set_shape(serial, Shape::Default);
                    }
                }
                PointerEventKind::Motion { .. } => {
                    self.pointer = p;
                    if let Some(last) = self.panning {
                        let size = self.sizef();
                        let moved = self.view.target.pan((p.0 - last.0, p.1 - last.1), size);
                        self.view.set(moved);
                        self.panning = Some(p);
                        self.request_draw(qh);
                    } else if self.flashlight {
                        self.request_draw(qh);
                    }
                }
                PointerEventKind::Axis { vertical, .. } => {
                    let steps = view::scroll_steps(vertical.value120, vertical.absolute);
                    if steps == 0.0 {
                        continue;
                    }
                    if self.ctrl {
                        self.radius = view::radius(self.radius, steps, self.step);
                    } else {
                        let size = self.sizef();
                        self.view.target =
                            self.view.target.zoom_at(p, self.step.powf(steps), size);
                    }
                    self.request_draw(qh);
                }
                _ => {}
            }
        }
    }

    pub fn key(&mut self, key: Keysym, qh: &QueueHandle<State>) {
        match command(key) {
            Some(Command::Leave) => {
                self.outcome.get_or_insert(Outcome::Leave);
            }
            Some(Command::Capture) => {
                let rect = self.view.target.visible_rect(self.size);
                tracing::info!("zoom capture {rect:?} at {:.2}x", self.view.target.scale);
                self.outcome.get_or_insert(Outcome::Capture(rect));
            }
            Some(Command::Flashlight) => {
                self.flashlight = !self.flashlight;
                self.request_draw(qh);
            }
            Some(Command::Reset) => {
                self.view.target = View::IDENTITY;
                self.request_draw(qh);
            }
            None => {}
        }
    }

    pub fn modifiers(&mut self, modifiers: Modifiers) {
        self.ctrl = modifiers.ctrl;
    }
}
```

- [ ] **Step 3: Route events in `src/wayland.rs`**

- `use crate::zoom::Zoom;`; field `/// Zoom mode, while it is open.` `pub zoom: Option<Zoom>,` after `overlay`; `zoom: None,` in `connect`.
- `frame`: add `if let Some(zoom) = &mut self.zoom { zoom.frame_done(surface, qh); }`.
- `closed`: add `if let Some(zoom) = &mut self.zoom { zoom.closed(layer); }`.
- `configure`: add `if let Some(zoom) = &mut self.zoom { zoom.configure(layer, configure.new_size, qh); }`.
- `press_key` (take `qh` instead of `_`):

```rust
        if let Some(overlay) = &mut self.overlay {
            match event.keysym {
                Keysym::Escape => overlay.cancel(),
                Keysym::space => overlay.switch_to_window(),
                _ => {}
            }
        }
        if let Some(zoom) = &mut self.zoom {
            zoom.key(event.keysym, qh);
        }
```

- `update_modifiers` (name the `Modifiers` argument `modifiers`): `if let Some(zoom) = &mut self.zoom { zoom.modifiers(modifiers); }`.
- `pointer_frame`: add `if let Some(zoom) = &mut self.zoom { zoom.pointer(events, self.cursor_device.as_ref(), qh); }`.

- [ ] **Step 4: `src/main.rs`**

Remove the Task 1 allowance. `enum Command` after `Window`:

```rust
    /// Freeze the screen and zoom in: wheel, drag, f, c, 0, Esc.
    Zoom {
        #[command(flatten)]
        common: Common,
    },
```

`Mode::Zoom`; in `run`: `Command::Zoom { common } => capture(Mode::Zoom, common),`; in `capture`'s match, after `Mode::Window`:

```rust
        Mode::Zoom => {
            let focused = focused_output(&outputs);
            let frame = wl.capture(&outputs[focused..=focused], cursor)?.remove(0);
            match zoom::run(&mut wl, &outputs[focused], &frame, &config.zoom)? {
                zoom::Outcome::Capture(r) => {
                    let source = frame.output.name.clone();
                    (vec![(frame.to_rgba(r), None)], 0, source)
                }
                zoom::Outcome::Leave => {
                    tracing::info!("left zoom without capturing");
                    return Ok(());
                }
            }
        }
```

- [ ] **Step 5: Run** — `cargo nextest run` (all pass, +1), clippy and fmt clean.

- [ ] **Step 6: Commit** — `jj commit -m "feat: valw zoom"`

---

### Task 4: Smoke run, checks and checklist

- [ ] **Step 1: Smoke run** (tell the user first: it covers the focused output and takes the keyboard for 3 s)

Run: `LD_LIBRARY_PATH=<libglvnd>/lib timeout -s INT 3 cargo run -q -- zoom; echo exit=$?` then read the newest log in `~/.local/state/valw/` (or wherever `log::default_dir()` points).
Expected: `zoom renderer: Mesa Intel(R) HD Graphics 4000 …`, no errors; exit from the signal.

- [ ] **Step 2: Full checks** — `nix flake check -L --keep-going`: all pass (headless sway now on libwayland-client).

- [ ] **Step 3: Checklist** — append to `docs/test-checklist.md`:

```markdown

## Zoom (Phase 4)

- [ ] `valw zoom` → the focused output freezes at 1×; Esc and `q` leave, nothing saved, exit 0, thumbnails come back.
- [ ] Wheel zooms towards the pointer (the point under it stays put), smoothly, up to 32×; pixels stay crisp.
- [ ] Touchpad scrolling zooms smoothly too.
- [ ] Left drag pans 1:1 and never shows anything outside the screenshot; the cursor turns into a grabbing hand.
- [ ] `f` toggles the flashlight (outside dimmed to 25 %), it follows the pointer; Ctrl+wheel resizes it.
- [ ] `0` animates back to 1×.
- [ ] `c` at some zoom → the saved image is exactly the visible area at native resolution; preview and clipboard as usual.
- [ ] On the 1366×768 laptop panel and the 1920×1080 monitor; stays smooth (no stutter) on the HD 4000.
- [ ] `[zoom] scroll_step` / `flashlight_radius` in the config take effect; `valw doctor` shows the EGL line.
```

- [ ] **Step 4: Commit** — `jj commit -m "docs: zoom checklist"`
