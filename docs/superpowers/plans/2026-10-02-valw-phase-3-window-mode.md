# valw Phase 3: window mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `valw window` captures the window the user clicks; Space in region mode (before dragging) switches to it.

**Architecture:** niri picks the window (`Request::PickWindow`) and renders it into a temporary PNG (`Action::ScreenshotWindow` with `path`); valw waits for niri's `ScreenshotCaptured` event on a second socket, decodes the file, deletes it, and feeds the image into the existing save → preview → clipboard flow. The region overlay gains a `Choice::Window` outcome for Space.

**Tech Stack:** Rust 2024, `niri-ipc =26.4.0`, SCTK 0.21 keyboard (`Keysym`), `image` 0.25.

**Spec:** `docs/superpowers/specs/2026-10-02-valw-phase-3-window-mode-design.md`

## Global Constraints

- Version control is jj only: commit with `jj commit -m "..."`, never git.
- `cargo clippy -- -D warnings`, `cargo fmt --check`, `cargo nextest run` (not `cargo test`), `nix flake check` (not nix2 commands).
- Code, comments and docs in plain English; match the surrounding comment density.
- niri requires an absolute `path` for `ScreenshotWindow`.
- Wait for the capture event at most 5 s.
- Don't run valw captures against the user's live session without telling them.

## Review Focus

1. **niri's own screenshot binding fires while valw waits:** the stream reports a capture with another path; valw must keep waiting for its own. Test: `niri::tests::only_our_capture_counts` (Task 1).
2. **The window closes between the pick and the shot, or niri fails to write:** a clear error within 5 s, never a hang. niri's error reply is surfaced by `request`; a silent failure hits the timeout (Task 1 code, real check in the checklist).
3. **The temporary file is left behind** after an error between "written" and "decoded": the guard removes it on every path. Test: `window::tests::temp_file_is_removed_on_drop` (Task 2).
4. **Esc during the pick:** `Cancelled` (exit 3), thumbnails come back (the `HideGuard` drops as before). Real check in the checklist.
5. **Space while dragging a region:** nothing happens; the drag continues. Test: `selection::tests::window_switch_only_before_a_drag` (Task 3).

---

## File Structure

| File | Change | Task |
|---|---|---|
| `src/niri.rs` | `pick_window`, `workspaces`, `screenshot_window`, `is_our_capture`, `output_of` | 1 |
| `src/window.rs` (new) | `capture(cursor)`, `TempFile` guard, `temp_path()` | 2 |
| `src/main.rs` | `mod window;`, `Command::Window`, `Mode::Window`, `window_shot` | 2, 3 |
| `src/selection.rs` | `Selection::allows_window_switch` | 3 |
| `src/region.rs` | `Choice` replaces `Picked`; `Overlay::switch_to_window` | 3 |
| `src/wayland.rs` | Space in `press_key` | 3 |
| `docs/test-checklist.md` | Phase 3 section | 4 |

---

### Task 1: niri window IPC

**Files:**
- Modify: `src/niri.rs`, `src/main.rs` (allowance)

**Interfaces:**
- Produces (in `niri`):
  - `pub fn pick_window() -> Result<Option<niri_ipc::Window>>`
  - `pub fn workspaces() -> Result<Vec<niri_ipc::Workspace>>`
  - `pub fn screenshot_window(id: u64, path: &Path, cursor: bool, timeout: Duration) -> Result<()>`
  - `pub fn is_our_capture(event: &Event, path: &Path) -> bool`
  - `pub fn output_of(workspace_id: Option<u64>, workspaces: &[Workspace]) -> Option<String>`

- [ ] **Step 1: Dead-code allowance**

At the very top of `src/main.rs`, followed by an empty line:

```rust
// Window-mode helpers land before main uses them; Task 2 removes this.
#![allow(dead_code)]
```

- [ ] **Step 2: Write the failing tests**

Add to the `tests` module of `src/niri.rs`:

```rust
    use niri_ipc::{Event, Workspace};
    use std::path::Path;

    fn ws(id: u64, output: Option<&str>) -> Workspace {
        Workspace {
            id,
            idx: 1,
            name: None,
            output: output.map(str::to_owned),
            is_urgent: false,
            is_active: true,
            is_focused: false,
            active_window_id: None,
        }
    }

    #[test]
    fn only_our_capture_counts() {
        let ours = Path::new("/run/user/1000/valw-window-7.png");
        let captured = |p: Option<&str>| Event::ScreenshotCaptured {
            path: p.map(str::to_owned),
        };
        assert!(is_our_capture(&captured(Some("/run/user/1000/valw-window-7.png")), ours));
        assert!(!is_our_capture(&captured(None), ours), "clipboard-only shot");
        assert!(
            !is_our_capture(&captured(Some("/home/u/Pictures/Screenshot.png")), ours),
            "niri's own binding"
        );
        assert!(!is_our_capture(&Event::WindowClosed { id: 3 }, ours));
    }

    #[test]
    fn output_of_follows_the_workspace() {
        let all = [ws(1, Some("DP-1")), ws(2, Some("HDMI-A-1")), ws(3, None)];
        assert_eq!(output_of(Some(2), &all), Some("HDMI-A-1".to_owned()));
        assert_eq!(output_of(None, &all), None, "no workspace");
        assert_eq!(output_of(Some(9), &all), None, "unknown workspace");
        assert_eq!(output_of(Some(3), &all), None, "workspace without output");
    }
```

