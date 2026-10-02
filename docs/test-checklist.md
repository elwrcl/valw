# valw: end-of-project test checklist

Manual checks on niri that each phase leaves for one test pass once every
phase is done. Each phase appends its own section; nothing here blocks a
phase from landing. Automated checks (`cargo clippy`, `cargo nextest`,
`nix flake check`) still run per phase.

Before starting: update copland's lock (`nix flake update valw` in
`~/copland`), rebuild, and check that `valw doctor` is all `ok`.

## Preview drag-out (2026-10-02)

Already seen working while building it: a drop into Telegram delivered the
image, a cancel brought the thumbnail back, and a swipe still dismissed it.

- [ ] Drop into Dolphin → the file is copied (the default name has spaces, so this also checks `%20`).
- [ ] Drop into a browser upload field → it uploads.
- [ ] Drop into a Discord message → it attaches.
- [ ] Drop into an image editor that takes `image/png` → the pixels arrive.
- [ ] Release on nothing, or Esc mid-drag → the thumbnail comes back.
- [ ] With `timeout_secs = 3`, hold a drag past 3 s, then release on nothing → it slides out.
- [ ] While dragging, press Print → after the capture the other thumbnails come back but the dragged one doesn't; finishing the drag then behaves normally.
- [ ] Same, with 5 thumbnails on that output and the oldest one being dragged → the dragged one is never evicted.
- [ ] A fast flick to the left and an immediate release → the thumbnail comes back (or the drag runs normally); it never stays invisible.
- [ ] Drag onto the other monitor and drop there → it works.
- [ ] Swipe right still dismisses; a click still opens Satty (installed build).

### Known minors (deferred from the drag-out review)

- A drag-out press that never became a drag (no data device, or a second drag refused) ignores an expired timer; it should slide out on release.
- A receiver that opens its pipe O_NONBLOCK can get a truncated large PNG; clear O_NONBLOCK in the writer thread.
- A press during the slide-in gives a shifted grab point; use (x - offset, y).
- A swipe released back near its start opens Satty (Phase 2 behaviour); a locked swipe should never become a click.
- The (hidden, dragging) visibility/input decision has no unit test; extract it as a pure function.

## Window mode (Phase 3)

- [ ] `valw window`, click a tiled window → that window alone is saved, preview on its monitor.
- [ ] Same for a floating window, and for a window partly covered by another (the covered part is still there).
- [ ] A window on the other monitor (different scale, if any) → right size and sharpness; preview on that monitor.
- [ ] Esc during the pick → nothing saved, exit code 3, thumbnails come back.
- [ ] `--cursor` with the pointer over the window → the pointer is in the shot.
- [ ] `valw region`, press Space before dragging → the overlay closes and the window pick starts; Space while dragging does nothing.
- [ ] `--clipboard-only` and `-o -` → clipboard / stdout get the window; no stray `valw-window-*.png` left in `$XDG_RUNTIME_DIR`.
- [ ] niri's own "Screenshot captured" notification appears (expected, see the spec); the clipboard ends up with the window.

### Known minors (deferred from the window mode review)

- The 5 s wait includes niri's blocking "Screenshot captured" D-Bus notification; a hung notification daemon fails the shot and leaves the late temp file. Try decoding the temp file once on timeout.
- niri sets the clipboard itself even with `copy_to_clipboard = false` (no niri flag to avoid it); the spec's "end state unchanged" is wrong for that config. Document it.
- The Space path keeps the Wayland connection open through the pick (the spec says it is dropped). Harmless; align the spec or drop it earlier.
- Clicking something that isn't a window during the pick exits 3 silently, like Esc. Log "no window under the click".

## Zoom (Phase 4)

- [ ] `valw zoom` → the focused output freezes at 1×; Esc and `q` leave, nothing saved, exit 0, thumbnails come back.
- [ ] Wheel zooms towards the pointer (the point under it stays put), smoothly, up to 32×; pixels stay crisp.
- [ ] Touchpad scrolling zooms smoothly too.
- [ ] Left drag pans 1:1 and never shows anything outside the screenshot; the cursor turns into a grabbing hand.
- [ ] `f` toggles the flashlight (outside dimmed to 25 %), it follows the pointer; Ctrl+wheel resizes it.
- [ ] `0` animates back to 1×.
- [ ] `c` at some zoom → the saved image is exactly the visible area at native resolution; preview and clipboard as usual.
- [ ] On the 1366×768 laptop panel and the 1920×1080 monitor; stays smooth (no stutter) on the HD 4000.
- [ ] `[zoom] scroll_step` / `flashlight_radius` in the config take effect; `valw doctor` shows the EGL line.

### Known minors (deferred from the zoom review)

- `c` during a zoom animation captures where the view is heading, not what is on screen.
- The crop can include an edge column/row that was never shown; use the shader's exact texel bounds.
- Panning during a zoom animation jumps; pan both the shown and target view.
- A cancelled pointer grab (no Release, only Leave) leaves panning on; clear it on Leave.
- The first frame of each animation uses dt = 0 (one frame of extra latency).
- libEGL is unloaded when the renderer drops; keep it loaded for the whole process.
- A panic during zoom frees the Wayland display before EGL is torn down (segfault instead of the panic message).
- `zoom.scroll_step = nan` passes validation.
- With Caps Lock on, `c`, `f` and `q` are ignored.
- The libglvnd `LD_LIBRARY_PATH` reaches Satty and the clipboard child; the dev shell has no libglvnd (zoom/doctor warn under `cargo run`).
- `valw doctor` shows only the EGL vendor; a software (llvmpipe) fallback would look fine. Report GL_RENDERER.
