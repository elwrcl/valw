# valw: Phase 2 design (floating preview)

Date: 2026-09-30
Status: approved in brainstorming, pending written-spec review
Builds on: `2026-09-29-valw-phase-0-1-design.md` (Phase 0+1, shipped as `main`)

## 1. Context and goal

After a capture, a small thumbnail slides in at the bottom-right corner of
the monitor, like macOS's floating screenshot thumbnail. Clicking it opens
the screenshot in Satty; swiping it right dismisses it; otherwise it slides
away after a few seconds.

Phase 2 is deliberately minimal. Once it ships, valw is wired into copland
and the user runs it as their only screenshot tool for at least a day. The
bugs and gaps found then decide what comes next.

### In scope

- Floating thumbnail per capture, stacking up to 5.
- Click → Satty; swipe right → dismiss; timeout → dismiss.
- Thumbnails never appear in screenshots and never take keyboard focus.
- `[preview]` config (`enabled`, `timeout_secs`) and `--no-preview`.
- Satty on valw's PATH through the Nix package.
- Wiring valw into copland for the test day (section 9).

### Out of scope (candidates after the test day)

- Hover pauses the timer.
- Notifications (error notifications, preview-off fallback).
- Right-click menu, drag-and-drop out of the thumbnail.
- Rounded corners, shadow, configurable corner or size.
- Configurable editor command; valw's own editor (Phase 5).
- home-manager / NixOS modules (end of the project).

## 2. Decisions

| Question | Decision | Why |
|---|---|---|
| When is the file written? | At capture time, as in Phase 1. The preview is only a shortcut. | Nothing is lost if the preview crashes or the session ends. |
| Next capture while a preview is visible | Previews **stack** (max 5, oldest evicted). | User choice. |
| Who owns the stack? | One on-demand **preview host** process. | A single owner avoids position races between processes. |
| Editing | Satty, hardcoded: `satty --filename <path> --output-filename <path>`. | Satty 0.22.0 has both flags; saving overwrites the screenshot. |

## 3. Preview host

### 3.1 Startup

After a capture has saved its file and set up the clipboard, and if the
preview is enabled (section 7):

1. Connect to `$XDG_RUNTIME_DIR/valw.sock`.
2. If that fails, start a host: `std::process::Command` on
   `/proc/self/exe` with the hidden subcommand `__preview-host`, `setsid` in
   `pre_exec`, stdio to `/dev/null`. A fresh exec inherits no Wayland
   connection, lock or threads from the capture process.
3. Retry connecting every 20 ms for up to 1 s. If it still fails, log a
   warning; the capture itself has already succeeded.

A new host takes `$XDG_RUNTIME_DIR/valw-preview.lock` (the existing `Lock`
type) before touching the socket. If the lock is busy, another host is
starting or running, and this one exits 0 without output. Holding the lock
makes it safe to delete a stale socket file (left by a crash) before
binding.

### 3.2 Protocol

Unix stream socket, one JSON object per line in each direction
(`serde_json`). Every request gets exactly one reply.

| Request | Host action | Reply |
|---|---|---|
| `{"cmd":"hide"}` | Mark this connection as hiding; if this is the first, hide all thumbnails (4.6); Wayland round trip so the compositor has processed it | `{"ok":true}` |
| `{"cmd":"add","path":"…","output":"HDMI-A-1"}` | Load and downscale the PNG, add a thumbnail on that output | `{"ok":true}` |

Any malformed or unknown request gets `{"ok":false,"error":"…"}` and the
connection stays open.

A connection that sent `hide` stops hiding when it **closes**, for any
reason: normal end, Esc, an error, or the process being killed. There is no
explicit `show` message. Thumbnails are visible only while no connection is
hiding.

### 3.3 Lifetime

The host exits when its stack is empty and no client connection is open. It
removes the socket file on the way out. If it crashes, the compositor
removes its surfaces and the next capture starts a new host.

The host writes its own per-run log (`log::init`), like every valw process.

### 3.4 Event loop

