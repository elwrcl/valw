# valw theme round Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A shared paint shader and palettes (T1, with a themed region dim), `valw backdrop` behind niri's overview (T2), and an Alt+Tab-style window picker (T3).

**Architecture:** Pure pieces first and tested (`theme` palettes, `render::tint`, backdrop timing, picker layout/navigation/state, icon lookup), then the GL renderer shared by backdrop and picker (`theme::gl`, reusing zoom's EGL setup), then the two surfaces.

**Tech Stack:** Rust 2024, SCTK 0.21, EGL + glow (GLES 3), tiny-skia, `resvg` 0.45-ish (whatever `cargo add resvg` resolves to; built on tiny-skia), `noctalia msg` at run time.

**Spec:** `docs/superpowers/specs/2026-10-03-valw-theme-design.md`

## Global Constraints

- jj only; `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo nextest run`, `nix flake check`.
- No Vulkan/wgpu. Shader code is our own (from the spike), not Balatro's.
- Default palette (no Noctalia): base `#1b1f2a`, body `#2e3a52`, highlight `#8fa6c9`.
- Themed dim: 55 % towards `base`. Backdrop: flow, slow (speed 0.35), re-read wallpaper after a > 2 s pause, colours ease over 1 s. Picker: icons 96 logical px, selected card 110 % with a 2 px border, colour ease 300 ms, motion alternates per run (state `$XDG_STATE_HOME/valw/picker`, missing/broken → swirl).
- Noctalia is optional everywhere; failures fall back to the default palette and log at debug.
- Tell the user before showing surfaces on the live session.

## Review Focus

1. **Noctalia missing / `noctalia msg` hanging**: the region overlay and picker must not wait on it. `noctalia msg` runs with a 1 s timeout (Task 1).
2. **The backdrop must cost nothing while hidden** and survive output hotplug and niri restarts (Task 4).
3. **Icon lookup edge cases**: app ids with dots/case differences, `StartupWMClass`, absolute `Icon=` paths, missing icons (Task 5).
4. **The picker must never end up in the shot** (same destroy/flush/round-trip as the toolbar) (Task 6).
5. **Many windows** (20+) still fit: rows wrap and shrink (Task 5 layout tests).

---

## File Structure

| File | Task |
|---|---|
| `src/theme/mod.rs` (Palette, sources) | 1 |
| `src/render.rs` (`tint`), `src/region.rs` | 2 |
| `src/theme/paint.frag`, `src/theme/gl.rs`; `src/zoom/gl.rs` (`open`, `config` → `pub(crate)`) | 3 |
| `src/backdrop.rs`, `src/main.rs`, `src/wayland.rs` | 4 |
| `src/niri.rs` (`windows`), `src/picker/{layout,icons,state}.rs` | 5 |
| `src/picker/mod.rs`, `src/main.rs`, `src/wayland.rs` | 6 |
| README, checklist, copland | 7 |

---

### Task 1: Palettes

**Files:** create `src/theme/mod.rs`; `src/main.rs`: `mod theme;` (+ temporary dead-code allowance until Task 2).

**Produces:** `theme::Palette { base: [f32; 3], body: [f32; 3], highlight: [f32; 3] }` with `Palette::DEFAULT`, `Palette::lerp(&self, other, t)`; `theme::hex(&str) -> Option<[f32; 3]>`; `theme::from_noctalia_file(text: &str, mode: &str) -> Option<Palette>`; `theme::from_image(img: &RgbaImage, icon: bool) -> Palette`; `theme::current() -> Palette` (Noctalia or default); `theme::wallpaper(connector: &str) -> Option<PathBuf>`; private `noctalia(args) -> Option<String>` (1 s timeout).

- [ ] **Step 1: Failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    const FILE: &str = r##"{"dark":{"mPrimary":"#cabaaa","mSecondary":"#73685F","mSurface":"#1e1d1b"},
                           "light":{"mPrimary":"#262524","mSecondary":"#736B5E","mSurface":"#D4C1A8"}}"##;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 0.01)
    }

    #[test]
    fn hex_parses() {
        assert!(close(hex("#ff8000").unwrap(), [1.0, 0.502, 0.0]));
        assert!(close(hex("D4C1A8").unwrap(), [0.831, 0.757, 0.659]));
        assert_eq!(hex("#12"), None);
    }

    #[test]
    fn noctalia_palette_per_mode() {
        let light = from_noctalia_file(FILE, "light").unwrap();
        assert!(close(light.base, hex("#D4C1A8").unwrap()));
        assert!(close(light.body, hex("#736B5E").unwrap()));
        assert!(close(light.highlight, hex("#262524").unwrap()));
        let dark = from_noctalia_file(FILE, "dark").unwrap();
        assert!(close(dark.base, hex("#1e1d1b").unwrap()));
        assert_eq!(from_noctalia_file(FILE, "sepia"), None);
        assert_eq!(from_noctalia_file("{}", "light"), None);
        assert_eq!(from_noctalia_file("not json", "light"), None);
    }

    #[test]
    fn image_palette_is_dark_to_light() {
        let img = RgbaImage::from_fn(30, 30, |x, _| match x / 10 {
            0 => Rgba([250, 240, 20, 255]),
            1 => Rgba([20, 30, 160, 255]),
            _ => Rgba([200, 30, 30, 255]),
        });
        let p = from_image(&img, false);
        let lum = |c: [f32; 3]| c[0] + c[1] + c[2];
        assert!(lum(p.base) < lum(p.body) && lum(p.body) < lum(p.highlight), "{p:?}");
    }

    #[test]
    fn icon_palettes_ignore_transparent_and_grey() {
        let img = RgbaImage::from_fn(30, 30, |x, y| {
            if x < 15 { Rgba([0, 0, 0, 0]) } else if y < 10 { Rgba([128, 128, 128, 255]) } else { Rgba([30, 160, 220, 255]) }
        });
        let p = from_image(&img, true);
        for c in [p.base, p.body, p.highlight] {
            assert!(c[2] > c[0], "blue family, not grey or black: {p:?}");
        }
    }

    #[test]
    fn lerp_eases() {
        let a = Palette::DEFAULT;
        let b = Palette { base: [1.0; 3], body: [1.0; 3], highlight: [1.0; 3] };
        assert_eq!(a.lerp(&b, 0.0), a);
        assert_eq!(a.lerp(&b, 1.0), b);
    }
}
```

Run → compile errors.

- [ ] **Step 2: Implement** (`serde_json` is already a dependency):

```rust
//! valw's colours: palettes for the paint shader and the overlays, from
//! Noctalia's theme, from an image, or a built-in default.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use image::RgbaImage;

