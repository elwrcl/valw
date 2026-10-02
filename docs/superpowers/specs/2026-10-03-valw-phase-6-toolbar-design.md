# valw Phase 6: the toolbar (design)

Date: 2026-10-03
Status: approved in brainstorming, pending written-spec review
Builds on: Phases 0–5 (on `main`); vision section 4.6

## 1. Goal

A Cmd+Shift+5-style bar: `valw toolbar` shows a floating bar at the
bottom of the focused output, the user picks a mode (and, if they like, a
timer, the cursor and the preview), and that mode runs as if started from
the command line.

Success: every mode starts from the bar with one click, the options are
remembered, and nothing of the bar or its countdown ends up in a shot.

### Out of scope

- A remembered region drawn on screen and a separate Capture button (the
  macOS "select, then capture" flow).
- Save-location choices; screen recording.

## 2. Decisions

| Question | Decision | Why |
|---|---|---|
| Flow | Clicking a mode starts it at once | Simple and fast |
| Options | Timer (None / 5 s / 10 s), Show cursor, Show preview | User choice |
| Rendering | A layer-shell surface drawn with tiny-skia and the editor's bundled font | Floats without niri rules; same stack as the overlay and preview; no new dependency |
| Memory | Last mode and options in `$XDG_STATE_HOME/valw/toolbar.toml` | The config may become a read-only home-manager file |

## 3. Behaviour

### 3.1 The bar

- A layer surface (namespace `valw-toolbar`, overlay layer, exclusive
  keyboard) covering niri's focused output. It is transparent except for
  the bar, which sits 24 logical px above the bottom edge, horizontally
  centred; covering the output is what lets a click beside the bar close
  it.
- Contents, left to right: **Screen**, **Window**, **Region**, a
  separator, **Zoom**, a separator, **Options ▾**. Labels in English,
  13 px, white; background `#1c1c1e` at 90 % opacity with 12 px corner
  radius; buttons 32 px tall with 12 px horizontal padding.
- The last used mode is highlighted (a lighter fill); the button under
  the pointer is highlighted more lightly.
- Drawn at the output's scale (buffer at physical size, viewporter to
  logical size), as the overlay does.

### 3.2 Options

**Options ▾** toggles a small menu above the bar, aligned to the button:

- Timer: None / 5 s / 10 s (one of three)
- Show cursor (check)
- Show preview (check)

Clicks in the menu change the option and keep the menu open. The options
apply only to captures started from the bar; the config is untouched.

### 3.3 Starting a mode

- A click on Screen, Window, Region or Zoom closes the bar (its surface is
  destroyed and the destruction flushed) and starts the mode with the
  options: Show cursor → `--cursor`; Show preview off → `--no-preview`.
- With a timer, a countdown pill (namespace `valw-countdown`, the same
  style, about 64 × 40 px, bottom-centred where the bar was) shows the
  seconds left, 5 → 1 (or 10 → 1), redrawn every second, and is destroyed
  before the mode starts so it is never captured. Esc during the
  countdown cancels (exit 3); the pill has exclusive keyboard focus.
- The chosen mode and options are saved before the mode starts.

### 3.4 Leaving

Esc, a click outside the bar and its menu, or the compositor closing the
surface: nothing is captured, exit code 3 (`Cancelled`), like Esc in
region mode.

### 3.5 Memory

`toolbar.toml` holds `mode` (`screen` | `window` | `region` | `zoom`),
`timer` (0, 5, 10), `cursor` and `preview` (booleans). Without a file the
defaults are: mode `region`, timer 0, `cursor = capture.show_cursor`,
`preview = preview.enabled` from the config. An unreadable or invalid file
logs a warning and uses the defaults; it is overwritten on the next save.

## 4. Structure

| File | Role |
|---|---|
| `src/toolbar/layout.rs` | Pure: button and menu rectangles from the measured label widths; `hit(point) -> Option<Target>` for the bar and the menu |
| `src/toolbar/state.rs` | `ToolbarState` load/save, defaults from the config |
| `src/toolbar/countdown.rs` | The countdown pill and its timing |
| `src/toolbar/mod.rs` | `run(wl, outputs, config) -> Result<Choice>`: the surface, drawing (tiny-skia + `editor::text`), input |
| `src/main.rs` | `Command::Toolbar`; the choice becomes `capture(mode, common)` |
| `src/wayland.rs` | Routing configure/pointer/keyboard/focus events to the toolbar |

## 5. Testing

Unit tests (`cargo nextest`):
- `layout`: buttons in order, non-overlapping, centred; hits on each
  button, the separators (none), outside (none); menu rows' hits.
- `state`: defaults from the config, save/load round trip, a broken file
  falls back to the defaults, unknown keys are ignored.
- `countdown`: the sequence of numbers for 5 and 10 s, and that 0 means
  no countdown.
- `main`: the CLI parses `toolbar`; a choice maps to the right mode and
  flags.

Manual checks go to the Phase 6 section of `docs/test-checklist.md`.

## 6. Definition of done

`cargo clippy -- -D warnings`, `cargo fmt --check`, `cargo nextest run`
and `nix flake check` pass; the checklist section is written.