- [ ] **Step 3: Run them and watch them fail**

Run: `cargo nextest run niri::`
Expected: compilation fails, `` cannot find function `is_our_capture` `` / `` `output_of` ``.

- [ ] **Step 4: Implement**

Replace the imports at the top of `src/niri.rs`:

```rust
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use niri_ipc::{Action, Event, Request, Response, Window, Workspace, socket::Socket};
```

Split the connection out of `request` so the event stream can reuse it:

```rust
fn connect() -> Result<Socket> {
    Socket::connect().context("could not connect to niri (is NIRI_SOCKET set?)")
}

fn request(request: Request) -> Result<Response> {
    connect()?
        .send(request)
        .context("niri IPC request failed")?
        .map_err(|e| anyhow!("niri replied with an error: {e}"))
}
```

Add after `focused_output`:

```rust
/// Lets the user click a window; niri shows its own pick cursor.
/// `None` if they pressed Esc.
pub fn pick_window() -> Result<Option<Window>> {
    match request(Request::PickWindow)? {
        Response::PickedWindow(window) => Ok(window),
        other => Err(anyhow!("unexpected niri reply: {other:?}")),
    }
}

pub fn workspaces() -> Result<Vec<Workspace>> {
    match request(Request::Workspaces)? {
        Response::Workspaces(workspaces) => Ok(workspaces),
        other => Err(anyhow!("unexpected niri reply: {other:?}")),
    }
}

/// Has niri render window `id` into `path` (absolute) and waits until the
/// file is written.
pub fn screenshot_window(id: u64, path: &Path, cursor: bool, timeout: Duration) -> Result<()> {
    // Listen before asking, so the event can't come and go unseen.
    let mut events = connect()?;
    events
        .send(Request::EventStream)
        .context("niri IPC request failed")?
        .map_err(|e| anyhow!("niri replied with an error: {e}"))?;
    let mut read = events.read_events();
    let (tx, rx) = mpsc::channel();
    let wanted = path.to_path_buf();
    std::thread::spawn(move || {
        while let Ok(event) = read() {
            if is_our_capture(&event, &wanted) {
                let _ = tx.send(());
                return;
            }
        }
    });

    let path = path
        .to_str()
        .context("the temporary screenshot path is not valid UTF-8")?;
    match request(Request::Action(Action::ScreenshotWindow {
        id: Some(id),
        write_to_disk: true,
        show_pointer: cursor,
        path: Some(path.to_owned()),
    }))? {
        Response::Handled => {}
        other => bail!("unexpected niri reply: {other:?}"),
    }
    rx.recv_timeout(timeout).map_err(|_| {
        anyhow!(
            "niri did not deliver the window screenshot within {}s",
            timeout.as_secs()
        )
    })
}

/// Whether `event` reports the screenshot written to `path`. niri reports
/// every screenshot on the stream, including its own bindings'.
pub fn is_our_capture(event: &Event, path: &Path) -> bool {
    matches!(event, Event::ScreenshotCaptured { path: Some(p) } if Path::new(p) == path)
}

/// The output showing the workspace a window is on, if niri knows it.
pub fn output_of(workspace_id: Option<u64>, workspaces: &[Workspace]) -> Option<String> {
    let id = workspace_id?;
    workspaces.iter().find(|w| w.id == id)?.output.clone()
}
```

Delete the old `request` body (it is replaced above). If `use std::path::Path;` in the test module now duplicates the outer import, drop it from the test module.

- [ ] **Step 5: Run the tests**

Run: `cargo nextest run niri::`
Expected: 4 passed (`parses_versions`, `ipc_version_matches_cargo_toml`, `only_our_capture_counts`, `output_of_follows_the_workspace`).

Run: `cargo clippy -- -D warnings`
Expected: no warnings.

- [ ] **Step 6: Commit**

```sh
jj commit -m "feat: niri window pick and screenshot IPC"
```

---

### Task 2: `valw window`

