use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::error::HintExt;

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub save: Save,
    pub capture: Capture,
    pub preview: Preview,
    pub zoom: Zoom,
    pub editor: Editor,
    pub sound: Sound,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Save {
    pub directory: String,
    pub filename: String,
    pub copy_to_clipboard: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(default)]
pub struct Capture {
    pub show_cursor: bool,
    /// A soft shadow around window shots, like macOS.
    pub window_shadow: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Preview {
    pub enabled: bool,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Zoom {
    pub scroll_step: f64,
    pub flashlight_radius: f64,
}

impl Default for Zoom {
    fn default() -> Self {
        Self {
            scroll_step: 1.15,
            flashlight_radius: 180.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(default)]
pub struct Editor {
    pub backend: Backend,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    /// `valw edit`.
    #[default]
    Builtin,
    Satty,
    /// Noctalia's annotator: it saves a new "annotated" file instead of
    /// overwriting the shot.
    Noctalia,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Sound {
    pub enabled: bool,
    /// 0.0–1.0.
    pub volume: f32,
    /// A pause this long starts the combo over.
    pub combo_reset_secs: u64,
}

impl Default for Sound {
    fn default() -> Self {
        Self {
            enabled: true,
            volume: 0.9,
            combo_reset_secs: 5,
        }
    }
}

impl Default for Preview {
    fn default() -> Self {
        Self {
            enabled: true,
            timeout_secs: 5,
        }
    }
}

impl Default for Save {
    fn default() -> Self {
        Self {
            directory: "~/Pictures/Screenshots".into(),
            filename: "Screenshot %Y-%m-%d at %H.%M.%S.png".into(),
            copy_to_clipboard: true,
        }
    }
}

impl Config {
    /// The save directory with `~` and `$HOME` expanded.
    pub fn save_dir(&self) -> PathBuf {
        expand_home(&self.save.directory, &home())
    }
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/".into())
}

/// `$XDG_CONFIG_HOME/valw/config.toml`, falling back to `~/.config`.
pub fn default_path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
        .join("valw/config.toml")
}

/// Loads the config. A missing file means defaults.
pub fn load(path: &Path) -> Result<Config> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(e).with_context(|| format!("could not read {}", path.display())),
    };
    let (config, unknown) = parse(&text)
        .with_context(|| format!("invalid config file {}", path.display()))
        .hint("fix the file or delete it to use the defaults")?;
    for key in unknown {
        tracing::warn!("ignoring unknown config key `{key}` in {}", path.display());
    }
    Ok(config)
}

/// Parses config text. Returns the config and the keys valw doesn't know.
pub fn parse(text: &str) -> Result<(Config, Vec<String>)> {
    let de = toml::Deserializer::parse(text)?;
    let mut unknown = Vec::new();
    let config: Config = serde_ignored::deserialize(de, |path| unknown.push(path.to_string()))?;
    if config.preview.timeout_secs == 0 {
        bail!("preview.timeout_secs must be at least 1");
    }
    if config.zoom.scroll_step <= 1.0 {
        bail!("zoom.scroll_step must be greater than 1");
    }
    if !(20.0..=2000.0).contains(&config.zoom.flashlight_radius) {
        bail!("zoom.flashlight_radius must be between 20 and 2000");
    }
    if !(0.0..=1.0).contains(&config.sound.volume) {
        bail!("sound.volume must be between 0 and 1");
    }
    if !(1..=60).contains(&config.sound.combo_reset_secs) {
        bail!("sound.combo_reset_secs must be between 1 and 60");
    }
    Ok((config, unknown))
}

/// Checks a config file strictly, for home-manager's build: invalid values
/// and unknown keys are errors (at run time unknown keys only warn).
pub fn check(path: &Path) -> Result<()> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("could not read {}", path.display()))?;
    let (_, unknown) =
        parse(&text).with_context(|| format!("invalid config file {}", path.display()))?;
    match unknown.len() {
        0 => Ok(()),
        1 => bail!("unknown config key: {}", unknown[0]),
        _ => bail!("unknown config keys: {}", unknown.join(", ")),
    }
}

