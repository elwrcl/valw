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
        assert!(
            p.to_string_lossy()
                .contains(&std::process::id().to_string())
        );
    }
}
