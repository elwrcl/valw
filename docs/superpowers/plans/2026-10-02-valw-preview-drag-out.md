# valw: drag the preview out — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Grab a preview thumbnail and drop the screenshot into any app. A mostly-rightward drag still dismisses it, and a click still opens Satty.

**Architecture:** A press on a thumbnail is classified once it has moved 4 px. Rightward within 45° is a swipe (unchanged); any other direction starts a Wayland drag-and-drop. The preview host creates a `wl_data_source` offering `text/uri-list` and `image/png`, starts the drag with the press serial, and shows the thumbnail image as the drag icon while the original hides. Data is written on a short-lived thread per request. `dnd_finished` closes the thumbnail; `cancelled` brings it back, or slides it out if its timer fired during the drag.

**Tech Stack:** Rust 2024; SCTK 0.21 `data_device_manager` (DataDeviceManagerState, DragSource, WritePipe); existing preview host (calloop); jj.

**Spec:** `docs/superpowers/specs/2026-10-01-valw-preview-drag-out-design.md`. The Phase 2 spec, `docs/superpowers/specs/2026-09-30-valw-phase-2-preview-design.md`, still applies to everything not changed here.

## Global Constraints

- Code, comments, error/log messages: English, plain and short, saying *why*.
- Version control is **jj**. Commit with `jj commit -m "<msg>"`; never use git commands. If a background `jj util snapshot` leaves the working copy stale or divergent: back up the changed files, run `jj workspace update-stale`, restore the newer files, and abandon an identical divergent twin.
- `cargo clippy` (never `cargo check`) and `cargo nextest run` (never `cargo test`), inside the devShell. When applying a diff with `patch`, use `patch -p1 --no-backup-if-mismatch`, so no `.orig` file ends up in a commit.
- Until Task 3, `src/main.rs` starts with `#![allow(dead_code)]`, because the new pure code lands before the host uses it. Task 3 removes it, and clippy must then be clean without it.
- Gesture: classify once at `CLICK_SLOP` (4 logical px). Swipe when `dx > 0 && |dy| <= dx`; everything else is drag-out.
- Offered MIME types, in order: `text/uri-list`, `image/png`. The action is `Copy`.
- `file_uri`: `file://` plus the path with every byte except `A-Z a-z 0-9 - . _ ~ /` percent-encoded; `text/uri-list` data is that URI plus `\r\n`.
- "Hidden for a capture" (`hidden`) and "hidden for a drag" (`dragging`) are separate flags; a thumbnail is visible only when neither is set.
- No new config option; no Nix changes.

## Review Focus

These are the inputs and failure modes most likely to bite in real use that no unit test fully covers. Each is pinned in the task that owns the code.

1. **A capture starts mid-drag** (Print pressed while dragging): the dragged thumbnail must not reappear when the capture ends, while the others do. Covered by the separate `dragging` flag (Task 3). Real check: Task 4, step 4.
2. **The timer fires during a drag:** after a cancel the thumbnail slides out; after a drop it closes. Test: `dnd::tests::after_drag_decisions` (Task 2). Real check: Task 4, step 4.
3. **A large PNG and a slow receiver:** the host stays responsive and doesn't exit before the data is fully written. Test: `dnd::tests::png_is_the_file_bytes` (200 KB, more than a pipe buffer, Task 2); the host waits for its `writers` counter (Task 3). Real check: Task 4, step 4.
4. **File names with spaces or Turkish characters:** receivers must get the right file. Tests: the `dnd::tests::*encoded*` tests (Task 2). Real check: Task 4, step 4 (the default file name has spaces).
5. **Swipe and click regressions:** a rightward swipe still dismisses, and a click still opens Satty. Tests: `stack::tests::rightward_within_45_degrees_is_a_swipe`, `small_moves_are_undecided` (Task 1), plus the existing `release_decisions`. Real check: Task 3, step 5.

---

## File Structure

| File | Change | Task |
|---|---|---|
| `src/stack.rs` | `enum Gesture { Swipe, DragOut }`, `classify(dx, dy)` | 1 |
| `src/dnd.rs` (new) | `MIME_TYPES`, `file_uri`, `write_offer`, `Outcome`, `AfterDrag`, `after_drag` | 2 |
| `src/thumbnail.rs` | press/gesture state, `DragStart`, `begin_drag`/`end_drag`, `draw_icon`, separate `dragging` flag | 3 |
| `src/host.rs` | `Drag`, free `pointer()` and `start_drag()`, `send`, `drag_ended`, `writers` | 3 |
| `src/wayland.rs` | `DataDeviceManagerState`/`DataDevice` in `State`; `DataSourceHandler`/`DataDeviceHandler`/`DataOfferHandler`; pointer routed to `host::pointer` | 3 |
| `src/main.rs` | allowance (Task 1), `mod dnd;` (Task 2), allowance removed (Task 3) | 1–3 |

---

### Task 1: Gesture classification

**Files:**
- Modify: `src/main.rs` (allowance), `src/stack.rs`

**Interfaces:**
- Consumes: `stack::CLICK_SLOP` (Phase 2).
- Produces: `pub enum Gesture { Swipe, DragOut }` and `pub fn classify(dx: f64, dy: f64) -> Option<Gesture>` in `stack`.

- [ ] **Step 1: Add the dead-code allowance**

At the very top of `src/main.rs`, followed by an empty line:

```rust
// Drag-out modules land before the host uses them; Task 3 removes this.
#![allow(dead_code)]
```

- [ ] **Step 2: Write the failing tests**

Apply to the test module of `src/stack.rs`:

