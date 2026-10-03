//! The picker remembers which motion it used last, to alternate them.

use std::path::{Path, PathBuf};

pub const SWIRL: f32 = 1.0;
pub const FLOW: f32 = 0.0;

/// `$XDG_STATE_HOME/valw/picker`.
pub fn default_path() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::config::home().join(".local/state"))
        .join("valw/picker")
}

/// The motion for this run: the other one than last time (a swirl the
/// first time, or when the file is unreadable). Remembers it.
pub fn next_motion(path: &Path) -> f32 {
    let last = std::fs::read_to_string(path).ok();
    let next = match last.as_deref().map(str::trim) {
        Some("swirl") => FLOW,
        _ => SWIRL,
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, if next == SWIRL { "swirl\n" } else { "flow\n" });
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motion_alternates_and_starts_with_a_swirl() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deep/picker");
        assert_eq!(next_motion(&path), SWIRL);
        assert_eq!(next_motion(&path), FLOW);
        assert_eq!(next_motion(&path), SWIRL);
        std::fs::write(&path, "garbage").unwrap();
        assert_eq!(next_motion(&path), SWIRL);
    }
}
