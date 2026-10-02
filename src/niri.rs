use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use niri_ipc::{Action, Event, Request, Response, Window, Workspace, socket::Socket};

/// The niri release the pinned `niri-ipc` crate belongs to. Keep in sync
/// with the `niri-ipc = "=X.Y.0"` line in Cargo.toml (a test checks this).
pub const IPC_VERSION: (u32, u32) = (26, 4);

fn connect() -> Result<Socket> {
    Socket::connect().context("could not connect to niri (is NIRI_SOCKET set?)")
}

fn request(request: Request) -> Result<Response> {
    connect()?
        .send(request)
        .context("niri IPC request failed")?
        .map_err(|e| anyhow!("niri replied with an error: {e}"))
}

/// Name of the output niri has focused, if any.
pub fn focused_output() -> Result<Option<String>> {
    match request(Request::FocusedOutput)? {
        Response::FocusedOutput(output) => Ok(output.map(|o| o.name)),
        other => Err(anyhow!("unexpected niri reply: {other:?}")),
    }
}

/// Lets the user click a window; niri shows its own pick cursor.
/// `None` if they pressed Esc.
pub fn pick_window() -> Result<Option<Window>> {
    match request(Request::PickWindow)? {
        Response::PickedWindow(window) => Ok(window),
        other => Err(anyhow!("unexpected niri reply: {other:?}")),
    }
}

pub fn workspaces() -> Result<Vec<Workspace>> {
    match request(Request::Workspaces)? {
        Response::Workspaces(workspaces) => Ok(workspaces),
        other => Err(anyhow!("unexpected niri reply: {other:?}")),
    }
}

/// Has niri render window `id` into `path` (absolute) and waits until the
/// file is written.
pub fn screenshot_window(id: u64, path: &Path, cursor: bool, timeout: Duration) -> Result<()> {
    // Listen before asking, so the event can't come and go unseen.
    let mut events = connect()?;
    events
        .send(Request::EventStream)
        .context("niri IPC request failed")?
        .map_err(|e| anyhow!("niri replied with an error: {e}"))?;
    let mut read = events.read_events();
    let (tx, rx) = mpsc::channel();
    let wanted = path.to_path_buf();
    std::thread::spawn(move || {
        while let Ok(event) = read() {
            if is_our_capture(&event, &wanted) {
                let _ = tx.send(());
                return;
            }
        }
    });

    let path = path
        .to_str()
        .context("the temporary screenshot path is not valid UTF-8")?;
    match request(Request::Action(Action::ScreenshotWindow {
        id: Some(id),
        write_to_disk: true,
        show_pointer: cursor,
        path: Some(path.to_owned()),
    }))? {
        Response::Handled => {}
        other => bail!("unexpected niri reply: {other:?}"),
    }
    rx.recv_timeout(timeout).map_err(|_| {
        anyhow!(
            "niri did not deliver the window screenshot within {}s",
            timeout.as_secs()
        )
    })
}

/// Whether `event` reports the screenshot written to `path`. niri reports
/// every screenshot on the stream, including its own bindings'.
pub fn is_our_capture(event: &Event, path: &Path) -> bool {
    matches!(event, Event::ScreenshotCaptured { path: Some(p) } if Path::new(p) == path)
}

/// The output showing the workspace a window is on, if niri knows it.
pub fn output_of(workspace_id: Option<u64>, workspaces: &[Workspace]) -> Option<String> {
    let id = workspace_id?;
    workspaces.iter().find(|w| w.id == id)?.output.clone()
}

/// niri's version string, e.g. "26.04 (Nixpkgs)".
pub fn version() -> Result<String> {
    match request(Request::Version)? {
        Response::Version(v) => Ok(v),
        other => Err(anyhow!("unexpected niri reply: {other:?}")),
    }
}

/// `(year, month)` from a niri version string like "26.04 (Nixpkgs)".
pub fn parse_version(v: &str) -> Option<(u32, u32)> {
    let (year, rest) = v.split_once('.')?;
    let month: String = rest.chars().take_while(char::is_ascii_digit).collect();
    Some((year.parse().ok()?, month.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws(id: u64, output: Option<&str>) -> Workspace {
        Workspace {
            id,
            idx: 1,
            name: None,
            output: output.map(str::to_owned),
            is_urgent: false,
            is_active: true,
            is_focused: false,
            active_window_id: None,
        }
    }

    #[test]
    fn only_our_capture_counts() {
        let ours = Path::new("/run/user/1000/valw-window-7.png");
        let captured = |p: Option<&str>| Event::ScreenshotCaptured {
            path: p.map(str::to_owned),
        };
        assert!(is_our_capture(
            &captured(Some("/run/user/1000/valw-window-7.png")),
            ours
        ));
        assert!(
            !is_our_capture(&captured(None), ours),
            "clipboard-only shot"
        );
        assert!(
            !is_our_capture(&captured(Some("/home/u/Pictures/Screenshot.png")), ours),
            "niri's own binding"
        );
        assert!(!is_our_capture(&Event::WindowClosed { id: 3 }, ours));
    }

    #[test]
    fn output_of_follows_the_workspace() {
        let all = [ws(1, Some("DP-1")), ws(2, Some("HDMI-A-1")), ws(3, None)];
        assert_eq!(output_of(Some(2), &all), Some("HDMI-A-1".to_owned()));
        assert_eq!(output_of(None, &all), None, "no workspace");
        assert_eq!(output_of(Some(9), &all), None, "unknown workspace");
        assert_eq!(output_of(Some(3), &all), None, "workspace without output");
    }

    #[test]
    fn parses_versions() {
        assert_eq!(parse_version("26.04 (Nixpkgs)"), Some((26, 4)));
        assert_eq!(parse_version("25.11"), Some((25, 11)));
        assert_eq!(parse_version("26.04.1-dev"), Some((26, 4)));
        assert_eq!(parse_version("unknown"), None);
    }

    #[test]
    fn ipc_version_matches_cargo_toml() {
        let (y, m) = IPC_VERSION;
        let pin = format!("niri-ipc = \"={y}.{m}.0\"");
        assert!(
            include_str!("../Cargo.toml").contains(&pin),
            "Cargo.toml should pin {pin}"
        );
    }
}