```diff
--- a/src/stack.rs
+++ b/src/stack.rs
@@ -46,6 +46,27 @@
     }
 
     #[test]
+    fn small_moves_are_undecided() {
+        assert_eq!(classify(0.0, 0.0), None);
+        assert_eq!(classify(3.0, 2.0), None);
+    }
+
+    #[test]
+    fn rightward_within_45_degrees_is_a_swipe() {
+        assert_eq!(classify(10.0, 0.0), Some(Gesture::Swipe));
+        assert_eq!(classify(10.0, -5.0), Some(Gesture::Swipe), "about 27° up");
+        assert_eq!(classify(10.0, 10.0), Some(Gesture::Swipe), "exactly 45°");
+    }
+
+    #[test]
+    fn every_other_direction_drags_out() {
+        assert_eq!(classify(-10.0, 0.0), Some(Gesture::DragOut), "left");
+        assert_eq!(classify(0.0, -10.0), Some(Gesture::DragOut), "up");
+        assert_eq!(classify(0.0, 10.0), Some(Gesture::DragOut), "down");
+        assert_eq!(classify(5.0, -10.0), Some(Gesture::DragOut), "steep right");
+    }
+
+    #[test]
     fn drag_follows_right_only() {
         assert_eq!(drag_offset(30.0), 30.0);
         assert_eq!(drag_offset(-30.0), 0.0);
```

- [ ] **Step 3: Run the tests and watch them fail**

Run: `cargo nextest run stack::`
Expected: compilation fails, e.g. `` cannot find function `classify` in this scope ``.

- [ ] **Step 4: Implement**

Apply above the test module:

```diff
--- a/src/stack.rs
+++ b/src/stack.rs
@@ -66,6 +66,29 @@
         Release::Dismiss
     } else {
         Release::SnapBack
+    }
+}
+
+/// What a press turns into once the pointer has moved.
+#[derive(Debug, Clone, Copy, PartialEq, Eq)]
+pub enum Gesture {
+    /// Mostly rightward: the thumbnail follows and may be dismissed.
+    Swipe,
+    /// Any other direction: carry the screenshot out by drag-and-drop.
+    DragOut,
+}
+
+/// Classifies a press that has moved (`dx`, `dy`), once it has moved at
+/// least `CLICK_SLOP`. Rightward and at most 45° off horizontal is a swipe;
+/// the thumbnails sit in the bottom-right corner, so every drop target is
+/// left or up anyway.
+pub fn classify(dx: f64, dy: f64) -> Option<Gesture> {
+    if dx.hypot(dy) < CLICK_SLOP {
+        None
+    } else if dx > 0.0 && dy.abs() <= dx {
+        Some(Gesture::Swipe)
+    } else {
+        Some(Gesture::DragOut)
     }
 }
 
```

- [ ] **Step 5: Run the tests and watch them pass**

Run: `cargo nextest run stack::`
Expected: all 13 tests in `stack::tests` pass.

- [ ] **Step 6: Lint and run the whole suite**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo nextest run`
Expected: no warnings; every test passes.

- [ ] **Step 7: Commit**

```sh
jj commit -m "feat: classify thumbnail drags into swipe and drag-out"
```

---

### Task 2: Drag data and outcomes (pure)

**Files:**
- Create: `src/dnd.rs`
- Modify: `src/main.rs` (`mod dnd;`)

**Interfaces:**
- Consumes: nothing.
- Produces: `pub const MIME_TYPES: [&str; 2]`; `pub fn file_uri(path: &Path) -> String`; `pub fn write_offer(mime: &str, path: &Path, out: &mut impl Write) -> Result<()>`; `pub enum Outcome { Dropped, Cancelled }`; `pub enum AfterDrag { Close, ComeBack, SlideOut }`; `pub fn after_drag(outcome: Outcome, expired: bool) -> AfterDrag`.

- [ ] **Step 1: Write the failing tests**

Create `src/dnd.rs` with only the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn plain_path() {
        assert_eq!(file_uri(Path::new("/tmp/a.png")), "file:///tmp/a.png");
    }

    #[test]
    fn spaces_are_encoded() {
        assert_eq!(
            file_uri(Path::new("/home/u/Screenshot 2026-10-01 at 14.03.22.png")),
            "file:///home/u/Screenshot%202026-10-01%20at%2014.03.22.png"
        );
    }

    #[test]
    fn non_ascii_is_encoded_as_utf8() {
        assert_eq!(
            file_uri(Path::new("/tmp/ışık.png")),
            "file:///tmp/%C4%B1%C5%9F%C4%B1k.png"
        );
    }

    #[test]
    fn percent_and_hash_are_encoded() {
        assert_eq!(
            file_uri(Path::new("/tmp/50%#1.png")),
            "file:///tmp/50%25%231.png"
        );
    }

    /// Writes through a real pipe, as a receiving app would read it.
    fn through_pipe(mime: &str, path: &Path) -> Result<Vec<u8>> {
        let (reader, writer) = rustix::pipe::pipe()?;
        let mut writer = std::fs::File::from(writer);
        let path = path.to_path_buf();
        let mime = mime.to_string();
        let handle = std::thread::spawn(move || write_offer(&mime, &path, &mut writer));
        let mut out = Vec::new();
        std::fs::File::from(reader).read_to_end(&mut out)?;
        handle.join().unwrap()?;
        Ok(out)
    }

    #[test]
    fn uri_list_is_one_crlf_line() {
        let out = through_pipe("text/uri-list", Path::new("/tmp/a b.png")).unwrap();
        assert_eq!(out, b"file:///tmp/a%20b.png\r\n");
    }

    #[test]
    fn png_is_the_file_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        // Larger than a pipe buffer, so the writer really has to wait.
        let bytes: Vec<u8> = (0..200_000u32).map(|i| i as u8).collect();
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(through_pipe("image/png", &path).unwrap(), bytes);
    }

    #[test]
    fn missing_file_is_an_error_not_a_panic() {
        let err = through_pipe("image/png", Path::new("/nonexistent/x.png")).unwrap_err();
        assert!(
            format!("{err:#}").contains("could not read /nonexistent/x.png"),
            "{err:#}"
        );
    }

    #[test]
    fn unknown_mime_is_an_error() {
        assert!(through_pipe("text/html", Path::new("/tmp/a.png")).is_err());
    }

    #[test]
    fn after_drag_decisions() {
        assert_eq!(after_drag(Outcome::Dropped, false), AfterDrag::Close);
        assert_eq!(after_drag(Outcome::Dropped, true), AfterDrag::Close);
        assert_eq!(after_drag(Outcome::Cancelled, false), AfterDrag::ComeBack);
        assert_eq!(after_drag(Outcome::Cancelled, true), AfterDrag::SlideOut);
    }
}
```

