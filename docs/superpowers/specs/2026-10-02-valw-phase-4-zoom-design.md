# valw Phase 4: zoom mode (design)

Date: 2026-10-02
Status: approved in brainstorming, pending written-spec review
Builds on: Phases 0–3 (on `main`); vision section 4.7

## 1. Goal

A woomer-like magnifier: `valw zoom` freezes the focused output and lets
the user zoom towards the pointer, pan, and spotlight an area, then either
leave or capture what they see.

Success: zooming and panning stay smooth (60 fps) on the user's Intel HD
4000 at 1920×1080, pixels stay crisp, and `c` hands a crop to the usual
save → preview → clipboard flow.

### Out of scope

- Zoom on every output at once, smooth (linear) filtering, several
  captures from one zoom session.
- Vulkan or wgpu (the user asked for no Vulkan).

## 2. Decisions

| Question | Decision | Why |
|---|---|---|
| Renderer | OpenGL ES 3.0 through EGL (`khronos-egl`, `wayland-egl`, `glow`) | No Vulkan; a single textured triangle doesn't need wgpu. A spike on the user's HD 4000 (Mesa, crocus) got GLES 3.0, a compiling `#version 300 es` shader, `MAX_TEXTURE_SIZE` 16384 and 61 fps at 1920×1080 on a layer surface |
| Wayland backend | `wayland-backend` with `client_system` (libwayland-client) for the whole program | EGL and `wayland-egl` need the C `wl_display*` and `wl_proxy*`; the API valw uses doesn't change |
| Output | The focused output only | Simple, like woomer |
| Filtering | Nearest | Crisp pixels for inspection |
| `c` | Crops the visible area of the original frame (native pixels), runs the normal capture flow, exits | Zoomed region select; no upscaled pixels |
| Esc / `q` | Leave without saving, exit code 0 | Leaving is the normal end of zoom, not a cancelled capture |

## 3. Behaviour

Start, as for the other modes: load the config, take the capture lock,
wait `--delay`, hide the thumbnails, capture niri's focused output (with
the cursor if `--cursor`/`show_cursor`). Then a full-screen layer surface
(namespace `valw-zoom`, overlay layer, exclusive keyboard) on that output
shows the frozen frame at 1×.

| Input | Effect |
|---|---|
| Wheel | Zoom towards the pointer: the frame point under the pointer stays put. One notch = one `zoom.scroll_step` (default 1.15); touchpad scrolling: one step per 15 logical px. Range 1×–32× |
| Left drag | Pan, 1:1 with the pointer |
| `f` | Flashlight on/off: outside a circle around the pointer the image is dimmed to 25 % brightness |
| Ctrl + wheel | Flashlight radius × / ÷ `scroll_step` (default `zoom.flashlight_radius` = 180 logical px, range 20–2000) |
| `0` | Back to 1× (animated) |
| `c` | Capture the visible area (see 2) and exit |
| Esc, `q` | Exit, nothing saved |

- The view never leaves the image: at 1× it is exactly the frame, and a
  zoomed view is clamped to the frame's edges.
- Zoom and reset animate: the shown scale and position approach their
  targets exponentially (about 90 % in 120 ms) and snap when close. Pan is
  immediate. Frames are drawn on frame callbacks only while something
  changes; an idle view draws nothing.
- The cursor is the default pointer; while panning it is `grabbing`.

## 4. Structure

| File | Role |
|---|---|
| `src/zoom/view.rs` | Pure view math: `View { scale, origin }` (origin = the frame pixel at the top-left of the screen), `zoom_at`, `pan`, `reset`, `clamp`, the animation step, `visible_rect(frame size) -> PixelRect`, scroll → zoom factor, flashlight radius bounds |
| `src/zoom/gl.rs` | EGL display/context/window surface on the layer surface, the texture upload, one shader program (fullscreen triangle; fragment shader maps screen → frame pixels, `texelFetch`/nearest, swizzles BGRX, flashlight dimming) |
| `src/zoom/mod.rs` | `run(wl, output, frame, config) -> Result<Option<PixelRect>>`: surface, input routing, the draw loop; `Some(rect)` for `c`, `None` for Esc/`q` |
| `src/wayland.rs` | `State::zoom`; pointer, axis, key and modifier events go to it while it is open |
| `src/main.rs` | `Command::Zoom`, `Mode::Zoom`; `None` returns `Ok(())` before saving |
| `src/config.rs` | `[zoom] scroll_step = 1.15`, `flashlight_radius = 180` |
| `src/doctor.rs` | A line for EGL + an OpenGL ES 3 config (`warn`, not `fail`, when missing: only zoom needs it) |
| `nix/package.ulu.nix` | `LD_LIBRARY_PATH` suffix with libglvnd (`libEGL.so.1`); Mesa's drivers come from `/run/opengl-driver` on NixOS |

A missing EGL/GLES 3 makes `valw zoom` fail with a clear error and the
hint to run `valw doctor`; the other modes don't touch EGL.

## 5. Testing

Unit tests (`cargo nextest`) for `view.rs`:
- `zoom_at` keeps the frame point under the pointer fixed (several
  scales, pointer at a corner and the centre).
- Scale is clamped to 1–32; at 1× the origin is 0,0; a zoomed origin is
  clamped to the frame edges; `pan` respects the same clamp.
- The animation reaches the target and then reports "idle".
- `visible_rect` at 1× is the whole frame; at 4× with a known origin it
  is the expected rectangle, rounded outwards and inside the frame.
- Scroll → factor for wheel notches and touchpad pixels; flashlight radius
  bounds.
- Config: the `[zoom]` defaults and overrides; doctor: the EGL line's
  levels.

The headless sway check keeps covering screencopy and the overlay, now on
`client_system`. Zoom needs a GPU, so it is checked by hand: the Phase 4
section of `docs/test-checklist.md`.

## 6. Definition of done

`cargo clippy -- -D warnings`, `cargo fmt --check`, `cargo nextest run`
and `nix flake check` pass; the checklist section is written.
