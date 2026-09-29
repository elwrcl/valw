use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rustix::fs::{FlockOperation, flock};
use rustix::io::Errno;

use crate::error::HintExt;

/// Held for the whole capture so two captures never overlap. The kernel
/// drops the lock when the process dies, so a crash can't leave it stuck.
#[derive(Debug)]
pub struct Lock {
    _file: File,
}

/// `$XDG_RUNTIME_DIR/valw.lock`, falling back to the temp dir.
pub fn default_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("valw.lock")
}

impl Lock {
    pub fn acquire(path: &Path) -> Result<Lock> {
        let file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)
            .with_context(|| format!("could not open lock file {}", path.display()))?;
        match flock(&file, FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => Ok(Lock { _file: file }),
            Err(Errno::WOULDBLOCK) => Err(anyhow::anyhow!("valw is already running"))
                .hint("wait for the other capture to finish, or press Esc in it"),
            Err(e) => {
                bail!("could not lock {}: {e}", path.display())
            }
        }
    }

    /// Releases the lock. Must happen before forking, or the child would
    /// inherit the file and keep the lock held.
    pub fn release(self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_fails_until_release() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("valw.lock");

        let first = Lock::acquire(&path).unwrap();
        let err = Lock::acquire(&path).unwrap_err();
        assert_eq!(
            err.to_string(),
            "wait for the other capture to finish, or press Esc in it"
        );
        assert_eq!(err.root_cause().to_string(), "valw is already running");

        first.release();
        Lock::acquire(&path).unwrap();
    }

    #[test]
    fn stale_lock_file_is_fine() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("valw.lock");
        File::create(&path).unwrap();
        Lock::acquire(&path).unwrap();
    }
}