Add `mod dnd;` to the `mod` list at the top of `src/main.rs` (keep it sorted).

- [ ] **Step 2: Run the tests and watch them fail**

Run: `cargo nextest run dnd::`
Expected: compilation fails, e.g. `` cannot find function `file_uri` in this scope ``.

- [ ] **Step 3: Implement**

Put this above the test module in `src/dnd.rs`:

```rust
//! Drag-and-drop out of a preview thumbnail: what is offered, how it is
//! written, and what the thumbnail does afterwards. No Wayland in here.

use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail};

/// What a drag offers; the receiving app picks one. File managers and chat
/// apps take the file, image editors take the pixels.
pub const MIME_TYPES: [&str; 2] = ["text/uri-list", "image/png"];

/// `file://` URI for an absolute path, with everything but RFC 3986
/// unreserved characters and `/` percent-encoded byte by byte. The default
/// file names have spaces; Turkish names have non-ASCII.
pub fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for &b in path.as_os_str().as_encoded_bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

/// Writes the screenshot at `path` in the requested `mime` type.
pub fn write_offer(mime: &str, path: &Path, out: &mut impl Write) -> Result<()> {
    match mime {
        "text/uri-list" => out.write_all(format!("{}\r\n", file_uri(path)).as_bytes())?,
        "image/png" => {
            let bytes = std::fs::read(path)
                .with_context(|| format!("could not read {}", path.display()))?;
            out.write_all(&bytes)?;
        }
        other => bail!("nobody offered {other}"),
    }
    out.flush()?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Dropped on an app that took it.
    Dropped,
    /// Released on nothing, rejected, or Esc.
    Cancelled,
}

/// What the thumbnail does once its drag is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterDrag {
    Close,
    ComeBack,
    SlideOut,
}

/// `expired`: the thumbnail's timer fired while it was being dragged.
pub fn after_drag(outcome: Outcome, expired: bool) -> AfterDrag {
    match (outcome, expired) {
        (Outcome::Dropped, _) => AfterDrag::Close,
        (Outcome::Cancelled, false) => AfterDrag::ComeBack,
        (Outcome::Cancelled, true) => AfterDrag::SlideOut,
    }
}
```

- [ ] **Step 4: Run the tests and watch them pass**

Run: `cargo nextest run dnd::`
Expected: all 9 tests in `dnd::tests` pass.

- [ ] **Step 5: Lint and run the whole suite**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo nextest run`
Expected: no warnings; every test passes.

- [ ] **Step 6: Commit**

```sh
jj commit -m "feat: drag-out data (uri-list, png) and outcomes"
```

---

### Task 3: Start the drag from a thumbnail

**Files:**
- Modify: `src/thumbnail.rs`, `src/host.rs`, `src/wayland.rs`, `src/main.rs` (remove the allowance)

**Interfaces:**
- Consumes: `stack::{classify, Gesture}` (Task 1); `dnd::*` (Task 2); Phase 2's `Thumbnail`, `Host`, `State`.
- Produces:
  - `thumbnail`: `DragStart { serial, grab }`; `Thumbnail::press(x, y, serial)`; `motion(x, y, qh) -> Option<DragStart>`; `begin_drag(qh)`, `end_drag(outcome, qh)`, `draw_icon(icon, viewport, grab)`, `surface()`.
  - `host`: free functions `pointer(state, events, qh)` and `start_drag(...)`; `Host::send(source, mime, fd)` and `Host::drag_ended(source, outcome, qh)`.
  - `State`: `data_devices: Option<DataDeviceManagerState>`, `data_device: Option<DataDevice>`, plus the three SCTK data handler impls.

