//! Drag-and-drop out of a preview thumbnail: what is offered, how it is
//! written, and what the thumbnail does afterwards. No Wayland in here.

use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail};

/// What a drag offers; the receiving app picks one. File managers and chat
/// apps take the file, image editors take the pixels.
pub const MIME_TYPES: [&str; 2] = ["text/uri-list", "image/png"];

/// `file://` URI for an absolute path, with everything but RFC 3986
/// unreserved characters and `/` percent-encoded byte by byte. The default
/// file names have spaces; Turkish names have non-ASCII.
pub fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for &b in path.as_os_str().as_encoded_bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

/// Writes the screenshot at `path` in the requested `mime` type.
pub fn write_offer(mime: &str, path: &Path, out: &mut impl Write) -> Result<()> {
    match mime {
        "text/uri-list" => out.write_all(format!("{}\r\n", file_uri(path)).as_bytes())?,
        "image/png" => {
            let bytes = std::fs::read(path)
                .with_context(|| format!("could not read {}", path.display()))?;
            out.write_all(&bytes)?;
        }
        other => bail!("nobody offered {other}"),
    }
    out.flush()?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Dropped on an app that took it.
    Dropped,
    /// Released on nothing, rejected, or Esc.
    Cancelled,
}

/// What the thumbnail does once its drag is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterDrag {
    Close,
    ComeBack,
    SlideOut,
}

/// `expired`: the thumbnail's timer fired while it was being dragged.
pub fn after_drag(outcome: Outcome, expired: bool) -> AfterDrag {
    match (outcome, expired) {
        (Outcome::Dropped, _) => AfterDrag::Close,
        (Outcome::Cancelled, false) => AfterDrag::ComeBack,
        (Outcome::Cancelled, true) => AfterDrag::SlideOut,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn plain_path() {
        assert_eq!(file_uri(Path::new("/tmp/a.png")), "file:///tmp/a.png");
    }

    #[test]
    fn spaces_are_encoded() {
        assert_eq!(
            file_uri(Path::new("/home/u/Screenshot 2026-10-01 at 14.03.22.png")),
            "file:///home/u/Screenshot%202026-10-01%20at%2014.03.22.png"
        );
    }

    #[test]
    fn non_ascii_is_encoded_as_utf8() {
        assert_eq!(
            file_uri(Path::new("/tmp/ışık.png")),
            "file:///tmp/%C4%B1%C5%9F%C4%B1k.png"
        );
    }

    #[test]
    fn percent_and_hash_are_encoded() {
        assert_eq!(
            file_uri(Path::new("/tmp/50%#1.png")),
            "file:///tmp/50%25%231.png"
        );
    }

    /// Writes through a real pipe, as a receiving app would read it.
    fn through_pipe(mime: &str, path: &Path) -> Result<Vec<u8>> {
        let (reader, writer) = rustix::pipe::pipe()?;
        let mut writer = std::fs::File::from(writer);
        let path = path.to_path_buf();
        let mime = mime.to_string();
        let handle = std::thread::spawn(move || write_offer(&mime, &path, &mut writer));
        let mut out = Vec::new();
        std::fs::File::from(reader).read_to_end(&mut out)?;
        handle.join().unwrap()?;
        Ok(out)
    }

    #[test]
    fn uri_list_is_one_crlf_line() {
        let out = through_pipe("text/uri-list", Path::new("/tmp/a b.png")).unwrap();
        assert_eq!(out, b"file:///tmp/a%20b.png\r\n");
    }

    #[test]
    fn png_is_the_file_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        // Larger than a pipe buffer, so the writer really has to wait.
        let bytes: Vec<u8> = (0..200_000u32).map(|i| i as u8).collect();
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(through_pipe("image/png", &path).unwrap(), bytes);
    }

    #[test]
    fn missing_file_is_an_error_not_a_panic() {
        let err = through_pipe("image/png", Path::new("/nonexistent/x.png")).unwrap_err();
        assert!(
            format!("{err:#}").contains("could not read /nonexistent/x.png"),
            "{err:#}"
        );
    }

    #[test]
    fn unknown_mime_is_an_error() {
        assert!(through_pipe("text/html", Path::new("/tmp/a.png")).is_err());
    }

    #[test]
    fn after_drag_decisions() {
        assert_eq!(after_drag(Outcome::Dropped, false), AfterDrag::Close);
        assert_eq!(after_drag(Outcome::Dropped, true), AfterDrag::Close);
        assert_eq!(after_drag(Outcome::Cancelled, false), AfterDrag::ComeBack);
        assert_eq!(after_drag(Outcome::Cancelled, true), AfterDrag::SlideOut);
    }
}
