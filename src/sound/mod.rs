//! The combo shutter sound, played with PipeWire's `pw-play`.

pub mod combo;
pub mod synth;

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::config;

fn runtime_dir() -> PathBuf {
    crate::lock::default_path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir)
}

/// The WAV for shot `n`, written into `dir` the first time.
fn cached(dir: &Path, n: u8) -> Result<PathBuf> {
    let path = dir.join(format!("{n}.wav"));
    if !path.exists() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("could not create {}", dir.display()))?;
        crate::output::write_atomic(&path, &synth::wav(&synth::sound(n)))?;
    }
    Ok(path)
}

/// Plays the next sound of the combo, unless sounds are off. Never fails the
/// capture: problems are logged.
pub fn play(config: &config::Sound, enabled: bool) {
    if !enabled {
        return;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let state = runtime_dir().join("valw-combo");
    let n = combo::next(combo::load(&state), now, config.combo_reset_secs * 1000);
    if let Err(e) = combo::save(&state, n, now) {
        tracing::warn!("could not remember the combo: {e:#}");
    }
    if let Err(e) = spawn(n, config.volume) {
        tracing::warn!("no shutter sound: {e:#}");
    }
}

fn spawn(n: u8, volume: f32) -> Result<()> {
    use std::os::unix::process::CommandExt;
    let path = cached(&runtime_dir().join("valw-sound"), n)?;
    let mut command = std::process::Command::new("pw-play");
    command
        .arg(format!("--volume={volume}"))
        .arg(&path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // SAFETY: setsid is async-signal-safe.
    unsafe {
        command.pre_exec(|| rustix::process::setsid().map(drop).map_err(Into::into));
    }
    command.spawn().context("could not start pw-play")?;
    Ok(())
}

/// `valw __combo-demo`: the whole combo, for tuning by ear.
pub fn demo(config: &config::Sound) -> Result<()> {
    for n in [1, 2, 3, 4, 5, 6, 7, 6, 7] {
        spawn(n, config.volume)?;
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cache_holds_seven_wavs() {
        let dir = tempfile::tempdir().unwrap();
        for n in 1..=7 {
            let path = cached(dir.path(), n).unwrap();
            assert!(path.ends_with(format!("{n}.wav")));
            assert_eq!(&std::fs::read(&path).unwrap()[0..4], b"RIFF");
        }
    }
}
