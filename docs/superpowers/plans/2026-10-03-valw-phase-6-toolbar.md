# valw Phase 6: the toolbar Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `valw toolbar`: a floating bar (Screen, Window, Region, Zoom, Options ▾) that starts the chosen mode at once, with a remembered timer/cursor/preview.

**Architecture:** Pure pieces first — `toolbar/state.rs` (remembered choices), `toolbar/layout.rs` (rectangles and hit-testing from measured label widths), `toolbar/draw.rs` (tiny-skia painting of the bar, menu and countdown pill into premultiplied pixels). Then `toolbar/mod.rs` puts the bar on a transparent full-output layer surface and the countdown on a small one, and `main.rs` turns the choice into the existing `capture(mode, common)`.

**Tech Stack:** Rust 2024, SCTK 0.21 layer-shell + shm + viewporter, tiny-skia 0.12, the editor's `text` module (DejaVu Sans Bold).

**Spec:** `docs/superpowers/specs/2026-10-03-valw-phase-6-toolbar-design.md`

## Global Constraints

- jj only; `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo nextest run`, `nix flake check`.
- Namespaces `valw-toolbar` and `valw-countdown`. Bar: 24 logical px above the bottom, centred; label 13 px white; background `#1c1c1e` alpha 230 (90 %); corner radius 12; buttons 32 px tall, 12 px side padding; bar padding 6 px. Menu rows 28 px. Selected mode fill white alpha 46; hover fill white alpha 26.
- State file `$XDG_STATE_HOME/valw/toolbar.toml` (fallback `~/.local/state`); defaults mode `region`, timer 0, cursor = `capture.show_cursor`, preview = `preview.enabled`.
- Esc / a click outside the bar and menu / the compositor closing the surface → `Cancelled` (exit 3).
- Tell the user before showing the toolbar on the live session.

## Review Focus

1. **The countdown or the bar ends up in the shot.** Both surfaces are destroyed and the connection flushed and round-tripped before `capture` starts (Task 3); real check in the checklist.
2. **A broken or hand-edited `toolbar.toml`** must not stop the toolbar. Test: `toolbar::state::tests::a_broken_file_falls_back` (Task 1).
3. **Hit-testing at the bar's edges and between buttons** (separators, padding) must not start a mode by accident. Test: `toolbar::layout::tests::gaps_hit_nothing` (Task 1).
4. **Fractional scale** (1.25): the bar must be crisp and hits must match what is drawn. Layout works in logical px, drawing scales by the output's scale (Task 2 test `draw::tests::scales`).
5. **The timer is meant for setting up the screen**: during the countdown the rest of the desktop must stay usable. Countdown surface takes no keyboard (Ruling below; Task 3).

**Ruling (plan):** the spec gives the countdown pill exclusive keyboard focus so Esc can cancel. That would block typing during the countdown, which is what a timer is for. The pill takes no keyboard focus; a click on the pill cancels instead (its label shows the seconds; a hover shows "Cancel"). Cost if wrong: Esc doesn't cancel a timer.

---

## File Structure

| File | Change | Task |
|---|---|---|
| `src/toolbar/state.rs` | `Mode`, `ToolbarState`, `defaults`, `load`, `save`, `default_path` | 1 |
| `src/toolbar/layout.rs` | `Target`, `MenuItem`, `Rect`, `Layout`, `layout`, `Layout::hit` | 1 |
| `src/toolbar/draw.rs` | `bar`, `pill`, `to_argb` | 2 |
| `src/toolbar/mod.rs` | `run`, `Toolbar`, `Pill` surfaces | 1 (module list), 3 |
| `src/wayland.rs` | `State::{toolbar, pill}` routing | 3 |
| `src/main.rs` | `Command::Toolbar` | 3 |
| `docs/test-checklist.md` | Phase 6 section | 4 |

---

### Task 1: State and layout

**Files:** create `src/toolbar/mod.rs` (`pub mod layout; pub mod state;`), `src/toolbar/state.rs`, `src/toolbar/layout.rs`; `src/main.rs`: `mod toolbar;` plus `// The toolbar lands in pieces; Task 3 removes this.` `#![allow(dead_code)]` at the top.

**Produces:**
- `state::Mode::{Screen, Window, Region, Zoom}` (serde lowercase), `state::ToolbarState { mode, timer: u32, cursor: bool, preview: bool }`, `state::defaults(&Config) -> ToolbarState`, `state::load(&Path, &Config) -> ToolbarState`, `state::save(&Path, &ToolbarState) -> Result<()>`, `state::default_path() -> PathBuf`.
- `layout::{Rect { x, y, w, h }, Target::{Mode(Mode), Options, Menu(MenuItem)}, MenuItem::{Timer(u32), Cursor, Preview}, Layout { bar, buttons: Vec<(Target, Rect, &'static str)>, separators: Vec<Rect>, menu, rows: Vec<(MenuItem, Rect, &'static str)> }, layout(output: (f32, f32), measure: impl Fn(&str) -> f32) -> Layout, Layout::hit(&self, p: (f32, f32), menu_open: bool) -> Option<Target>, Layout::inside(&self, p, menu_open) -> bool}` — all in logical px.

- [ ] **Step 1: Failing tests**