/// Three colours, dark to light, as linear 0–1 RGB.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub base: [f32; 3],
    pub body: [f32; 3],
    pub highlight: [f32; 3],
}

impl Palette {
    pub const DEFAULT: Palette = Palette {
        base: [0.106, 0.122, 0.165],
        body: [0.180, 0.227, 0.322],
        highlight: [0.561, 0.651, 0.788],
    };

    pub fn lerp(&self, other: &Palette, t: f32) -> Palette {
        let t = t.clamp(0.0, 1.0);
        let mix = |a: [f32; 3], b: [f32; 3]| [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t);
        Palette { base: mix(self.base, other.base), body: mix(self.body, other.body), highlight: mix(self.highlight, other.highlight) }
    }
}

pub fn hex(s: &str) -> Option<[f32; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let v = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok().map(|v| v as f32 / 255.0);
    Some([v(0)?, v(2)?, v(4)?])
}

/// A Noctalia community palette file at `mode`.
pub fn from_noctalia_file(text: &str, mode: &str) -> Option<Palette> {
    let json: serde_json::Value = serde_json::from_str(text).ok()?;
    let m = json.get(mode)?;
    let get = |k: &str| hex(m.get(k)?.as_str()?);
    Some(Palette { base: get("mSurface")?, body: get("mSecondary")?, highlight: get("mPrimary")? })
}

