# valw Phase 5b: editor text, pixelate, numbers, crop (design)

Date: 2026-10-03
Status: approved in brainstorming, pending written-spec review
Builds on: Phase 5a (`valw edit`), `docs/superpowers/specs/2026-10-02-valw-phase-5a-editor-design.md`

## 1. Goal

Finish the editor's tool set from vision 4.5: text, hiding sensitive
parts, numbered markers and crop, with the same guarantee as 5a: the
saved PNG is what the canvas shows.

### Out of scope

- Editing, moving or selecting finished shapes (text included).
- Blur (pixelate only), font choice, text background boxes.

## 2. Decisions

| Question | Decision | Why |
|---|---|---|
| Text editing | Place and type; Enter or a click elsewhere finishes; no re-editing (undo instead) | No selection model yet |
| Text look | Bold, chosen colour, thin dark outline; S/M/L = 16/24/36 px | Readable on any background |
| Font | DejaVu Sans Bold, vendored in `assets/fonts/` with its licence | Turkish glyphs, permissive licence, works without system fonts |
| Text rendering | Glyph outlines from `ab_glyph`, filled and stroked by tiny-skia | One renderer for canvas and export |
| Hiding | Pixelate only | Can't be undone by deblurring |
| Crop | Drag, adjust edges, Enter applies, Esc cancels; an undoable history step | User choice |

## 3. Behaviour

New tools, after the 5a ones in the toolbar: **Text (T), Pixelate (B),
Number (N), Crop (C)**. Ctrl+C stays Copy.

### 3.1 Text

- A click on the image starts a text at that point (its top-left).
  Typing inserts characters, Backspace deletes, Shift+Enter starts a new
  line, Enter or a press anywhere else finishes it. Esc while typing
  drops the text (and does not trigger the close flow).
- An empty text is discarded. While typing, the single-letter tool keys
  and Ctrl+Z/Ctrl+Y act on the text box only as text (letters) or not at
  all (Ctrl shortcuts are ignored until the text finishes).
- Look: the glyphs filled in the chosen colour over an outline in near
  black (`#1c1c1e` at 80 % opacity) of width `size / 8`, so it reads on
  light and dark backgrounds. White text gets the same dark outline.
- Size from the width control: S 16, M 24, L 36 px (image pixels); line
  height 1.2 × size; kerning from the font.
- A caret (a thin line after the last glyph) blinks while typing; it is
  never exported.

### 3.2 Pixelate

- Drag a rectangle (Shift: square). On release the area turns into
  blocks of S 8, M 12, L 20 px, aligned to the rectangle's top-left; each
  block takes the average colour (alpha included) of what is under it at
  that point in the history, so earlier shapes are pixelated too. Edge
  blocks are partial.
- A click (under 2 px) is discarded.

### 3.3 Numbers

- A click places a filled circle in the chosen colour with a white bold
  number centred in it. Diameter S 24, M 32, L 44 px; the number's size
  is 0.55 × diameter.
- The number is 1 + the count of numbers already in the document, so
  undo makes the next number reuse the freed one.

### 3.4 Crop

- Drag a rectangle: the outside darkens (50 % black over the canvas), the
  inside stays clear, with a 1 px white border. Dragging within 8 screen
  px of an edge or corner moves that edge (corners move two); the cursor
  shows the resize direction.
- Enter applies it, Esc cancels it (Esc then does not start the close
  flow). Switching tools cancels it.
- An applied crop is one history step: undo restores the previous view.
  The canvas shows only the cropped area, fitted as in 5a. Shapes keep
  their original image coordinates; drawing is clamped to the crop.
- A later crop happens inside the current one.
- Export: everything is drawn on the full image, then the last crop's
  rectangle is cut out. "Unsaved" compares the history including crops.

## 4. Structure

| File | Change |
|---|---|
| `assets/fonts/DejaVuSans-Bold.ttf`, `assets/fonts/LICENSE-DejaVu` | The vendored font |
| `src/editor/text.rs` (new) | The embedded font, layout (lines, advances, kerning), glyph outlines → tiny-skia paths, text width/height |
| `src/editor/shape.rs` | `Tool::{Text, Pixelate, Number, Crop}`; `Shape` gains the text payload; geometry for numbers |
| `src/editor/doc.rs` | History entries become `Item::{Shape(Shape), Crop(PixelRect)}`; `crop()`; next number |
| `src/editor/export.rs` | Text, pixelation, number glyphs; final crop |
| `src/editor/view.rs` | Fitting the crop rectangle instead of the whole image |
| `src/editor/mod.rs` | Text input, crop handles and keys, toolbar entries |

Dependency: `ab_glyph`.

## 5. Testing

Unit tests (`cargo nextest`):
- `text`: line breaking on `\n`, width grows with characters, kerning
  applied, Turkish letters (`ğüşıöçİ`) have glyphs, an empty string has no
  size.
- `export`: text pixels inside the glyph box use the colour; the outline
  darkens the edge; pixelation makes each block uniform with the block's
  average; a pixelate after a red line pixelates the line; a number draws
  a filled circle with white in its centre; a crop sets the output size
  and offset.
- `doc`: numbers count only `Number` shapes and follow undo; crops are
  undoable and nest; dirty includes crops.
- `view`: fitting a crop rectangle; screen ↔ image round trip with a
  crop.
- `mod`: the crop edge hit-test (edges, corners, inside, outside).

Manual checks go to the Phase 5b section of `docs/test-checklist.md`.

## 6. Definition of done

`cargo clippy -- -D warnings`, `cargo fmt --check`, `cargo nextest run`
and `nix flake check` pass; the checklist section is written.