`state.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_come_from_the_config() {
        let mut config = Config::default();
        config.capture.show_cursor = true;
        config.preview.enabled = false;
        assert_eq!(
            defaults(&config),
            ToolbarState { mode: Mode::Region, timer: 0, cursor: true, preview: false }
        );
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deep/toolbar.toml");
        let state = ToolbarState { mode: Mode::Zoom, timer: 10, cursor: true, preview: true };
        save(&path, &state).unwrap();
        assert_eq!(load(&path, &Config::default()), state);
    }

    #[test]
    fn a_missing_file_gives_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::default();
        assert_eq!(load(&dir.path().join("none.toml"), &config), defaults(&config));
    }

    #[test]
    fn a_broken_file_falls_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("toolbar.toml");
        let config = Config::default();
        for text in ["mode = \"video\"\n", "timer = 7\n", "not toml at all [", "cursor = \"yes\"\n"] {
            std::fs::write(&path, text).unwrap();
            assert_eq!(load(&path, &config), defaults(&config), "{text:?}");
        }
    }

    #[test]
    fn missing_and_unknown_keys_are_fine() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("toolbar.toml");
        std::fs::write(&path, "mode = \"window\"\ncolour = \"red\"\n").unwrap();
        let config = Config::default();
        let state = load(&path, &config);
        assert_eq!(state.mode, Mode::Window);
        assert_eq!(state.timer, 0);
        assert_eq!(state.preview, config.preview.enabled);
    }
}
```

`layout.rs` (labels measured as 7 px per character so numbers are easy):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn measure(s: &str) -> f32 {
        7.0 * s.chars().count() as f32
    }

    fn l() -> Layout {
        layout((1366.0, 768.0), measure)
    }

    fn centre(r: Rect) -> (f32, f32) {
        (r.x + r.w / 2.0, r.y + r.h / 2.0)
    }

    #[test]
    fn the_bar_is_centred_above_the_bottom() {
        let l = l();
        assert!(((l.bar.x + l.bar.w / 2.0) - 683.0).abs() < 0.01);
        assert_eq!(l.bar.y + l.bar.h, 768.0 - 24.0);
        assert_eq!(l.bar.h, 32.0 + 2.0 * 6.0);
    }

    #[test]
    fn buttons_are_in_order_and_do_not_overlap() {
        let l = l();
        let labels: Vec<_> = l.buttons.iter().map(|b| b.2).collect();
        assert_eq!(labels, ["Screen", "Window", "Region", "Zoom", "Options ▾"]);
        for pair in l.buttons.windows(2) {
            assert!(pair[0].1.x + pair[0].1.w <= pair[1].1.x);
        }
        // "Screen": 6 chars × 7 + 2 × 12 padding.
        assert_eq!(l.buttons[0].1.w, 42.0 + 24.0);
        assert_eq!(l.separators.len(), 2);
    }

    #[test]
    fn each_button_hits_its_target() {
        let l = l();
        for (target, rect, _) in &l.buttons {
            assert_eq!(l.hit(centre(*rect), false), Some(*target));
        }
    }

    #[test]
    fn gaps_hit_nothing() {
        let l = l();
        for sep in &l.separators {
            assert_eq!(l.hit(centre(*sep), false), None);
        }
        assert_eq!(l.hit((l.bar.x + 2.0, l.bar.y + 2.0), false), None, "padding");
        assert_eq!(l.hit((10.0, 10.0), false), None, "outside");
        assert!(l.inside((l.bar.x + 2.0, l.bar.y + 2.0), false));
        assert!(!l.inside((10.0, 10.0), false));
    }

    #[test]
    fn the_menu_sits_above_options_and_only_counts_when_open() {
        let l = l();
        let options = l.buttons[4].1;
        assert_eq!(l.menu.y + l.menu.h, l.bar.y - 8.0);
        assert!(l.menu.x <= options.x + 0.01 || l.menu.x + l.menu.w <= 1366.0);
        let rows: Vec<_> = l.rows.iter().map(|r| r.0).collect();
        assert_eq!(rows, [MenuItem::Timer(0), MenuItem::Timer(5), MenuItem::Timer(10), MenuItem::Cursor, MenuItem::Preview]);
        let cursor = centre(l.rows[3].1);
        assert_eq!(l.hit(cursor, true), Some(Target::Menu(MenuItem::Cursor)));
        assert_eq!(l.hit(cursor, false), None);
        assert!(l.inside(cursor, true) && !l.inside(cursor, false));
    }

    #[test]
    fn the_menu_stays_on_a_narrow_output() {
        let l = layout((400.0, 300.0), measure);
        assert!(l.menu.x >= 0.0 && l.menu.x + l.menu.w <= 400.0);
    }
}
```

Run `cargo nextest run toolbar::` → compile errors.

- [ ] **Step 2: Implement `state.rs`**

```rust
//! What the toolbar remembers between runs, kept out of the config so a
//! read-only (home-manager) config still works.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Screen,
    Window,
    Region,
    Zoom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ToolbarState {
    pub mode: Mode,
    /// Seconds before the capture: 0, 5 or 10.
    pub timer: u32,
    pub cursor: bool,
    pub preview: bool,
}

/// The file, every key optional.
#[derive(Deserialize)]
struct File {
    mode: Option<Mode>,
    timer: Option<u32>,
    cursor: Option<bool>,
    preview: Option<bool>,
}

pub const TIMERS: [u32; 3] = [0, 5, 10];