**Files:**
- Create: `src/window.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: Task 1's `niri::{pick_window, workspaces, screenshot_window, output_of}`; `thumbnail::load(&Path) -> Result<RgbaImage>`; `lock::default_path()`; `error::Cancelled`.
- Produces: `pub fn window::capture(cursor: bool) -> Result<(RgbaImage, Option<String>)>` (image, output name if known); `fn window_shot(outputs: &[wayland::Output], cursor: bool) -> Result<(Vec<(RgbaImage, Option<String>)>, usize, String)>` in `main.rs`, used again by Task 3.

- [ ] **Step 1: Write the failing tests**

Create `src/window.rs` with only the tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_file_is_removed_on_drop() {
        let dir = std::env::temp_dir().join(format!("valw-window-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shot.png");
        std::fs::write(&path, b"png").unwrap();
        drop(TempFile(path.clone()));
        assert!(!path.exists());
        // A file niri never wrote is fine too.
        drop(TempFile(dir.join("missing.png")));
        std::fs::remove_dir(&dir).unwrap();
    }

    #[test]
    fn temp_path_is_absolute_and_per_process() {
        let p = temp_path();
        assert!(p.is_absolute(), "niri rejects relative paths: {p:?}");
        assert!(p.to_string_lossy().contains(&std::process::id().to_string()));
    }
}
```

Add `mod window;` to the module list in `src/main.rs` (after `mod wayland;`). Add to the `flag_conflicts` test in `src/main.rs`:

```rust
        assert!(parses(&["window", "--cursor", "--delay", "2"]));
        assert!(!parses(&["window", "--clipboard-only", "-o", "a.png"]));
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo nextest run window:: flag_conflicts`
Expected: compilation fails (`TempFile`, `temp_path` not found).

- [ ] **Step 3: Implement `window.rs`**

Above the test module:

```rust
//! Window mode: niri picks the window and renders it; valw reads the result.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use image::RgbaImage;

use crate::error::Cancelled;
use crate::niri;

/// How long niri gets to render and write the window.
const TIMEOUT: Duration = Duration::from_secs(5);

/// Lets the user click a window. Returns its pixels and the output it is
/// on, if niri knows. Esc during the pick is `Cancelled`.
pub fn capture(cursor: bool) -> Result<(RgbaImage, Option<String>)> {
    let window = niri::pick_window()?.ok_or(Cancelled)?;
    tracing::info!("picked window {} ({:?})", window.id, window.app_id);
    let tmp = TempFile(temp_path());
    niri::screenshot_window(window.id, &tmp.0, cursor, TIMEOUT)?;
    let image = crate::thumbnail::load(&tmp.0)?;
    let workspaces = niri::workspaces().unwrap_or_else(|e| {
        tracing::warn!("no workspace list: {e:#}");
        Vec::new()
    });
    Ok((image, niri::output_of(window.workspace_id, &workspaces)))
}

/// Where niri writes the window: next to the capture lock, one per process.
fn temp_path() -> PathBuf {
    crate::lock::default_path().with_file_name(format!("valw-window-{}.png", std::process::id()))
}

/// Removes the temporary screenshot however `capture` ends.
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
```

- [ ] **Step 4: Wire `main.rs`**

Remove the Task 1 allowance (the two lines at the top of `src/main.rs`).

Add the subcommand to `enum Command`, after `Region`:

```rust
    /// Click a window to capture it (Cmd+Shift+4, then Space).
    Window {
        #[command(flatten)]
        common: Common,
    },
```

Add `Window` to `enum Mode`:

```rust
enum Mode {
    Screen { all: bool },
    Region,
    Window,
}
```

In `run`, after the `Region` arm:

```rust
        Command::Window { common } => capture(Mode::Window, common),
```

In `capture`'s `match mode`, after the `Mode::Region` arm:

```rust
        Mode::Window => window_shot(&outputs, cursor)?,
```

Add below `capture`:

```rust
/// A window shot, shaped like the other modes' results. The preview goes to
/// the window's output, or niri's focused one if that is unknown.
fn window_shot(
    outputs: &[wayland::Output],
    cursor: bool,
) -> Result<(Vec<(RgbaImage, Option<String>)>, usize, String)> {
    let (image, output) = window::capture(cursor)?;
    let source = output.unwrap_or_else(|| outputs[focused_output(outputs)].geom.name.clone());
    Ok((vec![(image, None)], 0, source))
}
```

- [ ] **Step 5: Run the tests and checks**

Run: `cargo nextest run`
Expected: all pass (previous count + 4: two `window::` tests, the two new `flag_conflicts` asserts live in an existing test).

Run: `cargo clippy -- -D warnings && cargo fmt --check`
Expected: clean.

- [ ] **Step 6: Commit**

```sh
jj commit -m "feat: valw window"
```

---