calloop (SCTK's default `calloop` feature, `WaylandSource`), one loop for:

- the Wayland connection,
- the listening socket and every client connection (line-buffered),
- a timer per thumbnail (`timeout_secs`),
- animation, paced by frame callbacks.

After each dispatch the loop drops finished thumbnails, restacks the rest
and checks the exit rule. A new host that nobody connects to within 2 s
exits (the capture that started it died).

## 4. Thumbnail

### 4.1 Surface

One layer-shell surface per thumbnail:

- namespace `valw-preview`, layer `Overlay`, anchored bottom + right,
- `keyboard_interactivity = None`: the compositor never gives it keyboard
  focus,
- input region = the image rectangle only (transparent padding lets clicks
  through),
- buffer at the output's physical scale, `wp_viewporter` to logical size
  (same approach as the region overlay).

### 4.2 Size and look

- Long edge 220 logical px, aspect ratio kept. Images smaller than that are
  not upscaled.
- Downscaled once on `add` with `image::imageops::resize` (CatmullRom).
- 1 px light border (`#e0e0e0` at 80% opacity). No rounded corners or
  shadow in Phase 2.

### 4.3 Stack layout

- Newest thumbnail in the corner: the image sits 16 px from the right and
  bottom screen edges. (The surface itself touches the right edge, see 4.4;
  the 16 px is padding inside it, outside the input region.)
- Older thumbnails above it, 12 px apart.
- When one closes, those above it move down (no animation).
- A 6th thumbnail evicts the oldest (slides out as on timeout).
- Stacks are per output.

### 4.4 Animation

- Slide in from the right edge: 200 ms, ease-out cubic.
- Slide out to the right: 150 ms, ease-in cubic.
- Implemented by moving the image's x offset inside the surface (the surface
  reaches the screen edge; its right margin is 0). The layer position never
  changes during an animation.

### 4.5 Input

- **Click:** press and release with less than 4 px of movement. Spawns
  `satty --filename <path> --output-filename <path>` (detached, stdio to
  `/dev/null`) and slides the thumbnail out. A failed spawn is logged.
- **Swipe:** press, then horizontal drag moves the image with the pointer
  (right only; leftward drag is clamped to 0). On release, if the offset is
  at least 40% of the image width, it slides out; otherwise it animates
  back.
- **Timeout:** after `timeout_secs`, slide out. The timer keeps running
  during a drag; if it fires mid-drag, the thumbnail closes on release.
  It also keeps running while thumbnails are hidden for a capture: after a
  long region selection, older thumbnails may already be gone. Simplest
  rule; revisit after the test day if it annoys.

### 4.6 Hiding

A hidden thumbnail stays mapped: it commits a fully transparent buffer and
an empty input region, so it is invisible in screenshots and clicks pass
through. Showing it again restores the image and the input region.

Unmapping (attaching no buffer) is not used. Verified on niri 26.04 while
prototyping: after an unmap, a commit without a buffer gets no new
configure, so the thumbnail never came back and the host never exited.

Hide and show are drawn immediately, even while a frame callback is
pending mid-animation; otherwise a moving thumbnail could still be on
screen when the capture starts.

## 5. Capture integration

In `main.rs`'s capture flow:

1. After the lock and `--delay`, **immediately before screencopy**, open a
   `HideGuard`: connect to the host socket if it exists, send `hide`, wait
   up to 300 ms for the reply. No host: no-op. Timeout: log a warning and
   carry on.
2. Keep the guard (and its connection) alive through region selection, so
   thumbnails can't sit above the overlay or take its clicks.
3. After `deliver` has written the file: if a preview applies (5.1), send
   `add` over the guard's connection, or, without a running host, start one
   (3.1) and send `add` there.
4. Drop the guard: the connection closes and the stack shows again, the new
   thumbnail sliding in. This must happen **before** the clipboard fork: the
   forked clipboard server would otherwise inherit the connection and keep
   the thumbnails hidden until the clipboard changes.

`deliver` is split accordingly: `save` writes the files and returns the path
to preview; the clipboard copy happens after the guard is dropped.

Cost: one extra round trip before screencopy, only when a host is running.

### 5.1 When a preview is shown

- Only when a file was written: default save location or `-o PATH` to a
  regular file.
- Not for `-o -` (stdout), `--clipboard-only`, or `-o` onto a device or
  FIFO.
- `--all`: one preview, for the focused output's file (the same one that
  goes to the clipboard).
- Target output: `region` → the anchor output; `screen` → the captured
  (focused) output; `--all` → the focused output.

## 6. Code layout

Flat files, pure logic kept separate:

