# valw Phase 3: window mode (design)

Date: 2026-10-02
Status: approved in brainstorming, pending written-spec review
Builds on: Phase 0+1 (region/screen), Phase 2 (preview), drag-out (all on `main`)

## 1. Goal

Capture a single window the macOS way: `valw window` lets the user click a
window and saves exactly that window; in region mode, Space (before
dragging) switches to window mode, like the camera icon on macOS.

Success: any window (tiled or floating, partly covered or not) can be
captured with a click, and the shot then goes through the same save,
clipboard, preview and drag-out flow as every other capture.

### Out of scope (later polish phase)

- The size indicator (`640 × 480`) next to the cursor.
- Shift / Alt / Space modifiers while dragging a region.
- The window shadow option.
- A blue highlight on the window under the pointer (see 2).

## 2. Decisions

| Question | Decision | Why |
|---|---|---|
| Who picks the window | niri (`Request::PickWindow`) | niri 26.04's IPC reports positions only for floating windows (`tile_pos_in_workspace_view` is `None` for tiled ones), so valw can't hit-test or highlight tiled windows itself |
| Who renders it | niri (`Action::ScreenshotWindow` with an explicit `path`) | Exact window pixels even when covered, rounded corners and transparency kept, correct scale |
| How valw gets the pixels | niri writes a temporary PNG; valw waits for the `ScreenshotCaptured` event, decodes the file and deletes it | One path for every target (`--output`, `-`, `--clipboard-only`); the rest of the pipeline stays unchanged |
| Highlight | None; niri's pick cursor is the feedback | Not possible for tiled windows (see above) |

## 3. Flow of `valw window`

1. As today: load the config, take the capture lock, wait `--delay`, hide
   the thumbnails (`HideGuard`).
2. `niri::pick_window()` sends `Request::PickWindow`; niri changes the
   cursor and replies after the click. `None` (Esc) → `Cancelled`.
3. Open a second niri socket with `Request::EventStream` **before**
   asking for the screenshot, so the event can't be missed.
4. Send `Action::ScreenshotWindow { id: Some(id), write_to_disk: true,
   show_pointer: cursor, path: Some(tmp) }`, where `tmp` is
   `$XDG_RUNTIME_DIR/valw-window-<pid>.png` (absolute, as niri requires).
5. Read events until `Event::ScreenshotCaptured { path: Some(p) }` with
   `p == tmp`. A reader thread sends events over a channel; the main side
   waits at most 5 s, then fails with "niri did not deliver the window
   screenshot". `ScreenshotCaptured { path: None }` or another path is
   ignored (niri also reports other screenshots on the same stream).
6. Decode `tmp` into an `RgbaImage` and delete it (also on every error
   after step 4, through a drop guard).
7. The preview's output: the output of the window's workspace (from
   `Request::Workspaces`); if that is unknown, niri's focused output, as
   in screen mode.
8. Continue with the existing `save` → preview → clipboard steps, exactly
   as for a region shot.

Side effects owned by niri, accepted: niri sets the clipboard itself and
shows its own "Screenshot captured" notification (niri's `dbus` build).
valw then sets the clipboard again with the same image (when configured),
so the end state is unchanged. The notification's image may be gone,
since valw deletes the temporary file.

## 4. Space in region mode

- The overlay's keyboard handler: Space while no drag is in progress
  ends the overlay with the outcome "switch to window mode". Space during
  a drag does nothing (reserved for "move the selection" in the polish
  phase).
- `region::select` returns `enum Choice { Region(usize, PixelRect), Window }`
  instead of `Picked`.
- `main`: `Window` drops the frozen frames and the Wayland connection and
  runs steps 2–8 of section 3 in the same process (lock, delay and hidden
  thumbnails carry over; the delay is not applied twice).

## 5. Code layout

| File | Change |
|---|---|
| `src/niri.rs` | `pick_window()`, `screenshot_window(id, path, cursor)`, `wait_for_capture(...)`, `output_of(window, workspaces)`, `is_our_capture(event, path)` |
| `src/window.rs` (new) | steps 2–7: temporary path + drop guard, decode, returns `(RgbaImage, output name)` |
| `src/main.rs` | `Command::Window`, `Mode::Window`, `Choice::Window` from region |
| `src/region.rs` | `Choice`; `Overlay::switch_to_window()` (no-op while dragging) |
| `src/wayland.rs` | Space in `press_key` calls `switch_to_window` |
| `src/doctor.rs` | nothing new (the niri version check already covers the IPC) |

## 6. Testing

Unit tests (`cargo nextest`):
- `niri::is_our_capture`: our path → true; `None`, another path → false.
- `niri::output_of`: window on a workspace with an output → that name;
  no workspace, unknown workspace, workspace without output → `None`.
- `region`: Space before a press → `Window`; Space while dragging →
  nothing.
- `window`: the temporary file is removed when the guard drops.

Manual checks are appended to `docs/test-checklist.md` (tiled, floating,
covered, HiDPI/other monitor, Esc during the pick, `--cursor`, Space from
region mode, `--clipboard-only`, `-o -`).

## 7. Definition of done

`cargo clippy -- -D warnings`, `cargo fmt --check`, `cargo nextest run`
and `nix flake check` pass; the checklist section is written.
