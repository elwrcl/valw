# valw

macOS-style screenshots for [niri](https://github.com/YaLTeR/niri) on
Wayland: the whole screen, a region, a window, a zoomable freeze-frame, a
Cmd+Shift+5-style toolbar, a floating preview you can drag into other apps,
and a built-in markup editor. Written in Rust, packaged with Nix.

## Install

Run it once:

```sh
nix run git+https://git.userevolt.app/elars/valw -- doctor
```

As a package from the flake (`packages.<system>.default`), or with the
home-manager module:

```nix
# flake.nix
inputs.valw.url = "git+https://git.userevolt.app/elars/valw";

# a home-manager module
{ inputs, ... }:
{
  imports = [ inputs.valw.homeModules.default ];

  programs.valw = {
    enable = true;
    settings = {
      preview.timeout_secs = 3;
      sound.volume = 0.4;
      capture.window_shadow = true;
    };
  };
}
```

`settings` becomes `~/.config/valw/config.toml`. valw checks it while the
configuration is built, so a typo or a bad value fails the rebuild instead
of surprising you later. The toolbar's remembered choices live in
`~/.local/state/valw/toolbar.toml` and stay writable.

## Bind it in niri

```kdl
binds {
    Print           { spawn "valw" "screen"; }
    Mod+Shift+S     { spawn "valw" "region"; }
    Mod+Shift+W     { spawn "valw" "window"; }
    Mod+Shift+Z     { spawn "valw" "zoom"; }
    Mod+Shift+T     { spawn "valw" "toolbar"; }
}
```

## Commands

| Command | What it does |
|---|---|
| `valw screen [--all]` | The focused output (or every output) |
| `valw region` | Drag a region; Shift locks an axis, Alt grows from the centre, Space moves it; Space before dragging switches to window mode |
| `valw window [--pick]` | Pick a window from cards over a liquid-paint background (Tab/arrows, Enter, Esc); `--pick` clicks it on screen instead |
| `valw zoom` | Freeze the screen and zoom: wheel, drag, `f` flashlight, `c` capture the view, `0` reset, Esc |
| `valw toolbar` | Pick a mode from a floating bar, with a timer and options |
| `valw edit FILE` | The markup editor (clicking a preview opens it) |
| `valw backdrop` | Keep running: the liquid-paint background behind niri's overview (below) |
| `valw doctor` | What the compositor and system support |

Capture commands take `--clipboard-only`, `-o PATH` (`-` for stdout),
`--delay SECS`, `--cursor` and `--no-preview`.

## The overview backdrop

`valw backdrop` draws a slow liquid-paint background, coloured from the
wallpaper (via Noctalia, if it runs), behind niri's overview. It only draws
while the overview shows it. Start it with niri and place it in the
backdrop:

```kdl
spawn-at-startup "valw" "backdrop"

layer-rule {
    match namespace="^valw-backdrop$"
    place-within-backdrop true
}
```

With Noctalia, turn its own backdrop off (`[backdrop] enabled = false`).
Colours elsewhere (the region dim, the window picker's fallback) follow
Noctalia's current community palette when there is one.

## Configuration

Every key is optional; these are the defaults.

```toml
[save]
directory = "~/Pictures/Screenshots"
filename = "Screenshot %Y-%m-%d at %H.%M.%S.png"
copy_to_clipboard = true

[capture]
show_cursor = false
window_shadow = false

[preview]
enabled = true
timeout_secs = 5

[zoom]
scroll_step = 1.15
flashlight_radius = 180

[editor]
backend = "builtin"   # or "satty"

[sound]
enabled = true
volume = 0.6
combo_reset_secs = 5
```

Logs are in `~/.local/state/valw/logs/`.
