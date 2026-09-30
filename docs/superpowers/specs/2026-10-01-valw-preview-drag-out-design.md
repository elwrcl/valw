# valw: drag the preview out (design)

Date: 2026-10-01
Status: approved in brainstorming, pending written-spec review
Builds on: `2026-09-30-valw-phase-2-preview-design.md` (Phase 2, on `main`)

## 1. Context and goal

After a day of real use, the one thing missing from the preview was being
able to grab a thumbnail and drop the screenshot into another app (a chat,
a browser upload field, a file manager, an image editor) without looking
for the file. The vision document lists this as "Dışarı sürükle" in 4.4.

Success: dragging a thumbnail out works with the user's everyday apps, and
click → Satty and swipe → dismiss keep working exactly as before.

### Out of scope

- A config switch for drag-and-drop (it is always on).
- Anything else from the post-test-day list (hover pause, notifications,
  right-click menu, the Phase 2 deferred minors).

## 2. Decisions

| Question | Decision | Why |
|---|---|---|
| Swipe vs drag-out | By direction: mostly rightward = swipe, anything else = drag-out | Thumbnails sit in the bottom-right corner, so drop targets are always left/up; nothing existing changes |
| What is carried | `text/uri-list` (the file) and `image/png` (the bytes) | Each app takes what it understands |
| After a drag | Dropped → the thumbnail closes; cancelled → it comes back | Like macOS; a missed drop can be retried |
| What moves under the pointer | The thumbnail image as the drag icon; the original hides | You see what you carry |
| Action | Copy | The saved file stays where it is |

## 3. Gesture

A press on a thumbnail is classified once, as soon as the pointer has moved
at least `CLICK_SLOP` (4 logical px) from the press point, and never
changes afterwards:

- **Swipe:** `dx > 0` and `|dy| ≤ dx` (rightward, at most 45° off
  horizontal). Behaviour unchanged from Phase 2.
- **Drag-out:** any other direction.
- **Click:** released before the slop is reached. Behaviour unchanged
  (opens Satty).

`stack::classify(dx, dy) -> Option<Gesture>` (`None` below the slop) is a
pure function; the thumbnail remembers the first `Some`.

## 4. Drag flow

1. On the move that classifies the press as a drag-out, the host creates a
   `wl_data_source` (SCTK `create_drag_and_drop_source`, mime types from
   section 5, action `Copy`) and calls `start_drag` with the thumbnail's
   surface as origin, a new icon surface, and the serial of the pointer
   press that started it.
2. The icon surface shows the thumbnail image at thumbnail size (buffer at
   the output's scale, viewporter to logical size, 1 px border as before).
   Its offset (`wl_surface.offset`, wl_compositor v5+) keeps the point the
   user grabbed under the pointer.
3. The original thumbnail hides itself (transparent buffer, empty input
   region: the Phase 2 mechanism) for the duration of the drag. Only this
   thumbnail; the rest of the stack is unaffected. "Hidden for a capture"
   and "hidden for a drag" are separate flags, and a thumbnail is visible
   only when neither is set: a capture that ends mid-drag must not bring
   back the thumbnail being dragged.
4. While dragging, the compositor owns the pointer. If the thumbnail's
   timer fires, it is only recorded (`expired`); the thumbnail doesn't
   close mid-drag.
5. `dnd_finished` (dropped and accepted): the thumbnail closes at once (it
   is already invisible), and the stack restacks.
6. `cancelled` (released on nothing, rejected, Esc): the icon goes away and
   the thumbnail reappears with the slide-in animation, or, if it expired
   during the drag, slides out right away.
7. The data source and icon surface are destroyed after either outcome.

The host binds `wl_data_device_manager` (SCTK `DataDeviceManagerState`)
and gets the seat's data device when the pointer capability appears. The
host never receives drops; SCTK's `DataDeviceHandler` and
`DataOfferHandler` get empty implementations.

A missing `wl_data_device_manager` (not the case on niri) disables
drag-out: the gesture falls back to snap-back, and a warning is logged
once.

## 5. Data

| Mime type | Written to the receiving app's pipe |
|---|---|
| `text/uri-list` | `file_uri(path)` followed by `\r\n` |
| `image/png` | the file's bytes, read from disk when requested |

`file_uri(path)`: `file://` plus the absolute path with every byte
percent-encoded except the RFC 3986 unreserved characters
(`A-Z a-z 0-9 - . _ ~`) and `/`. Spaces become `%20`, UTF-8 (e.g. Turkish
`ı`, `ş`) is encoded byte by byte, `%` and `#` are encoded.

Each `send_request` writes on its own short-lived thread, so a slow reader
or a large PNG can't block the host's event loop (and with it every
thumbnail). A failed read or write (file removed, reader gone) is logged;
the pipe is closed and the host keeps running.

## 6. Code layout

| File | Change |
|---|---|
| `src/stack.rs` | `enum Gesture { Swipe, DragOut }`, `classify(dx, dy)` |
| `src/dnd.rs` (new) | `file_uri`, `MIME_TYPES`, `write_offer(mime, path, fd)` (the thread body), the after-drag decision |
| `src/thumbnail.rs` | gesture state; drag icon surface; per-thumbnail hide/show around the drag |
| `src/host.rs` | data device, drag source, `DataSourceHandler` callbacks routed to the thumbnail |
| `src/wayland.rs` | `DataDeviceManagerState` in `State`, delegate impls |

## 7. Testing

### 7.1 Unit tests (`cargo nextest`)

- `stack::classify`: under 4 px → `None`; right and 30° off right →
  `Swipe`; exactly 45° → `Swipe`; left, up, down, and steep right →
  `DragOut`.
- `dnd::file_uri`: plain path; spaces; Turkish characters; `%` and `#`;
  `/` kept.
- After-drag decision: dropped → close; cancelled and not expired → come
  back; cancelled and expired → slide out.
- `dnd::write_offer` through a real pipe: `text/uri-list` produces the
  exact line; `image/png` produces the file bytes; a missing file is an
  error, not a panic.

### 7.2 Headless check

Unchanged: drag-and-drop needs synthetic pointer input. The existing
checks guard click/swipe/hide against regressions.

### 7.3 Manual checks on niri

- A drag can be started from the thumbnail (layer surface) at all
  (verified while prototyping, before the plan).
- Drop into Dolphin → file copied; into a browser upload field → uploads;
  into a Discord message → attaches.
- Swipe right still dismisses; click still opens Satty.
- Drag left/up and release on nothing → the thumbnail comes back.
- A thumbnail whose timer fires during a drag slides out after the cancel.
- Drag onto the other monitor works.

## 8. Definition of done

`cargo clippy -- -D warnings`, `cargo fmt --check`, `cargo nextest run`,
`nix flake check` pass, the 7.3 checks are done on niri, and copland picks
it up with `nix flake update valw`.