/// The dominant colours of `img`, dark to light. Icons skip transparent and
/// near-grey pixels so the brand colour wins.
pub fn from_image(img: &RgbaImage, icon: bool) -> Palette {
    let step = ((img.width() * img.height()) as usize / 4096).max(1);
    let mut buckets = std::collections::HashMap::<(u8, u8, u8), (u32, [u64; 3])>::new();
    for p in img.pixels().step_by(step) {
        let [r, g, b, a] = p.0;
        if icon {
            let (max, min) = (r.max(g).max(b), r.min(g).min(b));
            if a < 128 || max - min < 24 {
                continue;
            }
        }
        let e = buckets.entry((r >> 5, g >> 5, b >> 5)).or_default();
        e.0 += 1;
        for (s, v) in e.1.iter_mut().zip([r, g, b]) {
            *s += v as u64;
        }
    }
    let mut groups: Vec<_> = buckets.into_values().collect();
    groups.sort_by(|a, b| b.0.cmp(&a.0));
    let mut pick: Vec<[f32; 3]> = Vec::new();
    for (n, sum) in groups {
        let c = sum.map(|s| s as f32 / n as f32 / 255.0);
        if pick.iter().all(|p| (0..3).map(|i| (p[i] - c[i]).abs()).sum::<f32>() > 0.3) {
            pick.push(c);
        }
        if pick.len() == 3 {
            break;
        }
    }
    match pick.len() {
        0 => return Palette::DEFAULT,
        1 => {
            let c = pick[0];
            pick = vec![c.map(|v| v * 0.45), c, c.map(|v| v + (1.0 - v) * 0.5)];
        }
        2 => {
            let c = pick[1];
            pick.push(c.map(|v| v + (1.0 - v) * 0.5));
        }
        _ => {}
    }
    pick.sort_by(|a, b| (a[0] + a[1] + a[2]).total_cmp(&(b[0] + b[1] + b[2])));
    Palette { base: pick[0], body: pick[1], highlight: pick[2] }
}

/// `noctalia msg <args>`'s output, or None (not installed, failing, or
/// slower than a second).
fn noctalia(args: &[&str]) -> Option<String> {
    let mut child = Command::new("noctalia")
        .arg("msg")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
    Some(out.trim().to_string())
}

/// Noctalia's current palette, or the default.
pub fn current() -> Palette {
    noctalia_palette().unwrap_or_else(|| {
        tracing::debug!("using the default palette");
        Palette::DEFAULT
    })
}

fn noctalia_palette() -> Option<Palette> {
    let scheme = noctalia(&["color-scheme-get"])?; // "<source> <name>"
    let (source, name) = scheme.split_once(' ')?;
    if source != "community" {
        return None;
    }
    let mode = noctalia(&["theme-mode-get"])?;
    let dir = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::config::home().join(".local/state"));
    let file = dir.join("noctalia/community-palettes").join(format!("{}.json", name.replace(' ', "%20")));
    from_noctalia_file(&std::fs::read_to_string(file).ok()?, &mode)
}

/// The wallpaper Noctalia shows on `connector`.
pub fn wallpaper(connector: &str) -> Option<PathBuf> {
    noctalia(&["wallpaper-get", connector]).map(PathBuf::from).filter(|p| p.exists())
}
```

(Check `noctalia msg color-scheme-get`'s exact output on this machine and adapt the split; ledger what it printed.)

- [ ] **Step 3: Run**, clippy, fmt. **Step 4: Commit** `feat: theme palettes`.

---

### Task 2: Themed region dim

**Files:** `src/render.rs`, `src/region.rs`.

**Produces:** `render::tint(pixels: &[u8], colour: [f32; 3], amount: f32) -> Vec<u8>` (BGRX bytes; X untouched-ish); `render::DIM` stays for tests only or is removed if unused.

- [ ] **Step 1: Failing test** (`render.rs`):

```rust
    #[test]
    fn tint_blends_towards_the_colour() {
        // B, G, R, X = 200, 100, 0, 255; colour pure red (R = 1).
        let out = tint(&[200, 100, 0, 255], [1.0, 0.0, 0.0], 0.55);
        assert_eq!(out[0], (200.0 * 0.45f32).round() as u8, "blue fades");
        assert_eq!(out[1], (100.0 * 0.45f32).round() as u8);
        assert_eq!(out[2], (255.0 * 0.55f32).round() as u8, "red rises");
        assert_eq!(tint(&[10, 20, 30, 0], [0.0; 3], 0.0), vec![10, 20, 30, 0]);
    }