How it fits together:
- `State::pointer_frame` hands thumbnail events to `host::pointer`. It's a free function because starting a drag needs the data device, compositor and viewporter as well as the host.
- When `Thumbnail::motion` returns a `DragStart`, `start_drag` creates the source and an icon surface, calls `start_drag` with the press serial, draws the icon, and calls `begin_drag`. That hides this thumbnail through the separate `dragging` flag.
- `DataSourceHandler::send_request` → `Host::send` (a writer thread per request, counted in `writers` so the host doesn't exit early).
- `dnd_finished` and `cancelled` → `Host::drag_ended` → `Thumbnail::end_drag`, which uses `dnd::after_drag`.
- Without a `wl_data_device_manager`, a drag-out does nothing, and a warning is logged once.

There are no new unit tests here: the decisions live in Tasks 1 and 2, and this task is protocol plumbing, checked on niri in Step 5.

- [ ] **Step 1: Thumbnail**

```diff
--- a/src/thumbnail.rs
+++ b/src/thumbnail.rs
@@ -16,12 +16,16 @@
         slot::{Buffer, SlotPool},
     },
 };
-use wayland_client::{QueueHandle, protocol::wl_shm};
+use wayland_client::{
+    Proxy, QueueHandle,
+    protocol::{wl_shm, wl_surface::WlSurface},
+};
 use wayland_protocols::wp::viewporter::client::{
     wp_viewport::WpViewport, wp_viewporter::WpViewporter,
 };
 
-use crate::stack::{self, Anim, EDGE_MARGIN, Release};
+use crate::dnd::{self, AfterDrag, Outcome};
+use crate::stack::{self, Anim, EDGE_MARGIN, Gesture, Release};
 use crate::wayland::{Output, State};
 
 pub const NAMESPACE: &str = "valw-preview";
@@ -93,6 +97,24 @@
     OpenEditor,
 }
 
+/// A press turned into a drag-out: the host starts the drag with these.
+#[derive(Debug, Clone, Copy, PartialEq)]
+pub struct DragStart {
+    /// Serial of the press; the compositor only accepts a drag for it.
+    pub serial: u32,
+    /// Where on the image the press was, so it stays under the pointer.
+    pub grab: (f64, f64),
+}
+
+/// A press in progress and what it turned into once the pointer moved.
+#[derive(Debug, Clone, Copy)]
+struct Press {
+    x: f64,
+    y: f64,
+    serial: u32,
+    gesture: Option<Gesture>,
+}
+
 pub struct Thumbnail {
     pub id: u64,
     pub output: String,
@@ -108,13 +130,17 @@
     /// How far right of its resting place the image is drawn, logical px.
     offset: f64,
     anim: Option<(Anim, Instant)>,
-    drag: Option<(f64, f64)>,
+    press: Option<Press>,
     margin: u32,
     /// Input region while visible (the image) and while hidden (nothing).
     input: Region,
     no_input: Region,
     /// Hidden for a capture: drawn fully transparent, clicks pass through.
     hidden: bool,
+    /// Being dragged out: the drag icon shows it, so this one hides too.
+    /// Separate from `hidden` so a capture ending mid-drag can't bring it back.
+    dragging: bool,
+    icon_buffer: Option<Buffer>,
     configured: bool,
     frame_pending: bool,
     started: bool,
@@ -178,11 +204,13 @@
             scale: output.scale,
             offset: travel(size.0),
             anim: None,
-            drag: None,
+            press: None,
             margin: EDGE_MARGIN,
             input,
             no_input,
             hidden: false,
+            dragging: false,
+            icon_buffer: None,
             configured: false,
             frame_pending: false,
             started: false,
@@ -191,8 +219,12 @@
         })
     }
 
-    pub fn is(&self, surface: &wayland_client::protocol::wl_surface::WlSurface) -> bool {
+    pub fn is(&self, surface: &WlSurface) -> bool {
         self.layer.wl_surface() == surface
+    }
+
+    pub fn surface(&self) -> &WlSurface {
+        self.layer.wl_surface()
     }
 
     pub fn is_layer(&self, layer: &LayerSurface) -> bool {
@@ -205,7 +237,23 @@
 
     /// The slide-out has finished (or is invisible anyway).
     pub fn is_finished(&self) -> bool {
-        self.closing && (self.hidden || anim_over(self.anim, Instant::now()))
+        self.closing && (!self.visible() || anim_over(self.anim, Instant::now()))
+    }
+
+    fn visible(&self) -> bool {
+        !self.hidden && !self.dragging
+    }
+
+    /// Clicks reach the image only while it is visible.
+    fn apply_input(&self) {
+        let region = if self.visible() {
+            &self.input
+        } else {
+            &self.no_input
+        };
+        self.layer
+            .wl_surface()
+            .set_input_region(Some(region.wl_region()));
     }
 
     /// Makes the thumbnail invisible and click-through. The surface stays
@@ -215,9 +263,7 @@
             return;
         }
         self.hidden = true;
-        self.layer
-            .wl_surface()
-            .set_input_region(Some(self.no_input.wl_region()));
+        self.apply_input();
         self.draw(qh, true);
     }
 
@@ -226,9 +272,7 @@
             return;
         }
         self.hidden = false;
-        self.layer
-            .wl_surface()
-            .set_input_region(Some(self.input.wl_region()));
+        self.apply_input();
         self.draw(qh, true);
     }
 
@@ -252,15 +296,16 @@
 
     pub fn frame_done(&mut self, qh: &QueueHandle<State>) {
         self.frame_pending = false;
-        if self.anim.is_some() || self.drag.is_some() {
+        if self.anim.is_some() || self.press.is_some() {
             self.draw(qh, false);
         }
     }
 
-    /// The timeout fired: slide out, unless the user is dragging it.
+    /// The timeout fired: slide out, unless the user is holding or
+    /// dragging it; then it is decided when they let go.
     pub fn expire(&mut self, qh: &QueueHandle<State>) {
         self.expired = true;
-        if self.drag.is_none() {
+        if self.press.is_none() && !self.dragging {
             self.slide_out(qh);
         }
     }
@@ -277,23 +322,46 @@
         self.draw(qh, false);
     }
 
-    pub fn press(&mut self, x: f64, y: f64) {
-        if !self.closing {
+    pub fn press(&mut self, x: f64, y: f64, serial: u32) {
+        if !self.closing && !self.dragging {
             self.anim = None;
-            self.drag = Some((x, y));
-        }
-    }
-
-    pub fn motion(&mut self, x: f64, qh: &QueueHandle<State>) {
-        if let Some((x0, _)) = self.drag {
-            self.offset = stack::drag_offset(x - x0);
+            self.press = Some(Press {
+                x,
+                y,
+                serial,
+                gesture: None,
+            });
+        }
+    }
+
+    /// Follows a swipe, or returns a `DragStart` the moment the press turns
+    /// into a drag-out.
+    pub fn motion(&mut self, x: f64, y: f64, qh: &QueueHandle<State>) -> Option<DragStart> {
+        let press = self.press.as_mut()?;
+        let (dx, dy) = (x - press.x, y - press.y);
+        if press.gesture.is_none() {
+            press.gesture = stack::classify(dx, dy);
+            if press.gesture == Some(Gesture::DragOut) {
+                return Some(DragStart {
+                    serial: press.serial,
+                    grab: (press.x, press.y),
+                });
+            }
+        }
+        if press.gesture == Some(Gesture::Swipe) {
+            self.offset = stack::drag_offset(dx);
             self.draw(qh, false);
         }
+        None
     }
 
     pub fn release(&mut self, x: f64, y: f64, qh: &QueueHandle<State>) -> Option<Action> {
-        let (x0, y0) = self.drag.take()?;
-        match stack::release(x - x0, y - y0, self.size.0 as f64) {
+        let press = self.press.take()?;
+        if press.gesture == Some(Gesture::DragOut) {
+            // The drag never started (no data device); nothing moved.
+            return None;
+        }
+        match stack::release(x - press.x, y - press.y, self.size.0 as f64) {
             Release::Click => {
                 self.slide_out(qh);
                 Some(Action::OpenEditor)
@@ -312,6 +380,63 @@
                 None
             }
         }
+    }
+
+    /// The drag started: the icon carries the image, this one hides.
+    pub fn begin_drag(&mut self, qh: &QueueHandle<State>) {
+        self.press = None;
+        self.dragging = true;
+        self.anim = None;
+        self.offset = 0.0;
+        self.apply_input();
+        self.draw(qh, true);
+    }
+
+    pub fn end_drag(&mut self, outcome: Outcome, qh: &QueueHandle<State>) {
+        self.dragging = false;
+        self.icon_buffer = None;
+        self.apply_input();
+        match dnd::after_drag(outcome, self.expired) {
+            AfterDrag::Close => {
+                // Already invisible; nothing to animate.
+                self.closing = true;
+                self.anim = None;
+            }
+            AfterDrag::ComeBack => {
+                self.anim = Some((Anim::slide_in(travel(self.size.0)), Instant::now()));
+                self.draw(qh, true);
+            }
+            AfterDrag::SlideOut => self.slide_out(qh),
+        }
+    }
+
+    /// Draws the drag icon: the image alone, sharp at the output's scale,
+    /// placed so the grabbed point stays under the pointer.
+    pub fn draw_icon(&mut self, icon: &WlSurface, viewport: &WpViewport, grab: (f64, f64)) {
+        let (width, height) = self.small.dimensions();
+        let Ok((buffer, canvas)) = self.pool.create_buffer(
+            width as i32,
+            height as i32,
+            width as i32 * 4,
+            wl_shm::Format::Argb8888,
+        ) else {
+            tracing::warn!("could not allocate a drag icon buffer");
+            return;
+        };
+        paint(canvas, width, &self.small, 0);
+        viewport.set_destination(self.size.0 as i32, self.size.1 as i32);
+        // wl_surface.offset needs version 5; older compositors put the
+        // icon's corner at the pointer instead.
+        if icon.version() >= 5 {
+            icon.offset(-(grab.0.round() as i32), -(grab.1.round() as i32));
+        }
+        icon.damage_buffer(0, 0, width as i32, height as i32);
+        if buffer.attach_to(icon).is_err() {
+            tracing::warn!("could not attach the drag icon");
+            return;
+        }
+        icon.commit();
+        self.icon_buffer = Some(buffer);
     }
 
     fn draw(&mut self, qh: &QueueHandle<State>, force: bool) {
@@ -328,6 +453,7 @@
         }
         let width = canvas_width(self.size.0, self.scale);
         let height = self.small.height();
+        let visible = self.visible();
         let Ok((buffer, canvas)) = self.pool.create_buffer(
             width as i32,
             height as i32,
@@ -337,7 +463,7 @@
             tracing::warn!("could not allocate a thumbnail buffer");
             return;
         };
-        if self.hidden {
+        if !visible {
             canvas.fill(0);
         } else {
             paint(
@@ -351,7 +477,7 @@
         surface.damage_buffer(0, 0, width as i32, height as i32);
         // Keep drawing while something moves; motion that arrives while a
         // frame is pending is picked up by the next frame callback.
-        if self.anim.is_some() || self.drag.is_some() {
+        if self.anim.is_some() || self.press.is_some() {
             surface.frame(qh, FrameCallbackData(surface.clone()));
             self.frame_pending = true;
         }
```

- [ ] **Step 2: Wayland state and handlers**

```diff
--- a/src/wayland.rs
+++ b/src/wayland.rs
@@ -4,6 +4,12 @@
 use rustix::event::{PollFd, PollFlags, Timespec, poll};
 use smithay_client_toolkit::{
     compositor::{CompositorHandler, CompositorState},
+    data_device_manager::{
+        DataDeviceManagerState, WritePipe,
+        data_device::{DataDevice, DataDeviceHandler},
+        data_offer::{DataOfferHandler, DragOffer},
+        data_source::DataSourceHandler,
+    },
     delegate_registry,
     output::{OutputHandler, OutputState},
     registry::{ProvidesRegistryState, RegistryState},
@@ -19,7 +25,10 @@
 use wayland_client::{
     Connection, EventQueue, QueueHandle,
     globals::{GlobalList, registry_queue_init},
-    protocol::{wl_buffer::WlBuffer, wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface},
+    protocol::{
+        wl_buffer::WlBuffer, wl_data_device::WlDataDevice, wl_data_device_manager::DndAction,
+        wl_data_source::WlDataSource, wl_keyboard, wl_output, wl_pointer, wl_seat, wl_surface,
+    },
 };
 use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::WpCursorShapeDeviceV1;
 use wayland_protocols::wp::viewporter::client::{
@@ -28,6 +37,7 @@
 use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1;
 
 use crate::capture::Pending;
+use crate::dnd::Outcome;
 use crate::error::HintExt;
 use crate::frame::OutputGeom;
 use crate::host::Host;
@@ -61,6 +71,9 @@
     pub viewporter: Option<WpViewporter>,
     pub cursor_shape: Option<CursorShapeManager>,
     pub cursor_device: Option<WpCursorShapeDeviceV1>,
+    /// For dragging a preview out; the host never receives drops.
+    pub data_devices: Option<DataDeviceManagerState>,
+    pub data_device: Option<DataDevice>,
     pub keyboard: Option<wl_keyboard::WlKeyboard>,
     pub pointer: Option<wl_pointer::WlPointer>,
     /// Screencopy requests in flight.
@@ -92,6 +105,8 @@
             viewporter: globals.bind(&qh, 1..=1, ()).ok(),
             cursor_shape: CursorShapeManager::bind(&globals, &qh).ok(),
             cursor_device: None,
+            data_devices: DataDeviceManagerState::bind(&globals, &qh).ok(),
+            data_device: None,
             keyboard: None,
             pointer: None,
             captures: Vec::new(),
@@ -370,6 +385,10 @@
                 .cursor_shape
                 .as_ref()
                 .map(|m| m.get_shape_device(&pointer, qh));
+            self.data_device = self
+                .data_devices
+                .as_ref()
+                .map(|m| m.get_data_device(qh, &seat));
             self.pointer = Some(pointer);
         }
     }
@@ -481,9 +500,91 @@
         if let Some(overlay) = &mut self.overlay {
             overlay.pointer(events, self.cursor_device.as_ref(), qh);
         }
+        if self.preview.is_some() {
+            crate::host::pointer(self, events, qh);
+        }
+    }
+}
+
+impl DataSourceHandler for State {
+    fn accept_mime(
+        &mut self,
+        _: &Connection,
+        _: &QueueHandle<Self>,
+        _: &WlDataSource,
+        _: Option<String>,
+    ) {
+    }
+
+    fn send_request(
+        &mut self,
+        _: &Connection,
+        _: &QueueHandle<Self>,
+        source: &WlDataSource,
+        mime: String,
+        fd: WritePipe,
+    ) {
+        if let Some(preview) = &self.preview {
+            preview.send(source, mime, fd);
+        }
+    }
+
+    fn cancelled(&mut self, _: &Connection, qh: &QueueHandle<Self>, source: &WlDataSource) {
         if let Some(preview) = &mut self.preview {
-            preview.pointer(events, self.cursor_device.as_ref(), qh);
-        }
+            preview.drag_ended(source, Outcome::Cancelled, qh);
+        }
+    }
+
+    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}
+
+    fn dnd_finished(&mut self, _: &Connection, qh: &QueueHandle<Self>, source: &WlDataSource) {
+        if let Some(preview) = &mut self.preview {
+            preview.drag_ended(source, Outcome::Dropped, qh);
+        }
+    }
+
+    fn action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: DndAction) {}
+}
+
+// valw only ever starts drags; incoming offers are ignored.
+impl DataDeviceHandler for State {
+    fn enter(
+        &mut self,
+        _: &Connection,
+        _: &QueueHandle<Self>,
+        _: &WlDataDevice,
+        _: f64,
+        _: f64,
+        _: &wl_surface::WlSurface,
+    ) {
+    }
+
+    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
+
+    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, _: f64, _: f64) {}
+
+    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
+
+    fn drop_performed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
+}
+
+impl DataOfferHandler for State {
+    fn source_actions(
+        &mut self,
+        _: &Connection,
+        _: &QueueHandle<Self>,
+        _: &mut DragOffer,
+        _: DndAction,
+    ) {
+    }
+
+    fn selected_action(
+        &mut self,
+        _: &Connection,
+        _: &QueueHandle<Self>,
+        _: &mut DragOffer,
+        _: DndAction,
+    ) {
     }
 }
 
```

- [ ] **Step 3: Host**

```diff
--- a/src/host.rs
+++ b/src/host.rs
@@ -5,9 +5,12 @@
 use std::io::{ErrorKind, Read, Write};
 use std::os::unix::net::{UnixListener, UnixStream};
 use std::path::Path;
+use std::sync::Arc;
+use std::sync::atomic::{AtomicUsize, Ordering};
 use std::time::{Duration, Instant};
 
 use anyhow::{Context, Result};
+use smithay_client_toolkit::data_device_manager::{WritePipe, data_source::DragSource};
 use smithay_client_toolkit::reexports::calloop::{
     EventLoop, Interest, LoopHandle, Mode, PostAction,
     generic::Generic,
@@ -16,15 +19,16 @@
 use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
 use smithay_client_toolkit::seat::pointer::{BTN_LEFT, PointerEvent, PointerEventKind};
 use smithay_client_toolkit::shell::wlr_layer::LayerSurface;
+use wayland_client::protocol::{wl_data_device_manager::DndAction, wl_data_source::WlDataSource};
 use wayland_client::{Connection, QueueHandle, protocol::wl_surface::WlSurface};
-use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::{
-    Shape, WpCursorShapeDeviceV1,
-};
-
+use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::Shape;
+use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;
+
+use crate::dnd::{self, Outcome};
 use crate::ipc::{self, Reply, Request};
 use crate::lock::Lock;
 use crate::stack;
-use crate::thumbnail::{Action, Parts, Thumbnail};
+use crate::thumbnail::{Action, DragStart, Parts, Thumbnail};
 use crate::wayland::{State, Wayland};
 
 /// A host nobody connects to within this time gives up.
@@ -89,6 +93,19 @@
     conn: Connection,
     handle: LoopHandle<'static, State>,
     qh: QueueHandle<State>,
+    drag: Option<Drag>,
+    /// Threads still writing dropped data; the host waits for them, or the
+    /// receiving app would get a cut-off file.
+    writers: Arc<AtomicUsize>,
+    warned_no_dnd: bool,
+}
+
+/// The drag-out in progress. Dropping the source cancels the drag.
+struct Drag {
+    thumb: u64,
+    source: DragSource,
+    icon: WlSurface,
+    icon_viewport: WpViewport,
 }
 
 /// Runs the host until its last thumbnail closes. A second host exits
@@ -128,6 +145,9 @@
         conn,
         handle: handle.clone(),
         qh,
+        drag: None,
+        writers: Arc::new(AtomicUsize::new(0)),
+        warned_no_dnd: false,
     });
 
     let mut next_client = 0u64;
@@ -152,7 +172,8 @@
         .run(Duration::from_millis(500), &mut state, |state| {
             let host = state.preview.as_mut().expect("host state");
             host.tidy();
-            if host.core.should_exit(host.thumbs.len(), Instant::now()) {
+            let busy = host.thumbs.len() + host.writers.load(Ordering::SeqCst);
+            if host.core.should_exit(busy, Instant::now()) {
                 signal.stop();
             }
         })
@@ -253,38 +274,40 @@
         self.thumbs.retain(|t| !t.is_layer(layer));
     }
 
-    pub fn pointer(
-        &mut self,
-        events: &[PointerEvent],
-        cursor: Option<&WpCursorShapeDeviceV1>,
-        qh: &QueueHandle<State>,
-    ) {
-        for event in events {
-            let Some(t) = self.thumb(&event.surface) else {
-                continue;
-            };
-            let (x, y) = event.position;
-            match event.kind {
-                PointerEventKind::Enter { serial } => {
-                    if let Some(cursor) = cursor {
-                        cursor.set_shape(serial, Shape::Pointer);
-                    }
-                }
-                PointerEventKind::Press {
-                    button: BTN_LEFT, ..
-                } => t.press(x, y),
-                PointerEventKind::Motion { .. } => t.motion(x, qh),
-                PointerEventKind::Release {
-                    button: BTN_LEFT, ..
-                } => {
-                    let action = t.release(x, y, qh);
-                    if action == Some(Action::OpenEditor) {
-                        open_editor(&t.path);
-                    }
-                }
-                _ => {}
-            }
-        }
+    /// Writes the dragged screenshot for the receiving app, on its own
+    /// thread: a large PNG fills the pipe, and blocking here would freeze
+    /// every thumbnail.
+    pub fn send(&self, source: &WlDataSource, mime: String, fd: WritePipe) {
+        let Some(drag) = self.drag.as_ref().filter(|d| d.source.inner() == source) else {
+            return;
+        };
+        let Some(thumb) = self.thumbs.iter().find(|t| t.id == drag.thumb) else {
+            return;
+        };
+        let path = thumb.path.clone();
+        let writers = Arc::clone(&self.writers);
+        writers.fetch_add(1, Ordering::SeqCst);
+        std::thread::spawn(move || {
+            let mut fd = fd;
+            if let Err(e) = dnd::write_offer(&mime, &path, &mut fd) {
+                tracing::warn!("could not send the screenshot as {mime}: {e:#}");
+            }
+            drop(fd);
+            writers.fetch_sub(1, Ordering::SeqCst);
+        });
+    }
+
+    pub fn drag_ended(&mut self, source: &WlDataSource, outcome: Outcome, qh: &QueueHandle<State>) {
+        let Some(drag) = self.drag.take_if(|d| d.source.inner() == source) else {
+            return;
+        };
+        tracing::info!("drag-out {outcome:?}");
+        drag.icon_viewport.destroy();
+        drag.icon.destroy();
+        if let Some(t) = self.thumbs.iter_mut().find(|t| t.id == drag.thumb) {
+            t.end_drag(outcome, qh);
+        }
+        let _ = self.conn.flush();
     }
 
     fn expire(&mut self, id: u64) {
@@ -304,6 +327,78 @@
             }
         }
     }
+}
+
+/// Pointer events on thumbnails. A free function: starting a drag needs
+/// more of `State` than the host.
+pub fn pointer(state: &mut State, events: &[PointerEvent], qh: &QueueHandle<State>) {
+    for event in events {
+        let Some(host) = state.preview.as_mut() else {
+            return;
+        };
+        let Some(i) = host.thumbs.iter().position(|t| t.is(&event.surface)) else {
+            continue;
+        };
+        let (x, y) = event.position;
+        match event.kind {
+            PointerEventKind::Enter { serial } => {
+                if let Some(cursor) = &state.cursor_device {
+                    cursor.set_shape(serial, Shape::Pointer);
+                }
+            }
+            PointerEventKind::Press {
+                button: BTN_LEFT,
+                serial,
+                ..
+            } => host.thumbs[i].press(x, y, serial),
+            PointerEventKind::Motion { .. } => {
+                let start = host.thumbs[i].motion(x, y, qh);
+                if let Some(start) = start {
+                    start_drag(state, i, start, qh);
+                }
+            }
+            PointerEventKind::Release {
+                button: BTN_LEFT, ..
+            } => {
+                let t = &mut host.thumbs[i];
+                if t.release(x, y, qh) == Some(Action::OpenEditor) {
+                    open_editor(&t.path);
+                }
+            }
+            _ => {}
+        }
+    }
+}
+
+fn start_drag(state: &mut State, i: usize, start: DragStart, qh: &QueueHandle<State>) {
+    let host = state.preview.as_mut().expect("host state");
+    let (Some(manager), Some(device), Some(viewporter)) =
+        (&state.data_devices, &state.data_device, &state.viewporter)
+    else {
+        if !host.warned_no_dnd {
+            host.warned_no_dnd = true;
+            tracing::warn!("the compositor has no wl_data_device_manager; drag-out is off");
+        }
+        return;
+    };
+    if host.drag.is_some() {
+        return;
+    }
+    let source = manager.create_drag_and_drop_source(qh, dnd::MIME_TYPES, DndAction::Copy);
+    let icon = state.compositor.create_surface(qh);
+    let icon_viewport = viewporter.get_viewport(&icon, qh, ());
+    let thumb = &mut host.thumbs[i];
+    source.start_drag(device, thumb.surface(), Some(&icon), start.serial);
+    thumb.draw_icon(&icon, &icon_viewport, start.grab);
+    thumb.begin_drag(qh);
+    tracing::info!("drag-out started for {}", thumb.path.display());
+    host.drag = Some(Drag {
+        thumb: thumb.id,
+        source,
+        icon,
+        icon_viewport,
+    });
+    let _ = host.conn.flush();
 }
 
 fn disconnect(state: &mut State, id: u64) {
```

- [ ] **Step 4: Remove the allowance, then lint and test**

Apply to `src/main.rs`:

```diff
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,6 +1,3 @@
-// Drag-out modules land before the host uses them; Task 3 removes this.
-#![allow(dead_code)]
-
 mod capture;
 mod config;
 mod detach;
```

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo nextest run`
Expected: no warnings, in particular no `dead_code`; every test passes.

- [ ] **Step 5: Try it on niri**

Build `cargo build --release`. Use a throwaway config with `copy_to_clipboard = false` and `timeout_secs = 20`. Stop any running preview host first (an installed valw's host would answer instead):

```sh
for pid in $(pgrep -f __preview-host); do
  case "$(ps -o comm= -p $pid)" in valw|.valw-wrapped) kill $pid;; esac