/// `$XDG_STATE_HOME/valw/toolbar.toml`, falling back to `~/.local/state`.
pub fn default_path() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::config::home().join(".local/state"))
        .join("valw/toolbar.toml")
}

pub fn defaults(config: &Config) -> ToolbarState {
    ToolbarState {
        mode: Mode::Region,
        timer: 0,
        cursor: config.capture.show_cursor,
        preview: config.preview.enabled,
    }
}

/// The remembered state, or the defaults if there is none or it is broken.
pub fn load(path: &Path, config: &Config) -> ToolbarState {
    let fallback = defaults(config);
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return fallback,
        Err(e) => {
            tracing::warn!("could not read {}: {e}", path.display());
            return fallback;
        }
    };
    match parse(&text, fallback) {
        Ok(state) => state,
        Err(e) => {
            tracing::warn!("ignoring {}: {e:#}", path.display());
            fallback
        }
    }
}

fn parse(text: &str, fallback: ToolbarState) -> Result<ToolbarState> {
    let file: File = toml::from_str(text)?;
    let timer = file.timer.unwrap_or(fallback.timer);
    if !TIMERS.contains(&timer) {
        bail!("timer must be 0, 5 or 10, not {timer}");
    }
    Ok(ToolbarState {
        mode: file.mode.unwrap_or(fallback.mode),
        timer,
        cursor: file.cursor.unwrap_or(fallback.cursor),
        preview: file.preview.unwrap_or(fallback.preview),
    })
}

pub fn save(path: &Path, state: &ToolbarState) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("could not create {}", dir.display()))?;
    }
    let text = toml::to_string(state).context("could not encode the toolbar state")?;
    crate::output::write_atomic(path, text.as_bytes())
}
```

(Unknown keys are ignored because `File` doesn't deny them.)

- [ ] **Step 3: Implement `layout.rs`**

```rust
//! Where the toolbar's buttons and menu are, in logical px of the output,
//! and what a point hits.

use crate::toolbar::state::Mode;

