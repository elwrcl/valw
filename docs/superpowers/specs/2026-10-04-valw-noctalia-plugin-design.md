# valw Noctalia plugin (design)

Date: 2026-10-04
Status: approved in brainstorming, pending written-spec review
Builds on: every phase on `main`; the toolbar (`src/toolbar/`), the
home-manager module (`nix/home-module.ulu.nix`)

## 1. Goal

valw's own toolbar is drawn by valw and does not look like the user's
Noctalia shell. Instead of imitating Noctalia, give it a Noctalia plugin:
Noctalia draws the toolbar with its own components, so the theme, font,
corners and border follow Noctalia's settings with no theming code in
valw.

- A bar widget (a camera glyph) and a control-center tile.
- The toolbar as a Noctalia panel: the four modes, the timer and the
  three options.
- One source of truth: the plugin and `valw toolbar` share
  `$XDG_STATE_HOME/valw/toolbar.toml`, through three new `valw toolbar`
  flags; the plugin holds no capture logic.

`valw toolbar` (the self-drawn bar) stays as it is, for setups without
Noctalia.

### Out of scope

- A countdown on the bar while a timer runs.
- Dragging a shot out of the panel (`ui.dragSource` exists; a later idea).
- Noctalia styling for valw's own surfaces (preview, picker, region label).
- Publishing to Noctalia's community plugin source.

## 2. The bridge: new `valw toolbar` flags

Mutually exclusive with each other; without any of them `valw toolbar`
opens the bar as today.

- `valw toolbar --state`: prints the remembered state as one line of
  JSON, then exits 0:
  `{"mode":"region","timer":0,"cursor":false,"preview":true,"sound":true,"timers":[0,5,10]}`.
  A missing or broken file prints the defaults (as `load` does today).
- `valw toolbar --set KEY=VALUE` (repeatable): validates every pair, then
  writes them all at once (atomic write, the same file and format the bar
  saves). Keys: `mode` (`screen|window|region|zoom`), `timer` (one of
  `TIMERS`), `cursor`, `preview`, `sound` (`true|false`). Any unknown key
  or bad value: an error naming it, exit 2, nothing written.
- `valw toolbar --run [MODE]`: captures at once with the remembered
  options, exactly as choosing that mode in the bar would (timer as the
  delay, the `toolbar_cursor`/`toolbar_preview`/`toolbar_sound`
  overrides). A given `MODE` is remembered as the new mode; without one
  the remembered mode is used. No bar surface is created. Exit codes as
  for the capture commands (3 = cancelled).

The capture path the bar already uses is shared, not duplicated: the
bar's "mode chosen" step and `--run` call the same function.

## 3. The plugin

`noctalia-plugin/` in the repo, id `elars/valw`, `plugin_api = 3` (what
the installed Noctalia 5.2 and its official plugins use), MIT,
`dependencies = ["valw"]`, icon `camera`.

```
noctalia-plugin/
  plugin.toml
  bar.luau        [[widget]] id = "bar"
  panel.luau      [[panel]]  id = "toolbar"
  shortcut.luau   [[shortcut]] id = "tile"
  translations/en.json, translations/tr.json
  README.md
```

Commands run through `noctalia.runAsync` with fixed command strings (no
user text is ever interpolated into them).

### 3.1 Bar widget

- A `camera` glyph; tooltip "Screenshot".
- Left click: `noctalia.togglePanel("elars/valw:toolbar")`.
- Right click: `valw region` (plain, ignoring the toolbar's timer and
  options, like the keybind).

### 3.2 Toolbar panel

`placement = "attached"`, `position = "auto"`, `open_near_click = true`,
about 360 × 220.

- On open (`onOpen`): `valw toolbar --state` → `noctalia.json.decode` →
  render. If valw is missing (`noctalia.commandExists`) or the call fails:
  a single label "valw is not available" and `noctalia.notifyError` with
  the stderr's last line.
- A row of four buttons with glyphs and labels: Screen (`device-desktop`),
  Region (`crop`), Window (`app-window`), Zoom (`zoom-in`). The
  remembered mode is `variant = "primary"`, the others `"outline"`.
