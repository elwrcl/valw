# valw Phase 2 (floating preview) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** After every capture that writes a file, a thumbnail slides in at the bottom right. Clicking it opens Satty, swiping it right dismisses it, and it slides away after `timeout_secs`. It never takes focus and never shows up in a screenshot. valw is then wired into copland for a day of real use.

**Architecture:** One on-demand **preview host** process (`valw __preview-host`) owns every thumbnail. It listens on `$XDG_RUNTIME_DIR/valw.sock` (one JSON message per line, one reply each) and runs a calloop loop over Wayland, the socket and timers. A capture holds a `HideGuard` connection from just before its screencopy until it has saved the file. While any such connection is open, the host draws every thumbnail fully transparent and click-through. When the connection closes (for any reason), the thumbnails come back.

**Tech Stack:** Rust 2024; smithay-client-toolkit 0.21 (its re-exported calloop and calloop-wayland-source); serde_json (new); image (resize); Satty 0.22 (via the Nix wrapper); flake-parts; jj.

**Spec:** `docs/superpowers/specs/2026-09-30-valw-phase-2-preview-design.md` (read it first). Phase 1's spec, `docs/superpowers/specs/2026-09-29-valw-phase-0-1-design.md`, still applies to everything this plan doesn't change.

## Global Constraints

- Code, comments, error/log messages: English. Comments are plain and short and say *why*.
- Version control is **jj**. Commit with `jj commit -m "<msg>"`; never use git commands in the valw repo. copland (Task 9) is also a jj repo.
- Use `cargo clippy` (never `cargo check`) and `cargo nextest run` (never `cargo test`), inside the devShell (direnv, or prefix with `nix develop -c`).
- A background `jj util snapshot` in the user's session can race with `jj commit` and leave the working copy stale or divergent. If `jj` reports a stale working copy, **back up the changed files first**, then run `jj workspace update-stale`, restore the newer files from the backup, and abandon an identical divergent twin (`jj diff --from A --to B` shows nothing).
- Never write a manual `imports = [ ... ]` list for flake modules; `nix/**/*.ulu.nix` is discovered automatically.
- Until Task 6, `src/main.rs` starts with `#![allow(dead_code)]`, because the new modules land before the CLI uses them. Task 6 removes it, and clippy must then be clean without it.
- Socket `$XDG_RUNTIME_DIR/valw.sock`, host lock `$XDG_RUNTIME_DIR/valw-preview.lock`, layer namespace `valw-preview`, hidden subcommand `__preview-host`.
- Thumbnail: long edge 220 logical px (never upscaled), 16 px from the screen edges, 12 px gap, at most 5 per output, slide-in 200 ms ease-out, slide-out 150 ms ease-in, click under 4 px of movement, dismissing swipe at least 40% of the width.
- Config: `[preview] enabled = true, timeout_secs = 5`; `timeout_secs = 0` is a config error.
- Never unmap a thumbnail to hide it: niri 26.04 doesn't send a new configure afterwards (verified). Hide means a transparent buffer plus an empty input region.
- The `HideGuard` must be dropped **before** the clipboard fork; the forked child would otherwise inherit the connection and keep thumbnails hidden.

## Review Focus

These are the inputs and failure modes a real user will hit that no automated test fully exercises. Each one is pinned in the task that owns the code.

1. **The capture dies while thumbnails are hidden** (Esc, an error, `kill -9` during region selection): the thumbnails must come back. Tests: `ipc::tests::guard_hides_until_dropped_and_adds_on_the_same_connection` (the host sees EOF when the guard drops, Task 3) and `host::tests::hidden_until_every_hider_leaves` (Task 5). Real check: Task 8, step 5.
2. **A stale socket file after a host crash:** the next capture must neither hang nor fail. It skips hiding, and a new host replaces the file. Test: `ipc::tests::stale_socket_is_ignored` (Task 3). Real check: Task 8, step 5.
3. **Two captures overlapping** (the hotkey pressed during region selection, or `--delay` runs): thumbnails stay hidden until the *last* hider leaves. Tests: `host::tests::hidden_until_every_hider_leaves` and `a_client_that_never_hid_does_not_show` (Task 5).
4. **Satty missing** (valw run without the Nix wrapper, e.g. `cargo run`): a click must only log a warning and close the thumbnail, never crash the host. Real check: Task 8, step 5.
5. **The preview can't be shown** (the file was removed before the host loaded it, or the host fails to start): the capture still succeeds (exit 0) and logs `no preview: …`. Test: `ipc::tests::host_errors_reach_the_caller` and `host_that_never_comes_up_is_an_error` (Task 3). Real check: Task 6, step 6.

---

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `src/config.rs` | `[preview]` section, `timeout_secs` validation | 1 |
| `src/stack.rs` | Pure: thumbnail size, stack margins, eviction, click/swipe decision, easing, `Anim` | 2 |
| `src/ipc.rs` | Protocol types, JSON lines, `HideGuard`, `start_host` | 3 |
| `src/wayland.rs` | `Output::scale`, `State::output_list`, `State::preview` and event routing | 4, 5 |
| `src/thumbnail.rs` | One thumbnail: layer surface, `downscale`/`paint`, animation, gestures, hide/show | 4 |
| `src/host.rs` | Pure `Core` (hide counter, exit rule); socket server; calloop loop; `add`; Satty launch | 5 |
| `src/main.rs` | `__preview-host`, `--no-preview`, `HideGuard` around the capture, `save` split | 1, 5, 6 |
| `nix/package.ulu.nix` | Satty on PATH via `wrapProgram` | 7 |
| `nix/checks.ulu.nix` | Headless preview checks | 7 |
| copland: `flake.nix`, `modules/home/home-valw.ulu.nix`, `hosts/copland.ulu.nix`, `modules/home/niri/desktop-niri-keybinds.ulu.nix` | Test-day wiring | 8 |

---

### Task 1: `[preview]` config section

**Files:**
- Modify: `src/main.rs` (dead-code allowance), `src/config.rs`

**Interfaces:**
- Consumes: Phase 1's `config::parse`/`load` and `Config`.
- Produces: `pub struct Preview { pub enabled: bool, pub timeout_secs: u64 }` (defaults `true`, `5`) as `Config::preview`; `parse` fails with `preview.timeout_secs must be at least 1` when it is 0.

- [ ] **Step 1: Add the dead-code allowance**

Put these lines at the very top of `src/main.rs`, above the `mod` list:

```rust
// Phase 2 modules land before the CLI uses them; Task 6 removes this.
#![allow(dead_code)]
```

- [ ] **Step 2: Write the failing tests**

Apply this change to the test module of `src/config.rs`. It adds two tests and moves the unknown-key example from `[preview]` (a real section now) to `[zoom]`. Unknown keys are reported in alphabetical order, so the expected list is reordered too.

```diff
--- a/src/config.rs
+++ b/src/config.rs
@@ -19,13 +19,10 @@
 
     #[test]
     fn unknown_keys_are_reported_not_rejected() {
-        let text = "[preview]\nenabled = true\n\n[save]\nfolder = \"x\"\n";
+        let text = "[zoom]\nscroll_step = 1.15\n\n[save]\nfolder = \"x\"\n";
         let (config, unknown) = parse(text).unwrap();
         assert_eq!(config, Config::default());
-        assert_eq!(
-            unknown,
-            vec!["preview".to_string(), "save.folder".to_string()]
-        );
+        assert_eq!(unknown, vec!["save.folder".to_string(), "zoom".to_string()]);
     }
 
     #[test]
@@ -37,6 +34,27 @@
     #[test]
     fn wrong_type_is_an_error() {
         assert!(parse("[capture]\nshow_cursor = \"yes\"\n").is_err());
+    }
+
+    #[test]
+    fn preview_defaults_and_overrides() {
+        assert_eq!(
+            Config::default().preview,
+            Preview {
+                enabled: true,
+                timeout_secs: 5
+            }
+        );
+        let (config, _) = parse("[preview]\ntimeout_secs = 2\n").unwrap();
+        assert!(config.preview.enabled);
+        assert_eq!(config.preview.timeout_secs, 2);
+    }
+
+    #[test]
+    fn preview_timeout_must_be_positive() {
+        let err = parse("[preview]\ntimeout_secs = 0\n").unwrap_err();
+        assert_eq!(err.to_string(), "preview.timeout_secs must be at least 1");
+        assert!(parse("[preview]\ntimeout_secs = -3\n").is_err());
     }
 
     #[test]
```

- [ ] **Step 3: Run the tests and watch them fail**

Run: `cargo nextest run config::`
Expected: compilation fails, e.g. `` no field `preview` on type `Config` ``.

- [ ] **Step 4: Implement**

Apply this change above the test module:

```diff
--- a/src/config.rs
+++ b/src/config.rs
@@ -1,6 +1,6 @@
 use std::path::{Path, PathBuf};
 
-use anyhow::{Context, Result};
+use anyhow::{Context, Result, bail};
 use serde::Deserialize;
 
 use crate::error::HintExt;
@@ -10,6 +10,7 @@
 pub struct Config {
     pub save: Save,
     pub capture: Capture,
+    pub preview: Preview,
 }
 
 #[derive(Debug, Clone, PartialEq, Deserialize)]
@@ -24,6 +25,22 @@
 #[serde(default)]
 pub struct Capture {
     pub show_cursor: bool,
+}
+
+#[derive(Debug, Clone, PartialEq, Deserialize)]
+#[serde(default)]
+pub struct Preview {
+    pub enabled: bool,
+    pub timeout_secs: u64,
+}
+
+impl Default for Preview {
+    fn default() -> Self {
+        Self {
+            enabled: true,
+            timeout_secs: 5,
+        }
+    }
 }
 
 impl Default for Save {
@@ -77,7 +94,10 @@
 pub fn parse(text: &str) -> Result<(Config, Vec<String>)> {
     let de = toml::Deserializer::parse(text)?;
     let mut unknown = Vec::new();
-    let config = serde_ignored::deserialize(de, |path| unknown.push(path.to_string()))?;
+    let config: Config = serde_ignored::deserialize(de, |path| unknown.push(path.to_string()))?;
+    if config.preview.timeout_secs == 0 {
+        bail!("preview.timeout_secs must be at least 1");
+    }
     Ok((config, unknown))
 }
 
```

- [ ] **Step 5: Run the tests and watch them pass**

Run: `cargo nextest run config::`
Expected: all 9 tests in `config::tests` pass.