pub const MARGIN_BOTTOM: f32 = 24.0;
pub const PADDING: f32 = 6.0;
pub const BUTTON_H: f32 = 32.0;
pub const BUTTON_PAD: f32 = 12.0;
pub const SEPARATOR_GAP: f32 = 8.0;
pub const MENU_GAP: f32 = 8.0;
pub const ROW_H: f32 = 28.0;
/// Room for the check or radio mark left of a menu label.
pub const MARK_W: f32 = 24.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, p: (f32, f32)) -> bool {
        p.0 >= self.x && p.0 < self.x + self.w && p.1 >= self.y && p.1 < self.y + self.h
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItem {
    Timer(u32),
    Cursor,
    Preview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Mode(Mode),
    Options,
    Menu(MenuItem),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub bar: Rect,
    pub buttons: Vec<(Target, Rect, &'static str)>,
    pub separators: Vec<Rect>,
    pub menu: Rect,
    pub rows: Vec<(MenuItem, Rect, &'static str)>,
}

/// `None` stands for a separator.
const BAR: [Option<(Target, &str)>; 7] = [
    Some((Target::Mode(Mode::Screen), "Screen")),
    Some((Target::Mode(Mode::Window), "Window")),
    Some((Target::Mode(Mode::Region), "Region")),
    None,
    Some((Target::Mode(Mode::Zoom), "Zoom")),
    None,
    Some((Target::Options, "Options ▾")),
];

const MENU: [(MenuItem, &str); 5] = [
    (MenuItem::Timer(0), "No timer"),
    (MenuItem::Timer(5), "5 second timer"),
    (MenuItem::Timer(10), "10 second timer"),
    (MenuItem::Cursor, "Show cursor"),
    (MenuItem::Preview, "Show preview"),
];

/// Lays the toolbar out on an output of `output` logical px; `measure`
/// gives a label's width in logical px.
pub fn layout(output: (f32, f32), measure: impl Fn(&str) -> f32) -> Layout {
    let widths: Vec<f32> = BAR
        .iter()
        .map(|item| match item {
            Some((_, label)) => measure(label) + 2.0 * BUTTON_PAD,
            None => 2.0 * SEPARATOR_GAP + 1.0,
        })
        .collect();
    let w = widths.iter().sum::<f32>() + 2.0 * PADDING;
    let h = BUTTON_H + 2.0 * PADDING;
    let bar = Rect { x: (output.0 - w) / 2.0, y: output.1 - MARGIN_BOTTOM - h, w, h };

    let mut buttons = Vec::new();
    let mut separators = Vec::new();
    let mut x = bar.x + PADDING;
    for (item, width) in BAR.iter().zip(widths) {
        match item {
            Some((target, label)) => {
                buttons.push((*target, Rect { x, y: bar.y + PADDING, w: width, h: BUTTON_H }, *label));
            }
            None => separators.push(Rect { x: x + SEPARATOR_GAP, y: bar.y + PADDING + 6.0, w: 1.0, h: BUTTON_H - 12.0 }),
        }
        x += width;
    }

    let menu_w = MENU.iter().map(|(_, l)| measure(l)).fold(0.0, f32::max) + MARK_W + 2.0 * BUTTON_PAD;
    let menu_h = MENU.len() as f32 * ROW_H + 2.0 * PADDING;
    let options = buttons.last().expect("the bar has buttons").1;
    let menu_x = options.x.min(output.0 - menu_w).max(0.0);
    let menu = Rect { x: menu_x, y: bar.y - MENU_GAP - menu_h, w: menu_w, h: menu_h };
    let rows = MENU
        .iter()
        .enumerate()
        .map(|(i, (item, label))| {
            (*item, Rect { x: menu.x, y: menu.y + PADDING + i as f32 * ROW_H, w: menu.w, h: ROW_H }, *label)
        })
        .collect();
    Layout { bar, buttons, separators, menu, rows }
}

impl Layout {
    pub fn hit(&self, p: (f32, f32), menu_open: bool) -> Option<Target> {
        if menu_open
            && let Some(row) = self.rows.iter().find(|r| r.1.contains(p))
        {
            return Some(Target::Menu(row.0));
        }
        self.buttons.iter().find(|b| b.1.contains(p)).map(|b| b.0)
    }

    /// Whether `p` is on the bar or the open menu (a click elsewhere closes).
    pub fn inside(&self, p: (f32, f32), menu_open: bool) -> bool {
        self.bar.contains(p) || (menu_open && self.menu.contains(p))
    }
}
```

- [ ] **Step 4: Run** (+11), clippy, fmt. **Step 5: Commit** `feat: toolbar state and layout`.

---

### Task 2: Drawing

**Files:** `src/toolbar/draw.rs`; `src/toolbar/mod.rs`: `pub mod draw;`.

**Produces:** `draw::LABEL: f32 = 13.0`; `draw::measure(label) -> f32` (logical px, via `editor::text::layout`); `draw::bar(layout, size_px: (u32, u32), scale: f32, state: &ToolbarState, hover: Option<Target>, menu_open: bool) -> Pixmap` (whole output, transparent outside); `draw::pill(size_px, scale, text: &str) -> Pixmap`; `draw::to_argb(&Pixmap, &mut [u8])` (premultiplied RGBA → wl_shm ARGB8888 bytes B, G, R, A).

- [ ] **Step 1: Failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolbar::layout::layout;
    use crate::toolbar::state::Mode;

    const STATE: ToolbarState = ToolbarState { mode: Mode::Region, timer: 0, cursor: false, preview: true };

    fn alpha(p: &Pixmap, x: f32, y: f32, scale: f32) -> u8 {
        p.pixel((x * scale) as u32, (y * scale) as u32).unwrap().alpha()
    }

    #[test]
    fn the_bar_is_opaque_and_the_rest_transparent() {
        let l = layout((800.0, 600.0), measure);
        let p = bar(&l, (800, 600), 1.0, &STATE, None, false);
        assert_eq!(alpha(&p, 5.0, 5.0, 1.0), 0);
        assert!(alpha(&p, l.bar.x + l.bar.w / 2.0, l.bar.y + 3.0, 1.0) >= 230);
        assert_eq!(alpha(&p, l.menu.x + 5.0, l.menu.y + 5.0, 1.0), 0, "menu closed");
        let open = bar(&l, (800, 600), 1.0, &STATE, None, true);
        assert!(alpha(&open, l.menu.x + l.menu.w / 2.0, l.menu.y + 3.0, 1.0) >= 230);
    }

    #[test]
    fn the_selected_mode_is_lighter() {
        let l = layout((800.0, 600.0), measure);
        let p = bar(&l, (800, 600), 1.0, &STATE, None, false);
        let lum = |r: crate::toolbar::layout::Rect| p.pixel((r.x + 3.0) as u32, (r.y + 3.0) as u32).unwrap().demultiply().red();
        let region = l.buttons[2].1;
        let screen = l.buttons[0].1;
        assert!(lum(region) > lum(screen));
    }

    #[test]
    fn scales() {
        let l = layout((800.0, 600.0), measure);
        let p = bar(&l, (1000, 750), 1.25, &STATE, None, false);
        assert_eq!((p.width(), p.height()), (1000, 750));
        assert!(alpha(&p, l.bar.x + l.bar.w / 2.0, l.bar.y + 3.0, 1.25) >= 230);
        assert_eq!(alpha(&p, l.bar.x - 3.0, l.bar.y + 3.0, 1.25), 0);
    }

    #[test]
    fn argb_swaps_red_and_blue() {
        let mut p = Pixmap::new(1, 1).unwrap();
        p.fill(tiny_skia::Color::from_rgba8(10, 20, 30, 255));
        let mut out = [0u8; 4];
        to_argb(&p, &mut out);
        assert_eq!(out, [30, 20, 10, 255]);
    }

    #[test]
    fn the_pill_shows_its_text() {
        let p = pill((72, 40), 1.0, "5");
        let lit = p.pixels().iter().filter(|c| c.demultiply().red() > 200 && c.alpha() > 200).count();
        assert!(lit > 10, "white digit pixels: {lit}");
    }
}
```

Run → compile errors.

- [ ] **Step 2: Implement**

```rust
//! Painting the toolbar, its menu and the countdown pill with tiny-skia.

use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Rect as SkRect, Transform};

use crate::editor::text;
use crate::toolbar::layout::{Layout, MenuItem, Rect, Target};
use crate::toolbar::state::ToolbarState;

pub const LABEL: f32 = 13.0;
const RADIUS: f32 = 12.0;
const BACKGROUND: [u8; 4] = [0x1c, 0x1c, 0x1e, 230];
const SELECTED: [u8; 4] = [255, 255, 255, 46];
const HOVER: [u8; 4] = [255, 255, 255, 26];
const SEPARATOR: [u8; 4] = [255, 255, 255, 60];
const WHITE: [u8; 4] = [255, 255, 255, 255];

/// A label's width in logical px.
pub fn measure(label: &str) -> f32 {
    text::layout(label, LABEL).width
}

fn paint(c: [u8; 4]) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], c[3]);
    p.anti_alias = true;
    p
}

fn rounded(pixmap: &mut Pixmap, r: Rect, radius: f32, color: [u8; 4], scale: f32) {
    let (x, y, w, h, k) = (r.x * scale, r.y * scale, r.w * scale, r.h * scale, radius * scale);
    let mut pb = PathBuilder::new();
    pb.move_to(x + k, y);
    pb.line_to(x + w - k, y);
    pb.quad_to(x + w, y, x + w, y + k);
    pb.line_to(x + w, y + h - k);
    pb.quad_to(x + w, y + h, x + w - k, y + h);
    pb.line_to(x + k, y + h);
    pb.quad_to(x, y + h, x, y + h - k);
    pb.line_to(x, y + k);
    pb.quad_to(x, y, x + k, y);
    pb.close();
    if let Some(path) = pb.finish() {
        pixmap.fill_path(&path, &paint(color), FillRule::Winding, Transform::identity(), None);
    }
}

/// `label` centred vertically in `r`, starting `left` logical px in.
fn label(pixmap: &mut Pixmap, r: Rect, left: f32, s: &str, scale: f32) {
    let size = LABEL * scale;
    let height = text::line_height(size);
    let origin = ((r.x + left) * scale, (r.y + r.h / 2.0) * scale - height / 2.0);
    if let Some(path) = text::path(s, size, origin) {
        pixmap.fill_path(&path, &paint(WHITE), FillRule::Winding, Transform::identity(), None);
    }
}

/// The whole output: transparent but for the bar and, if open, the menu.
pub fn bar(
    layout: &Layout,
    size: (u32, u32),
    scale: f32,
    state: &ToolbarState,
    hover: Option<Target>,
    menu_open: bool,
) -> Pixmap {
    let mut p = Pixmap::new(size.0.max(1), size.1.max(1)).expect("the output has a size");
    rounded(&mut p, layout.bar, RADIUS, BACKGROUND, scale);
    for (target, r, text) in &layout.buttons {
        let fill = if *target == Target::Mode(state.mode) || (*target == Target::Options && menu_open) {
            Some(SELECTED)
        } else if hover == Some(*target) {
            Some(HOVER)
        } else {
            None
        };
        if let Some(fill) = fill {
            rounded(&mut p, *r, 8.0, fill, scale);
        }
        label(&mut p, *r, crate::toolbar::layout::BUTTON_PAD, text, scale);
    }
    for s in &layout.separators {
        rounded(&mut p, *s, 0.0, SEPARATOR, scale);
    }
    if menu_open {
        rounded(&mut p, layout.menu, RADIUS, BACKGROUND, scale);
        for (item, r, text) in &layout.rows {
            if hover == Some(Target::Menu(*item)) {
                rounded(&mut p, *r, 6.0, HOVER, scale);
            }
            let on = match item {
                MenuItem::Timer(t) => state.timer == *t,
                MenuItem::Cursor => state.cursor,
                MenuItem::Preview => state.preview,
            };
            if on {
                label(&mut p, *r, crate::toolbar::layout::BUTTON_PAD, "✓", scale);
            }
            label(&mut p, *r, crate::toolbar::layout::BUTTON_PAD + crate::toolbar::layout::MARK_W, text, scale);
        }
    }
    p
}

/// The countdown pill: a rounded background with `text` centred.
pub fn pill(size: (u32, u32), scale: f32, s: &str) -> Pixmap {
    let mut p = Pixmap::new(size.0.max(1), size.1.max(1)).expect("the pill has a size");
    let r = Rect { x: 0.0, y: 0.0, w: size.0 as f32 / scale, h: size.1 as f32 / scale };
    rounded(&mut p, r, RADIUS, BACKGROUND, scale);
    let left = (r.w - measure(s)) / 2.0;
    label(&mut p, r, left, s, scale);
    p
}

/// Copies premultiplied RGBA into wl_shm ARGB8888 (bytes B, G, R, A).
pub fn to_argb(pixmap: &Pixmap, out: &mut [u8]) {
    for (dst, src) in out.chunks_exact_mut(4).zip(pixmap.pixels()) {
        dst.copy_from_slice(&[src.blue(), src.green(), src.red(), src.alpha()]);
    }
}
```

(`SkRect` and `Color` imports only if used; drop the unused ones. If DejaVu Sans Bold has no "✓" glyph — check with `text::FONT.glyph_id('✓')` — draw the check as a short stroked polyline instead and ledger it.)

- [ ] **Step 3: Run** (+5), clippy, fmt. **Step 4: Commit** `feat: toolbar drawing`.

---

### Task 3: Surfaces, events and `valw toolbar`

**Files:** `src/toolbar/mod.rs`, `src/wayland.rs`, `src/main.rs`.

**Consumes:** Tasks 1–2. **Produces:** `toolbar::run(config: &Config) -> Result<Picked>` where `Picked { mode: state::Mode, cursor: bool, preview: bool }` (the countdown has already run); `State::{toolbar: Option<Toolbar>, pill: Option<Pill>}`; `Command::Toolbar`.

- [ ] **Step 1: Failing tests** — `src/main.rs` `flag_conflicts`: `assert!(parses(&["toolbar"]));` and a new test

```rust
    #[test]
    fn a_toolbar_pick_becomes_a_capture() {
        use toolbar::state::Mode as T;
        let pick = |mode, cursor, preview| toolbar::Picked { mode, cursor, preview };
        let (mode, common) = from_toolbar(pick(T::Screen, true, false));
        assert!(matches!(mode, Mode::Screen { all: false }));
        assert!(common.cursor && common.no_preview);
        assert!(matches!(from_toolbar(pick(T::Window, false, true)).0, Mode::Window));
        assert!(matches!(from_toolbar(pick(T::Region, false, true)).0, Mode::Region));
        let (mode, common) = from_toolbar(pick(T::Zoom, false, true));
        assert!(matches!(mode, Mode::Zoom));
        assert!(!common.cursor && !common.no_preview && common.delay.is_none() && common.output.is_none());
    }
```

Run → compile errors.

- [ ] **Step 2: `main.rs`**

`Command::Toolbar` documented "Pick a mode from a floating bar (Cmd+Shift+5).", then in `run`:

```rust
        Command::Toolbar => {
            let config = config::load(&config::default_path())?;
            let (mode, common) = from_toolbar(toolbar::run(&config)?);
            capture(mode, common)
        }
```

and

```rust
/// What the toolbar picked, as the command line would have said it.
fn from_toolbar(pick: toolbar::Picked) -> (Mode, Common) {
    let mode = match pick.mode {
        toolbar::state::Mode::Screen => Mode::Screen { all: false },
        toolbar::state::Mode::Window => Mode::Window,
        toolbar::state::Mode::Region => Mode::Region,
        toolbar::state::Mode::Zoom => Mode::Zoom,
    };
    let common = Common {
        clipboard_only: false,
        output: None,
        delay: None,
        cursor: pick.cursor,
        no_preview: !pick.preview,
    };
    (mode, common)
}
```

(`Common` needs no derive changes; it is constructed by field.)

- [ ] **Step 3: `toolbar/mod.rs`**

```rust
//! `valw toolbar`: a floating bar to pick a mode, then the optional
//! countdown. The choice runs through the normal capture.

pub mod draw;
pub mod layout;
pub mod state;

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use smithay_client_toolkit::{
    seat::{
        keyboard::Keysym,
        pointer::{BTN_LEFT, PointerEvent, PointerEventKind},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerSurface},
    },
    shm::slot::{Buffer, SlotPool},
};
use wayland_client::{QueueHandle, protocol::wl_shm};
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::{Shape, WpCursorShapeDeviceV1};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;

use self::layout::{Layout, MenuItem, Target};
use self::state::{Mode, ToolbarState};
use crate::config::Config;
use crate::error::{Cancelled, HintExt};
use crate::wayland::{Output, State, Wayland};

pub const NAMESPACE: &str = "valw-toolbar";
pub const COUNTDOWN_NAMESPACE: &str = "valw-countdown";
const PILL: (f32, f32) = (72.0, 40.0);

/// The mode and options the user picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Picked {
    pub mode: Mode,
    pub cursor: bool,
    pub preview: bool,
}

/// Shows the bar, waits for a pick, saves it, runs the countdown.
pub fn run(config: &Config) -> Result<Picked> {
    let path = state::default_path();
    let mut wl = Wayland::connect()?;
    let outputs = wl.outputs();
    anyhow::ensure!(!outputs.is_empty(), "the compositor reported no outputs");
    let output = outputs[crate::focused_output(&outputs)].clone();
    let initial = state::load(&path, config);

    let picked = pick(&mut wl, &output, initial)?;
    if let Err(e) = state::save(&path, &picked.0) {
        tracing::warn!("could not remember the toolbar choice: {e:#}");
    }
    countdown(&mut wl, &output, picked.0.timer)?;
    let s = picked.0;
    Ok(Picked { mode: picked.1, cursor: s.cursor, preview: s.preview })
}
```

(`crate::focused_output` is the existing private fn in `main.rs`; make it `pub(crate)`.)

The bar surface:

```rust
pub struct Toolbar {
    layer: LayerSurface,
    viewport: WpViewport,
    pool: SlotPool,
    buffer: Option<Buffer>,
    scale: f64,
    /// Logical size once configured.
    size: Option<(u32, u32)>,
    layout: Option<Layout>,
    state: ToolbarState,
    hover: Option<Target>,
    menu_open: bool,
    outcome: Option<Option<Mode>>,
}

fn pick(wl: &mut Wayland, output: &Output, initial: ToolbarState) -> Result<(ToolbarState, Mode)> {
    let qh = wl.queue.handle();
    let s = &wl.state;
    let layer_shell = s.layer_shell.as_ref().context("compositor does not support wlr-layer-shell").hint("run `valw doctor` to see what the compositor supports")?;
    let viewporter = s.viewporter.as_ref().context("compositor does not support wp-viewporter").hint("run `valw doctor` to see what the compositor supports")?;
    let surface = s.compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(&qh, surface, Layer::Overlay, Some(NAMESPACE), Some(&output.wl));
    layer.set_anchor(Anchor::all());
    layer.set_exclusive_zone(-1);
    layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
    layer.commit();
    let viewport = viewporter.get_viewport(layer.wl_surface(), &qh, ());
    let pool = SlotPool::new(4096, &s.shm).context("could not create shm pool")?;
    wl.state.toolbar = Some(Toolbar {
        layer, viewport, pool, buffer: None, scale: output.scale, size: None, layout: None,
        state: initial, hover: None, menu_open: false, outcome: None,
    });
    let result = wl.dispatch_blocking(|s| s.toolbar.as_ref().is_some_and(|t| t.outcome.is_some()));
    let toolbar = wl.state.toolbar.take();
    // The bar must be gone before anything is captured.
    let _ = wl.conn.flush();
    let _ = wl.queue.roundtrip(&mut wl.state);
    result?;
    let toolbar = toolbar.context("the toolbar vanished")?;
    match toolbar.outcome.flatten() {
        Some(mode) => Ok((ToolbarState { mode, ..toolbar.state }, mode)),
        None => Err(Cancelled.into()),
    }
}

impl Toolbar {
    fn is(&self, surface: &wayland_client::protocol::wl_surface::WlSurface) -> bool {
        self.layer.wl_surface() == surface
    }

    pub fn configure(&mut self, layer: &LayerSurface, (w, h): (u32, u32)) {
        if !self.is(layer.wl_surface()) || w == 0 || h == 0 {
            return;
        }
        self.size = Some((w, h));
        self.layout = Some(layout::layout((w as f32, h as f32), draw::measure));
        self.viewport.set_destination(w as i32, h as i32);
        self.draw();
    }

    pub fn closed(&mut self, layer: &LayerSurface) {
        if self.is(layer.wl_surface()) {
            self.outcome.get_or_insert(None);
        }
    }

    fn draw(&mut self) {
        let (Some((w, h)), Some(layout)) = (self.size, &self.layout) else { return };
        let (pw, ph) = ((w as f64 * self.scale).round() as u32, (h as f64 * self.scale).round() as u32);
        let pixmap = draw::bar(layout, (pw, ph), self.scale as f32, &self.state, self.hover, self.menu_open);
        let Ok((buffer, canvas)) = self.pool.create_buffer(pw as i32, ph as i32, pw as i32 * 4, wl_shm::Format::Argb8888) else {
            tracing::warn!("could not allocate a toolbar buffer");
            return;
        };
        draw::to_argb(&pixmap, canvas);
        let surface = self.layer.wl_surface();
        surface.damage_buffer(0, 0, pw as i32, ph as i32);
        if buffer.attach_to(surface).is_err() {
            tracing::warn!("could not attach the toolbar buffer");
            return;
        }
        self.layer.commit();
        self.buffer = Some(buffer);
    }

    pub fn pointer(&mut self, events: &[PointerEvent], cursor: Option<&WpCursorShapeDeviceV1>) {
        let Some(layout) = self.layout.clone() else { return };
        for event in events {
            if !self.is(&event.surface) {
                continue;
            }
            let p = (event.position.0 as f32, event.position.1 as f32);
            match event.kind {
                PointerEventKind::Enter { serial } => {
                    if let Some(cursor) = cursor {
                        cursor.set_shape(serial, Shape::Default);
                    }
                }
                PointerEventKind::Motion { .. } => {
                    let hover = layout.hit(p, self.menu_open);
                    if hover != self.hover {
                        self.hover = hover;
                        self.draw();
                    }
                }
                PointerEventKind::Press { button: BTN_LEFT, .. } => match layout.hit(p, self.menu_open) {
                    Some(Target::Mode(mode)) => self.outcome = Some(Some(mode)),
                    Some(Target::Options) => {
                        self.menu_open = !self.menu_open;
                        self.draw();
                    }
                    Some(Target::Menu(item)) => {
                        match item {
                            MenuItem::Timer(t) => self.state.timer = t,
                            MenuItem::Cursor => self.state.cursor = !self.state.cursor,
                            MenuItem::Preview => self.state.preview = !self.state.preview,
                        }
                        self.draw();
                    }
                    None if !layout.inside(p, self.menu_open) => self.outcome = Some(None),
                    None => {}
                },
                _ => {}
            }
        }
    }

    pub fn key(&mut self, key: Keysym) {
        if key == Keysym::Escape {
            self.outcome.get_or_insert(None);
        }
    }
}
```

The countdown:

```rust
pub struct Pill {
    layer: LayerSurface,
    viewport: WpViewport,
    pool: SlotPool,
    buffer: Option<Buffer>,
    scale: f64,
    configured: bool,
    text: String,
    cancelled: bool,
}

/// Shows `timer`, `timer - 1`, … 1, one second each; a click on the pill
/// cancels. The pill takes no keyboard focus, so the desktop stays usable.
fn countdown(wl: &mut Wayland, output: &Output, timer: u32) -> Result<()> {
    if timer == 0 {
        return Ok(());
    }
    let qh = wl.queue.handle();
    let s = &wl.state;
    let layer_shell = s.layer_shell.as_ref().context("compositor does not support wlr-layer-shell")?;
    let viewporter = s.viewporter.as_ref().context("compositor does not support wp-viewporter")?;
    let surface = s.compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(&qh, surface, Layer::Overlay, Some(COUNTDOWN_NAMESPACE), Some(&output.wl));
    layer.set_anchor(Anchor::BOTTOM);
    layer.set_margin(0, 0, layout::MARGIN_BOTTOM as i32, 0);
    layer.set_size(PILL.0 as u32, PILL.1 as u32);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.commit();
    let viewport = viewporter.get_viewport(layer.wl_surface(), &qh, ());
    let pool = SlotPool::new(4096, &s.shm).context("could not create shm pool")?;
    wl.state.pill = Some(Pill { layer, viewport, pool, buffer: None, scale: output.scale, configured: false, text: timer.to_string(), cancelled: false });

    let start = Instant::now();
    let mut result = Ok(());
    for left in (1..=timer).rev() {
        if let Some(pill) = &mut wl.state.pill {
            pill.set_text(&left.to_string());
        }
        let until = start + Duration::from_secs((timer - left + 1) as u64);
        // dispatch_until fails when the deadline passes: that is the tick.
        let _ = wl.dispatch_until(until, |s| s.pill.as_ref().is_some_and(|p| p.cancelled));
        if wl.state.pill.as_ref().is_some_and(|p| p.cancelled) {
            result = Err(Cancelled.into());
            break;
        }
    }
    wl.state.pill = None;
    // The pill must be gone before anything is captured.
    let _ = wl.conn.flush();
    let _ = wl.queue.roundtrip(&mut wl.state);
    result
}

impl Pill {
    fn is(&self, surface: &wayland_client::protocol::wl_surface::WlSurface) -> bool {
        self.layer.wl_surface() == surface
    }

    pub fn configure(&mut self, layer: &LayerSurface) {
        if self.is(layer.wl_surface()) {
            self.configured = true;
            self.viewport.set_destination(PILL.0 as i32, PILL.1 as i32);
            self.draw();
        }
    }

    fn set_text(&mut self, text: &str) {
        self.text = text.to_string();
        self.draw();
    }

    fn draw(&mut self) {
        if !self.configured {
            return;
        }
        let (pw, ph) = ((PILL.0 as f64 * self.scale).round() as u32, (PILL.1 as f64 * self.scale).round() as u32);
        let pixmap = draw::pill((pw, ph), self.scale as f32, &self.text);
        let Ok((buffer, canvas)) = self.pool.create_buffer(pw as i32, ph as i32, pw as i32 * 4, wl_shm::Format::Argb8888) else {
            return;
        };
        draw::to_argb(&pixmap, canvas);
        let surface = self.layer.wl_surface();
        surface.damage_buffer(0, 0, pw as i32, ph as i32);
        if buffer.attach_to(surface).is_ok() {
            self.layer.commit();
            self.buffer = Some(buffer);
        }
    }

    pub fn pointer(&mut self, events: &[PointerEvent]) {
        for event in events {
            if self.is(&event.surface) && matches!(event.kind, PointerEventKind::Press { button: BTN_LEFT, .. }) {
                self.cancelled = true;
            }
        }
    }

    pub fn closed(&mut self, layer: &LayerSurface) {
        if self.is(layer.wl_surface()) {
            self.cancelled = true;
        }
    }
}
```

(The "Cancel on hover" label from the plan's ruling is dropped for simplicity: the pill shows the seconds; a click cancels. Ledger it.)

- [ ] **Step 4: `wayland.rs` routing**

`use crate::toolbar::{Pill, Toolbar};`; fields `pub toolbar: Option<Toolbar>`, `pub pill: Option<Pill>` (initialised `None`); in `closed`: `toolbar.closed(layer)` and `pill.closed(layer)`; in `configure`: `toolbar.configure(layer, configure.new_size)` and `pill.configure(layer)`; in `press_key`: `if let Some(t) = &mut self.toolbar { t.key(event.keysym); }`; in `pointer_frame`: `toolbar.pointer(events, self.cursor_device.as_ref())` and `pill.pointer(events)`.

Remove the Task 1 allowance in `main.rs`.

- [ ] **Step 5: Run** — all pass (+1 test, +1 assert), clippy, fmt.

- [ ] **Step 6: Smoke run** (tell the user first) — `timeout -s INT 4 valw toolbar` in the dev shell: the bar appears bottom-centre on the focused output; a niri screenshot shows it; exit by the signal; no errors in the log. Then `valw toolbar` with a temporary state file `timer = 5` is not run (it would capture); the countdown is checked by hand later.

- [ ] **Step 7: Commit** `feat: valw toolbar`.

---

### Task 4: Checks and checklist

- [ ] `nix flake check -L --keep-going` passes.
- [ ] Append to `docs/test-checklist.md`:

```markdown

## Toolbar (Phase 6)

- [ ] `valw toolbar` shows the bar bottom-centre on the focused output (both monitors), crisp; the last mode is highlighted; hovering highlights buttons.
- [ ] Screen, Window, Region and Zoom each start their mode at once; the bar is not in the shot.
- [ ] Options ▾ opens the menu; timer None/5/10, Show cursor and Show preview toggle (✓) and are remembered next time (`~/.local/state/valw/toolbar.toml`).
- [ ] With a 5 s timer the pill counts 5 → 1 bottom-centre, the desktop stays usable meanwhile, and the pill is not in the shot; clicking the pill cancels (exit 3).
- [ ] Esc or a click beside the bar closes it without capturing (exit 3).
- [ ] A broken `toolbar.toml` doesn't stop the bar (warning in the log, defaults used).
- [ ] Bind it in niri (Mod+Shift+5 is taken in copland; pick a key).
```

- [ ] Commit `docs: toolbar checklist`.
