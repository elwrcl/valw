use std::fmt;
use std::path::Path;

use anyhow::{Result, bail};

use crate::config;
use crate::niri;
use crate::wayland::Wayland;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub level: Level,
    pub text: String,
}

impl fmt::Display for Check {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let tag = match self.level {
            Level::Ok => "ok",
            Level::Warn => "warn",
            Level::Fail => "fail",
        };
        write!(f, "{tag:<5} {}", self.text)
    }
}

fn check(level: Level, text: impl Into<String>) -> Check {
    Check {
        level,
        text: text.into(),
    }
}

/// Globals valw can't work without: (interface, minimum version, used for).
const REQUIRED: [(&str, u32, &str); 7] = [
    ("wl_compositor", 1, "surfaces"),
    ("wl_shm", 1, "pixel buffers"),
    ("wl_seat", 1, "mouse and keyboard"),
    ("zxdg_output_manager_v1", 2, "output names and positions"),
    ("zwlr_layer_shell_v1", 1, "the selection overlay"),
    ("zwlr_screencopy_manager_v1", 1, "screen capture"),
    ("wp_viewporter", 1, "sharp overlays at fractional scale"),
];

/// Globals that enable a feature: (any of these interfaces, feature).
const OPTIONAL: [(&[&str], &str); 2] = [
    (
        &["wp_cursor_shape_manager_v1"],
        "crosshair cursor in region mode",
    ),
    (
        &[
            "ext_data_control_manager_v1",
            "zwlr_data_control_manager_v1",
        ],
        "clipboard copy",
    ),
];

pub fn check_globals(globals: &[(String, u32)]) -> Vec<Check> {
    let version = |name: &str| globals.iter().find(|(i, _)| i == name).map(|&(_, v)| v);
    let mut checks = Vec::new();
    for (name, min, purpose) in REQUIRED {
        checks.push(match version(name) {
            Some(v) if v >= min => check(Level::Ok, format!("{name} v{v} ({purpose})")),
            Some(v) => check(
                Level::Fail,
                format!("{name} v{v} is too old, need v{min} ({purpose})"),
            ),
            None => check(Level::Fail, format!("{name} missing ({purpose})")),
        });
    }
    for (names, feature) in OPTIONAL {
        checks.push(
            match names.iter().find_map(|&n| version(n).map(|v| (n, v))) {
                Some((n, v)) => check(Level::Ok, format!("{n} v{v} ({feature})")),
                None => check(
                    Level::Warn,
                    format!("{} missing ({feature} disabled)", names.join(" / ")),
                ),
            },
        );
    }
    checks
}

pub fn check_niri(version: Result<String>) -> Check {
    let (y, m) = niri::IPC_VERSION;
    match version {
        Err(e) => check(
            Level::Warn,
            format!("niri IPC unavailable ({e:#}); using the first output for `screen`"),
        ),
        Ok(v) => match niri::parse_version(&v) {
            Some(found) if found == niri::IPC_VERSION => check(Level::Ok, format!("niri {v}")),
            _ => check(
                Level::Warn,
                format!(
                    "niri {v}, but valw was built against niri-ipc {y}.{m}; IPC replies may not parse"
                ),
            ),
        },
    }
}

pub fn check_config(path: &Path) -> Check {
    match config::load(path) {
        Ok(_) if path.exists() => check(Level::Ok, format!("config {}", path.display())),
        Ok(_) => check(
            Level::Ok,
            format!("config {} not found, using defaults", path.display()),
        ),
        Err(e) => check(Level::Fail, format!("config: {e:#}")),
    }
}

pub fn check_save_dir(dir: &Path) -> Check {
    let probe = dir.join(".valw-doctor");
    let result = std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::write(&probe, b""))
        .and_then(|()| std::fs::remove_file(&probe));
    match result {
        Ok(()) => check(
            Level::Ok,
            format!("save directory {} is writable", dir.display()),
        ),
        Err(e) => check(
            Level::Fail,
            format!("save directory {}: {e}", dir.display()),
        ),
    }
}

