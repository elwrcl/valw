# valw Phase 5a: the built-in editor, basics (design)

Date: 2026-10-02
Status: approved in brainstorming, pending written-spec review
Builds on: Phases 0–4 (on `main`); vision section 4.5

## 1. Goal

Replace Satty with valw's own editor for everyday markup: click a preview
thumbnail, draw arrows, boxes and highlights, save or copy, done. Phase 5b
adds text, blur/pixelate, numbered markers and crop on top of this.

Success: the user can mark up a screenshot as fast as in macOS Markup, and
what is saved is pixel-for-pixel what the canvas shows (at the image's own
resolution).

### Out of scope (5b or later)

- Text, blur/pixelate, numbered markers, crop (5b).
- Selecting, moving or editing a drawn shape; filled shapes; zoom inside
  the editor; a custom colour picker.

## 2. Decisions

| Question | Decision | Why |
|---|---|---|
| Split | 5a basics now, 5b afterwards, each with its own spec and plan | The editor is the largest phase; 5a is already usable on its own |
| Toolkit | `eframe`/`egui` with the `glow` backend (OpenGL ES through EGL), no Vulkan, no wgpu | Ready-made buttons, shortcuts and (for 5b) text input; a normal Wayland window |
| Process | `valw edit <file>`, its own process | Same as Satty today; the preview host stays small |
| Look | macOS Markup: one slim toolbar on top, the image large below | User choice |
| Ctrl+S | Overwrite the file; Ctrl+Shift+S saves a copy next to it | Like macOS; the preview already points at that file |
| Rendering | Shapes become shared geometry (polylines, polygons) in image pixels; egui draws it on screen, `tiny-skia` draws it for export | One geometry, two painters: the export matches the screen |

## 3. Behaviour

### 3.1 Window

`valw edit <file>` opens a window titled `<file name> — valw`, app id
`valw-editor` (for niri window rules). The image is fitted into the area
below the toolbar, centred, never upscaled beyond 100 %. A file that can't
be read is an error on stderr and exit code 1, before any window appears.

### 3.2 Toolbar (left to right)

| Group | Items |
|---|---|
| Tools | Arrow, Rectangle, Ellipse, Line, Pen, Highlighter |
| Colour | red `#ff3b30` (default), orange `#ff9500`, yellow `#ffcc00`, green `#34c759`, blue `#007aff`, black, white |
| Width | S 2 px, M 4 px (default), L 8 px (image pixels) |
| History | Undo, Redo (disabled when there is nothing to do) |
| Output | Copy, Save |

The active tool, colour and width are highlighted. Default tool: Arrow.

### 3.3 Drawing

- Press, drag, release on the image creates one shape from the press point
  to the release point (Pen and Highlighter: every point on the way).
  A press outside the image does nothing; a drag that leaves the image is
  clamped to its edges.
- A shape whose drag is shorter than 2 image px (a click) is discarded,
  except for Pen, which keeps a dot.
- Shift while dragging: Line and Arrow snap to 45° steps; Rectangle and
  Ellipse become a square and a circle.
- Arrow: a line with a filled triangular head at the release point; the
  head grows with the width (length `max(12, 4 × width)`, 30° half-angle).
- Rectangle and Ellipse: outlines only.
- Highlighter: the chosen colour at 40 % opacity, width × 4, round caps;
  drawn over everything below it.
- Width is in image pixels, so a 4 px arrow is 4 px in the saved file.

### 3.4 Keys

| Key | Effect |
|---|---|
| A, R, O, L, P, H | Arrow, Rectangle, Ellipse (O), Line, Pen, Highlighter |
| Ctrl+Z | Undo |
| Ctrl+Shift+Z, Ctrl+Y | Redo |
| Ctrl+C | Copy the edited image to the clipboard |
| Ctrl+S | Save over the file |
| Ctrl+Shift+S | Save a copy: `<stem> edited.png` next to it (`unique_path` if taken) |
| Esc | Close; with unsaved changes the first Esc shows a bar "Unsaved changes — Esc to discard, Ctrl+S to save", the second closes |

Closing the window (compositor close) with unsaved changes behaves like
the first Esc: the bar appears and the window stays.

### 3.5 Undo

The document is the base image plus a list of shapes. Every finished shape
pushes onto the list and clears the redo stack; undo moves the last shape
to the redo stack, redo moves it back. Saving does not clear history.
"Unsaved" means the shape list differs from what was last saved.

### 3.6 Save and copy

Export composites the shapes onto the base image at its full resolution
with `tiny-skia` (anti-aliased), then encodes PNG with the existing
`output::encode_png`.

- Save: `output::write_atomic` over the file; the window stays open.
- Save a copy: as above to the `edited` path; the window stays open.
- Copy: the existing clipboard path (`output::copy_to_clipboard`, which
  forks a server); the window stays open.
- Each prints a short status in the toolbar ("Saved", "Copied") for 2 s;
  a failure shows its message there instead and is logged.

### 3.7 Opening from the preview

The preview host reads `[editor] backend` when a thumbnail is clicked:
`"builtin"` (default) runs `valw edit <file>` (the running executable),
`"satty"` keeps today's Satty command. Unknown values are a config error.

## 4. Structure

| File | Role |
|---|---|
| `src/editor/mod.rs` | `run(path)`: load, start eframe, the `App` (toolbar, canvas, keys, status bar) |
| `src/editor/doc.rs` | `Doc` (base image, shapes, redo stack, saved marker): add / undo / redo / dirty |
| `src/editor/shape.rs` | `Tool`, `Shape`, `Style`; constraints (Shift), shape → `Geometry` (polylines and polygons in image px) |
| `src/editor/export.rs` | `Geometry` → pixels with `tiny-skia`; `edited_path` |
| `src/editor/view.rs` | Fit the image into the canvas: image ↔ screen transforms |
| `src/config.rs` | `[editor] backend = "builtin" \| "satty"` |
| `src/host.rs` | `open_editor` picks the backend |
| `src/main.rs` | `Command::Edit { file }` |
| `nix/package.ulu.nix` | `LD_LIBRARY_PATH` gains `wayland` and `libxkbcommon` (winit loads them at run time) |

Dependencies: `eframe` (default features off; `glow`, `wayland`,
`default_fonts`), `tiny-skia`.

## 5. Testing

Unit tests (`cargo nextest`):
- `shape`: Shift snapping (angles, square, circle); the arrow head's size
  and direction; highlighter width and alpha; a click is discarded but a
  Pen dot is kept.
- `doc`: add / undo / redo order, redo cleared by a new shape, dirty after
  edits and clean after save, undo past a save makes it dirty.
- `export`: a red 4 px line on a white image has red pixels on the line
  and untouched pixels away from it; a highlighter over a known colour
  blends at 40 %; the output keeps the image size.
- `view`: fitting (never above 100 %, centred) and the round trip
  image → screen → image.
- `edited_path`: `a.png` → `a edited.png`, unique when taken.
- Config: `[editor] backend` default and the unknown-value error; the
  preview host's backend choice.

Manual checks go to the Phase 5a section of `docs/test-checklist.md`.

## 6. Definition of done

`cargo clippy -- -D warnings`, `cargo fmt --check`, `cargo nextest run`
and `nix flake check` pass; the checklist section is written.
