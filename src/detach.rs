use std::fs::File;
use std::io;

use anyhow::{Context, Result};

use crate::lock::Lock;

/// Runs `work` in a forked child that outlives valw, like `wl-copy` does.
/// The parent returns right away.
///
/// The child drops the capture lock (so the next capture isn't blocked) and
/// points stdio at /dev/null (so `valw ... -o - | x` sees EOF without waiting
/// for the clipboard to be replaced).
pub fn spawn(lock: Lock, work: impl FnOnce()) -> Result<()> {
    lock.release();
    // SAFETY: valw is single-threaded at this point, and the child only
    // touches its own state before exiting with _exit.
    match unsafe { libc::fork() } {
        -1 => Err(io::Error::last_os_error()).context("could not fork the clipboard server"),
        0 => {
            let _ = rustix::process::setsid();
            let _ = detach_stdio();
            work();
            // SAFETY: skips atexit handlers and destructors that belong to the parent.
            unsafe { libc::_exit(0) }
        }
        _ => Ok(()),
    }
}

fn detach_stdio() -> io::Result<()> {
    let null = File::options().read(true).write(true).open("/dev/null")?;
    rustix::stdio::dup2_stdin(&null)?;
    rustix::stdio::dup2_stdout(&null)?;
    rustix::stdio::dup2_stderr(&null)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_holds_neither_lock_nor_stdio() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("valw.lock");
        let lock = Lock::acquire(&path).unwrap();
        let (report_r, report_w) = rustix::pipe::pipe().unwrap();
        let (go_r, go_w) = rustix::pipe::pipe().unwrap();

        spawn(lock, move || {
            // Report whether stdout is /dev/null, then wait for the parent.
            let null = rustix::fs::stat("/dev/null").unwrap();
            let out = rustix::fs::fstat(rustix::stdio::stdout()).unwrap();
            let is_null = out.st_rdev == null.st_rdev && out.st_ino == null.st_ino;
            rustix::io::write(&report_w, &[is_null as u8]).unwrap();
            let _ = rustix::io::read(&go_r, &mut [0u8; 1]);
        })
        .unwrap();

        // The child is still running, yet the lock is free.
        Lock::acquire(&path).unwrap();

        let mut is_null = [0u8; 1];
        rustix::io::read(&report_r, &mut is_null).unwrap();
        assert_eq!(is_null, [1], "child stdout should be /dev/null");

        // Closing our end lets the child exit.
        drop(go_w);
    }
}