- [ ] **Step 6: Lint and run the whole suite**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo nextest run`
Expected: no warnings; every test passes.

- [ ] **Step 7: Commit**

```sh
jj commit -m "feat: [preview] config section"
```

---

### Task 2: Stack layout, gestures and animation (pure)

**Files:**
- Create: `src/stack.rs`
- Modify: `src/main.rs` (`mod stack;`)

**Interfaces:**
- Consumes: nothing.
- Produces: consts `LONG_EDGE = 220`, `EDGE_MARGIN = 16`, `GAP = 12`, `MAX = 5`, `CLICK_SLOP = 4.0`, `SWIPE_FRACTION = 0.4`, `SLIDE_IN_MS = 200`, `SLIDE_OUT_MS = 150`; `thumb_size(width: u32, height: u32) -> (u32, u32)`; `bottom_margins(heights_newest_first: &[u32]) -> Vec<u32>`; `excess(count: usize) -> usize`; `enum Release { Click, Dismiss, SnapBack }` and `release(dx: f64, dy: f64, width: f64) -> Release`; `drag_offset(dx: f64) -> f64`; `enum Ease { OutCubic, InCubic }` with `apply(t)`; `struct Anim { from, to, duration_ms, ease }` with `slide_in(travel)`, `slide_out(from, travel)`, `snap_back(from)` and `at(elapsed_ms: f64) -> (f64, bool)`.

- [ ] **Step 1: Write the failing tests for `stack`**

Create `src/stack.rs` with only the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn landscape_and_portrait_fit_the_long_edge() {
        assert_eq!(thumb_size(1920, 1080), (220, 124));
        assert_eq!(thumb_size(1080, 1920), (124, 220));
        assert_eq!(thumb_size(1366, 768), (220, 124));
    }

    #[test]
    fn small_images_are_not_scaled_up() {
        assert_eq!(thumb_size(100, 50), (100, 50));
        assert_eq!(thumb_size(220, 10), (220, 10));
    }

    #[test]
    fn extreme_aspect_ratio_keeps_one_pixel() {
        assert_eq!(thumb_size(10_000, 2), (220, 1));
    }

    #[test]
    fn stack_margins() {
        assert_eq!(bottom_margins(&[]), Vec::<u32>::new());
        assert_eq!(bottom_margins(&[124]), vec![16]);
        assert_eq!(bottom_margins(&[124, 124, 220]), vec![16, 152, 288]);
    }

    #[test]
    fn eviction_beyond_five() {
        assert_eq!(excess(5), 0);
        assert_eq!(excess(6), 1);
        assert_eq!(excess(0), 0);
    }

    #[test]
    fn release_decisions() {
        let w = 220.0;
        assert_eq!(release(0.0, 0.0, w), Release::Click);
        assert_eq!(release(2.0, 3.0, w), Release::Click);
        assert_eq!(release(0.39 * w, 0.0, w), Release::SnapBack);
        assert_eq!(release(0.40 * w, 0.0, w), Release::Dismiss);
        assert_eq!(release(-100.0, 0.0, w), Release::SnapBack);
        assert_eq!(release(0.0, 50.0, w), Release::SnapBack);
    }

    #[test]
    fn drag_follows_right_only() {
        assert_eq!(drag_offset(30.0), 30.0);
        assert_eq!(drag_offset(-30.0), 0.0);
    }

    #[test]
    fn easing_endpoints_and_monotonic() {
        for ease in [Ease::OutCubic, Ease::InCubic] {
            assert_eq!(ease.apply(0.0), 0.0);
            assert_eq!(ease.apply(1.0), 1.0);
            assert_eq!(ease.apply(2.0), 1.0, "clamped");
            let samples: Vec<f64> = (0..=10).map(|i| ease.apply(i as f64 / 10.0)).collect();
            assert!(
                samples.windows(2).all(|w| w[0] <= w[1]),
                "{ease:?} {samples:?}"
            );
        }
    }

    #[test]
    fn slide_in_runs_from_travel_to_rest() {
        let a = Anim::slide_in(236.0);
        assert_eq!(a.at(0.0), (236.0, false));
        assert_eq!(a.at(200.0), (0.0, true));
        assert_eq!(a.at(500.0), (0.0, true));
        let (mid, done) = a.at(100.0);
        assert!(mid > 0.0 && mid < 118.0, "ease-out is past halfway: {mid}");
        assert!(!done);
    }

    #[test]
    fn slide_out_and_snap_back() {
        let out = Anim::slide_out(50.0, 236.0);
        assert_eq!(out.at(0.0), (50.0, false));
        assert_eq!(out.at(150.0), (236.0, true));
        assert_eq!(Anim::snap_back(40.0).at(150.0), (0.0, true));
    }
}
```

Add `mod stack;` to the `mod` list at the top of `src/main.rs` (keep it sorted).

- [ ] **Step 2: Run the tests and watch them fail**

Run: `cargo nextest run stack::`
Expected: compilation fails, e.g. `` cannot find function `thumb_size` in this scope ``.

- [ ] **Step 3: Implement `stack`**

Put this above the test module in `src/stack.rs`:

```rust
//! Pure layout, gesture and animation rules for the preview thumbnails.

/// Long edge of a thumbnail, in logical pixels.
pub const LONG_EDGE: u32 = 220;
/// Distance between a thumbnail and the screen's right and bottom edges.
pub const EDGE_MARGIN: u32 = 16;
/// Vertical gap between stacked thumbnails.
pub const GAP: u32 = 12;
/// Thumbnails per output; a new one beyond this evicts the oldest.
pub const MAX: usize = 5;
/// Pointer movement below this (logical px) still counts as a click.
pub const CLICK_SLOP: f64 = 4.0;
/// A swipe of at least this fraction of the image width dismisses it.
pub const SWIPE_FRACTION: f64 = 0.4;
pub const SLIDE_IN_MS: u32 = 200;
pub const SLIDE_OUT_MS: u32 = 150;

/// Logical thumbnail size for an image of `width` x `height`: long edge
/// `LONG_EDGE`, aspect ratio kept, never scaled up.
pub fn thumb_size(width: u32, height: u32) -> (u32, u32) {
    let long = width.max(height);
    if long <= LONG_EDGE {
        return (width.max(1), height.max(1));
    }
    let scale = LONG_EDGE as f64 / long as f64;
    let fit = |v: u32| ((v as f64 * scale).round() as u32).max(1);
    (fit(width), fit(height))
}

/// Bottom margin of each thumbnail, given their heights newest first.
/// The newest sits `EDGE_MARGIN` above the screen edge, older ones stack up.
pub fn bottom_margins(heights_newest_first: &[u32]) -> Vec<u32> {
    let mut next = EDGE_MARGIN;
    heights_newest_first
        .iter()
        .map(|h| {
            let margin = next;
            next += h + GAP;
            margin
        })
        .collect()
}

/// How many of the oldest thumbnails to evict when there are `count`.
pub fn excess(count: usize) -> usize {
    count.saturating_sub(MAX)
}

/// What a pointer release on a thumbnail means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Release {
    /// Barely moved: open the editor.
    Click,
    /// Dragged far enough right: slide out and close.
    Dismiss,
    /// Dragged, but not far enough: animate back.
    SnapBack,
}

/// Classifies a release after a press-drag of (`dx`, `dy`) on an image
/// `width` logical px wide.
pub fn release(dx: f64, dy: f64, width: f64) -> Release {
    if dx.hypot(dy) < CLICK_SLOP {
        Release::Click
    } else if dx >= SWIPE_FRACTION * width {
        Release::Dismiss
    } else {
        Release::SnapBack
    }
}

/// Horizontal image offset while dragging: follows the pointer, right only.
pub fn drag_offset(dx: f64) -> f64 {
    dx.max(0.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ease {
    OutCubic,
    InCubic,
}

impl Ease {
    /// Maps linear progress `t` in 0..=1 to eased progress in 0..=1.
    pub fn apply(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Ease::OutCubic => 1.0 - (1.0 - t).powi(3),
            Ease::InCubic => t.powi(3),
        }
    }
}

/// An animation of the image's horizontal offset.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Anim {
    pub from: f64,
    pub to: f64,
    pub duration_ms: u32,
    pub ease: Ease,
}

impl Anim {
    /// Slides in from fully off-screen (`travel` px to the right) to rest.
    pub fn slide_in(travel: f64) -> Anim {
        Anim {
            from: travel,
            to: 0.0,
            duration_ms: SLIDE_IN_MS,
            ease: Ease::OutCubic,
        }
    }

    /// Slides out from the current offset to fully off-screen.
    pub fn slide_out(from: f64, travel: f64) -> Anim {
        Anim {
            from,
            to: travel,
            duration_ms: SLIDE_OUT_MS,
            ease: Ease::InCubic,
        }
    }

    /// Animates back to rest after a short drag.
    pub fn snap_back(from: f64) -> Anim {
        Anim {
            from,
            to: 0.0,
            duration_ms: SLIDE_OUT_MS,
            ease: Ease::OutCubic,
        }
    }

    /// Offset after `elapsed_ms`, and whether the animation has finished.
    pub fn at(&self, elapsed_ms: f64) -> (f64, bool) {
        let t = elapsed_ms / self.duration_ms as f64;
        let offset = self.from + (self.to - self.from) * self.ease.apply(t);
        (offset, t >= 1.0)
    }
}
```

- [ ] **Step 4: Run the tests and watch them pass**

Run: `cargo nextest run stack::`
Expected: all 10 tests in `stack::tests` pass.