pub fn expand_home(path: &str, home: &Path) -> PathBuf {
    if path == "~" || path == "$HOME" {
        return home.to_path_buf();
    }
    for prefix in ["~/", "$HOME/"] {
        if let Some(rest) = path.strip_prefix(prefix) {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_gives_defaults() {
        let (config, unknown) = parse("").unwrap();
        assert_eq!(config, Config::default());
        assert!(unknown.is_empty());
    }

    #[test]
    fn partial_file_keeps_other_defaults() {
        let (config, _) = parse("[save]\ncopy_to_clipboard = false\n").unwrap();
        assert!(!config.save.copy_to_clipboard);
        assert_eq!(config.save.directory, "~/Pictures/Screenshots");
        assert!(!config.capture.show_cursor);
        assert!(!config.capture.window_shadow);
    }

    #[test]
    fn unknown_keys_are_reported_not_rejected() {
        let text = "[toolbar]\nposition = \"top\"\n\n[save]\nfolder = \"x\"\n";
        let (config, unknown) = parse(text).unwrap();
        assert_eq!(config, Config::default());
        assert_eq!(
            unknown,
            vec!["save.folder".to_string(), "toolbar".to_string()]
        );
    }

    #[test]
    fn invalid_toml_names_the_line() {
        let err = parse("[save]\ndirectory = \n").unwrap_err();
        assert!(format!("{err:#}").contains("line 2"), "{err:#}");
    }

    #[test]
    fn wrong_type_is_an_error() {
        assert!(parse("[capture]\nshow_cursor = \"yes\"\n").is_err());
    }

    #[test]
    fn preview_defaults_and_overrides() {
        assert_eq!(
            Config::default().preview,
            Preview {
                enabled: true,
                timeout_secs: 5
            }
        );
        let (config, _) = parse("[preview]\ntimeout_secs = 2\n").unwrap();
        assert!(config.preview.enabled);
        assert_eq!(config.preview.timeout_secs, 2);
    }

    #[test]
    fn preview_timeout_must_be_positive() {
        let err = parse("[preview]\ntimeout_secs = 0\n").unwrap_err();
        assert_eq!(err.to_string(), "preview.timeout_secs must be at least 1");
        assert!(parse("[preview]\ntimeout_secs = -3\n").is_err());
    }

    #[test]
    fn zoom_defaults_overrides_and_bounds() {
        assert_eq!(
            Config::default().zoom,
            Zoom {
                scroll_step: 1.15,
                flashlight_radius: 180.0
            }
        );
        let (config, unknown) = parse("[zoom]\nscroll_step = 1.3\n").unwrap();
        assert!(unknown.is_empty());
        assert_eq!(config.zoom.scroll_step, 1.3);
        assert_eq!(config.zoom.flashlight_radius, 180.0);
        let err = parse("[zoom]\nscroll_step = 1.0\n").unwrap_err();
        assert_eq!(err.to_string(), "zoom.scroll_step must be greater than 1");
        let err = parse("[zoom]\nflashlight_radius = 5.0\n").unwrap_err();
        assert_eq!(
            err.to_string(),
            "zoom.flashlight_radius must be between 20 and 2000"
        );
    }

    #[test]
    fn editor_backend_default_and_values() {
        assert_eq!(Config::default().editor.backend, Backend::Builtin);
        let (config, _) = parse("[editor]\nbackend = \"satty\"\n").unwrap();
        assert_eq!(config.editor.backend, Backend::Satty);
        let (config, _) = parse("[editor]\nbackend = \"noctalia\"\n").unwrap();
        assert_eq!(config.editor.backend, Backend::Noctalia);
        assert!(parse("[editor]\nbackend = \"gimp\"\n").is_err());
    }

    #[test]
    fn sound_defaults_and_bounds() {
        let s = Config::default().sound;
        assert!(s.enabled);
        assert_eq!((s.volume, s.combo_reset_secs), (0.9, 5));
        for bad in ["volume = 1.5", "volume = nan", "volume = -0.1"] {
            let err = parse(&format!("[sound]\n{bad}\n")).unwrap_err();
            assert_eq!(
                err.to_string(),
                "sound.volume must be between 0 and 1",
                "{bad}"
            );
        }
        let err = parse("[sound]\ncombo_reset_secs = 0\n").unwrap_err();
        assert_eq!(
            err.to_string(),
            "sound.combo_reset_secs must be between 1 and 60"
        );
    }

    #[test]
    fn check_accepts_valid_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[preview]\ntimeout_secs = 3\n[sound]\nvolume = 0.4\n",
        )
        .unwrap();
        check(&path).unwrap();
        std::fs::write(&path, "").unwrap();
        check(&path).unwrap();
    }

    #[test]
    fn check_rejects_invalid_values_with_valws_message() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[sound]\nvolume = 2.0\n").unwrap();
        let err = format!("{:#}", check(&path).unwrap_err());
        assert!(
            err.contains("sound.volume must be between 0 and 1"),
            "{err}"
        );
    }

    #[test]
    fn check_rejects_unknown_keys_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[preview]\ntimout_secs = 3\n").unwrap();
        assert_eq!(
            check(&path).unwrap_err().to_string(),
            "unknown config key: preview.timout_secs"
        );
        std::fs::write(&path, "[preview]\ntimout_secs = 3\n[sund]\nx = 1\n").unwrap();
        assert_eq!(
            check(&path).unwrap_err().to_string(),
            "unknown config keys: preview.timout_secs, sund"
        );
    }

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            load(&dir.path().join("nope.toml")).unwrap(),
            Config::default()
        );
    }

    #[test]
    fn expands_home() {
        let home = Path::new("/home/u");
        assert_eq!(
            expand_home("~/Pictures", home),
            Path::new("/home/u/Pictures")
        );
        assert_eq!(
            expand_home("$HOME/Pictures", home),
            Path::new("/home/u/Pictures")
        );
        assert_eq!(expand_home("~", home), Path::new("/home/u"));
        assert_eq!(expand_home("/srv/shots", home), Path::new("/srv/shots"));
        assert_eq!(expand_home("~other/x", home), Path::new("~other/x"));
    }
}
