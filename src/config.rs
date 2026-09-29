use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::error::HintExt;

#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub save: Save,
    pub capture: Capture,
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
    let config = serde_ignored::deserialize(de, |path| unknown.push(path.to_string()))?;
    Ok((config, unknown))
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
    }

    #[test]
    fn unknown_keys_are_reported_not_rejected() {
        let text = "[preview]\nenabled = true\n\n[save]\nfolder = \"x\"\n";
        let (config, unknown) = parse(text).unwrap();
        assert_eq!(config, Config::default());
        assert_eq!(
            unknown,
            vec!["preview".to_string(), "save.folder".to_string()]
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
