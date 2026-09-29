# valw: Phase 0 + 1 design

Date: 2026-09-29
Status: approved in brainstorming, pending written-spec review

## 1. Context

valw is a macOS-style (Cmd+Shift+3/4/5) screenshot tool for niri on NixOS,
written in Rust. The full product vision (preview thumbnail, editor, zoom mode,
toolbar) is described in the user's original spec
(`wayland_screenshot_tool_spec.md`, where the project was called `snapr`).
That vision is split into six phases; each phase gets its own
design → plan → implementation cycle.

This document covers **Phase 0 (discovery + scaffolding)** and
**Phase 1 (core capture)** only.

### Goals

- A dendritic Nix flake that builds valw and provides a devShell.
- `valw screen` and `valw region` capture pixel-exact PNGs, save them and copy
  them to the clipboard.
- Errors explain what happened, why, and what to do next, and every run leaves
  a log good enough to debug from.
- `valw doctor` reports what the running compositor supports.

### Non-goals (later phases)

- Preview thumbnail, desktop notifications, `--no-preview` (Phase 2)
- Window mode, Shift/Alt/Space modifiers, size indicator, fonts (Phase 3)
- Zoom mode (Phase 4)
- Editor (Phase 5)
- Toolbar, drag and drop, home-manager module, README (Phase 6)
- `ext-image-copy-capture-v1` backend (not offered by niri 26.04)

## 2. Verified environment

Checked on the user's machine on 2026-09-29:

| Item | Value |
|---|---|
| niri | 26.04 (nixpkgs) |
| `zwlr_screencopy_manager_v1` | v3 |
| `ext_image_copy_capture_*` | **not offered** |
| `zwlr_layer_shell_v1` | v5 |
| `wp_fractional_scale_manager_v1` | v1 |
| `wp_viewporter` | v1 |
| `wp_cursor_shape_manager_v1` | v2 |
| `zxdg_output_manager_v1` | v3 |
| `wl_output` | v4 |
| `wl_seat` / `wl_shm` | v9 / v2 |
| `ext_data_control_manager_v1` / `zwlr_data_control_manager_v1` | v1 / v2 |
| Outputs | LVDS-1 1366×768 @ scale 1, HDMI-A-1 1920×1080 @ scale 1 |
| Shell | Noctalia |
| jj | 0.45.1, repo is **non-colocated** (no `.git` at the root) |
| direnv | 2.37.1 with nix-direnv (home-manager) |

Consequences:

- Screen capture uses wlr-screencopy only. The `Capturer` trait stays so a
  second backend can be added later, but only one implementation is written now.
- Fractional scaling must be tested by temporarily setting a scale
  (`niri msg output HDMI-A-1 scale 1.25`).

## 3. Conventions

- Code, comments, error messages, logs and docs are in **English**. Comments
  are plain and short; they explain why, not what.
- Version control is jj. The repo is non-colocated, so Nix treats it as a
  `path:` flake.
- Rust toolchain: stable from nixpkgs (no rust-overlay/fenix).
- Test runner: `cargo nextest`. Lints: `cargo clippy -- -D warnings`.

## 4. Repository layout

```
.envrc
.gitignore
flake.nix              # flake-parts; auto-discovers nix/**/*.ulu.nix
flake.lock
nix/
  package.ulu.nix      # perSystem.packages.default (buildRustPackage)
  devshell.ulu.nix     # perSystem.devShells.default
  formatter.ulu.nix    # nixfmt-tree, same as copland
  checks.ulu.nix       # perSystem.checks: clippy, fmt, nextest, headless sway
Cargo.toml
Cargo.lock
src/
  main.rs              # clap CLI, subcommand dispatch
  config.rs            # config file loading
  error.rs             # user-facing error type and rendering
  log.rs               # tracing setup, per-run log files
  lock.rs              # single-instance lock
  detach.rs            # fork a background child (clipboard server)
  wayland.rs           # SCTK state: registry, outputs, seat, shm
  frame.rs             # Frame type, pixel format conversion, transforms
  capture.rs           # Capturer trait + wlr-screencopy implementation
  niri.rs              # niri IPC wrapper (focused output, version)
  selection.rs         # selection state machine, logical → pixel math
  region.rs            # region selection overlay (layer-shell surfaces)
  render.rs            # overlay pixels: dim, paste selection, outline
  output.rs            # PNG saving, filename template, clipboard
  doctor.rs            # `valw doctor`
docs/
```

