# valw theme round: the paint shader, overview backdrop, window picker (design)

Date: 2026-10-03
Status: approved in brainstorming (after the throwaway `valw-lab` spike), pending written-spec review
Builds on: every phase on `main`; zoom's EGL/GLES renderer (`src/zoom/gl.rs`)

## 1. Goal

Give valw a theme of its own: a Balatro-like liquid-paint background,
written from scratch as a shader, coloured from the user's theme, a
wallpaper or a window. Three uses:

- **T1** A shared theme module (shader, palettes); the region overlay's
  dark dim takes the theme's colour (no shader there: the user must see
  what they select).
- **T2** `valw backdrop`: the shader, calm and slow, behind niri's overview,
  coloured from the wallpaper (replacing Noctalia's blurred backdrop).
- **T3** An Alt+Tab-style window picker (`valw window` from now on): cards
  for every window over the shader, the shader coloured from the selected
  window's icon; Enter or a click captures that window.

### Out of scope

- Live window thumbnails in the picker: niri 26.04 gives clients no way to
  read a window's pixels except `screenshot-window`, which also sets the
  clipboard and shows a notification every time. Recorded as a research
  item; cards show icons until then.
- The shader on the toolbar, zoom or editor (possible later on the same
  module).

## 2. The paint shader

The spike's shader, cleaned up (our own code, not Balatro's):

- Two octaves of domain warping (six sine-warp iterations each, a fixed
  rotation between them), banded into a 3-colour palette with soft edges,
  dark rims, glossy streaks and a soft vignette.
- Uniforms: time, resolution, `motion` (0 = flow, 1 = swirl around the
  centre), `speed`, `scale` (blob size), palette `base`, `body`,
  `highlight`.
- Lives in `src/theme/paint.frag` and is drawn by a renderer shared with
  zoom's GL code (EGL + glow; no Vulkan).

## 3. Palettes

`theme::Palette { base, body, highlight }` (linear 0–1 RGB), from:

1. **Noctalia's theme**: `noctalia msg color-scheme-get` and
   `theme-mode-get`; for a `community` scheme the palette file
   `$XDG_STATE_HOME/noctalia/community-palettes/<id>.json` at the current
   mode: base = `mSurface`, body = `mSecondary`, highlight = `mPrimary`
   (accent `mTertiary` kept for later). Other sources (builtin,
   wallpaper, custom) fall back to (3) until their files are mapped.
2. **An image** (a wallpaper, an icon): dominant colours by coarse
   bucketing of a downscaled copy; the three most frequent distinct
   colours (ignoring transparent and near-grey pixels for icons), sorted
   dark → light.
3. **A built-in default** (dark grey-blue), when Noctalia isn't there or
   anything above fails (logged at debug level, never an error).

## 4. T1: themed region dim

The overlay outside the selection becomes the frozen screen blended 55 %
towards the theme's `base` colour (today: darkened to 60 %), so it stays
see-through. Computed once per output before the overlay appears (no cost
per frame).

## 5. T2: `valw backdrop`

- A long-running process: one background-layer surface per output
  (namespace `valw-backdrop`, no keyboard, empty input region), drawing
  the shader with `motion = flow` at a slow speed, coloured from that
  output's wallpaper (`noctalia msg wallpaper-get <connector>`, palette
  source 2).
- Draws only on frame callbacks, so it costs nothing while niri doesn't
  show it. When frames resume after more than 2 s, it re-reads the
  wallpaper (a changed wallpaper changes the colours, easing over 1 s).
- Outputs coming and going are followed; a second `valw backdrop` exits
  at once (a lock, like the preview host's).
- Wiring (in copland, documented in the README): `spawn-at-startup "valw"
  "backdrop"`, niri `layer-rule { match namespace="^valw-backdrop$";
  place-within-backdrop true; }`, Noctalia `[backdrop] enabled = false`.

## 6. T3: the window picker

- `valw window` now opens the picker; the click-a-window mode stays as
  `valw window --pick` (and Space in region mode keeps using it).
- A full-screen overlay layer surface on the focused output (exclusive
  keyboard): the shader full screen, and over it a centred row (wrapping
  to more rows when needed) of cards for every window, most recently
  focused first (niri's `focus_timestamp`), the focused window's card
  selected.
- A card: a rounded translucent panel, the app icon (96 logical px), the
  window title (one line, ellipsised) and the app name below. The
  selected card is larger (110 %), brighter, with a 2 px light border.
- Keys: Tab / → / ↓ next, Shift+Tab / ← / ↑ previous, Enter capture,
  Esc cancel (exit 3). Mouse: hover selects, a click captures, a click
  outside the cards cancels.
- The shader's motion alternates per run (swirl, flow, swirl, …; the last
  one is remembered in `$XDG_STATE_HOME/valw/picker`), and stays fixed
  while the picker is open. Its colours come from the selected window's
  icon (palette source 2) and ease to the next card's over 300 ms.
- Icons: the window's `app_id` → its `.desktop` file (`$XDG_DATA_DIRS`
  applications, `StartupWMClass` or file name) → `Icon=` → the icon
  theme from `~/.config/gtk-3.0/settings.ini` (`gtk-icon-theme-name`),
  its `Inherits`, then `hicolor`; SVGs rendered with `resvg`. No icon: a
  rounded square with the app name's initial.
- Capturing: the picker closes (surface destroyed, flushed and
  round-tripped), then the Phase 3 path (`niri screenshot-window --id`),
  shadow, sound, preview, clipboard as usual.

## 7. Structure

| File | Role |
|---|---|
| `src/theme/mod.rs` | `Palette`, sources (Noctalia, image, default) |
| `src/theme/paint.frag`, `src/theme/gl.rs` | the shader and its renderer (shared EGL setup with `zoom::gl`) |
| `src/render.rs`, `src/region.rs` | themed dim |
| `src/backdrop.rs` | `valw backdrop` |
| `src/picker/{mod,layout,icons}.rs` | the picker, card layout, icon lookup |
| `src/main.rs` | `Command::Backdrop`, `Command::Window { pick }` |
| `src/niri.rs` | `windows()` (all windows with focus timestamps) |
| `README.md` | backdrop wiring |

New dependency: `resvg` (SVG icons; built on tiny-skia).

## 8. Testing

Unit tests (`cargo nextest`):
- palettes: Noctalia palette file parsing (light and dark, missing keys →
  default), image palette (dark → light order, transparent and grey
  pixels ignored for icons), default.
- themed dim: a pixel is blended towards `base` by the right amount.
- picker layout: card positions for 1, 5 and 20 windows on 1366×768 and
  1920×1080 (rows wrap, centred, selected card scaled), hit-testing,
  keyboard navigation wrapping, MRU order from focus timestamps.
- picker motion alternation from the state file (missing/broken file →
  swirl).
- icon lookup against a fake data dir (desktop file by name and by
  `StartupWMClass`, theme inheritance, hicolor fallback, SVG and PNG).
- backdrop: the "resume after a pause" rule and colour easing.

Manual checks go to the theme section of `docs/test-checklist.md`
(including how it feels on the HD 4000).

## 9. Definition of done

`cargo clippy -- -D warnings`, `cargo fmt --check`, `cargo nextest run`
and `nix flake check` pass; README and checklist updated; copland wiring
done with the user's approval.
