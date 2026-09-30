//! The line-based JSON protocol between captures and the preview host, and
//! its client side.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

/// How long a capture waits for the host to hide its thumbnails.
pub const HIDE_TIMEOUT: Duration = Duration::from_millis(300);
/// How long a capture waits for the host to load and show a thumbnail.
pub const ADD_TIMEOUT: Duration = Duration::from_secs(2);
/// How long a capture keeps retrying to reach a host it just started.
pub const START_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "lowercase")]
pub enum Request {
    /// Hide every thumbnail until this connection closes.
    Hide,
    /// Show a thumbnail for the screenshot at `path` on `output`.
    Add { path: PathBuf, output: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Reply {
    pub fn ok() -> Reply {
        Reply {
            ok: true,
            error: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Reply {
        Reply {
            ok: false,
            error: Some(message.into()),
        }
    }
}

/// One message as a JSON line.
pub fn encode<T: Serialize>(message: &T) -> String {
    let mut line = serde_json::to_string(message).expect("messages always serialize");
    line.push('\n');
    line
}

pub fn parse_request(line: &str) -> Result<Request, String> {
    serde_json::from_str(line.trim()).map_err(|e| format!("bad request: {e}"))
}

/// `$XDG_RUNTIME_DIR/valw.sock`, next to the capture lock.
pub fn socket_path() -> PathBuf {
    crate::lock::default_path().with_file_name("valw.sock")
}

/// Held by the running host so only one exists at a time.
pub fn host_lock_path() -> PathBuf {
    crate::lock::default_path().with_file_name("valw-preview.lock")
}

/// One request/reply connection to the host.
struct Conn {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Conn {
    fn new(stream: UnixStream) -> Result<Conn> {
        Ok(Conn {
            writer: stream
                .try_clone()
                .context("could not clone the host socket")?,
            reader: BufReader::new(stream),
        })
    }

    fn request(&mut self, request: &Request, timeout: Duration) -> Result<()> {
        self.writer
            .write_all(encode(request).as_bytes())
            .context("could not send to the preview host")?;
        self.reader.get_ref().set_read_timeout(Some(timeout))?;
        let mut line = String::new();
        self.reader
            .read_line(&mut line)
            .context("the preview host did not answer")?;
        if line.is_empty() {
            bail!("the preview host closed the connection");
        }
        let reply: Reply =
            serde_json::from_str(&line).context("bad reply from the preview host")?;
        match reply {
            Reply { ok: true, .. } => Ok(()),
            Reply { error, .. } => Err(anyhow!(error.unwrap_or_else(|| "unknown error".into()))),
        }
    }
}

/// Keeps the host's thumbnails hidden for as long as it lives, so they never
/// end up in a screenshot. Closing the connection, however the capture ends,
/// shows them again.
pub struct HideGuard {
    conn: Option<Conn>,
}

impl HideGuard {
    /// Asks a running host to hide. Without a host this does nothing; if the
    /// host doesn't answer in time the capture goes ahead anyway.
    pub fn new(socket: &Path) -> HideGuard {
        let Ok(stream) = UnixStream::connect(socket) else {
            return HideGuard { conn: None };
        };
        let conn = Conn::new(stream).and_then(|mut conn| {
            conn.request(&Request::Hide, HIDE_TIMEOUT)?;
            Ok(conn)
        });
        match conn {
            Ok(conn) => HideGuard { conn: Some(conn) },
            Err(e) => {
                tracing::warn!("capturing without hiding the previews: {e:#}");
                HideGuard { conn: None }
            }
        }
    }

    /// Shows a thumbnail for `path` on `output`. Uses this guard's
    /// connection if there is one, otherwise calls `start` to launch a host
    /// and connects to it.
    pub fn add(
        &mut self,
        socket: &Path,
        path: &Path,
        output: &str,
        start: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        let request = Request::Add {
            path: path.to_path_buf(),
            output: output.to_string(),
        };
        if let Some(conn) = &mut self.conn {
            return conn.request(&request, ADD_TIMEOUT);
        }
        let mut conn = connect_or_start(socket, start)?;
        conn.request(&request, ADD_TIMEOUT)
    }
}

fn connect_or_start(socket: &Path, start: impl FnOnce() -> Result<()>) -> Result<Conn> {
    if let Ok(stream) = UnixStream::connect(socket) {
        return Conn::new(stream);
    }
    start().context("could not start the preview host")?;
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        match UnixStream::connect(socket) {
            Ok(stream) => return Conn::new(stream),
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(e).context("the preview host did not come up"),
        }
    }
}

/// Starts `valw __preview-host` in its own session, detached from our stdio.
/// A fresh exec inherits nothing from the capture (its Wayland connection,
/// lock or threads).
pub fn start_host() -> Result<()> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let exe = std::env::current_exe().context("could not find the valw binary")?;
    let mut command = Command::new(exe);
    command
        .arg("__preview-host")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid is async-signal-safe.
    unsafe {
        command.pre_exec(|| rustix::process::setsid().map(drop).map_err(Into::into));
    }
    command
        .spawn()
        .context("could not run valw __preview-host")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    #[test]
    fn requests_as_json_lines() {
        assert_eq!(encode(&Request::Hide), "{\"cmd\":\"hide\"}\n");
        let add = Request::Add {
            path: "/tmp/a.png".into(),
            output: "HDMI-A-1".into(),
        };
        assert_eq!(
            encode(&add),
            "{\"cmd\":\"add\",\"path\":\"/tmp/a.png\",\"output\":\"HDMI-A-1\"}\n"
        );
        assert_eq!(parse_request(&encode(&add)), Ok(add));
    }

    #[test]
    fn replies_as_json_lines() {
        assert_eq!(encode(&Reply::ok()), "{\"ok\":true}\n");
        assert_eq!(
            encode(&Reply::error("no")),
            "{\"ok\":false,\"error\":\"no\"}\n"
        );
    }

    #[test]
    fn bad_requests_are_errors() {
        assert!(parse_request("nonsense").is_err());
        assert!(parse_request("{\"cmd\":\"launch\"}").is_err());
        assert!(parse_request("{\"cmd\":\"add\"}").is_err());
    }

    /// A fake host: answers every request with `reply` (or never, if None)
    /// and records the lines it received, plus "EOF" when the client leaves.
    fn fake_host(socket: &Path, reply: Option<Reply>) -> Arc<Mutex<Vec<String>>> {
        let listener = UnixListener::bind(socket).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        log.lock().unwrap().push("EOF".into());
                        break;
                    }
                    log.lock().unwrap().push(line.trim().to_string());
                    match &reply {
                        Some(r) => stream.write_all(encode(r).as_bytes()).unwrap(),
                        None => thread::sleep(Duration::from_secs(5)),
                    }
                }
            }
        });
        seen
    }

    fn wait_for(seen: &Arc<Mutex<Vec<String>>>, n: usize) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(2);
        while seen.lock().unwrap().len() < n && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        seen.lock().unwrap().clone()
    }

    #[test]
    fn no_host_means_no_hiding() {
        let dir = tempfile::tempdir().unwrap();
        let guard = HideGuard::new(&dir.path().join("valw.sock"));
        assert!(guard.conn.is_none());
    }

    #[test]
    fn stale_socket_is_ignored() {
        // A crashed host leaves its socket file behind with nobody listening.
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("valw.sock");
        drop(UnixListener::bind(&socket).unwrap());
        assert!(socket.exists());

        let started = Instant::now();
        let guard = HideGuard::new(&socket);
        assert!(guard.conn.is_none());
        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn guard_hides_until_dropped_and_adds_on_the_same_connection() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("valw.sock");
        let seen = fake_host(&socket, Some(Reply::ok()));

        let mut guard = HideGuard::new(&socket);
        assert!(guard.conn.is_some());
        guard
            .add(&socket, Path::new("/tmp/a.png"), "DP-1", || {
                panic!("host is running")
            })
            .unwrap();
        drop(guard);

        assert_eq!(
            wait_for(&seen, 3),
            vec![
                "{\"cmd\":\"hide\"}".to_string(),
                "{\"cmd\":\"add\",\"path\":\"/tmp/a.png\",\"output\":\"DP-1\"}".to_string(),
                "EOF".to_string(),
            ]
        );
    }

    #[test]
    fn silent_host_does_not_block_the_capture() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("valw.sock");
        let _seen = fake_host(&socket, None);

        let started = Instant::now();
        let guard = HideGuard::new(&socket);
        assert!(guard.conn.is_none());
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn add_starts_a_host_when_none_runs() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("valw.sock");
        let mut guard = HideGuard::new(&socket);

        let seen = Arc::new(Mutex::new(None));
        let slot = Arc::clone(&seen);
        let start_socket = socket.clone();
        guard
            .add(&socket, Path::new("/tmp/b.png"), "DP-1", move || {
                *slot.lock().unwrap() = Some(fake_host(&start_socket, Some(Reply::ok())));
                Ok(())
            })
            .unwrap();

        let seen = seen.lock().unwrap().clone().expect("start was called");
        assert_eq!(
            wait_for(&seen, 1)[0],
            "{\"cmd\":\"add\",\"path\":\"/tmp/b.png\",\"output\":\"DP-1\"}"
        );
    }

    #[test]
    fn host_errors_reach_the_caller() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("valw.sock");
        let _seen = fake_host(&socket, Some(Reply::error("could not read /x.png")));
        let mut guard = HideGuard { conn: None };
        let err = guard
            .add(&socket, Path::new("/x.png"), "DP-1", || Ok(()))
            .unwrap_err();
        assert_eq!(err.to_string(), "could not read /x.png");
    }

    #[test]
    fn host_that_never_comes_up_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("valw.sock");
        let mut guard = HideGuard { conn: None };
        let err = guard
            .add(&socket, Path::new("/x.png"), "DP-1", || Ok(()))
            .unwrap_err();
        assert_eq!(err.to_string(), "the preview host did not come up");
    }
}