### Task 3: Space switches region mode to window mode

**Files:**
- Modify: `src/selection.rs`, `src/region.rs`, `src/wayland.rs`, `src/main.rs`

**Interfaces:**
- Consumes: Task 2's `window_shot`.
- Produces: `Selection::allows_window_switch(&self) -> bool`; `pub enum region::Choice { Region(usize, PixelRect), Window }`; `region::select(...) -> Result<Choice>`; `Overlay::switch_to_window(&mut self)`.

- [ ] **Step 1: Write the failing test**

Add to the tests of `src/selection.rs`:

```rust
    #[test]
    fn window_switch_only_before_a_drag() {
        let mut s = Selection::default();
        assert!(s.allows_window_switch());
        s.press(Point { x: 1.0, y: 1.0 });
        assert!(!s.allows_window_switch(), "Space during a drag is reserved");
        s.release(Point { x: 5.0, y: 5.0 });
        assert!(s.allows_window_switch());
    }
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo nextest run selection::`
Expected: compilation fails, no method `allows_window_switch`.

- [ ] **Step 3: Implement**

In `impl Selection` (`src/selection.rs`), after `release`:

```rust
    /// Space switches to window mode only before a drag starts.
    pub fn allows_window_switch(&self) -> bool {
        *self == Selection::Idle
    }
```

In `src/region.rs`, replace the `Picked` alias:

```rust
/// What the user chose in the overlay.
#[derive(Debug, PartialEq)]
pub enum Choice {
    /// A rectangle in the frame of output `.0`.
    Region(usize, PixelRect),
    /// Space before dragging: capture a window instead.
    Window,
}
```

In `struct Overlay`: `outcome: Option<Option<Choice>>,`.

`select`'s signature becomes `-> Result<Choice>` (body unchanged) and its doc comment gains: "Space before dragging returns `Choice::Window`."

In `pointer`'s release arm: `self.outcome = Some(Some(Choice::Region(a, rect)));`.

In `impl Overlay`, after `cancel`:

```rust
    pub fn switch_to_window(&mut self) {
        if self.selection.allows_window_switch() {
            tracing::info!("switching to window mode");
            self.outcome.get_or_insert(Some(Choice::Window));
        }
    }
```

In `src/wayland.rs`, `press_key` body becomes:

```rust
        let Some(overlay) = &mut self.overlay else {
            return;
        };
        match event.keysym {
            Keysym::Escape => overlay.cancel(),
            Keysym::space => overlay.switch_to_window(),
            _ => {}
        }
```

In `src/main.rs`, the `Mode::Region` arm becomes:

```rust
        Mode::Region => {
            let frames = wl.capture(&outputs, cursor)?;
            match region::select(&mut wl, &outputs, &frames)? {
                region::Choice::Region(i, r) => {
                    let source = frames[i].output.name.clone();
                    (vec![(frames[i].to_rgba(r), None)], 0, source)
                }
                region::Choice::Window => {
                    drop(frames);
                    window_shot(&outputs, cursor)?
                }
            }
        }
```

- [ ] **Step 4: Run the tests and checks**

Run: `cargo nextest run`
Expected: all pass (+1).

Run: `cargo clippy -- -D warnings && cargo fmt --check`
Expected: clean.

- [ ] **Step 5: Commit**

```sh
jj commit -m "feat: space switches region mode to window mode"
```

---

### Task 4: Checks and checklist

**Files:**
- Modify: `docs/test-checklist.md`

- [ ] **Step 1: Full checks**

Run: `nix flake check -L --keep-going`
Expected: every check passes (the headless sway check is unchanged; window mode needs niri and is not covered there).

- [ ] **Step 2: Append the checklist section**

Append at the end of `docs/test-checklist.md` (after the drag-out "Known minors"):

```markdown

## Window mode (Phase 3)

- [ ] `valw window`, click a tiled window → that window alone is saved, preview on its monitor.
- [ ] Same for a floating window, and for a window partly covered by another (the covered part is still there).
- [ ] A window on the other monitor (different scale, if any) → right size and sharpness; preview on that monitor.
- [ ] Esc during the pick → nothing saved, exit code 3, thumbnails come back.
- [ ] `--cursor` with the pointer over the window → the pointer is in the shot.
- [ ] `valw region`, press Space before dragging → the overlay closes and the window pick starts; Space while dragging does nothing.
- [ ] `--clipboard-only` and `-o -` → clipboard / stdout get the window; no stray `valw-window-*.png` left in `$XDG_RUNTIME_DIR`.
- [ ] niri's own "Screenshot captured" notification appears (expected, see the spec); the clipboard ends up with the window.
```

- [ ] **Step 3: Commit**

```sh
jj commit -m "docs: window mode checklist"
```
