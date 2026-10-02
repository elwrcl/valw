//! What the toolbar remembers between runs, kept out of the config so a
//! read-only (home-manager) config still works.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Screen,
    Window,
    Region,
    Zoom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ToolbarState {
    pub mode: Mode,
    /// Seconds before the capture: 0, 5 or 10.
    pub timer: u32,
    pub cursor: bool,
    pub preview: bool,
    pub sound: bool,
}

/// The file, every key optional.
#[derive(Deserialize)]
struct File {
    mode: Option<Mode>,
    timer: Option<u32>,
    cursor: Option<bool>,
    preview: Option<bool>,
    sound: Option<bool>,
}

pub const TIMERS: [u32; 3] = [0, 5, 10];

/// `$XDG_STATE_HOME/valw/toolbar.toml`, falling back to `~/.local/state`.
pub fn default_path() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::config::home().join(".local/state"))
        .join("valw/toolbar.toml")
}

pub fn defaults(config: &Config) -> ToolbarState {
    ToolbarState {
        mode: Mode::Region,
        timer: 0,
        cursor: config.capture.show_cursor,
        preview: config.preview.enabled,
        sound: config.sound.enabled,
    }
}

/// The remembered state, or the defaults if there is none or it is broken.
pub fn load(path: &Path, config: &Config) -> ToolbarState {
    let fallback = defaults(config);
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return fallback,
        Err(e) => {
            tracing::warn!("could not read {}: {e}", path.display());
            return fallback;
        }
    };
    match parse(&text, fallback) {
        Ok(state) => state,
        Err(e) => {
            tracing::warn!("ignoring {}: {e:#}", path.display());
            fallback
        }
    }
}

fn parse(text: &str, fallback: ToolbarState) -> Result<ToolbarState> {
    let file: File = toml::from_str(text)?;
    let timer = file.timer.unwrap_or(fallback.timer);
    if !TIMERS.contains(&timer) {
        bail!("timer must be 0, 5 or 10, not {timer}");
    }
    Ok(ToolbarState {
        mode: file.mode.unwrap_or(fallback.mode),
        timer,
        cursor: file.cursor.unwrap_or(fallback.cursor),
        preview: file.preview.unwrap_or(fallback.preview),
        sound: file.sound.unwrap_or(fallback.sound),
    })
}

pub fn save(path: &Path, state: &ToolbarState) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("could not create {}", dir.display()))?;
    }
    let text = toml::to_string(state).context("could not encode the toolbar state")?;
    crate::output::write_atomic(path, text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_come_from_the_config() {
        let mut config = Config::default();
        config.capture.show_cursor = true;
        config.preview.enabled = false;
        config.sound.enabled = false;
        assert_eq!(
            defaults(&config),
            ToolbarState {
                mode: Mode::Region,
                timer: 0,
                cursor: true,
                preview: false,
                sound: false,
            }
        );
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("deep/toolbar.toml");
        let state = ToolbarState {
            mode: Mode::Zoom,
            timer: 10,
            cursor: true,
            preview: true,
            sound: false,
        };
        save(&path, &state).unwrap();
        assert_eq!(load(&path, &Config::default()), state);
    }

    #[test]
    fn a_missing_file_gives_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::default();
        assert_eq!(
            load(&dir.path().join("none.toml"), &config),
            defaults(&config)
        );
    }

    #[test]
    fn a_broken_file_falls_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("toolbar.toml");
        let config = Config::default();
        for text in [
            "mode = \"video\"\n",
            "timer = 7\n",
            "not toml at all [",
            "cursor = \"yes\"\n",
        ] {
            std::fs::write(&path, text).unwrap();
            assert_eq!(load(&path, &config), defaults(&config), "{text:?}");
        }
    }

    #[test]
    fn missing_and_unknown_keys_are_fine() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("toolbar.toml");
        std::fs::write(&path, "mode = \"window\"\ncolour = \"red\"\n").unwrap();
        let config = Config::default();
        let state = load(&path, &config);
        assert_eq!(state.mode, Mode::Window);
        assert_eq!(state.timer, 0);
        assert_eq!(state.preview, config.preview.enabled);
    }
}