| File | Responsibility |
|---|---|
| `src/stack.rs` | Pure: stack positions, eviction, swipe/click decisions, easing, thumbnail size |
| `src/ipc.rs` | Message types (serde), `HideGuard`, `add`, starting the host |
| `src/host.rs` | Host entry point, lock, socket server, calloop loop, hide counter |
| `src/thumbnail.rs` | One thumbnail: surface, drawing, animation state, pointer handling |

`wayland.rs` gains what the host needs, following the existing `overlay`
pattern: a `preview: Option<Host>` field on `State`, routing of
pointer/frame/configure/closed events to it, `State::output_list()` (so the
host can look up outputs from inside the event loop), and an `Output::scale`
field (physical / logical width, from the current mode). `main.rs` gains the
hidden `__preview-host` subcommand and the capture integration.

The existing config test that used `[preview]` as its example of an unknown
key switches to `[zoom]` (unknown keys are reported in alphabetical order).

New dependency: `serde_json`. calloop comes with SCTK's default features.

## 7. Config and CLI

```toml
[preview]
enabled = true
timeout_secs = 5
```

- `timeout_secs` must be at least 1; 0 or negative is a config error with a
  hint.
- `--no-preview` (on `screen` and `region`) disables the preview for that
  run.
- `valw doctor`: no new checks in Phase 2 (Satty ships in the package).

## 8. Packaging

`nix/package.ulu.nix` wraps the binary with `makeWrapper`:
`--suffix PATH : ${lib.makeBinPath [ pkgs.satty ]}`. A user's own Satty on
PATH still wins because of `--suffix`.

## 9. copland wiring (test day)

In the separate `~/copland` repo, done with the user's go-ahead:

- Flake input: `valw.url = "path:/home/elars/Projects/valw";` with
  `inputs.nixpkgs.follows = "nixpkgs"`. After valw changes:
  `nix flake update valw`.
- Package: add `inputs.valw.packages.${system}.default` to the user's
  packages (a small `modules/home/home-valw.ulu.nix`, no module options).
- Binds in `modules/home/niri/desktop-niri-keybinds.ulu.nix`:
  - `Mod+Shift+S` → `valw region` (replaces Noctalia's `screenshot-region`)
  - `Print` → `valw screen`
  - `Mod+Shift+3/4` stay workspace moves.

## 10. Testing

### 10.1 Unit tests (`cargo nextest`)

- `stack`: positions for 1–5 thumbnails; eviction at 6; swipe 39% → back,
  40% → close, leftward → back; click vs drag (< 4 px); easing endpoints
  and monotonicity; thumbnail size (1920×1080 → 220×124, 1080×1920 →
  124×220, 100×50 unchanged).
- `ipc`: JSON round trip for each message; malformed and unknown requests
  → `ok:false`.
- host state: hide counter (hide, hide, close, close → visible only at 0);
  exit condition (empty stack and no connections).
- host startup: a second host fails the lock and exits quietly.
- config: `[preview]` defaults, `timeout_secs = 0` rejected.

### 10.2 Headless sway check (`nix flake check`)

The headless background is solid black (no swaybg in the sandbox), which
makes these deterministic. Config: `timeout_secs = 2`.

1. `valw screen` with default target → `valw.sock` appears within 1 s.
2. After 0.5 s, `grim` sees more than one colour in the bottom-right
   300×200 px (the thumbnail is really there, so step 3 isn't vacuous).
3. Immediately after, `valw screen --no-preview -o hidden.png` → that corner
   is a single colour (thumbnails never end up in screenshots).
4. The socket is gone within 4 s (host exits after the timeout).
5. `--no-preview` → no socket appears.

Click and swipe need real pointer input and stay manual.

### 10.3 Test-day checklist (manual, niri)

- Thumbnail on the correct monitor; sharp at scale 1.25.
- Typing is never interrupted while a thumbnail is visible.
- Click opens Satty; saving there overwrites the screenshot file.
- Swipe right closes; a short drag snaps back.
- Several captures in a row stack up to 5; none appear in later
  screenshots.
- During region selection thumbnails are hidden and not clickable.
- After the timeout, `pgrep -f __preview-host` finds nothing.

## 11. Definition of done

`cargo clippy -- -D warnings`, `cargo fmt --check`, `cargo nextest run`
and `nix flake check` pass; the section 10.3 checklist is done on niri;
valw is wired into copland and the test day can start.