- [ ] **Step 5: Lint and run the whole suite**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo nextest run`
Expected: no warnings; every test passes.

- [ ] **Step 6: Commit**

```sh
jj commit -m "feat: preview stack layout, gestures and easing"
```

---

### Task 3: IPC protocol and client (`HideGuard`)

**Files:**
- Modify: `Cargo.toml` (`serde_json`), `src/main.rs` (`mod ipc;`)
- Create: `src/ipc.rs`

**Interfaces:**
- Consumes: `lock::default_path()` (Phase 1).
- Produces: `enum Request { Hide, Add { path: PathBuf, output: String } }` (serde tag `cmd`, lowercase); `struct Reply { ok: bool, error: Option<String> }` with `Reply::ok()`, `Reply::error(msg)`; `encode<T: Serialize>(&T) -> String` (JSON + newline); `parse_request(&str) -> Result<Request, String>`; `socket_path()`, `host_lock_path()`; `struct HideGuard` with `HideGuard::new(socket: &Path) -> HideGuard` and `add(&mut self, socket, path, output, start: impl FnOnce() -> Result<()>) -> Result<()>`; `start_host() -> Result<()>` (runs `current_exe() __preview-host` in its own session); consts `HIDE_TIMEOUT` 300 ms, `ADD_TIMEOUT` 2 s, `START_TIMEOUT` 1 s.

The tests run a fake host in a thread over a real Unix socket, so the client is tested without Wayland. `add` takes the host-starting function as a parameter for the same reason.

- [ ] **Step 1: Add the dependency**

In `Cargo.toml`, under `[dependencies]`, right after `serde_ignored`:

```toml
serde_json = "1.0.151"
```

- [ ] **Step 2: Write the failing tests for `ipc`**

Create `src/ipc.rs` with only the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    #[test]
    fn requests_as_json_lines() {
        assert_eq!(encode(&Request::Hide), "{\"cmd\":\"hide\"}\n");
        let add = Request::Add {
            path: "/tmp/a.png".into(),
            output: "HDMI-A-1".into(),
        };
        assert_eq!(
            encode(&add),
            "{\"cmd\":\"add\",\"path\":\"/tmp/a.png\",\"output\":\"HDMI-A-1\"}\n"
        );
        assert_eq!(parse_request(&encode(&add)), Ok(add));
    }

    #[test]
    fn replies_as_json_lines() {
        assert_eq!(encode(&Reply::ok()), "{\"ok\":true}\n");
        assert_eq!(
            encode(&Reply::error("no")),
            "{\"ok\":false,\"error\":\"no\"}\n"
        );
    }

    #[test]
    fn bad_requests_are_errors() {
        assert!(parse_request("nonsense").is_err());
        assert!(parse_request("{\"cmd\":\"launch\"}").is_err());
        assert!(parse_request("{\"cmd\":\"add\"}").is_err());
    }

    /// A fake host: answers every request with `reply` (or never, if None)
    /// and records the lines it received, plus "EOF" when the client leaves.
    fn fake_host(socket: &Path, reply: Option<Reply>) -> Arc<Mutex<Vec<String>>> {
        let listener = UnixListener::bind(socket).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        log.lock().unwrap().push("EOF".into());
                        break;
                    }
                    log.lock().unwrap().push(line.trim().to_string());
                    match &reply {
                        Some(r) => stream.write_all(encode(r).as_bytes()).unwrap(),
                        None => thread::sleep(Duration::from_secs(5)),
                    }
                }
            }
        });
        seen
    }

    fn wait_for(seen: &Arc<Mutex<Vec<String>>>, n: usize) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        while seen.lock().unwrap().len() < n && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        seen.lock().unwrap().clone()
    }

    #[test]
    fn no_host_means_no_hiding() {
        let dir = tempfile::tempdir().unwrap();
        let guard = HideGuard::new(&dir.path().join("valw.sock"));
        assert!(guard.conn.is_none());
    }

    #[test]
    fn stale_socket_is_ignored() {
        // A crashed host leaves its socket file behind with nobody listening.
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("valw.sock");
        drop(UnixListener::bind(&socket).unwrap());
        assert!(socket.exists());

        let started = Instant::now();
        let guard = HideGuard::new(&socket);
        assert!(guard.conn.is_none());
        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn guard_hides_until_dropped_and_adds_on_the_same_connection() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("valw.sock");
        let seen = fake_host(&socket, Some(Reply::ok()));

        let mut guard = HideGuard::new(&socket);
        assert!(guard.conn.is_some());
        guard
            .add(&socket, Path::new("/tmp/a.png"), "DP-1", || {
                panic!("host is running")
            })
            .unwrap();
        drop(guard);

        assert_eq!(
            wait_for(&seen, 3),
            vec![
                "{\"cmd\":\"hide\"}".to_string(),
                "{\"cmd\":\"add\",\"path\":\"/tmp/a.png\",\"output\":\"DP-1\"}".to_string(),
                "EOF".to_string(),
            ]
        );
    }

    #[test]
    fn silent_host_does_not_block_the_capture() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("valw.sock");
        let _seen = fake_host(&socket, None);

        let started = Instant::now();
        let guard = HideGuard::new(&socket);
        assert!(guard.conn.is_none());
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn add_starts_a_host_when_none_runs() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("valw.sock");
        let mut guard = HideGuard::new(&socket);

        let seen = Arc::new(Mutex::new(None));
        let slot = Arc::clone(&seen);
        let start_socket = socket.clone();
        guard
            .add(&socket, Path::new("/tmp/b.png"), "DP-1", move || {
                *slot.lock().unwrap() = Some(fake_host(&start_socket, Some(Reply::ok())));
                Ok(())
            })
            .unwrap();

        let seen = seen.lock().unwrap().clone().expect("start was called");
        assert_eq!(
            wait_for(&seen, 1)[0],
            "{\"cmd\":\"add\",\"path\":\"/tmp/b.png\",\"output\":\"DP-1\"}"
        );
    }

    #[test]
    fn host_errors_reach_the_caller() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("valw.sock");
        let _seen = fake_host(&socket, Some(Reply::error("could not read /x.png")));
        let mut guard = HideGuard { conn: None };
        let err = guard
            .add(&socket, Path::new("/x.png"), "DP-1", || Ok(()))
            .unwrap_err();
        assert_eq!(err.to_string(), "could not read /x.png");
    }

    #[test]
    fn host_that_never_comes_up_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("valw.sock");
        let mut guard = HideGuard { conn: None };
        let err = guard
            .add(&socket, Path::new("/x.png"), "DP-1", || Ok(()))
            .unwrap_err();
        assert_eq!(err.to_string(), "the preview host did not come up");
    }
}
```

Add `mod ipc;` to the `mod` list at the top of `src/main.rs` (keep it sorted).

- [ ] **Step 3: Run the tests and watch them fail**

Run: `cargo nextest run ipc::`
Expected: compilation fails, e.g. `` cannot find function `encode` in this scope ``.

- [ ] **Step 4: Implement `ipc`**

Put this above the test module in `src/ipc.rs`:

```rust
//! The line-based JSON protocol between captures and the preview host, and
//! its client side.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

/// How long a capture waits for the host to hide its thumbnails.
pub const HIDE_TIMEOUT: Duration = Duration::from_millis(300);
/// How long a capture waits for the host to load and show a thumbnail.
pub const ADD_TIMEOUT: Duration = Duration::from_secs(2);
/// How long a capture keeps retrying to reach a host it just started.
pub const START_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "lowercase")]
pub enum Request {
    /// Hide every thumbnail until this connection closes.
    Hide,
    /// Show a thumbnail for the screenshot at `path` on `output`.
    Add { path: PathBuf, output: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Reply {
    pub fn ok() -> Reply {
        Reply {
            ok: true,
            error: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Reply {
        Reply {
            ok: false,
            error: Some(message.into()),
        }
    }
}

/// One message as a JSON line.
pub fn encode<T: Serialize>(message: &T) -> String {
    let mut line = serde_json::to_string(message).expect("messages always serialize");
    line.push('\n');
    line
}

pub fn parse_request(line: &str) -> Result<Request, String> {
    serde_json::from_str(line.trim()).map_err(|e| format!("bad request: {e}"))
}

/// `$XDG_RUNTIME_DIR/valw.sock`, next to the capture lock.
pub fn socket_path() -> PathBuf {
    crate::lock::default_path().with_file_name("valw.sock")
}

/// Held by the running host so only one exists at a time.
pub fn host_lock_path() -> PathBuf {
    crate::lock::default_path().with_file_name("valw-preview.lock")
}

/// One request/reply connection to the host.
struct Conn {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Conn {
    fn new(stream: UnixStream) -> Result<Conn> {
        Ok(Conn {
            writer: stream
                .try_clone()
                .context("could not clone the host socket")?,
            reader: BufReader::new(stream),
        })
    }

    fn request(&mut self, request: &Request, timeout: Duration) -> Result<()> {
        self.writer
            .write_all(encode(request).as_bytes())
            .context("could not send to the preview host")?;
        self.reader.get_ref().set_read_timeout(Some(timeout))?;
        let mut line = String::new();
        self.reader
            .read_line(&mut line)
            .context("the preview host did not answer")?;
        if line.is_empty() {
            bail!("the preview host closed the connection");
        }
        let reply: Reply =
            serde_json::from_str(&line).context("bad reply from the preview host")?;
        match reply {
            Reply { ok: true, .. } => Ok(()),
            Reply { error, .. } => Err(anyhow!(error.unwrap_or_else(|| "unknown error".into()))),
        }
    }
}

/// Keeps the host's thumbnails hidden for as long as it lives, so they never
/// end up in a screenshot. Closing the connection, however the capture ends,
/// shows them again.
pub struct HideGuard {
    conn: Option<Conn>,
}

impl HideGuard {
    /// Asks a running host to hide. Without a host this does nothing; if the
    /// host doesn't answer in time the capture goes ahead anyway.
    pub fn new(socket: &Path) -> HideGuard {
        let Ok(stream) = UnixStream::connect(socket) else {
            return HideGuard { conn: None };
        };
        let conn = Conn::new(stream).and_then(|mut conn| {
            conn.request(&Request::Hide, HIDE_TIMEOUT)?;
            Ok(conn)
        });
        match conn {
            Ok(conn) => HideGuard { conn: Some(conn) },
            Err(e) => {
                tracing::warn!("capturing without hiding the previews: {e:#}");
                HideGuard { conn: None }
            }
        }
    }

    /// Shows a thumbnail for `path` on `output`. Uses this guard's
    /// connection if there is one, otherwise calls `start` to launch a host
    /// and connects to it.
    pub fn add(
        &mut self,
        socket: &Path,
        path: &Path,
        output: &str,
        start: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        let request = Request::Add {
            path: path.to_path_buf(),
            output: output.to_string(),
        };
        if let Some(conn) = &mut self.conn {
            return conn.request(&request, ADD_TIMEOUT);
        }
        let mut conn = connect_or_start(socket, start)?;
        conn.request(&request, ADD_TIMEOUT)
    }
}

fn connect_or_start(socket: &Path, start: impl FnOnce() -> Result<()>) -> Result<Conn> {
    if let Ok(stream) = UnixStream::connect(socket) {
        return Conn::new(stream);
    }
    start().context("could not start the preview host")?;
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        match UnixStream::connect(socket) {
            Ok(stream) => return Conn::new(stream),
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(e).context("the preview host did not come up"),
        }
    }
}

/// Starts `valw __preview-host` in its own session, detached from our stdio.
/// A fresh exec inherits nothing from the capture (its Wayland connection,
/// lock or threads).
pub fn start_host() -> Result<()> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let exe = std::env::current_exe().context("could not find the valw binary")?;
    let mut command = Command::new(exe);
    command
        .arg("__preview-host")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid is async-signal-safe.
    unsafe {
        command.pre_exec(|| rustix::process::setsid().map(drop).map_err(Into::into));
    }
    command
        .spawn()
        .context("could not run valw __preview-host")?;
    Ok(())
}
```

- [ ] **Step 5: Run the tests and watch them pass**

Run: `cargo nextest run ipc::`
Expected: all 10 tests in `ipc::tests` pass.

- [ ] **Step 6: Lint and run the whole suite**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo nextest run`
Expected: no warnings; every test passes.

- [ ] **Step 7: Commit**

```sh
jj commit -m "feat: preview host protocol and hide guard"
```

---

### Task 4: Thumbnail surface and pixels

**Files:**
- Modify: `src/wayland.rs` (`Output::scale`, `State::output_list`), `src/main.rs` (`mod thumbnail;`)
- Create: `src/thumbnail.rs`

**Interfaces:**
- Consumes: `stack::*` (Task 2); `wayland::{Output, State}`.
- Produces: `Output::scale: f64` (physical / logical width from the current mode, orientation-aware); `State::output_list(&self) -> Vec<Output>` (what `Wayland::outputs` now calls); `thumbnail::{NAMESPACE, downscale(img, scale) -> (RgbaImage, (u32, u32)), paint(canvas, canvas_width, small, x0), Action::OpenEditor, Parts { compositor, layer_shell, viewporter, shm }, Thumbnail}`. `Thumbnail` has `new(id, path, image, output, parts, qh) -> Result<Thumbnail>` plus `is`, `is_layer`, `is_closing`, `is_finished`, `hide(qh)`, `show(qh)`, `set_margin`, `configure`, `frame_done`, `expire`, `slide_out`, `press`, `motion`, `release(x, y, qh) -> Option<Action>`, and the pub fields `id`, `output`, `path`, `size`.

Why hide/show work this way (spec 4.6): a hidden thumbnail commits a transparent buffer and an empty input region. Unmapping is off limits, because niri 26.04 doesn't send a configure on the next commit, so the thumbnail would never return. `draw(qh, force)` bypasses a pending frame callback for hide and show, so a mid-animation thumbnail is gone before the capture starts.

- [ ] **Step 1: Output scale and an output list on `State`**

The host (Task 5) has to look outputs up from inside the event loop, where it only has `State`, and it needs each output's scale to render sharp thumbnails. Apply to `src/wayland.rs`:

```diff
--- a/src/wayland.rs
+++ b/src/wayland.rs
@@ -38,6 +38,8 @@
     pub wl: wl_output::WlOutput,
     pub geom: OutputGeom,
     pub transform: wl_output::Transform,