done
```

Then run `$CARGO_TARGET_DIR/release/valw screen` and ask the user to:
1. Drag the thumbnail left or up into Dolphin, a browser upload field, or Discord.
2. Run it again, drag left or up, and release on the empty desktop.
3. Run it once more and swipe right; then once more and click.

Expected:
- (1) The file arrives, and the thumbnail image travels under the pointer. The log has `drag-out started for …` and `drag-out Dropped`.
- (2) The thumbnail comes back, and the log has `drag-out Cancelled`.
- (3) The swipe dismisses, and the click opens Satty, both as before.

- [ ] **Step 6: Commit**

```sh
jj commit -m "feat: drag the preview out"
```

---

### Task 4: Checks, rollout and test with the user

**Files:** none in valw. In `~/copland`, only `flake.lock` changes, through `nix flake update valw`; tell the user before running it.

**Interfaces:**
- Consumes: Tasks 1–3.
- Produces: the feature on the user's system.

- [ ] **Step 1: Full checks**

Run: `nix flake check -L --keep-going`
Expected: every check passes. The headless preview check is unchanged and still guards hide/show and click/swipe regressions.

- [ ] **Step 2: Update copland's lock**

```sh
cd ~/copland && nix flake update valw && jj commit -m "valw: drag the preview out"
```

Expected: `flake.lock`'s `valw` node moves to the new tree.

- [ ] **Step 3: Hand over the rebuild**

Ask the user to rebuild as usual. Then `valw doctor` should be all `ok`.

- [ ] **Step 4: Checklist with the user**

- Drop into Dolphin → the file is copied (the default name has spaces, so this also checks `%20`).
- Drop into a browser upload field → it uploads. Drop into a Discord message → it attaches.
- Drop into an image editor that takes `image/png` (if they have one) → the pixels arrive.
- Release on nothing, or Esc mid-drag → the thumbnail comes back.
- **Review Focus 2:** with `timeout_secs = 3`, start a drag and hold it past 3 s, then release on nothing → it slides out.
- **Review Focus 1:** while dragging, press Print → after the capture the other thumbnails reappear but the dragged one doesn't; finishing the drag behaves normally.
- Drag onto the other monitor and drop there → it works.
- Swipe right still dismisses; click still opens Satty.