Files stay flat for now; `overlay/` and `editor/` subdirectories appear once
Phases 3 to 5 give them enough content. Pure logic (`frame`, `selection`,
`render`, `output`, `config`, `error`) is kept apart from Wayland code so it
can be unit tested without a compositor.

### 4.1 Nix

`flake.nix` follows copland's pattern (flake-parts + automatic discovery), but
discovery is limited to `./nix` so `src/` is not walked:

```nix
imports = filter (hasSuffix ".ulu.nix") (listFilesRecursive ./nix);
```

Systems: `x86_64-linux`, `aarch64-linux`.

`package.ulu.nix`:

- `pkgs.rustPlatform.buildRustPackage`, `cargoLock.lockFile = ./Cargo.lock`.
- `src` is filtered with `lib.cleanSourceWith` to drop `target`, `result`,
  `.jj` and `.direnv`.
- `nativeBuildInputs`: `pkg-config`. `buildInputs`: `wayland`,
  `libxkbcommon`.
- `meta.mainProgram = "valw"`.
- `doCheck = false`; tests run through `checks` instead.

`devshell.ulu.nix`: `rustc`, `cargo`, `clippy`, `rustfmt`, `rust-analyzer`,
`cargo-nextest`, `pkg-config`, `wayland`, `libxkbcommon`, `wayland-utils`,
`grim`, `imagemagick` (the last three for manual verification). It sets:

```sh
CARGO_TARGET_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/valw/target"
```

Because the repo has no `.git`, Nix copies the whole directory into the store
on every evaluation and ignores `.gitignore`. Keeping `target/` out of the tree
keeps that copy small.

`checks.ulu.nix` provides `clippy`, `fmt`, `nextest` and `headless`
(section 10.2).

### 4.2 direnv

`.envrc`:

```sh
watch_file nix/*.ulu.nix
use flake
```

nix-direnv only watches `flake.nix` and `flake.lock` by default; the devShell
lives in `nix/devshell.ulu.nix`, so it must be watched explicitly.

### 4.3 .gitignore

Whitelist style:

```gitignore
/*
!/.gitignore
!/.envrc
!/flake.nix
!/flake.lock
!/Cargo.toml
!/Cargo.lock
!/nix/
!/src/
!/docs/
!/README.md
```

Any new top-level file must be added here. Otherwise jj silently ignores it,
while the `path:` flake still sees it: the build works locally but the commit
is incomplete.

## 5. Dependencies

| Purpose | Crate |
|---|---|
| Wayland client | `smithay-client-toolkit`, `wayland-client`, `wayland-protocols`, `wayland-protocols-wlr` |
| PNG | `image` (png feature only) |
| Clipboard | `wl-clipboard-rs` |
| niri IPC | `niri-ipc` (`=26.4.0`, matches niri 26.04) |
| CLI | `clap` (derive) |
| Config | `serde`, `toml`, `serde_ignored` |
| Time / filenames | `chrono` |
| Errors | `anyhow` |
| Logging | `tracing`, `tracing-subscriber` |
| Lock, poll, stdio | `rustix` |
| fork | `libc` |

Not needed in Phase 1: `tiny-skia` (the overlay is plain byte copies; shapes
and text arrive in Phase 3) and `tracing-appender` (one file per run needs no
rolling writer).

Dependencies are built with `opt-level = 3` in the dev profile; unoptimised
PNG encoding is too slow to test with.

## 6. Capture pipeline

Every capture command runs these steps in order.