+    /// Physical pixels per logical pixel, e.g. 1.25.
+    pub scale: f64,
 }
 
 pub struct Wayland {
@@ -112,29 +114,7 @@
 
     /// All outputs with a known name and logical geometry.
     pub fn outputs(&self) -> Vec<Output> {
-        self.state
-            .outputs
-            .outputs()
-            .filter_map(|wl| {
-                let info = self.state.outputs.info(&wl)?;
-                let (x, y) = info.logical_position?;
-                let (width, height) = info.logical_size?;
-                Some(Output {
-                    geom: OutputGeom {
-                        name: info
-                            .name
-                            .clone()
-                            .unwrap_or_else(|| format!("output-{}", info.id)),
-                        x,
-                        y,
-                        width,
-                        height,
-                    },
-                    transform: info.transform,
-                    wl,
-                })
-            })
-            .collect()
+        self.state.output_list()
     }
 
     /// `(interface, version)` of every global, sorted.
@@ -218,6 +198,57 @@
     }
 }
 
+impl State {
+    /// All outputs with a known name and logical geometry.
+    pub fn output_list(&self) -> Vec<Output> {
+        self.outputs
+            .outputs()
+            .filter_map(|wl| {
+                let info = self.outputs.info(&wl)?;
+                let (x, y) = info.logical_position?;
+                let (width, height) = info.logical_size?;
+                Some(Output {
+                    scale: output_scale(&info, width),
+                    geom: OutputGeom {
+                        name: info
+                            .name
+                            .clone()
+                            .unwrap_or_else(|| format!("output-{}", info.id)),
+                        x,
+                        y,
+                        width,
+                        height,
+                    },
+                    transform: info.transform,
+                    wl,
+                })
+            })
+            .collect()
+    }
+}
+
+/// Physical pixels per logical pixel: the current mode's width (turned to
+/// match the logical orientation) over the logical width.
+fn output_scale(info: &smithay_client_toolkit::output::OutputInfo, logical_width: i32) -> f64 {
+    let Some(mode) = info.modes.iter().find(|m| m.current) else {
+        return info.scale_factor as f64;
+    };
+    let (w, h) = mode.dimensions;
+    let rotated = matches!(
+        info.transform,
+        wl_output::Transform::_90
+            | wl_output::Transform::_270
+            | wl_output::Transform::Flipped90
+            | wl_output::Transform::Flipped270
+    );
+    let physical_width = if rotated { h } else { w };
+    if logical_width > 0 {
+        physical_width as f64 / logical_width as f64
+    } else {
+        1.0
+    }
+}
+
 impl CompositorHandler for State {
     fn scale_factor_changed(
         &mut self,
```

Run: `cargo clippy --all-targets -- -D warnings && cargo nextest run`
Expected: no warnings; every test still passes (no behaviour change yet).

- [ ] **Step 2: Write the failing tests for `thumbnail`**

Create `src/thumbnail.rs` with only the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downscale_fits_long_edge_at_scale_one() {
        let img = RgbaImage::new(1920, 1080);
        let (small, logical) = downscale(&img, 1.0);
        assert_eq!(logical, (220, 124));
        assert_eq!(small.dimensions(), (220, 124));
    }

    #[test]
    fn downscale_renders_physical_pixels_at_fractional_scale() {
        let img = RgbaImage::new(1920, 1080);
        let (small, logical) = downscale(&img, 1.25);
        assert_eq!(logical, (220, 124));
        assert_eq!(small.dimensions(), (275, 155));
    }

    #[test]
    fn downscale_leaves_small_images_alone() {
        let img = RgbaImage::from_pixel(100, 50, image::Rgba([1, 2, 3, 255]));
        let (small, logical) = downscale(&img, 1.0);
        assert_eq!(logical, (100, 50));
        assert_eq!(small, img);
    }

    fn pixel(canvas: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * width + x) * 4) as usize;
        canvas[i..i + 4].try_into().unwrap()
    }

    /// 4x3 red image, so the inner pixels are (1,1) and (2,1).
    fn red() -> RgbaImage {
        RgbaImage::from_pixel(4, 3, image::Rgba([255, 0, 0, 255]))
    }

    #[test]
    fn paint_at_rest_has_border_image_and_transparent_padding() {
        let mut canvas = vec![9u8; 6 * 3 * 4];
        paint(&mut canvas, 6, &red(), 0);
        assert_eq!(pixel(&canvas, 6, 0, 0), BORDER);
        assert_eq!(pixel(&canvas, 6, 1, 1), [0, 0, 255, 255], "BGRA red");
        assert_eq!(pixel(&canvas, 6, 3, 1), BORDER);
        assert_eq!(pixel(&canvas, 6, 4, 1), [0; 4], "padding is transparent");
        assert_eq!(pixel(&canvas, 6, 5, 2), [0; 4]);
    }

    #[test]
    fn paint_with_offset_shifts_and_clips() {
        let mut canvas = vec![0u8; 6 * 3 * 4];
        paint(&mut canvas, 6, &red(), 3);
        assert_eq!(pixel(&canvas, 6, 2, 1), [0; 4], "left of the image");
        assert_eq!(pixel(&canvas, 6, 3, 1), BORDER);
        assert_eq!(pixel(&canvas, 6, 4, 1), [0, 0, 255, 255]);
        assert_eq!(
            pixel(&canvas, 6, 5, 1),
            [0, 0, 255, 255],
            "clipped at the edge"
        );
    }

    #[test]
    fn paint_fully_off_screen_is_empty() {
        let mut canvas = vec![7u8; 6 * 3 * 4];
        paint(&mut canvas, 6, &red(), 6);
        assert!(canvas.iter().all(|&b| b == 0));
    }

    #[test]
    fn canvas_includes_padding() {
        assert_eq!(canvas_width(220, 1.0), 236);
        assert_eq!(canvas_width(220, 1.25), 295);
        assert_eq!(travel(220), 236.0);
    }
}
```

Add `mod thumbnail;` to the `mod` list at the top of `src/main.rs` (keep it sorted).

- [ ] **Step 3: Run the tests and watch them fail**

Run: `cargo nextest run thumbnail::`
Expected: compilation fails, e.g. `` cannot find function `downscale` in this scope ``.

- [ ] **Step 4: Implement `thumbnail`**

Put this above the test module in `src/thumbnail.rs`:

```rust
//! One preview thumbnail: its layer surface, pixels, animation and gestures.

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use image::{RgbaImage, imageops};
use smithay_client_toolkit::{
    compositor::{CompositorState, FrameCallbackData, Region},
    shell::{
        WaylandSurface,
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerSurface},
    },
    shm::{
        Shm,
        slot::{Buffer, SlotPool},
    },
};
use wayland_client::{QueueHandle, protocol::wl_shm};
use wayland_protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
};

use crate::stack::{self, Anim, EDGE_MARGIN, Release};
use crate::wayland::{Output, State};

pub const NAMESPACE: &str = "valw-preview";

/// Premultiplied BGRA of the 1 px border: #e0e0e0 at 80% opacity.
const BORDER: [u8; 4] = [179, 179, 179, 204];

/// The thumbnail image for `img` on an output with `scale` physical pixels
/// per logical pixel: returns the physical-size image and its logical size.
pub fn downscale(img: &RgbaImage, scale: f64) -> (RgbaImage, (u32, u32)) {
    let logical = stack::thumb_size(img.width(), img.height());
    let physical = (
        ((logical.0 as f64 * scale).round() as u32).max(1),
        ((logical.1 as f64 * scale).round() as u32).max(1),
    );
    let small = if physical == img.dimensions() {
        img.clone()
    } else {
        imageops::resize(
            img,
            physical.0,
            physical.1,
            imageops::FilterType::CatmullRom,
        )
    };
    (small, logical)
}

/// Paints `small` into a transparent ARGB8888 `canvas` that is `canvas_width`
/// px wide and as tall as `small`, starting at column `x0` and clipped to the
/// canvas, with a 1 px border on the image's edges.
pub fn paint(canvas: &mut [u8], canvas_width: u32, small: &RgbaImage, x0: i64) {
    canvas.fill(0);
    let (w, h) = small.dimensions();
    for y in 0..h {
        for x in 0..w {
            let cx = x0 + x as i64;
            if cx < 0 || cx >= canvas_width as i64 {
                continue;
            }
            let edge = x == 0 || y == 0 || x == w - 1 || y == h - 1;
            let px = if edge {
                BORDER
            } else {
                let [r, g, b, _] = small.get_pixel(x, y).0;
                [b, g, r, 255]
            };
            let i = ((y * canvas_width) as usize + cx as usize) * 4;
            canvas[i..i + 4].copy_from_slice(&px);
        }
    }
}

/// What the host should do after a pointer release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    OpenEditor,
}

pub struct Thumbnail {
    pub id: u64,
    pub output: String,
    pub path: PathBuf,
    layer: LayerSurface,
    viewport: WpViewport,
    pool: SlotPool,
    buffer: Option<Buffer>,
    small: RgbaImage,
    /// Logical size of the image (not the surface, which has padding).
    pub size: (u32, u32),
    scale: f64,
    /// How far right of its resting place the image is drawn, logical px.
    offset: f64,
    anim: Option<(Anim, Instant)>,
    drag: Option<(f64, f64)>,
    margin: u32,
    /// Input region while visible (the image) and while hidden (nothing).
    input: Region,
    no_input: Region,
    /// Hidden for a capture: drawn fully transparent, clicks pass through.
    hidden: bool,
    configured: bool,
    frame_pending: bool,
    started: bool,
    closing: bool,
    expired: bool,
}

/// The Wayland objects a new thumbnail needs.
pub struct Parts<'a> {
    pub compositor: &'a CompositorState,
    pub layer_shell: &'a LayerShell,
    pub viewporter: &'a WpViewporter,
    pub shm: &'a Shm,
}

impl Thumbnail {
    pub fn new(
        id: u64,
        path: PathBuf,
        image: &RgbaImage,
        output: &Output,
        parts: &Parts,
        qh: &QueueHandle<State>,
    ) -> Result<Thumbnail> {
        let (small, size) = downscale(image, output.scale);
        let surface = parts.compositor.create_surface(qh);
        let layer = parts.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Overlay,
            Some(NAMESPACE),
            Some(&output.wl),
        );
        layer.set_anchor(Anchor::BOTTOM | Anchor::RIGHT);
        // Never take keyboard focus away from what the user is typing in.
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_size(size.0 + EDGE_MARGIN, size.1);
        layer.set_margin(0, 0, EDGE_MARGIN as i32, 0);
        // Clicks on the transparent padding go through to what's below.
        let input = Region::new(parts.compositor).context("could not create an input region")?;
        input.add(0, 0, size.0 as i32, size.1 as i32);
        let no_input = Region::new(parts.compositor).context("could not create an input region")?;
        layer.wl_surface().set_input_region(Some(input.wl_region()));
        let viewport = parts.viewporter.get_viewport(layer.wl_surface(), qh, ());
        viewport.set_destination((size.0 + EDGE_MARGIN) as i32, size.1 as i32);
        // The first commit has no buffer; the compositor answers with a
        // configure, and the first draw starts the slide-in.
        layer.commit();

        let buffer_len = (small.height() * canvas_width(size.0, output.scale) * 4) as usize;
        Ok(Thumbnail {
            id,
            output: output.geom.name.clone(),
            path,
            layer,
            viewport,
            pool: SlotPool::new(buffer_len * 2, parts.shm).context("could not create shm pool")?,
            buffer: None,
            small,
            size,
            scale: output.scale,
            offset: travel(size.0),
            anim: None,
            drag: None,
            margin: EDGE_MARGIN,
            input,
            no_input,
            hidden: false,
            configured: false,
            frame_pending: false,
            started: false,
            closing: false,
            expired: false,
        })
    }

    pub fn is(&self, surface: &wayland_client::protocol::wl_surface::WlSurface) -> bool {
        self.layer.wl_surface() == surface
    }

    pub fn is_layer(&self, layer: &LayerSurface) -> bool {
        self.layer.wl_surface() == layer.wl_surface()
    }

    pub fn is_closing(&self) -> bool {
        self.closing
    }

    /// The slide-out has finished (or is invisible anyway).
    pub fn is_finished(&self) -> bool {
        self.closing && (self.anim.is_none() || self.hidden)
    }

    /// Makes the thumbnail invisible and click-through. The surface stays
    /// mapped: remapping would need a configure that niri doesn't send.
    pub fn hide(&mut self, qh: &QueueHandle<State>) {
        if self.hidden {
            return;
        }
        self.hidden = true;
        self.layer
            .wl_surface()
            .set_input_region(Some(self.no_input.wl_region()));
        self.draw(qh, true);
    }

    pub fn show(&mut self, qh: &QueueHandle<State>) {
        if !self.hidden {
            return;
        }
        self.hidden = false;
        self.layer
            .wl_surface()
            .set_input_region(Some(self.input.wl_region()));
        self.draw(qh, true);
    }

    pub fn set_margin(&mut self, margin: u32) {
        if margin == self.margin {
            return;
        }
        self.margin = margin;
        self.layer.set_margin(0, 0, margin as i32, 0);
        self.layer.commit();
    }

    pub fn configure(&mut self, qh: &QueueHandle<State>) {
        self.configured = true;
        if !self.started {
            self.started = true;
            self.anim = Some((Anim::slide_in(travel(self.size.0)), Instant::now()));
        }
        self.draw(qh, false);
    }

    pub fn frame_done(&mut self, qh: &QueueHandle<State>) {
        self.frame_pending = false;
        if self.anim.is_some() || self.drag.is_some() {
            self.draw(qh, false);
        }
    }

    /// The timeout fired: slide out, unless the user is dragging it.
    pub fn expire(&mut self, qh: &QueueHandle<State>) {
        self.expired = true;
        if self.drag.is_none() {
            self.slide_out(qh);
        }
    }

    pub fn slide_out(&mut self, qh: &QueueHandle<State>) {
        if self.closing {
            return;
        }
        self.closing = true;
        self.anim = Some((
            Anim::slide_out(self.offset, travel(self.size.0)),
            Instant::now(),
        ));
        self.draw(qh, false);
    }

    pub fn press(&mut self, x: f64, y: f64) {
        if !self.closing {
            self.anim = None;
            self.drag = Some((x, y));
        }
    }

    pub fn motion(&mut self, x: f64, qh: &QueueHandle<State>) {
        if let Some((x0, _)) = self.drag {
            self.offset = stack::drag_offset(x - x0);
            self.draw(qh, false);
        }
    }

    pub fn release(&mut self, x: f64, y: f64, qh: &QueueHandle<State>) -> Option<Action> {
        let (x0, y0) = self.drag.take()?;
        match stack::release(x - x0, y - y0, self.size.0 as f64) {
            Release::Click => {
                self.slide_out(qh);
                Some(Action::OpenEditor)
            }
            Release::Dismiss => {
                self.slide_out(qh);
                None
            }
            Release::SnapBack if self.expired => {
                self.slide_out(qh);
                None
            }
            Release::SnapBack => {
                self.anim = Some((Anim::snap_back(self.offset), Instant::now()));
                self.draw(qh, false);
                None
            }
        }
    }

    fn draw(&mut self, qh: &QueueHandle<State>, force: bool) {
        if let Some((anim, start)) = self.anim {
            let (offset, done) = anim.at(start.elapsed().as_secs_f64() * 1000.0);
            self.offset = offset;
            if done {
                self.anim = None;
            }
        }
        // Hide and show must reach the screen now, even mid-animation.
        if !self.configured || (self.frame_pending && !force) {
            return;
        }
        let width = canvas_width(self.size.0, self.scale);
        let height = self.small.height();
        let Ok((buffer, canvas)) = self.pool.create_buffer(
            width as i32,
            height as i32,
            width as i32 * 4,
            wl_shm::Format::Argb8888,
        ) else {
            tracing::warn!("could not allocate a thumbnail buffer");
            return;
        };
        if self.hidden {
            canvas.fill(0);
        } else {
            paint(
                canvas,
                width,
                &self.small,
                (self.offset * self.scale).round() as i64,
            );
        }
        let surface = self.layer.wl_surface();
        surface.damage_buffer(0, 0, width as i32, height as i32);
        // Keep drawing while something moves; motion that arrives while a
        // frame is pending is picked up by the next frame callback.
        if self.anim.is_some() || self.drag.is_some() {
            surface.frame(qh, FrameCallbackData(surface.clone()));
            self.frame_pending = true;
        }
        if buffer.attach_to(surface).is_err() {
            tracing::warn!("could not attach a thumbnail buffer");
            return;
        }
        self.layer.commit();
        self.buffer = Some(buffer);
    }
}

impl Drop for Thumbnail {
    fn drop(&mut self) {
        self.viewport.destroy();
    }
}

/// How far right the image moves to be fully off-screen.
fn travel(image_width: u32) -> f64 {
    (image_width + EDGE_MARGIN) as f64
}

/// Physical width of the surface: the image plus the edge padding.
fn canvas_width(image_width: u32, scale: f64) -> u32 {
    (((image_width + EDGE_MARGIN) as f64 * scale).round() as u32).max(1)
}
```

- [ ] **Step 5: Run the tests and watch them pass**

Run: `cargo nextest run thumbnail::`
Expected: all 7 tests in `thumbnail::tests` pass.

- [ ] **Step 6: Lint and run the whole suite**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo nextest run`
Expected: no warnings; every test passes.

- [ ] **Step 7: Commit**

```sh
jj commit -m "feat: preview thumbnail surface and pixels"
```

---

### Task 5: Preview host

**Files:**
- Create: `src/host.rs`
- Modify: `src/wayland.rs` (`State::preview`, event routing), `src/main.rs` (`mod host;`, `__preview-host`)

**Interfaces:**
- Consumes: `ipc::{socket_path, host_lock_path, parse_request, encode, Reply, Request}` (Task 3); `thumbnail::*` (Task 4); `stack::{bottom_margins, excess}` (Task 2); `lock::Lock`, `config::load` (Phase 1).
- Produces: `host::run() -> Result<()>`; `host::Core` (pure: `new(now)`, `connect(id)`, `hide(id) -> bool`, `disconnect(id) -> bool`, `hidden()`, `should_exit(thumbnails, now)`); `host::Host` with `configure`, `frame_done`, `closed`, `pointer` (called from `State`'s handlers); `State::preview: Option<Host>`; the hidden CLI subcommand `valw __preview-host`.

How it runs: the host takes `valw-preview.lock` (a second host exits 0 quietly), removes any stale socket, binds `valw.sock`, connects to Wayland, and runs a calloop loop. The loop covers the Wayland source, the listener, one line-buffered source per client, and one timer per thumbnail. After every dispatch it drops finished thumbnails, restacks the rest, and exits once there are no thumbnails and no clients. A new host that nobody connects to within 2 s also exits. `hide` from the first hider hides every thumbnail and does a Wayland round trip before replying. A hider's connection closing (EOF) shows them again when it was the last hider.

- [ ] **Step 1: Write the failing tests for `host`**

Create `src/host.rs` with only the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_until_every_hider_leaves() {
        let mut core = Core::new(Instant::now());
        core.connect(1);
        core.connect(2);
        assert!(core.hide(1), "first hide hides");
        assert!(!core.hide(2), "second hide changes nothing");
        assert!(!core.disconnect(1), "still hidden by 2");
        assert!(core.hidden());
        assert!(core.disconnect(2), "last hider leaving shows");
        assert!(!core.hidden());
    }

    #[test]
    fn a_client_that_never_hid_does_not_show() {
        let mut core = Core::new(Instant::now());
        core.connect(1);
        core.connect(2);
        core.hide(1);
        assert!(!core.disconnect(2));
        assert!(core.hidden());
    }

    #[test]
    fn exits_only_when_empty_and_alone() {
        let now = Instant::now();
        let mut core = Core::new(now);
        assert!(!core.should_exit(0, now), "waits for its first client");
        assert!(
            core.should_exit(0, now + Duration::from_secs(3)),
            "gives up after the grace period"
        );

        core.connect(1);
        assert!(!core.should_exit(0, now), "a client is connected");
        core.disconnect(1);
        assert!(!core.should_exit(1, now), "a thumbnail is up");
        assert!(core.should_exit(0, now));
    }

    #[test]
    fn a_second_host_gives_up_quietly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("valw-preview.lock");
        let _first = Lock::acquire(&path).unwrap();
        assert!(Lock::acquire(&path).is_err());
    }
}
```

- [ ] **Step 2: Run the tests and watch them fail**

Run: `cargo nextest run host::`
Expected: compilation fails, e.g. `` cannot find type `Core` in this scope ``.

- [ ] **Step 3: Implement `host`**

Put this above the test module in `src/host.rs`:

```rust
//! The preview host: one process that owns every thumbnail, started on
//! demand by a capture and gone once its last thumbnail is.

use std::collections::BTreeSet;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use smithay_client_toolkit::reexports::calloop::{
    EventLoop, Interest, LoopHandle, Mode, PostAction,
    generic::Generic,
    timer::{TimeoutAction, Timer},
};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::seat::pointer::{BTN_LEFT, PointerEvent, PointerEventKind};
use smithay_client_toolkit::shell::wlr_layer::LayerSurface;
use wayland_client::{Connection, QueueHandle, protocol::wl_surface::WlSurface};
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::{
    Shape, WpCursorShapeDeviceV1,
};

use crate::ipc::{self, Reply, Request};
use crate::lock::Lock;
use crate::stack;
use crate::thumbnail::{Action, Parts, Thumbnail};
use crate::wayland::{State, Wayland};

/// A host nobody connects to within this time gives up.
const STARTUP_GRACE: Duration = Duration::from_secs(2);

/// Connection bookkeeping and the exit rule, without any Wayland.
#[derive(Debug)]
pub struct Core {
    clients: BTreeSet<u64>,
    hiding: BTreeSet<u64>,
    seen_client: bool,
    started: Instant,
}

impl Core {
    pub fn new(now: Instant) -> Core {
        Core {
            clients: BTreeSet::new(),
            hiding: BTreeSet::new(),
            seen_client: false,
            started: now,
        }
    }

    pub fn connect(&mut self, id: u64) {
        self.clients.insert(id);
        self.seen_client = true;
    }

    /// Returns true if this request is what hides the thumbnails.
    pub fn hide(&mut self, id: u64) -> bool {
        let was_hidden = self.hidden();
        self.hiding.insert(id);
        !was_hidden
    }

    /// Returns true if this disconnect is what shows the thumbnails again.
    pub fn disconnect(&mut self, id: u64) -> bool {
        self.clients.remove(&id);
        self.hiding.remove(&id) && self.hiding.is_empty()
    }

    pub fn hidden(&self) -> bool {
        !self.hiding.is_empty()
    }

    /// Nothing to show and nobody talking to us. A fresh host waits a little
    /// for the capture that started it to connect.
    pub fn should_exit(&self, thumbnails: usize, now: Instant) -> bool {
        thumbnails == 0
            && self.clients.is_empty()
            && (self.seen_client || now.duration_since(self.started) > STARTUP_GRACE)
    }
}

pub struct Host {
    core: Core,
    /// Oldest first.
    thumbs: Vec<Thumbnail>,
    next_id: u64,
    timeout: Duration,
    conn: Connection,
    handle: LoopHandle<'static, State>,
    qh: QueueHandle<State>,
}

/// Runs the host until its last thumbnail closes. A second host exits
/// right away: the first one owns the socket.
pub fn run() -> Result<()> {
    let Ok(_lock) = Lock::acquire(&ipc::host_lock_path()) else {
        tracing::info!("another preview host is running");
        return Ok(());
    };
    let socket = ipc::socket_path();
    // Safe under the lock: whatever is there is left over from a crash.
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket)
        .with_context(|| format!("could not listen on {}", socket.display()))?;
    listener.set_nonblocking(true)?;

    let config = crate::config::load(&crate::config::default_path())?;
    let Wayland {
        conn,
        queue,
        mut state,
        ..
    } = Wayland::connect()?;
    let mut event_loop: EventLoop<'static, State> =
        EventLoop::try_new().context("could not create the event loop")?;
    let handle = event_loop.handle();
    let qh = queue.handle();
    WaylandSource::new(conn.clone(), queue)
        .insert(handle.clone())
        .map_err(|e| anyhow::anyhow!("could not watch the Wayland socket: {e}"))?;

    state.preview = Some(Host {
        core: Core::new(Instant::now()),
        thumbs: Vec::new(),
        next_id: 0,
        timeout: Duration::from_secs(config.preview.timeout_secs),
        conn,
        handle: handle.clone(),
        qh,
    });

    let mut next_client = 0u64;
    handle
        .insert_source(
            Generic::new(listener, Interest::READ, Mode::Level),
            move |_, listener, state| {
                while let Ok((stream, _)) = listener.accept() {
                    next_client += 1;
                    if let Some(host) = &mut state.preview {
                        host.add_client(stream, next_client);
                    }
                }
                Ok(PostAction::Continue)
            },
        )
        .map_err(|e| anyhow::anyhow!("could not watch the socket: {e}"))?;

    tracing::info!("preview host listening on {}", socket.display());
    let signal = event_loop.get_signal();
    event_loop
        .run(Duration::from_millis(500), &mut state, |state| {
            let host = state.preview.as_mut().expect("host state");
            host.tidy();
            if host.core.should_exit(host.thumbs.len(), Instant::now()) {
                signal.stop();
            }
        })
        .context("preview host event loop failed")?;

    let _ = std::fs::remove_file(&socket);
    tracing::info!("preview host done");
    Ok(())
}

impl Host {
    fn add_client(&mut self, stream: UnixStream, id: u64) {
        if stream.set_nonblocking(true).is_err() {
            return;
        }
        self.core.connect(id);
        let mut buf = Vec::new();
        let inserted = self.handle.insert_source(
            Generic::new(stream, Interest::READ, Mode::Level),
            move |_, stream, state| {
                let mut chunk = [0u8; 4096];
                loop {
                    let mut reader: &UnixStream = stream;
                    match reader.read(&mut chunk) {
                        Ok(0) => {
                            disconnect(state, id);
                            return Ok(PostAction::Remove);
                        }
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                        Err(_) => {
                            disconnect(state, id);
                            return Ok(PostAction::Remove);
                        }
                    }
                }
                while let Some(end) = buf.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = buf.drain(..=end).collect();
                    let reply = match ipc::parse_request(&String::from_utf8_lossy(&line)) {
                        Ok(request) => handle(state, id, request),
                        Err(e) => Reply::error(e),
                    };
                    let mut writer: &UnixStream = stream;
                    let _ = writer.write_all(ipc::encode(&reply).as_bytes());
                }
                Ok(PostAction::Continue)
            },
        );
        if inserted.is_err() {
            self.core.disconnect(id);
        }
    }

    /// Drops finished thumbnails and stacks the rest per output.
    fn tidy(&mut self) {
        let before = self.thumbs.len();
        self.thumbs.retain(|t| !t.is_finished());
        if self.thumbs.len() != before {
            self.restack();
            let _ = self.conn.flush();
        }
    }

    fn restack(&mut self) {
        let outputs: BTreeSet<String> = self.thumbs.iter().map(|t| t.output.clone()).collect();
        for output in outputs {
            let newest_first: Vec<usize> = (0..self.thumbs.len())
                .rev()
                .filter(|&i| self.thumbs[i].output == output)
                .collect();
            let heights: Vec<u32> = newest_first
                .iter()
                .map(|&i| self.thumbs[i].size.1)
                .collect();
            for (&i, margin) in newest_first.iter().zip(stack::bottom_margins(&heights)) {
                self.thumbs[i].set_margin(margin);
            }
        }
    }

    fn thumb(&mut self, surface: &WlSurface) -> Option<&mut Thumbnail> {
        self.thumbs.iter_mut().find(|t| t.is(surface))
    }

    pub fn configure(&mut self, layer: &LayerSurface, qh: &QueueHandle<State>) {
        if let Some(t) = self.thumbs.iter_mut().find(|t| t.is_layer(layer)) {
            t.configure(qh);
        }
    }

    pub fn frame_done(&mut self, surface: &WlSurface, qh: &QueueHandle<State>) {
        if let Some(t) = self.thumb(surface) {
            t.frame_done(qh);
        }
    }

    pub fn closed(&mut self, layer: &LayerSurface) {
        self.thumbs.retain(|t| !t.is_layer(layer));
    }

    pub fn pointer(
        &mut self,
        events: &[PointerEvent],
        cursor: Option<&WpCursorShapeDeviceV1>,
        qh: &QueueHandle<State>,
    ) {
        for event in events {
            let Some(t) = self.thumb(&event.surface) else {
                continue;
            };
            let (x, y) = event.position;
            match event.kind {
                PointerEventKind::Enter { serial } => {
                    if let Some(cursor) = cursor {
                        cursor.set_shape(serial, Shape::Pointer);
                    }
                }
                PointerEventKind::Press {
                    button: BTN_LEFT, ..
                } => t.press(x, y),
                PointerEventKind::Motion { .. } => t.motion(x, qh),
                PointerEventKind::Release {
                    button: BTN_LEFT, ..
                } => {
                    let action = t.release(x, y, qh);
                    if action == Some(Action::OpenEditor) {
                        open_editor(&t.path);
                    }
                }
                _ => {}
            }
        }
    }

    fn expire(&mut self, id: u64) {
        let qh = self.qh.clone();
        if let Some(t) = self.thumbs.iter_mut().find(|t| t.id == id) {
            t.expire(&qh);
        }
    }

    fn set_hidden(&mut self, hidden: bool) {
        let qh = self.qh.clone();
        for t in &mut self.thumbs {
            if hidden {
                t.hide(&qh);
            } else {
                t.show(&qh);
            }
        }
    }
}

fn disconnect(state: &mut State, id: u64) {
    let Some(host) = &mut state.preview else {
        return;
    };
    if host.core.disconnect(id) {
        host.set_hidden(false);
        let _ = host.conn.flush();
    }
}

fn handle(state: &mut State, id: u64, request: Request) -> Reply {
    match request {
        Request::Hide => {
            let host = state.preview.as_mut().expect("host state");
            if host.core.hide(id) {
                host.set_hidden(true);
                // The capture must not start until the compositor has
                // actually taken the thumbnails off the screen.
                if let Err(e) = host.conn.roundtrip() {
                    return Reply::error(format!("Wayland round trip failed: {e}"));
                }
            }
            Reply::ok()
        }
        Request::Add { path, output } => match add(state, &path, &output) {
            Ok(()) => Reply::ok(),
            Err(e) => {
                tracing::warn!("could not show a preview: {e:#}");
                Reply::error(format!("{e:#}"))
            }
        },
    }
}

fn add(state: &mut State, path: &Path, output_name: &str) -> Result<()> {
    let image = image::open(path)
        .with_context(|| format!("could not read {}", path.display()))?
        .to_rgba8();
    let outputs = state.output_list();
    let output = outputs
        .iter()
        .find(|o| o.geom.name == output_name)
        .or_else(|| outputs.first())
        .context("the compositor reported no outputs")?;
    let parts = Parts {
        compositor: &state.compositor,
        layer_shell: state
            .layer_shell
            .as_ref()
            .context("compositor has no wlr-layer-shell")?,
        viewporter: state
            .viewporter
            .as_ref()
            .context("compositor has no wp-viewporter")?,
        shm: &state.shm,
    };
    let host = state.preview.as_mut().expect("host state");
    host.next_id += 1;
    let id = host.next_id;
    let mut thumb = Thumbnail::new(id, path.to_path_buf(), &image, output, &parts, &host.qh)?;
    if host.core.hidden() {
        thumb.hide(&host.qh);
    }
    host.thumbs.push(thumb);

    // Beyond MAX on this output, the oldest slide away.
    let qh = host.qh.clone();
    let open: Vec<usize> = (0..host.thumbs.len())
        .filter(|&i| host.thumbs[i].output == output.geom.name && !host.thumbs[i].is_closing())
        .collect();
    for &i in open.iter().take(stack::excess(open.len())) {
        host.thumbs[i].slide_out(&qh);
    }
    host.restack();

    host.handle
        .insert_source(Timer::from_duration(host.timeout), move |_, _, state| {
            if let Some(host) = &mut state.preview {
                host.expire(id);
            }
            TimeoutAction::Drop
        })
        .map_err(|e| anyhow::anyhow!("could not start the preview timer: {e}"))?;
    let _ = host.conn.flush();
    Ok(())
}

/// Opens the screenshot in Satty; saving there overwrites the file.
fn open_editor(path: &Path) {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let mut command = Command::new("satty");
    command
        .arg("--filename")
        .arg(path)
        .arg("--output-filename")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid is async-signal-safe.
    unsafe {
        command.pre_exec(|| rustix::process::setsid().map(drop).map_err(Into::into));
    }
    if let Err(e) = command.spawn() {
        tracing::warn!("could not start satty: {e}");
    }
}
```

- [ ] **Step 4: Run the tests and watch them pass**

Run: `cargo nextest run host::`
Expected: all 4 tests in `host::tests` pass.

- [ ] **Step 5: Route Wayland events to the host**

Apply to `src/wayland.rs`:

```diff
--- a/src/wayland.rs
+++ b/src/wayland.rs
@@ -30,6 +30,7 @@
 use crate::capture::Pending;
 use crate::error::HintExt;
 use crate::frame::OutputGeom;
+use crate::host::Host;
 use crate::region::Overlay;
 
 /// An output as valw sees it.
@@ -66,6 +67,8 @@
     pub captures: Vec<Pending>,
     /// The region selection overlay, while it is open.
     pub overlay: Option<Overlay>,
+    /// The preview thumbnails, in the preview host process.
+    pub preview: Option<Host>,
 }
 
 impl Wayland {
@@ -93,6 +96,7 @@
             pointer: None,
             captures: Vec::new(),
             overlay: None,
+            preview: None,
         };
         // Two round trips: one for wl_output, one for the xdg-output details.
         queue
@@ -278,6 +282,9 @@
         if let Some(overlay) = &mut self.overlay {
             overlay.frame_done(surface, qh);
         }
+        if let Some(preview) = &mut self.preview {
+            preview.frame_done(surface, qh);
+        }
     }
 
     fn surface_enter(
@@ -312,9 +319,12 @@
 }
 
 impl LayerShellHandler for State {
-    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
+    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
         if let Some(overlay) = &mut self.overlay {
             overlay.cancel();
+        }
+        if let Some(preview) = &mut self.preview {
+            preview.closed(layer);
         }
     }
 
@@ -328,6 +338,9 @@
     ) {
         if let Some(overlay) = &mut self.overlay {
             overlay.configure(layer, configure.new_size, qh);
+        }
+        if let Some(preview) = &mut self.preview {
+            preview.configure(layer, qh);
         }
     }
 }
@@ -468,6 +481,9 @@
         if let Some(overlay) = &mut self.overlay {
             overlay.pointer(events, self.cursor_device.as_ref(), qh);
         }
+        if let Some(preview) = &mut self.preview {
+            preview.pointer(events, self.cursor_device.as_ref(), qh);
+        }
     }
 }
 
```

- [ ] **Step 6: Add the hidden subcommand**

Apply to `src/main.rs`. This also adds `mod host;` and a test that the subcommand parses but stays out of `--help`:

```diff
--- a/src/main.rs
+++ b/src/main.rs
@@ -7,6 +7,7 @@
 mod doctor;
 mod error;
 mod frame;
+mod host;
 mod ipc;
 mod lock;
 mod log;
@@ -59,6 +60,9 @@
     },
     /// Report what the compositor and system support.
     Doctor,
+    /// Internal: the process that shows preview thumbnails.
+    #[command(name = "__preview-host", hide = true)]
+    PreviewHost,
 }
 
 #[derive(Args)]
@@ -115,6 +119,7 @@
 fn run(cli: Cli) -> Result<()> {
     match cli.command {
         Command::Doctor => doctor::run(),
+        Command::PreviewHost => host::run(),
         Command::Screen { all, common } => capture(Mode::Screen { all }, common),
         Command::Region { common } => capture(Mode::Region, common),
     }
@@ -247,6 +252,13 @@
     }
 
     #[test]
+    fn preview_host_is_hidden_from_help() {
+        assert!(parses(&["__preview-host"]));
+        let help = Cli::command().render_help().to_string();
+        assert!(!help.contains("preview-host"), "{help}");
+    }
+
+    #[test]
     fn delay_values() {
         assert_eq!(parse_delay("1.5"), Ok(Duration::from_millis(1500)));
         assert_eq!(parse_delay("0"), Ok(Duration::ZERO));
```

- [ ] **Step 7: Lint and run the whole suite**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo nextest run`
Expected: no warnings; every test passes.

- [ ] **Step 8: Smoke-test the host on its own**

```sh
cargo build --release
V=$CARGO_TARGET_DIR/release/valw
time $V __preview-host; echo "exit=$?"
```

Expected: after about 2 s (nobody connected, so the startup grace period ends), `exit=0`. The newest log in `~/.local/state/valw/logs/` has `preview host listening on …/valw.sock` and `preview host done`, and `$XDG_RUNTIME_DIR/valw.sock` is gone afterwards.

```sh
$V __preview-host & sleep 0.3; $V __preview-host; echo "second exit=$?"; wait
```

Expected: `second exit=0` right away (the log says `another preview host is running`); the first exits about 2 s later.

- [ ] **Step 9: Commit**

```sh
jj commit -m "feat: preview host process"
```

---

### Task 6: Capture integration

**Files:**
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `ipc::{HideGuard, socket_path, start_host}` (Task 3); `config.preview` (Task 1); `host::run` via `__preview-host` (Task 5).
- Produces: `--no-preview` on `screen` and `region`; `save(shots, clip, target, config) -> Result<(Vec<Vec<u8>>, Option<PathBuf>)>`, which replaces Phase 1's `deliver`. The returned path is the canonical path of the clipboard shot when it went to a regular file, and it's what gets previewed.

The order in `capture` matters (spec section 5):
1. `HideGuard::new` right before the screencopy.
2. Capture (and region selection).
3. `save`.
4. `hide.add(…)` if the preview is enabled and a regular file was written.
5. `drop(hide)`, which shows the stack.
6. Only then the clipboard fork.

The previewed output is the anchor output for `region` and the focused output for `screen` and `--all`.

- [ ] **Step 1: Write the failing CLI tests and the integration**

Apply to `src/main.rs`. This removes the dead-code allowance, adds `--no-preview`, splits `deliver` into `save` plus the clipboard step, and wraps the capture in the `HideGuard`:

```diff
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,6 +1,3 @@
-// Phase 2 modules land before the CLI uses them; Task 6 removes this.
-#![allow(dead_code)]
-
 mod capture;
 mod config;
 mod detach;
@@ -79,6 +76,9 @@
     /// Include the mouse cursor.
     #[arg(long)]
     cursor: bool,
+    /// Don't show the preview thumbnail.
+    #[arg(long)]
+    no_preview: bool,
 }
 
 fn parse_delay(s: &str) -> Result<Duration, String> {
@@ -138,33 +138,57 @@
     }
     let outputs = wl.outputs();
     anyhow::ensure!(!outputs.is_empty(), "the compositor reported no outputs");
-
-    // (image, output name for the file name) and which one goes to the clipboard.
-    let (shots, clip): (Vec<(RgbaImage, Option<String>)>, usize) = match mode {
+    // Thumbnails from earlier captures must not end up in this one.
+    let mut hide = ipc::HideGuard::new(&ipc::socket_path());
+
+    // (image, output name for the file name), which one goes to the
+    // clipboard, and the output the preview appears on.
+    let (shots, clip, source): (Vec<(RgbaImage, Option<String>)>, usize, String) = match mode {
         Mode::Screen { all } => {
             let focused = focused_output(&outputs);
+            let source = outputs[focused].geom.name.clone();
             if all {
                 let frames = wl.capture(&outputs, cursor)?;
                 let shots = frames
                     .into_iter()
                     .map(|f| (f.to_rgba(f.full()), Some(f.output.name)))
                     .collect();
-                (shots, focused)
+                (shots, focused, source)
             } else {
                 let frame = wl.capture(&outputs[focused..=focused], cursor)?.remove(0);
-                (vec![(frame.to_rgba(frame.full()), None)], 0)
+                (vec![(frame.to_rgba(frame.full()), None)], 0, source)
             }
         }
         Mode::Region => {
             let frames = wl.capture(&outputs, cursor)?;
             let (i, r) = region::select(&mut wl, &outputs, &frames)?;
-            (vec![(frames[i].to_rgba(r), None)], 0)
+            let source = frames[i].output.name.clone();
+            (vec![(frames[i].to_rgba(r), None)], 0, source)
         }
     };
     drop(wl);
 
     let target = Target::from_args(common.output, common.clipboard_only);
-    deliver(shots, clip, &target, &config, lock)
+    let (pngs, saved) = save(&shots, clip, &target, &config)?;
+    if config.preview.enabled
+        && !common.no_preview
+        && let Some(file) = saved
+        && let Err(e) = hide.add(&ipc::socket_path(), &file, &source, ipc::start_host)
+    {
+        tracing::warn!("no preview: {e:#}");
+    }
+    // Show the thumbnails again before forking the clipboard server; the
+    // child would inherit this connection and keep them hidden.
+    drop(hide);
+
+    if target == Target::ClipboardOnly || config.save.copy_to_clipboard {
+        let png = pngs
+            .into_iter()
+            .nth(clip)
+            .context("no image for the clipboard")?;
+        output::copy_to_clipboard(png, lock)?;
+    }
+    Ok(())
 }
 
 /// Index of niri's focused output, or 0 if niri can't tell us.
@@ -186,45 +210,50 @@
         })
 }
 
