# valw

valw is desktop helper for 3N' [nixos] (https://nixos.org/) [niri](https://github.com/YaLTeR/niri) [noctalia](https://github.com/noctalia-dev/noctalia)

## installation

run first:

```sh
nix run git+https://git.userevolt.app/elars/valw -- doctor
```

use as package from the flake (`packages.<system>.default`), or with the
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
## binding on niri

```kdl
binds {
    Print           { spawn "valw" "screen"; }
    Mod+Shift+S     { spawn "valw" "region"; }
    Mod+Shift+W     { spawn "valw" "window"; }
    Mod+Shift+Z     { spawn "valw" "zoom"; }
    Mod+Shift+T     { spawn "valw" "toolbar"; }
}
```

## commands

| Command | What it does |
|---|---|
| `valw screen [--all]` | The focused output (or every output) |
| `valw region` | Drag a region; Shift locks an axis, Alt grows from the centre, Space moves it; Space before dragging switches to window mode |
| `valw window [--pick]` | Pick a window from cards over a liquid-paint background (Tab/arrows, Enter, Esc); `--pick` clicks it on screen instead |
| `valw zoom` | Freeze the screen and zoom: wheel, drag, `f` flashlight, `c` capture the view, `0` reset, Esc |
| `valw toolbar` | Pick a mode from a floating bar, with a timer and options; `--state`, `--set KEY=VALUE` and `--run [MODE]` drive it without the bar (the Noctalia plugin uses them) |
| `valw edit FILE` | The markup editor (clicking a preview opens it) |
| `valw backdrop` | Keep running: the liquid-paint background behind niri's overview (below) |
| `valw doctor` | What the compositor and system support |

Capture commands take `--clipboard-only`, `-o PATH` (`-` for stdout),
`--delay SECS`, `--cursor` and `--no-preview`.


## configuration

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
backend = "builtin"   # or "satty","noctalia" choose which one is for you.

[sound]
enabled = true
volume = 0.9
combo_reset_secs = 5
```

logs are in `~/.local/state/valw/logs/`.