- A click on a mode: `panel.close()`, then
  `sleep 0.3 && valw toolbar --run <mode>` (the wait lets the panel
  disappear before the screen is frozen or captured). A non-zero exit
  other than 3 → `noctalia.notifyError("valw", <stderr's last line>)`.
- Below: "Timer" `ui.select` with `Off`, `5 s`, `10 s` (from `timers`),
  and three `ui.toggle`s: "Show cursor", "Show preview", "Play sound".
  A change re-renders at once and runs `valw toolbar --set <key>=<value>`;
  a failure → `notifyError`.
- Colours only by Noctalia role names (`primary`, `on_surface`,
  `outline`, …); no hex colours anywhere in the plugin.

### 3.3 Control-center tile

- Label "Screenshot", icon `camera`.
- Click: `noctalia msg panel-close; sleep 0.3 && valw region` (the control
  center closes first, so it is not in the shot).
- Right click: `noctalia.togglePanel("elars/valw:toolbar")`.

### 3.4 Translations

All visible strings through `noctalia.tr`, with `en.json` and `tr.json`.

## 4. Packaging and home-manager

- The package installs `noctalia-plugin/` to
  `$out/share/valw/noctalia-plugin` (the source filter keeps the folder).
- `programs.valw.noctalia.enable` (bool, default false): links that folder
  to `$XDG_DATA_HOME/noctalia/plugins/valw` with `xdg.dataFile`, where
  Noctalia finds local plugins. Enabling the plugin and placing the
  widget stay in the user's Noctalia settings (copland), since the valw
  module does not own Noctalia's config.
- The folder name Noctalia expects under `plugins/` and whether it
  follows a symlink into the store are checked against the live Noctalia
  during implementation; if a symlink is refused, the module copies the
  files instead (`recursive = true`).

## 5. Wiring (copland, with the user's approval, at the end)

- `programs.valw.noctalia.enable = true`.
- Noctalia: `"elars/valw"` in `plugins.enabled`; the widget
  (`"elars/valw:bar"`) in the bar's `end` list.
- niri: `Mod+Shift+T` → `noctalia msg panel-toggle elars/valw:toolbar`.

## 6. Structure

| File | Role |
|---|---|
| `src/toolbar/state.rs` | `to_json`, `apply_sets` (parse and validate `KEY=VALUE`) |
| `src/toolbar/mod.rs` | the shared "capture with these options" step |
| `src/main.rs` | `--state`, `--set`, `--run` on `Command::Toolbar` |
| `noctalia-plugin/*` | the plugin |
| `nix/package.ulu.nix` | install the plugin folder |
| `nix/home-module.ulu.nix` | `programs.valw.noctalia.enable` |
| `nix/checks.ulu.nix` | plugin files present; Luau syntax; HM option |
| `README.md`, `docs/test-checklist.md` | wiring; a Noctalia section |

## 7. Testing

Unit tests (`cargo nextest`):
- `--state` JSON: the defaults, a saved file, a broken file (defaults).
- `--set`: every key; several pairs at once; unknown key, bad mode,
  `timer=7`, `cursor=maybe`, a pair without `=` all rejected with nothing
  written; the written file loads back to the same state.
- `--run`: a given mode is remembered; the options it passes to the
  capture equal the bar's for the same state (the shared step, tested
  without a compositor).
- CLI: the three flags conflict with each other.

`nix flake check`:
- The package has `share/valw/noctalia-plugin/plugin.toml` with
  `id = "elars/valw"`, and every `entry` named in it exists.
- Every `.luau` file compiles with `luau-compile` (syntax only; the
  Noctalia globals are not checked).
- The home-manager check: with `noctalia.enable = true` the data file is
  set; without it, not.

Manual checks go to a Noctalia section of `docs/test-checklist.md`: the
widget appears, both clicks, the panel follows the theme (try a palette
and light/dark switch), every mode button, the timer and toggles persist
and match `valw toolbar`, the panel is not in the shot, the tile, the
error label without valw on `PATH`.

## 8. Definition of done

`cargo clippy -- -D warnings`, `cargo fmt --check`, `cargo nextest run`
and `nix flake check` pass; README and checklist updated; the plugin
loads in the live Noctalia; copland wiring done with the user's approval.
