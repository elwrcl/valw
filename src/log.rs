use std::fs::{self, File};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use tracing::Level;
use tracing_subscriber::filter::{EnvFilter, filter_fn};
use tracing_subscriber::prelude::*;

use crate::error::{Hint, render};

/// Number of run logs kept on disk, including the current one.
const KEEP: usize = 10;

static START: OnceLock<Instant> = OnceLock::new();

/// Time since logging was set up, i.e. roughly since the process started.
pub fn since_start() -> Duration {
    START.get_or_init(Instant::now).elapsed()
}

/// `$XDG_STATE_HOME/valw/logs`, falling back to `~/.local/state/valw/logs`.
pub fn default_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::config::home().join(".local/state"))
        .join("valw/logs")
}

/// Sets up tracing: this run's log file at `VALW_LOG` level (default info),
/// plus warnings and errors on stderr. Returns the log file path.
///
/// If the log file can't be created, logging falls back to stderr only and
/// the returned path is `None`; a broken log dir must not block a screenshot.
pub fn init(dir: &Path, now: DateTime<Local>) -> Option<PathBuf> {
    START.get_or_init(Instant::now);
    let file = open_run_log(dir, now);

    // Errors reach stderr through error::render, so only warnings go here.
    let stderr = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .without_time()
        .with_target(false)
        .with_filter(filter_fn(|m| *m.level() == Level::WARN));

    let (file_layer, path, failure) = match file {
        Ok((file, path)) => {
            let filter =
                EnvFilter::try_from_env("VALW_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
            let layer = tracing_subscriber::fmt::layer()
                .with_writer(Mutex::new(file))
                .with_ansi(false)
                .with_filter(filter);
            (Some(layer), Some(path), None)
        }
        Err(e) => (None, None, Some(e)),
    };

    tracing_subscriber::registry()
        .with(stderr)
        .with(file_layer)
        .init();

    if let Some(e) = failure {
        tracing::warn!("logging to stderr only: {e:#}");
    }
    path
}

fn open_run_log(dir: &Path, now: DateTime<Local>) -> Result<(File, PathBuf)> {
    fs::create_dir_all(dir).with_context(|| format!("could not create {}", dir.display()))?;
    let path = dir.join(log_name(now));
    let file =
        File::create(&path).with_context(|| format!("could not create {}", path.display()))?;
    prune(dir, KEEP)?;
    Ok((file, path))
}

/// Log names sort chronologically, so pruning can compare names.
fn log_name(now: DateTime<Local>) -> String {
    now.format("%Y-%m-%dT%H-%M-%S%.3f.log").to_string()
}

/// Deletes all but the `keep` newest `.log` files in `dir`.
fn prune(dir: &Path, keep: usize) -> Result<()> {
    let mut logs: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "log"))
        .collect();
    logs.sort();
    let excess = logs.len().saturating_sub(keep);
    for old in &logs[..excess] {
        fs::remove_file(old)?;
    }
    Ok(())
}

/// Sends panics to the log and prints them in the usual error format.
pub fn install_panic_hook(log: Option<PathBuf>) {
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!("panic: {info}\n{backtrace}");
        let err = anyhow::anyhow!("{info}").context(Hint(
            "this is a bug in valw; please report it with the log".into(),
        ));
        eprint!("{}", render(&err, log.as_deref()));
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn log_names_sort_by_time() {
        let a = Local.with_ymd_and_hms(2026, 9, 29, 9, 5, 1).unwrap();
        let b = Local.with_ymd_and_hms(2026, 9, 29, 19, 0, 0).unwrap();
        assert_eq!(log_name(a), "2026-09-29T09-05-01.000.log");
        assert!(log_name(a) < log_name(b));
    }

    #[test]
    fn prune_keeps_newest() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..12 {
            File::create(dir.path().join(format!("2026-09-29T10-00-{i:02}.000.log"))).unwrap();
        }
        File::create(dir.path().join("notes.txt")).unwrap();

        prune(dir.path(), 10).unwrap();

        let mut left: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(left.len(), 11);
        assert_eq!(left[0], "2026-09-29T10-00-02.000.log");
        assert!(left.contains(&"notes.txt".to_string()));
    }

    #[test]
    fn open_run_log_fails_on_unwritable_dir() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("file");
        File::create(&blocker).unwrap();
        // A regular file where the log dir should be.
        assert!(open_run_log(&blocker, Local::now()).is_err());
    }
}
