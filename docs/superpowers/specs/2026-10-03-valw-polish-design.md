# valw polish phase (design)

Date: 2026-10-03
Status: approved in brainstorming, pending written-spec review
Builds on: Phases 0–6 (on `main`)

## 1. Goal

Three finishing touches, in this order:

- **P1** Region selection: a size label next to the pointer and macOS's
  Shift / Alt / Space modifiers while dragging.
- **P2** An optional soft shadow around window shots.
- **P3** The combo shutter sound: escalating with quick successive shots,
  ending in an endlessly rising loop.

### Out of scope

The deferred minors in `docs/test-checklist.md` (they wait for the end
test pass). Nothing binds to Caps Lock (the user maps it to Ctrl).

## 2. P1: Region selection

### 2.1 Size label

While a selection is being dragged, a label shows its size in physical
pixels, `W × H` (the same numbers the saved PNG will have), in white
13 px text on a rounded `#1c1c1e` pill at 90 % opacity, 6 px padding.
It sits 16 logical px right of and below the pointer; if it would leave
the output it moves to the other side of the pointer (left and/or above).
It is drawn only on the output where the drag happens, and disappears
with the drag.

### 2.2 Modifiers while dragging

The selection is computed from the press point `a`, the pointer `p` and
the modifiers, as a pure function in `selection.rs`:

| Held | Effect |
|---|---|
| none | rectangle from `a` to `p` |
| Shift | only one dimension follows the pointer: the one that changes more in the first motion after Shift goes down; the other stays as it was when Shift was pressed. Releasing Shift returns to the plain rectangle |
| Alt (Option) | `a` is the centre: the rectangle spans `a ± (p − a)` |
| Space (held) | the whole rectangle moves with the pointer, keeping its size; on release the far corner follows the pointer again from where it is |

Shift and Alt combine (one dimension, from the centre). Space before a
drag still switches to window mode (Phase 3); Space during a drag now
moves the selection. Releasing the mouse button finishes the selection
with whatever modifiers are held.

The overlay learns the modifier state from the keyboard (SCTK
`update_modifiers`) and Space press/release events.

## 3. P2: Window shadow

`[capture] window_shadow = false` (default off). When on, a window shot
(Phase 3, from `valw window`, Space in region mode, or the toolbar) gets
a shadow before it is saved:

- The image is padded by 40 px on every side (transparent).
- The shadow is the window's own alpha (so it follows rounded corners),
  black at 50 % opacity, blurred with a radius of 20 px (three box blur
  passes approximating a Gaussian), offset 12 px down.
- The window is drawn over it unchanged.

The preview, clipboard and file all get the shadowed image (it has
alpha; Phase 3's preview already handles that).

## 4. P3: The combo sound

### 4.1 The sounds

All sounds are synthesised in code (no samples): 16-bit mono PCM at
44 100 Hz, written as WAV.

- **The shutter** (~150 ms): a short filtered noise click (5 ms attack,
  fast decay) followed after 60 ms by a second, softer and longer
  "shk" (noise through a gentle band-pass, 80 ms decay).
- **Shots 1–5**: the shutter played faster and higher: speed
  1.00, 1.12, 1.25, 1.40, 1.60 (resampled; shorter and brighter).
- **Shot 6**: the shutter at speed 1.80 followed by a Shepard-tone
  glide rising half an octave over 400 ms.
- **Shot 7**: the same shutter, then the glide continuing over the next
  half octave, so that its end equals the start of shot 6's glide.
- From then on shots alternate 6, 7, 6, 7…, which sounds like an endless
  rise. The Shepard tone is a sum of octave-spaced partials under a
  fixed bell-shaped loudness curve over log-frequency, so a whole-octave
  shift sounds identical.

### 4.2 The combo

- A shot's number is 1 + the previous one's, if the previous shot was
  less than `combo_reset_secs` ago (default 5); otherwise 1. After 7 it
  goes back to 6.
- The state (last number, last time) is a small file in
  `$XDG_RUNTIME_DIR` (`valw-combo`), so separate `valw` runs share it.
  A missing or unreadable file means number 1.

### 4.3 Playing

- The sound plays right after the screen was captured (before saving),
  for every mode that produces a shot (screen, `--all` once, region,
  window, zoom's `c`). Cancelled captures play nothing.
- The seven WAVs are written once to `$XDG_RUNTIME_DIR/valw-sound/`
  (regenerated if missing) and played by spawning `pw-play` (PipeWire)
  detached, with the volume from the config; valw doesn't wait for it.
  `pw-play` comes with the Nix package's `PATH`. If it can't be started,
  a warning is logged once per run and the capture goes on.

### 4.4 Settings

```toml
[sound]
enabled = true
volume = 0.6          # 0.0–1.0
combo_reset_secs = 5  # 1–60
```

The toolbar's Options menu gains **Play sound** (a check, remembered in
`toolbar.toml`, default from `sound.enabled`), which wins over the config
for captures started from the bar, like Show cursor.

## 5. Structure

| File | Change |
|---|---|
| `src/selection.rs` | modifier-aware selection: `Mods`, the Shift axis lock, centre and move |
| `src/region.rs` | modifiers, Space during a drag, the size label |
| `src/render.rs` | blending a small premultiplied label into the overlay buffer |
| `src/wayland.rs` | `update_modifiers` and Space release to the overlay |
| `src/shadow.rs` (new) | `add_shadow(&RgbaImage) -> RgbaImage` |
| `src/window.rs` / `src/main.rs` | apply the shadow when configured; play the sound after capture |
| `src/sound/` (new) | `synth.rs` (shutter, speed-up, Shepard glide, WAV), `combo.rs` (numbering and state file), `mod.rs` (cache + `pw-play`) |
| `src/config.rs` | `capture.window_shadow`, `[sound]` |
| `src/toolbar/*` | the Play sound option |
| `nix/package.ulu.nix` | `pipewire` on the wrapper's `PATH` |

## 6. Testing

Unit tests (`cargo nextest`):
- `selection`: plain, Shift locking each axis, Alt from the centre, Shift
  + Alt, Space moving and then resizing again, clipping to the output.
- the size label's placement near each edge and its text.
- `shadow`: output size, the shadow below the window (alpha > 0 under,
  ~0 far away), the window's own pixels unchanged, rounded corners
  followed.
- `synth`: lengths per speed, no clipping, the WAV header; the Shepard
  glide's end matching its start an octave apart (6's start = 7's end).
- `combo`: numbering 1…7 then 6, 7, 6; reset after the gap; a broken
  state file.
- config defaults and bounds; the toolbar's sound option.

Manual checks go to the polish section of `docs/test-checklist.md`.

## 7. Definition of done

`cargo clippy -- -D warnings`, `cargo fmt --check`, `cargo nextest run`
and `nix flake check` pass; the checklist section is written.