1. **Log setup** (section 9.2).
2. **Lock.** Take a non-blocking `flock` on `$XDG_RUNTIME_DIR/valw.lock`. If it
   is held, fail with "valw is already running" (exit 1). A second press of the
   hotkey is ignored rather than queued. The kernel releases the lock if the
   process dies.
3. **Delay.** Sleep for `--delay <secs>` if given.
4. **Connect.** Bind the registry with SCTK and collect outputs (name, logical
   position and size via xdg-output, physical size, scale), seat, shm,
   screencopy and layer-shell. A missing required global is an error that
   names the protocol.
5. **Capture.** `Capturer::capture(&[Output], cursor: bool) -> Vec<Frame>`
   issues one screencopy request per output in parallel and waits up to 5 s
   for every `ready`. A `Frame` holds upright, opaque RGBA8 pixels at physical
   resolution plus its output name and logical geometry; the scale is
   `frame width / logical width`. `XRGB8888`, `ARGB8888`, `XBGR8888` and
   `ABGR8888` are converted; `y_invert` is honoured; any other format is an
   error that names the format. Alpha is always 255, even where the
   compositor's buffer says 0. `cursor` comes from `--cursor` or
   `capture.show_cursor`.
   - **niri quirk (verified in its source):** niri rejects a shm buffer
     with `invalid buffer` unless the whole `wl_shm_pool` is exactly the
     buffer's size. Every capture therefore gets its own pool.
   - **Rotated outputs:** the buffer arrives in the output's native
     orientation. It is turned upright the way grim does it: y-invert,
     rotate clockwise by the transform's angle, then mirror if flipped.

### 6.1 `valw screen`

- Default: ask niri IPC for the focused output (`FocusedOutput`), capture only
  that output, save it and copy it to the clipboard.
- `--all`: capture every output and save one file per output, with the output
  name appended to the filename (`... (HDMI-A-1).png`). Only the focused
  output goes to the clipboard.
- If niri IPC is unreachable (another compositor), log a warning and use the
  first output.

## 7. `valw region`

### 7.1 Surfaces

Capture all outputs first (section 6), then open one layer-shell surface per
output:

- namespace `valw-overlay`, layer `Overlay`, anchored to all four edges,
  `exclusive_zone = -1`, `keyboard_interactivity = Exclusive`
- background: that output's frozen frame, darkened by 40%
- the selection area is copied from the undarkened frame and outlined with a
  1 physical pixel white border
- cursor shape: crosshair (`wp_cursor_shape`)

Surfaces are owned by the process, so the compositor removes them if valw
dies. No overlay can outlive the process.

### 7.2 HiDPI

- Each surface's buffer is the frozen frame's size (physical pixels), and
  `wp_viewporter` sets the destination to the logical size from the layer
  configure. The frame is shown 1:1 with no resampling. The frame already
  says how many physical pixels the output has, so `wp_fractional_scale` is
  not needed.
- Pointer events arrive in surface-local logical coordinates. They are
  converted to global logical coordinates by adding the output's logical
  position; the selection is stored in global logical coordinates.

### 7.3 Selection state machine

```
Idle --press--> Dragging{start} --release--> Done(rect)
  ^                  |
  |                  +--release with size < 2 physical px--> Idle
  +--Esc / right click (any state)--> Cancelled
```

- While `Dragging`, pointer motion updates the rectangle. Redraws are
  throttled with frame callbacks. The darkened base pixmap is computed once;
  each redraw copies it and pastes the bright selection on top.
- On release, the rectangle is clipped to the output where the drag started
  (v1 rule: one output per capture).
- Cancel closes all surfaces and exits with code 3 without saving anything.

### 7.4 Logical to physical conversion

For an output at logical position `(ox, oy)` with scale `s` (per axis:
frame size / logical size) and a selection `(x, y, w, h)` in global logical
coordinates:

```
x0 = floor((x - ox) * s)       y0 = floor((y - oy) * s)
x1 = ceil((x + w - ox) * s)    y1 = ceil((y + h - oy) * s)
```