```

- [ ] **Step 2: Implement**:

```rust
/// B, G, R, X pixels blended `amount` of the way towards `colour` (RGB 0–1).
/// Runs once per output before the overlay shows, like `dim` did.
pub fn tint(pixels: &[u8], colour: [f32; 3], amount: f32) -> Vec<u8> {
    let keep = 1.0 - amount;
    let [r, g, b] = colour.map(|c| c * 255.0 * amount);
    let add = [b, g, r, 0.0];
    pixels
        .chunks_exact(4)
        .flat_map(|p| [0, 1, 2, 3].map(|i| if i == 3 { p[3] } else { (p[i] as f32 * keep + add[i]).round() as u8 }))
        .collect()
}
```

(If clippy prefers `as_chunks`, follow it.) `region.rs`: `select` computes `let palette = crate::theme::current();` once and `Surface::new` uses `render::tint(frame.pixels.as_raw(), palette.base, TINT)` with `const TINT: f32 = 0.55;` in `render.rs`; drop `dim`/`DIM` if nothing else uses them (update its test accordingly). Remove Task 1's allowance.

Note the performance: `tint` is a float pass over every pixel; on 1920×1080 that is ~8 M bytes — keep it a tight loop (it replaced `dim`'s integer pass). If it adds more than ~10 ms in release (measure with the existing "overlay buffers ready … after start" log line), switch to an integer form `(p * keep256 + add256) >> 8`.

- [ ] **Step 3: Run**, clippy, fmt. **Step 4: Commit** `feat: theme-coloured region dim`.

---

### Task 3: The paint renderer

**Files:** create `src/theme/paint.frag` (the spike's final `paint()` and `warp()`, with uniforms `res`, `t`, `motion`, `speed`, `scale`, `c1`, `c2`, `c3`, plus an optional overlay texture `overlay` and `use_overlay`), `src/theme/gl.rs`; `src/zoom/gl.rs`: `open`, `config`, `Egl` become `pub(crate)`.

**Produces:** `theme::gl::Paint::new(conn, surface: &WlSurface, size_px: (u32, u32)) -> Result<Paint>`; `Paint::resize(size_px)`; `Paint::draw(&self, frame: &Frame) -> Result<()>` with `pub struct Frame { pub time: f32, pub motion: f32, pub speed: f32, pub scale: f32, pub palette: Palette }`; `Paint::set_overlay(&mut self, pixmap: Option<&tiny_skia::Pixmap>)` (premultiplied RGBA, drawn over the paint with `ONE, ONE_MINUS_SRC_ALPHA` blending). Drop order as in `zoom::gl::Renderer` (EGL surface before `wl_egl_window`).

Fragment shader (final):

```glsl
#version 300 es
precision highp float;
uniform vec2 res;
uniform float t;
uniform float motion;   // 0 flow, 1 swirl
uniform float speed;
uniform float scale;    // blob size: 1 = the spike's
uniform vec3 c1;
uniform vec3 c2;
uniform vec3 c3;
uniform sampler2D overlay;
uniform bool use_overlay;
out vec4 color;

vec2 warp(vec2 p, float time) {
    for (int i = 0; i < 6; i++) {
        float fi = float(i);
        p += 0.42 * sin(p.yx * vec2(1.55, 1.28) + time * vec2(0.27, 0.33) + fi * vec2(1.3, 2.1));
        p = mat2(0.82, -0.57, 0.57, 0.82) * p;
    }
    return p;
}

vec3 paint(vec2 frag) {
    vec2 uv = (frag - 0.5 * res) / res.y;
    float time = t * speed;
    if (motion > 0.0) {
        float r = length(uv);
        float a = atan(uv.y, uv.x) + motion * (2.2 / (r + 0.35)) + time * 0.25 * motion;
        uv = r * vec2(cos(a), sin(a));
    }
    vec2 p = warp(uv * 4.0 / scale, time);
    p = warp(p * 1.7, time * 1.25 + 3.0);
    float v = sin(p.x) * cos(p.y);
    float w = sin(length(p) * 0.6 - time * 0.8);
    float m = v * 0.75 + w * 0.25;
    vec3 col = c1;
    col = mix(col, c2, smoothstep(-0.32, -0.18, m));
    col = mix(col, c3, smoothstep(0.38, 0.5, m) * 0.85);
    float rim = 1.0 - smoothstep(0.0, 0.06, abs(m + 0.25));
    col *= 1.0 - 0.3 * rim;
    float gloss = pow(abs(cos(p.x * 1.3 + p.y)), 36.0);
    col += gloss * 0.18 * (c3 + 0.25);
    col *= 1.0 - 0.3 * dot(uv, uv);
    return col;
}