-fn deliver(
-    shots: Vec<(RgbaImage, Option<String>)>,
+/// Encodes and writes the shots. Returns the PNGs and, when the clipboard
+/// shot went to a regular file, that file's absolute path for the preview.
+fn save(
+    shots: &[(RgbaImage, Option<String>)],
     clip: usize,
     target: &Target,
     config: &Config,
-    lock: Lock,
-) -> Result<()> {
+) -> Result<(Vec<Vec<u8>>, Option<PathBuf>)> {
     let now = Local::now();
     let pngs = shots
         .iter()
         .map(|(img, _)| output::encode_png(img))
         .collect::<Result<Vec<_>>>()?;
 
-    match target {
+    let saved = match target {
         Target::Default => {
             let dir = config.save_dir();
-            for (png, (_, name)) in pngs.iter().zip(&shots) {
+            let mut paths = Vec::new();
+            for (png, (_, name)) in pngs.iter().zip(shots) {
                 let file = output::file_name(&config.save.filename, &now, name.as_deref())?;
                 let path = output::unique_path(&dir, &file);
                 output::write_atomic(&path, png)?;
                 println!("{}", path.display());
+                paths.push(path);
             }
+            paths.into_iter().nth(clip)
         }
         Target::Path(path) => {
             output::write_atomic(path, &pngs[0])?;
             println!("{}", path.display());
-        }
-        Target::Stdout => output::write_stdout(&pngs[0])?,
-        Target::ClipboardOnly => {}
-    }
-
-    if *target == Target::ClipboardOnly || config.save.copy_to_clipboard {
-        let png = pngs
-            .into_iter()
-            .nth(clip)
-            .context("no image for the clipboard")?;
-        output::copy_to_clipboard(png, lock)?;
-    }
-    Ok(())
+            // No preview for devices and pipes (`-o /dev/null`).
+            std::fs::metadata(path)
+                .is_ok_and(|m| m.is_file())
+                .then(|| path.clone())
+        }
+        Target::Stdout => {
+            output::write_stdout(&pngs[0])?;
+            None
+        }
+        Target::ClipboardOnly => None,
+    };
+    // The host may run in another directory than this capture.
+    let saved = saved.map(|p| std::fs::canonicalize(&p).unwrap_or(p));
+    Ok((pngs, saved))
 }
 
 #[cfg(test)]
@@ -249,6 +278,8 @@
         assert!(!parses(&["screen", "--all", "-o", "-"]));
         assert!(!parses(&["screen", "--all", "--clipboard-only"]));
         assert!(!parses(&["region", "--clipboard-only", "-o", "a.png"]));
+        assert!(parses(&["region", "--no-preview"]));
+        assert!(parses(&["screen", "--all", "--no-preview"]));
     }
 
     #[test]
