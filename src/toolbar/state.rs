//! What the toolbar remembers between runs, kept out of the config so a
//! read-only (home-manager) config still works.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
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

/// One line of JSON for the Noctalia plugin: the state and the timer choices.
pub fn to_json(state: &ToolbarState) -> String {
    serde_json::json!({
        "mode": state.mode,
        "timer": state.timer,
        "cursor": state.cursor,
        "preview": state.preview,
        "sound": state.sound,
        "timers": TIMERS,
    })
    .to_string()
}

/// `state` with every `KEY=VALUE` applied in order; the first bad pair is
/// an error naming it.
pub fn apply_sets(mut state: ToolbarState, sets: &[String]) -> Result<ToolbarState> {
    for pair in sets {
        let Some((key, value)) = pair.split_once('=') else {
            bail!("expected KEY=VALUE, not {pair:?}");
        };
        let flag = || match value {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(anyhow::anyhow!(
                "{key} must be true or false, not {value:?}"
            )),
        };
        match key {
            "mode" => {
                state.mode = <Mode as clap::ValueEnum>::from_str(value, false).map_err(|_| {
                    anyhow::anyhow!("mode must be screen, window, region or zoom, not {value:?}")
                })?;
            }
            "timer" => {
                state.timer = value
                    .parse()
                    .ok()
                    .filter(|t| TIMERS.contains(t))
                    .with_context(|| format!("timer must be 0, 5 or 10, not {value:?}"))?;
            }
            "cursor" => state.cursor = flag()?,
            "preview" => state.preview = flag()?,
            "sound" => state.sound = flag()?,
            _ => bail!("unknown toolbar key {key:?} (mode, timer, cursor, preview or sound)"),
        }
    }
    Ok(state)
}

/// `valw toolbar --set`: `sets` applied to the remembered state and saved;
/// nothing changes if any pair is bad.
pub fn set(path: &Path, config: &Config, sets: &[String]) -> Result<()> {
    let state = apply_sets(load(path, config), sets)?;
    save(path, &state)
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

    #[test]
    fn json_for_the_plugin() {
        let state = ToolbarState {
            mode: Mode::Zoom,
            timer: 5,
            cursor: true,
            preview: false,
            sound: true,
        };
        let text = to_json(&state);
        assert!(!text.contains('\n'), "one line");
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "mode": "zoom",
                "timer": 5,
                "cursor": true,
                "preview": false,
                "sound": true,
                "timers": [0, 5, 10],
            })
        );
    }

    #[test]
    fn sets_apply_every_key_in_order() {
        let start = defaults(&Config::default());
        let sets: Vec<String> = [
            "mode=window",
            "timer=10",
            "cursor=true",
            "preview=false",
            "sound=false",
        ]
        .map(String::from)
        .into();
        assert_eq!(
            apply_sets(start, &sets).unwrap(),
            ToolbarState {
                mode: Mode::Window,
                timer: 10,
                cursor: true,
                preview: false,
                sound: false,
            }
        );
        let twice = ["timer=5".to_string(), "timer=0".to_string()];
        assert_eq!(
            apply_sets(start, &twice).unwrap().timer,
            0,
            "the last one wins"
        );
    }

    #[test]
    fn bad_sets_are_rejected_by_name() {
        let start = defaults(&Config::default());
        for (pair, message) in [
            ("colour=red", "unknown toolbar key \"colour\""),
            ("mode=video", "mode must be screen, window, region or zoom"),
            ("timer=7", "timer must be 0, 5 or 10"),
            ("timer=soon", "timer must be 0, 5 or 10"),
            ("cursor=maybe", "cursor must be true or false"),
            ("cursor", "expected KEY=VALUE"),
        ] {
            let err = format!("{:#}", apply_sets(start, &[pair.to_string()]).unwrap_err());
            assert!(err.contains(message), "{pair}: {err}");
        }
    }

    #[test]
    fn set_writes_all_or_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("valw/toolbar.toml");
        let config = Config::default();
        let bad = ["timer=5".to_string(), "mode=video".to_string()];
        assert!(set(&path, &config, &bad).is_err());
        assert!(!path.exists(), "nothing written");

        let good = ["timer=5".to_string(), "sound=false".to_string()];
        set(&path, &config, &good).unwrap();
        let state = load(&path, &config);
        assert_eq!(
            (state.timer, state.sound, state.mode),
            (5, false, Mode::Region)
        );

        std::fs::write(&path, "not toml [").unwrap();
        set(&path, &config, &["cursor=true".to_string()]).unwrap();
        assert_eq!(
            load(&path, &config),
            ToolbarState {
                cursor: true,
                ..defaults(&config)
            },
            "a broken file is replaced, starting from the defaults"
        );
    }
}