The result is clamped to the frame bounds and cropped from the in-memory
frame. The output always matches the frozen image the user saw, even if the
screen changed during selection.

## 8. Output, config and CLI

### 8.1 Config

Path: `$XDG_CONFIG_HOME/valw/config.toml`. Phase 1 reads:

```toml
[save]
directory = "~/Pictures/Screenshots"
filename = "Screenshot %Y-%m-%d at %H.%M.%S.png"
copy_to_clipboard = true

[capture]
show_cursor = false
```

- A missing file means defaults (the values above).
- Invalid TOML is an error that includes the line number (exit 1).
- Unknown keys (e.g. `[preview]`, which later phases read) print a warning
  through `serde_ignored` and are otherwise ignored.
- `~` and `$HOME` in `directory` are expanded. The directory is created if
  missing.

### 8.2 Saving

- The filename is rendered with `chrono` strftime. If the file exists,
  ` (2)`, ` (3)`, … is inserted before the extension.
- PNGs are written to a temporary file in the target directory and then
  renamed, so a partial file never appears.
- Each saved path is printed to stdout on its own line.

### 8.3 Clipboard

A Wayland clipboard needs a process that stays alive to answer paste
requests. `wl-clipboard-rs` does **not** fork: its background mode serves from
a thread, which dies when valw exits. So valw does what `wl-copy` does:

1. `Options::foreground(true).prepare_copy(...)` offers the PNG as
   `image/png` (no threads are started).
2. `detach::spawn` releases the capture lock, then forks. The child calls
   `setsid`, points stdin/stdout/stderr at `/dev/null`, serves paste requests
   until something else is copied, and exits with `_exit`.
3. The parent exits right away.

The lock is released before the fork because the child would otherwise
inherit it and block the next capture. stdio goes to `/dev/null` because an
inherited stdout pipe would keep `valw region -o - | consumer` waiting until
the clipboard changes. A unit test covers both, and the headless test runs
`screen -o -` through a pipe.

### 8.4 CLI

```
valw screen [--all] [COMMON]
valw region [COMMON]
valw doctor

COMMON:
  --clipboard-only   copy to clipboard, do not save a file
  -o, --output <path>  write to this path instead of directory + template;
                       "-" writes PNG to stdout
  --delay <secs>     wait before capturing (non-negative, fractions allowed)
  --cursor           include the cursor in the capture
```

Flag interactions (enforced by clap, so they are usage errors, exit 2):

- `--clipboard-only` conflicts with `--output`.
- `--all` conflicts with `--output` and `--clipboard-only`.
- Otherwise the clipboard follows `save.copy_to_clipboard`, whatever the
  output target (including `--output -`).
- `--clipboard-only` copies even if `save.copy_to_clipboard = false`.

### 8.5 Exit codes

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | error (missing protocol, lock busy, I/O, …) |
| 2 | usage error (clap) |
| 3 | cancelled by the user |

## 9. Errors and diagnostics

### 9.1 User-facing errors

Every error shown to the user has three parts:

```
error: could not capture output HDMI-A-1
  cause: compositor offered only XBGR2101010, which valw can't convert yet
  hint:  run `valw doctor` and include its output in a bug report
  log:   ~/.local/state/valw/logs/2026-09-29T19-40-12.log
```

- `error` is the top-level `anyhow` context; `cause` lines are the rest of
  the context chain.
- `hint` is attached with a small extension trait (`.hint("...")`). Hints are
  added only where there is a concrete action; no generic "something went
  wrong" hints.
- `log` points at this run's log file.

### 9.2 Per-run logs

- `tracing` writes one file per run to `$XDG_STATE_HOME/valw/logs/`; the 10
  most recent are kept.
- Each log starts with an environment summary: valw version and arguments,
  niri version, all globals and their versions (one line), outputs (name,
  position, size, transform). Each capture logs the buffer formats offered
  and the one used, and how long capturing took.