```

(The CLI tests and the code they test are one diff because clap's derive attributes are the implementation.)

- [ ] **Step 2: Run the tests**

Run: `cargo nextest run`
Expected: every test passes, including `tests::flag_conflicts` (now with `--no-preview`) and `tests::preview_host_is_hidden_from_help`.

- [ ] **Step 3: Lint without the allowance**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings`
Expected: no warnings, in particular no `dead_code`.

- [ ] **Step 4: Preview on niri**

Use a throwaway config so nothing touches the user's clipboard or Pictures. A thumbnail appears at the bottom right of the focused monitor for 4 s; say so before running.

```sh
t=$(mktemp -d); mkdir -p $t/config/valw
printf '[save]\ndirectory = "%s/shots"\ncopy_to_clipboard = false\n[preview]\ntimeout_secs = 4\n' $t > $t/config/valw/config.toml
export XDG_CONFIG_HOME=$t/config XDG_STATE_HOME=$t/state
cargo build --release; V=$CARGO_TARGET_DIR/release/valw
F=$(niri msg --json focused-output | jq -r .name)
read W H < <(niri msg --json focused-output | jq -r '"\(.modes[.current_mode].width) \(.modes[.current_mode].height)"')
corner() { magick "$1" -crop 260x160+$((W-260))+$((H-160)) +repage -format '%k' info:; }
$V screen >/dev/null; sleep 0.6
grim -o $F $t/a.png                               # thumbnail visible
$V screen --no-preview -o $t/b.png >/dev/null     # capture while it is up
sleep 0.5; grim -o $F $t/c.png                    # back after the capture
echo "before: $(corner $t/a.png)  valw: $(corner $t/b.png)  after: $(corner $t/c.png)"
```