void main() {
    vec2 frag = vec2(gl_FragCoord.x, res.y - gl_FragCoord.y);
    vec3 col = paint(frag);
    if (use_overlay) {
        vec4 o = texture(overlay, frag / res);
        col = o.rgb + col * (1.0 - o.a);
    }
    color = vec4(col, 1.0);
}
```

(One pass: the overlay is composited in the shader, no blending state needed.)

- [ ] **Step 1** (no unit test for GL; the spike proved the shader on this GPU): implement, `cargo clippy`, and a compile-only test that the shader source contains every uniform `Paint` sets (`#[test] fn shader_declares_its_uniforms`).
- [ ] **Step 2: Commit** `feat: paint shader renderer`.

---

### Task 4: `valw backdrop`

**Files:** create `src/backdrop.rs`; `src/main.rs` (`Command::Backdrop`), `src/wayland.rs` (routing).

**Produces:** pure `backdrop::should_refresh(last_frame: Option<Instant>, now: Instant) -> bool` (true when no frame yet or more than 2 s since the last) and `backdrop::ease(from: &Palette, to: &Palette, since: Duration) -> Palette` (linear over 1 s); `backdrop::run() -> Result<()>`.

- [ ] **Step 1: Failing tests** for `should_refresh` (None → true, 1 s → false, 3 s → true) and `ease` (0 → from, 0.5 s → halfway, 2 s → to).
- [ ] **Step 2: Implement** `run`:
  - Single instance: `Lock::acquire(lock::default_path().with_file_name("valw-backdrop.lock"))`, else log and exit 0.
  - `Wayland::connect`, one `Backdrop` per output in `State::backdrops: Vec<Backdrop>`: background-layer surface, `Anchor::all()`, exclusive zone -1 (cover panels too — niri draws it inside the backdrop), `KeyboardInteractivity::None`, empty input region, namespace `valw-backdrop`, a `theme::gl::Paint` sized to the output's physical size, a palette (current/target/eased since), `last_frame`.
  - Draw on configure and on each frame callback (request the next one before swapping). In the frame handler: if `should_refresh(last_frame, now)`, read `theme::wallpaper(name)` → `image` decode (downscale to ~256 px with `thumbnail`) → `from_image(.., false)` as the new target (keep the old if anything fails).
  - Outputs: new outputs get a surface; destroyed ones drop theirs (SCTK `OutputHandler::new_output` / `output_destroyed`).
  - Dispatch forever (`dispatch_blocking` loop); exit when the compositor goes away.
- [ ] **Step 3: Run tests**, clippy, fmt. **Step 4: Smoke** (tell the user: without the niri layer rule the backdrop shows over the wallpaper for a few seconds): `timeout -s INT 4 valw backdrop`, grim shot, check the log. **Step 5: Commit** `feat: valw backdrop`.

---

### Task 5: Picker data — windows, icons, layout, state

**Files:** `src/niri.rs` (`windows() -> Result<Vec<Window>>`), create `src/picker/mod.rs` (module list), `src/picker/layout.rs`, `src/picker/icons.rs`, `src/picker/state.rs`; `Cargo.toml`: `resvg`.

**Produces:**
- `niri::windows()`; `picker::mru(windows) -> Vec<Window>` (focused first, then by `focus_timestamp` descending, windows without a timestamp last by id).
- `layout::cards(n: usize, output: (f32, f32), selected: usize) -> Vec<Rect>` (logical px; card base 200 × 200: 96 px icon + title + app name; 24 px gaps; rows wrap to the output width minus 10 % margins; when rows exceed 60 % of the height the whole grid scales down to fit; the selected card scaled 1.1 about its centre); `layout::hit(cards, p) -> Option<usize>`; `layout::step(selected, n, delta: i32) -> usize` (wraps).
- `state::next_motion(path) -> f32` (reads the last motion, returns the other, writes it; missing/broken → swirl = 1.0 and writes it).
- `icons::find(app_id: &str, dirs: &Dirs) -> Option<PathBuf>`; `Dirs { data: Vec<PathBuf>, theme: String }` from `$XDG_DATA_HOME`, `$XDG_DATA_DIRS`, and `gtk-icon-theme-name` in `~/.config/gtk-3.0/settings.ini` (default `hicolor`); `icons::load(path, size_px) -> Option<RgbaImage>` (PNG via `image`, SVG via `resvg`).

