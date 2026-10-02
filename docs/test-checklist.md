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
- `valw doctor` shows only the EGL vendor; a software (llvmpipe) fallback would look fine. Report GL_RENDERER.

## Editor (Phase 5a)

- [ ] Clicking a preview opens `valw edit` (title `<name> — valw`); `[editor] backend = "satty"` still opens Satty.
- [ ] Each tool draws (arrow head at the release point, outline rectangle/ellipse, line, pen dot on a click, translucent highlighter); Shift snaps lines to 45° and makes squares/circles.
- [ ] Colours and S/M/L change new shapes only; A/R/O/L/P/H switch tools.
- [ ] Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y and the toolbar buttons undo and redo.
- [ ] Ctrl+S overwrites the file (the preview's file shows the edits); Ctrl+Shift+S writes `… edited.png`; "Saved" appears for 2 s.
- [ ] The saved PNG matches the canvas (same positions and widths), also for a window shot with transparent corners.
- [ ] Ctrl+C then Esc: the edited image is still pasteable afterwards.
- [ ] Esc with unsaved changes shows the bar; Esc again discards; Ctrl+S in the bar saves and closes. Closing the window with Mod+Q behaves the same.
- [ ] A large (1920×1080) shot fits the window; a small one isn't blown up; the window can be resized.

### Known minors (deferred from the editor review)

- Esc during a drag commits the half-drawn shape.
- A Pen click always stores two equal points; a touchpad tap in one frame draws nothing; Pen points aren't de-duplicated.
- Any mouse button draws; it should be the primary one.
- "Copied" shows even if the clipboard child fails; finished clipboard children aren't reaped until the editor exits.
- After undoing back to the saved state, Ctrl+S still closes; the unsaved bar can come back without a new Esc.
- The close guard runs only while the window is shown (App::ui, not App::logic).
- Saving round-trips pixels through premultiplied alpha (tiny RGB changes on nearly transparent pixels).
- The dev shell's LD_LIBRARY_PATH applies to every tool; Cargo.lock has duplicate smithay-client-toolkit/calloop/glow versions.

## Editor tools (Phase 5b)

- [ ] T: click, type (Turkish: ğüşıöçİ), Shift+Enter for a new line, Enter finishes; a click elsewhere also finishes; Esc drops it; while typing, letters don't switch tools and Ctrl+Z/Esc don't undo or close.
- [ ] Text is bold in the chosen colour with a dark outline, readable on light and dark backgrounds; S/M/L sizes; the saved PNG matches the canvas.
- [ ] B: dragging pixelates the area (also earlier arrows inside it); S/M/L block sizes; edges of the image are fine.
- [ ] N: clicks place 1, 2, 3 …; undo then a new click reuses the freed number.
- [ ] C: drag a crop, adjust its edges and corners (the cursor changes), Enter applies, Esc cancels; the canvas then shows only the crop; drawing after a crop lands where you click; undo restores the full image; the saved PNG has the crop's size.
- [ ] The toolbar shows all 10 tools and still wraps in a narrow window.

### Known minors (deferred from the editor tools review)

- A crop thinner than 16 screen px grabs both opposite edges when pressed and collapses.
- Redo while a crop is being adjusted can apply a crop that isn't inside the current one.
- The canvas edge of a crop can show a faint fringe from outside it (export unaffected).
- Every keystroke re-renders the whole image; fine at 1080p, may lag on 4K.
- The pixelate drag preview is hard to see on white.
- The unsaved bar mentions Ctrl+S, which is ignored while typing.

## Toolbar (Phase 6)

- [ ] `valw toolbar` shows the bar bottom-centre on the focused output (both monitors), crisp; the last mode is highlighted; hovering highlights buttons.
- [ ] Screen, Window, Region and Zoom each start their mode at once; the bar is not in the shot.
- [ ] Options ▾ opens the menu; timer None/5/10, Show cursor and Show preview toggle (✓) and are remembered next time (`~/.local/state/valw/toolbar.toml`).
- [ ] With a 5 s timer the pill counts 5 → 1 bottom-centre, the desktop stays usable meanwhile, and the pill is not in the shot; clicking the pill cancels (exit 3).
- [ ] Esc or a click beside the bar closes it without capturing (exit 3).
- [ ] A broken `toolbar.toml` doesn't stop the bar (warning in the log, defaults used).
- [ ] Bind it in niri (Mod+Shift+5 is taken in copland; pick a key).

### Known minors (deferred from the toolbar review)

- A Wayland error during the countdown counts as a tick: the timer is skipped instead of failing.
- Each hover change allocates a full-output buffer; the pool can grow large on 4K.
- Hover isn't set when the pointer enters and isn't cleared when it leaves.
- On two monitors, a click on the other one doesn't close the bar (Esc does).
- With the menu open, a click elsewhere closes the whole bar, and option changes made before a cancel aren't saved.
- The toolbar doesn't take the capture lock: two bars can stack, and a capture started during the countdown makes the toolbar's capture fail.
- The bar can cover a bottom panel, while the countdown pill sits above it.

## Polish

- [ ] Region: while dragging, a `W × H` label follows the pointer (physical pixels, same as the saved PNG), flipping near the edges.
- [ ] Shift locks the dimension you move first; Alt grows from the centre; Space held moves the selection, and after letting go resizing continues without a jump; Space before a drag still switches to window mode.
- [ ] `[capture] window_shadow = true`: window shots get a soft shadow following the rounded corners; preview, clipboard and file all have it.
- [ ] Shots in quick succession play sounds 1 → 5 getting faster and brighter, then 6, 7, 6, 7 rising endlessly; after a 5 s pause it starts at 1 again. `valw __combo-demo` plays the whole sequence for tuning.
- [ ] `[sound] enabled = false`, `volume`, `combo_reset_secs` take effect; the toolbar's Play sound option overrides the config; no `pw-play` → no sound, capture still works (warning in the log).

### Known minors (deferred from the polish review)

- Releasing Shift mid-drag only updates the rectangle on the next pointer motion.
- The Shift axis is decided by the very first motion; a few pixels of threshold would feel steadier.
- `__combo-demo` and a capture regenerating the same WAV at once can race on the temp file (one silent shot).
- Without `XDG_RUNTIME_DIR` the sound cache and combo state in `/tmp` are shared between users.
- `pipewire` on the wrapper's PATH adds its whole closure to the package.
- Tuning by ear: shots 6–7 are about 9–10 dB louder than 1–5; adjust with `__combo-demo`.
