# valw: a region across monitors (design)

Date: 2026-10-04
Status: approved in brainstorming (transparent gaps), pending written-spec review
Builds on: region mode (`src/region.rs`, `src/selection.rs`), capture (`src/main.rs`, `src/frame.rs`)

## 1. Goal

`valw region` today keeps a selection on the output where the drag
started and clips it there. Let a drag carry on onto other outputs: each
output shows its part of the selection, and the shot is one image of the
whole rectangle, stitched from every output it covers.

The user's layout: LVDS-1 1366×768 at (0, 0), HDMI-A-1 1920×1080 at
(−277, −1080), both scale 1, so a box dragged from the laptop up onto the
monitor is the main case.

## 2. Behaviour

- The selection lives in global logical coordinates (it already does);
  Shift, Alt and Space keep working across outputs.
- While dragging, every output whose area the rectangle touches shows its
  part of it (bright inside, dimmed outside); the others stay fully dimmed.
  Wayland keeps a drag's pointer events on the surface where it started,
  with positions beyond its edges, so the global position is still
  `start output origin + event position`.
- The `W × H` label shows the size of the whole shot, next to the pointer,
  on the output the pointer is over (found from the global position; when
  it is over no output, on the nearest one).
- On release:
  - inside one output: exactly as today (`Choice::Region(output, rect)`,
    opaque, the same pixels);
  - across outputs: one image the size of the whole rectangle in physical
    pixels at the largest scale among the outputs it covers; each output's
    part is cropped from its frozen frame and placed at its offset
    (resampled when that output's scale is smaller); areas that no output
    covers are transparent (alpha 0). Everything else (sound, clipboard,
    file, preview) as for any shot; the preview appears on the output the
    pointer was over at release.
- A drag smaller than the click threshold (`MIN_PIXELS`) in the whole
  rectangle is still a click.

## 3. Structure

| File | Role |
|---|---|
| `src/selection.rs` | `span(a, b, outputs) -> Option<Span>`: the rectangle's parts per output (output index, its `PixelRect`, where it goes and at what size in the stitched image) and the image size; `locate(point, outputs)`: the output a global point is over (or the nearest) |
| `src/stitch.rs` (new) | `stitch(frames, span) -> RgbaImage`: transparent canvas, each part's crop pasted (resized when needed) |
| `src/region.rs` | every surface draws its part (`selection_on` no longer anchor-only), redraw only surfaces whose part changed; label on the pointer's output with the whole size; release returns `Choice::Region` for one output or `Choice::Span(Span, preview_output)` |
| `src/main.rs` | `Choice::Span` → `stitch`, source = the preview output |

## 4. Testing

Unit tests (`cargo nextest`):
- `span`: a rectangle inside one output → one part equal to today's `clip`;
  the user's layout, a box from the laptop up onto the monitor → two
  parts, their places, the image size; a box partly over no output → the
  image is the whole box; mixed scales (1 and 2) → the image at scale 2,
  the scale-1 part placed and sized ×2; nothing under `MIN_PIXELS` → None.
- `locate`: a point on each output, in the gap (nearest output).
- `stitch`: pixels land at the right offsets, gaps are alpha 0, covered
  pixels alpha 255, a scaled part is resized to its target size.
- region: the surface where the drag did not start draws its part; the
  release decision (one output → `Region`, two → `Span`) as a pure function
  of the corners and the outputs.

Manual checks (a section in `docs/test-checklist.md`): drag from the laptop
up onto the monitor and back, Shift/Alt/Space across the edge, the label on
the right output with the whole size, the stitched PNG (gaps transparent,
seams exact), preview on the release output, single-output shots unchanged.

## 5. Definition of done

`cargo clippy -- -D warnings`, `cargo fmt --check`, `cargo nextest run` and
`nix flake check` pass; checklist updated; copland lock updated after the
user's approval.