- `region` logs the time from process start to the first overlay commit, and
  the selected rectangle in both logical and pixel coordinates.
- The default level is `info`; `VALW_LOG=debug` (EnvFilter syntax) raises it.
- stderr gets warnings from tracing (with colour only on a terminal) and
  errors in the three-part format above. Processes spawned by niri log stderr
  to niri's journal (`journalctl --user -u niri`), but the log file is the
  primary record.
- If the log directory can't be written, valw keeps working and logs to
  stderr only; a broken log dir must not block a screenshot.

### 9.3 Panics

A panic hook writes the panic and its backtrace to the log and prints the
same three-part format to stderr, with "this is a bug in valw" as the hint.

### 9.4 `valw doctor`

Prints one `ok` / `warn` / `fail` line per check and exits 1 if any check
fails:

- required globals present with a sufficient version (compositor, shm, seat,
  xdg-output v2, layer-shell, screencopy, viewporter); optional ones warn
  when missing (cursor-shape, ext/wlr data-control)
- outputs with name, logical size, position and transform
- niri IPC reachable, and niri's version compatible with the pinned `niri-ipc`
- config file parses
- save directory is writable

Phase 0's discovery report is produced by this command. Later phases add
checks.

Desktop notifications for errors come in Phase 2.

## 10. Testing

### 10.1 Unit tests (`cargo nextest`)

Wayland-independent logic lives in pure functions:

- logical to physical conversion at scales 1, 1.25 and 1.5, negative output
  offsets, clamping to output bounds
- selection state machine: drag, sub-2px click, Esc, right click, drag that
  crosses into another output
- pixel format conversion for the four formats, `y_invert`, unsupported
  format error
- filename template, collision suffix, `--output -`
- config: defaults when missing, unknown key warning, invalid TOML line number
- lock: acquire, busy error; forked child holds neither the lock nor stdout
- output transforms (all eight) against grim's rules
- error rendering: exact text of the three-part output
- `niri-ipc` pin in Cargo.toml matches `niri::IPC_VERSION`

### 10.2 Headless integration test (`nix flake check`)

A check starts **sway** (`sway-unwrapped`; the wrapped one needs D-Bus) with
`WLR_BACKENDS=headless` and `WLR_RENDERER=pixman` inside the build sandbox,
then:

- runs `valw doctor` and asserts no `fail` lines
- runs `valw screen -o - | cat` under a 10 s timeout (so a clipboard child
  holding the pipe fails the check) and asserts the PNG is 1280×720, the
  headless output's size

This exercises the real screencopy path on every build. It runs on wlroots,
not niri. `region` is not covered because it needs synthetic pointer input.

### 10.3 Manual checks on niri (end of each phase)

- **Pixel exactness:** capture a region with `valw region` and the same
  geometry with `grim -g "<geo>"`; `magick compare -metric AE` must report 0.
  Repeat at scale 1.25 (`niri msg output HDMI-A-1 scale 1.25`) and on both
  outputs.
- **Latency:** the log records time from process start to the first overlay
  commit; target under 150 ms.
- **No stuck overlay:** `kill -9` during selection; the overlay disappears and
  the next run gets the lock.
- **Noctalia:** the overlay covers the bar.
- **Rotated output:** `niri msg output HDMI-A-1 transform 90`, then
  `valw screen` and `grim -o HDMI-A-1` must match pixel for pixel.

## 11. Phase 0 exit criteria

- `nix build`, `nix develop` and direnv work.
- `nix build` does not copy `target/` (verified by store path size).
- A minimal layer-shell surface opens and closes on niri.
- The pinned `niri-ipc` version talks to niri 26.04's socket.
- wl-clipboard-rs fork behaviour and its interaction with the lock are
  confirmed.
- `valw doctor` produces its first report.

## 12. Definition of done (every phase)

`cargo clippy -- -D warnings`, `cargo fmt --check`, `cargo nextest run` and
`nix build` pass, and the manual checks in 10.3 that apply to the phase are
done on niri.
