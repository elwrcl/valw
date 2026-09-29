use anyhow::{Context, Result, anyhow};
use niri_ipc::{Request, Response, socket::Socket};

/// The niri release the pinned `niri-ipc` crate belongs to. Keep in sync
/// with the `niri-ipc = "=X.Y.0"` line in Cargo.toml (a test checks this).
pub const IPC_VERSION: (u32, u32) = (26, 4);

fn request(request: Request) -> Result<Response> {
    let mut socket =
        Socket::connect().context("could not connect to niri (is NIRI_SOCKET set?)")?;
    socket
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