- [ ] **Step 1: Failing tests**
  - `mru`: focused first, then newest timestamps, then no-timestamp by id.
  - `cards`: 1 card centred; 5 cards in one row on 1920×1080; 20 cards on 1366×768 wrap into rows that fit inside 90 % of the width and 60 % of the height (scaled), no overlaps; the selected card is 1.1× and still centred on its slot; `hit` on a card centre and in a gap.
  - `step`: +1 at the end wraps to 0, −1 at 0 wraps to n−1, n = 0 → 0.
  - `next_motion`: missing → 1.0 then 0.0 then 1.0; garbage file → 1.0.
  - `icons::find` against a temp data dir:
    - `applications/org.telegram.desktop.desktop` with `Icon=telegram` and `icons/hicolor/256x256/apps/telegram.png` → found for app id `org.telegram.desktop`;
    - a desktop file `foo.desktop` with `StartupWMClass=FooApp` → found for `FooApp`;
    - theme `mytheme` with `index.theme` `Inherits=hicolor` and the icon only in hicolor → found;
    - `Icon=/abs/path.png` → that path;
    - prefers `scalable/apps/x.svg` over smaller PNGs;
    - unknown app id → None.
  - `icons::load`: a tiny SVG (`<svg …><rect fill="#f00" …/></svg>`) renders to the requested size with red pixels; a PNG loads and resizes.
- [ ] **Step 2: Implement.** Desktop lookup: for each data dir `applications/`, try `<app_id>.desktop`, then case-insensitive file name match, then scan for `StartupWMClass=<app_id>` (case-insensitive). Icon theme lookup: for theme, then its `Inherits` chain, then `hicolor`: directories in order `scalable/apps`, `256x256/apps`, `128x128/apps`, `96x96/apps`, `64x64/apps`, `48x48/apps`, and any `*/apps` under the theme (first hit), extensions `svg` then `png`; also `pixmaps/<name>.(png|svg)`. Bound the search (no recursion beyond `theme/size/apps`).
- [ ] **Step 3: Run**, clippy, fmt. **Step 4: Commit** `feat: picker windows, layout, state and icons`.

---

### Task 6: The picker

**Files:** `src/picker/mod.rs`, `src/main.rs`, `src/wayland.rs`.

**Produces:** `picker::run(config: &Config) -> Result<u64>` (the chosen window id; `Cancelled` on Esc / click outside); `Command::Window { pick: bool, common }` — `--pick` keeps the old niri pick; without it, `picker::run` chooses and the existing `window_shot` path captures that id (`window::capture_id(id, cursor)` split out of `window::capture`).

- Surface: overlay layer on the focused output, `Anchor::all()`, exclusive keyboard, namespace `valw-picker`; `theme::gl::Paint` with `motion = state::next_motion(..)`, `speed = 0.6`, `scale = 1.0`; palette eases (300 ms) towards `from_image(icon, true)` of the selected window (icons loaded once at start, at 96 × scale px; a missing icon → the default palette and a letter tile).
- Cards are drawn with tiny-skia into a full-output `Pixmap` (rounded panel `#000000` alpha 90 with 1 px `#ffffff` alpha 40 border; selected panel alpha 140 and a 2 px border in the palette's highlight; the icon; title 14 px white ellipsised to the card width; app name 12 px white alpha 170) and uploaded as the overlay only when the selection or hover changes.
- Frames: frame-callback driven while open (the shader animates), the overlay re-uploaded only on change.
- Input as the spec says (Tab/Shift+Tab/arrows, Enter, Esc, hover, click, click outside).
- On choice: drop the picker (surface destroyed), flush, round-trip, return the id. The caller (`capture`) then runs `window_shot` for that id.
- Tests: `main` CLI (`window`, `window --pick`), and the mapping in `capture` stays covered by existing tests; the picker's own logic is covered by Task 5.
- Smoke (tell the user): open the picker for a few seconds, grim shot, check the cards and the shader.
- Commit `feat: window picker`.

---

### Task 7: Wiring, docs, checks

- [ ] README: the picker (`valw window`, `--pick`), `valw backdrop` with the niri layer rule and Noctalia's `[backdrop] enabled = false`.
- [ ] Checklist section "Theme".
- [ ] copland (with the user's approval): `spawn-at-startup` `valw backdrop`, the niri `layer-rule`, Noctalia backdrop off (wherever copland sets Noctalia's config; if it is not managed by copland, tell the user to toggle it in Noctalia's settings), `nix flake update valw`.
- [ ] `nix flake check`, review, push.