Expected: `before` and `after` show many colours (the thumbnail), and `valw` shows the corner without it. If the corner of the screen itself is a single colour, that's `1`. Then:

```sh
sleep 5; ls $XDG_RUNTIME_DIR/valw.sock 2>/dev/null || echo "host gone"
```

Expected: `host gone`.

- [ ] **Step 5: Overlapping captures**

```sh
$V screen >/dev/null; sleep 0.5
$V screen --delay 1 --no-preview -o $t/d.png >/dev/null & sleep 0.2
$V screen --no-preview -o $t/e.png; echo "exit=$?"; wait
```

Expected: the second capture fails fast with `valw is already running` (`exit=1`, Phase 1's lock); the first finishes, and the thumbnail is back afterwards. Nothing stays hidden.

- [ ] **Step 6: The preview can't be shown (Review Focus 5)**

Point the runtime dir at a place where `valw.sock` is a directory, so no host can ever bind. Keep Wayland reachable with an absolute `WAYLAND_DISPLAY`:

```sh
r=$(mktemp -d); mkdir $r/valw.sock
# Compute the display path first: in `A=x B=$A cmd`, B already sees the new A.
wd=$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY
XDG_RUNTIME_DIR=$r WAYLAND_DISPLAY=$wd $V screen -o $t/f.png; echo "exit=$?"
grep -h "no preview" $t/state/valw/logs/*.log | tail -1
```

Expected: the path is printed, `exit=0` after about 1 s, and the log has `no preview: … the preview host did not come up`.

- [ ] **Step 7: Commit**

```sh
jj commit -m "feat: floating preview after captures"
```

---

### Task 7: Packaging and headless checks

**Files:**
- Modify: `nix/package.ulu.nix`, `nix/checks.ulu.nix`

**Interfaces:**
- Consumes: the whole preview flow (Tasks 1–6).
- Produces: `result/bin/valw` with Satty on its PATH; `checks.headless` extended with the preview scenarios from spec 10.2.

Notes:
- `--suffix PATH` keeps a user's own Satty first.
- The cargo checks replace `installPhase`, so `postInstall` (the wrapper) doesn't run for them, which is fine.
- The sandbox has no swaybg, so the headless background is solid black. A visible thumbnail therefore adds colours to the bottom-right corner. grim (which knows nothing about valw) must see those colours right before valw's own capture must not.

- [ ] **Step 1: Wrap the binary**

```diff
--- a/nix/package.ulu.nix
+++ b/nix/package.ulu.nix
@@ -21,7 +21,10 @@
 
         cargoLock.lockFile = ../Cargo.lock;
 
-        nativeBuildInputs = [ pkgs.pkg-config ];
+        nativeBuildInputs = [
+          pkgs.makeWrapper
+          pkgs.pkg-config
+        ];
         buildInputs = [
           pkgs.libxkbcommon
           pkgs.wayland
@@ -29,6 +32,11 @@
 
         # Tests run through `nix flake check` instead.
         doCheck = false;
+
+        # Clicking a preview opens Satty. --suffix keeps a user's own Satty first.
+        postInstall = ''
+          wrapProgram $out/bin/valw --suffix PATH : ${lib.makeBinPath [ pkgs.satty ]}
+        '';
 
         meta = {
           description = "macOS-style screenshots for niri";
```

- [ ] **Step 2: Extend the headless check**

```diff
--- a/nix/checks.ulu.nix
+++ b/nix/checks.ulu.nix
@@ -32,6 +32,8 @@
             {
               nativeBuildInputs = [
                 pkgs.file
+                pkgs.grim
+                pkgs.imagemagick
                 pkgs.sway-unwrapped
                 valw
               ];
@@ -63,6 +65,34 @@
               file shot.png | tee file.txt
               grep -q 'PNG image data, 1280 x 720' file.txt
 
+              # Preview. The background is solid black, so a thumbnail in the
+              # bottom-right corner shows up as more than one colour there.
+              mkdir -p $HOME/.config/valw
+              printf '[preview]\ntimeout_secs = 2\n' > $HOME/.config/valw/config.toml
+              socket=$XDG_RUNTIME_DIR/valw.sock
+              corner() { magick "$1" -crop 300x200+980+520 +repage -format '%k' info:; }
+              wait_for() { # $1: test expression, $2: tenths of a second
+                for _ in $(seq "$2"); do eval "$1" && return 0; sleep 0.1; done
+                echo "timed out waiting for: $1"
+                exit 1
+              }
+
+              valw screen > /dev/null
+              wait_for '[ -S $socket ]' 10
+              sleep 0.5
+              grim with-thumbnail.png
+              [ "$(corner with-thumbnail.png)" -gt 1 ] || { echo "thumbnail not visible"; exit 1; }
+              valw screen --no-preview -o hidden.png > /dev/null
+              [ "$(corner hidden.png)" -eq 1 ] || { echo "thumbnail ended up in a screenshot"; exit 1; }
+              wait_for '[ ! -S $socket ]' 40
+
+              valw screen --no-preview > /dev/null
+              sleep 1
+              if [ -S $socket ]; then
+                echo "--no-preview started a host"
+                exit 1
+              fi
+
               touch $out
             '';
       };
```

- [ ] **Step 3: Build and check Satty is reachable**

```sh
nix build && grep -c satty result/bin/valw
```

Expected: a non-zero count (the wrapper script mentions Satty's store path).

- [ ] **Step 4: Run every check**

Run: `nix flake check -L --keep-going`
Expected: all four checks pass. The `valw-headless` log ends without `thumbnail not visible`, `thumbnail ended up in a screenshot`, `timed out waiting for`, or `--no-preview started a host`.

- [ ] **Step 5: Format the Nix files**

Run: `nix fmt`
Expected: no changes, or whitespace only (rerun Step 4 if anything changed).

- [ ] **Step 6: Commit**

```sh
jj commit -m "build: satty on PATH; headless preview checks"
```

---

### Task 8: Wire valw into copland and start the test day

**Files (in `~/copland`, a separate jj repo; ask the user before touching it):**
- Modify: `flake.nix` (input), `hosts/copland.ulu.nix` (module list), `modules/home/niri/desktop-niri-keybinds.ulu.nix` (binds)
- Create: `modules/home/home-valw.ulu.nix`

**Interfaces:**
- Consumes: valw's `packages.<system>.default` (Task 7).
- Produces: `valw` on the user's PATH; `Mod+Shift+S` → `valw region`, `Print` → `valw screen`.

This is a change outside the valw repo, and the rebuild changes the running system. Show the user the four edits below and get an explicit go-ahead before making them. The rebuild itself is the user's to run.

- [ ] **Step 1: Flake input**

In `~/copland/flake.nix`, inside `inputs = { … }` after `scx_soryu`:

```nix
    valw = {
      url = "path:/home/elars/Projects/valw";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-parts.follows = "flake-parts";
    };
```

- [ ] **Step 2: A module that installs the package**

Create `~/copland/modules/home/home-valw.ulu.nix` (discovered automatically):

```nix
{ ... }:
{
  flake.homeModules.home-valw =
    { inputs, pkgs, ... }:
    {
      home.packages = [ inputs.valw.packages.${pkgs.stdenv.hostPlatform.system}.default ];
    };
}
```

In `~/copland/hosts/copland.ulu.nix`, add it to the host's home module list, next to `config.flake.homeModules.home-nyi`:

```nix
              config.flake.homeModules.home-valw
```

- [ ] **Step 3: Binds**

In `~/copland/modules/home/niri/desktop-niri-keybinds.ulu.nix`, replace

```nix
          "Mod+Shift+S".action.spawn-sh = "noctalia msg screenshot-region";
```

with

```nix
          "Mod+Shift+S".action.spawn = [ "valw" "region" ];
          "Print".action.spawn = [ "valw" "screen" ];
```

`Mod+Shift+3` and `Mod+Shift+4` stay workspace moves.

- [ ] **Step 4: Lock, check, commit, and hand over the rebuild**

```sh
cd ~/copland
nix flake lock
nix flake check --no-build
jj commit -m "valw: install and bind for the test day"
```

Expected: `flake.lock` gains a `valw` node, and the check evaluates cleanly. Then ask the user to run their usual rebuild. After it, `which valw` resolves to a Nix store path and `valw doctor` is all `ok`.

After later valw changes: `nix flake update valw` in copland, then rebuild.

- [ ] **Step 5: Test-day checklist (with the user)**

Ask the user to go through these once and report back:

- Thumbnail on the correct monitor; sharp at scale 1.25.
- Typing is never interrupted while a thumbnail is visible.
- Click opens Satty; saving there overwrites the screenshot (`ls -l` shows a new mtime).
- Swipe right closes it; a short drag snaps back.
- Several captures in a row stack up to 5; none appear in later screenshots.
- During region selection the thumbnails are hidden and not clickable.
- After the timeout, `pgrep -af __preview-host` finds nothing.
- **Review Focus 1:** start `valw region` while thumbnails are visible and press Esc, or `kill -9` it from another terminal. The thumbnails come back.
- **Review Focus 2:** while a thumbnail is visible, `kill -9` the host (`pgrep -f __preview-host`). The thumbnail disappears. The next Print still works and shows a new thumbnail.
- **Review Focus 4:** run an unwrapped build (`$CARGO_TARGET_DIR/release/valw screen`, which has no Satty on PATH unless the user installed it) and click the thumbnail. It closes, and the host log has `could not start satty`.

Then the test day starts: valw is the only screenshot tool for at least a day, and every bug or annoyance gets written down for the next round.