/// Runs every check and prints one line each. Fails if any check failed.
pub fn run() -> Result<()> {
    let mut checks = Vec::new();
    match Wayland::connect() {
        Ok(wl) => {
            checks.extend(check_globals(&wl.global_list()));
            for o in wl.outputs() {
                let g = &o.geom;
                checks.push(check(
                    Level::Ok,
                    format!(
                        "output {}: {}x{} logical at {},{}, transform {:?}",
                        g.name, g.width, g.height, g.x, g.y, o.transform
                    ),
                ));
            }
        }
        Err(e) => checks.push(check(Level::Fail, format!("wayland: {e:#}"))),
    }
    checks.push(check_niri(niri::version()));
    let config_path = config::default_path();
    checks.push(check_config(&config_path));
    if let Ok(config) = config::load(&config_path) {
        checks.push(check_save_dir(&config.save_dir()));
    }

    for c in &checks {
        println!("{c}");
    }
    let failed = checks.iter().filter(|c| c.level == Level::Fail).count();
    if failed > 0 {
        bail!("{failed} check(s) failed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn globals(list: &[(&str, u32)]) -> Vec<(String, u32)> {
        list.iter().map(|&(n, v)| (n.to_string(), v)).collect()
    }

    fn niri_26_04() -> Vec<(String, u32)> {
        globals(&[
            ("wl_compositor", 6),
            ("wl_shm", 2),
            ("wl_seat", 9),
            ("zxdg_output_manager_v1", 3),
            ("zwlr_layer_shell_v1", 5),
            ("zwlr_screencopy_manager_v1", 3),
            ("wp_viewporter", 1),
            ("wp_cursor_shape_manager_v1", 2),
            ("zwlr_data_control_manager_v1", 2),
            ("ext_data_control_manager_v1", 1),
        ])
    }

    #[test]
    fn all_present_is_all_ok() {
        let checks = check_globals(&niri_26_04());
        assert!(checks.iter().all(|c| c.level == Level::Ok), "{checks:#?}");
        assert!(
            checks
                .iter()
                .any(|c| c.text == "ext_data_control_manager_v1 v1 (clipboard copy)")
        );
    }

    #[test]
    fn missing_required_fails_missing_optional_warns() {
        let list: Vec<_> = niri_26_04()
            .into_iter()
            .filter(|(n, _)| n != "zwlr_screencopy_manager_v1" && n != "wp_cursor_shape_manager_v1")
            .collect();
        let checks = check_globals(&list);
        assert!(checks.contains(&check(
            Level::Fail,
            "zwlr_screencopy_manager_v1 missing (screen capture)"
        )));
        assert!(checks.contains(&check(
            Level::Warn,
            "wp_cursor_shape_manager_v1 missing (crosshair cursor in region mode disabled)"
        )));
    }

    #[test]
    fn old_version_fails() {
        let mut list = niri_26_04();
        list.retain(|(n, _)| n != "zxdg_output_manager_v1");
        list.push(("zxdg_output_manager_v1".into(), 1));
        assert!(check_globals(&list).contains(&check(
            Level::Fail,
            "zxdg_output_manager_v1 v1 is too old, need v2 (output names and positions)"
        )));
    }

    #[test]
    fn niri_versions() {
        assert_eq!(check_niri(Ok("26.04 (Nixpkgs)".into())).level, Level::Ok);
        assert_eq!(check_niri(Ok("25.11".into())).level, Level::Warn);
        assert_eq!(
            check_niri(Err(anyhow::anyhow!("no socket"))).level,
            Level::Warn
        );
    }

    #[test]
    fn save_dir_checks() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(check_save_dir(&dir.path().join("new")).level, Level::Ok);
        let file = dir.path().join("file");
        std::fs::write(&file, b"").unwrap();
        assert_eq!(check_save_dir(&file).level, Level::Fail);
    }

    #[test]
    fn line_format() {
        assert_eq!(check(Level::Warn, "x").to_string(), "warn  x");
        assert_eq!(check(Level::Ok, "x").to_string(), "ok    x");
    }
}
