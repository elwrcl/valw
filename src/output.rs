use std::fmt::Write as _;
use std::fs;
use std::io::{Cursor, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Local};
use image::{ImageFormat, RgbaImage};
use wl_clipboard_rs::copy::{MimeType, Options, Source};

use crate::error::HintExt;
use crate::lock::Lock;

/// Renders the strftime `template`. With `output`, appends ` (NAME)` before
/// the extension, for `valw screen --all`.
pub fn file_name(template: &str, now: &DateTime<Local>, output: Option<&str>) -> Result<String> {
    let mut name = String::new();
    write!(name, "{}", now.format(template))
        .map_err(|_| anyhow!("invalid strftime pattern in save.filename: {template:?}"))
        .hint("see https://docs.rs/chrono/latest/chrono/format/strftime for the syntax")?;
    if name.is_empty() || name.contains('/') {
        return Err(anyhow!(
            "save.filename must render to a plain file name, got {name:?}"
        ))
        .hint("put folders in save.directory, not save.filename");
    }
    if let Some(output) = output {
        name = with_suffix(&name, &format!(" ({output})"));
    }
    Ok(name)
}

/// Inserts `suffix` before the extension: `a.png` + ` (2)` = `a (2).png`.
fn with_suffix(name: &str, suffix: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem}{suffix}.{ext}"),
        _ => format!("{name}{suffix}"),
    }
}

/// `dir/name`, or `dir/name (2)`, `(3)`, … if taken.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|n| dir.join(with_suffix(name, &format!(" ({n})"))))
        .find(|p| !p.exists())
        .expect("some suffix is free")
}

pub fn encode_png(img: &RgbaImage) -> Result<Vec<u8>> {
    let mut png = Vec::new();
    img.write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .context("could not encode PNG")?;
    Ok(png)
}

/// Writes via a temp file in the same directory and a rename, so a partial
/// screenshot never appears under the final name.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    // Devices and pipes (`-o /dev/null`, a FIFO) can't be replaced by a
    // rename, and there is no partial file to protect; write them directly.
    if fs::metadata(path).is_ok_and(|m| !m.is_file() && !m.is_dir()) {
        return fs::write(path, bytes)
            .with_context(|| format!("could not write {}", path.display()));
    }
    // "a.png" has the parent "", which means the current directory.
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    fs::create_dir_all(dir)
        .with_context(|| format!("could not create {}", dir.display()))
        .hint("check save.directory in the config")?;
    let name = path.file_name().context("path has no file name")?;
    let tmp = dir.join(format!(".{}.tmp", name.to_string_lossy()));
    let result = fs::write(&tmp, bytes).and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.with_context(|| format!("could not write {}", path.display()))
}

pub fn write_stdout(bytes: &[u8]) -> Result<()> {
    let mut out = std::io::stdout().lock();
    out.write_all(bytes)
        .and_then(|()| out.flush())
        .context("could not write to stdout")
}

/// Copies `png` to the clipboard. The data is served by a forked child that
/// lives until something else is copied, so this consumes the lock.
pub fn copy_to_clipboard(png: Vec<u8>, lock: Lock) -> Result<()> {
    let mut options = Options::new();
    options.foreground(true);
    let prepared = options
        .prepare_copy(
            Source::Bytes(png.into()),
            MimeType::Specific("image/png".into()),
        )
        .context("could not copy to the clipboard")
        .hint("the compositor needs ext-data-control or wlr-data-control; see `valw doctor`")?;
    crate::detach::spawn(lock, move || {
        let _ = prepared.serve();
    })
}

/// Where a capture goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Save under the config's directory and filename template.
    Default,
    /// `--output <path>`.
    Path(PathBuf),
    /// `--output -`.
    Stdout,
    /// `--clipboard-only`.
    ClipboardOnly,
}

impl Target {
    /// clap rejects `--output` together with `--clipboard-only`.
    pub fn from_args(output: Option<PathBuf>, clipboard_only: bool) -> Target {
        match output {
            _ if clipboard_only => Target::ClipboardOnly,
            Some(p) if p.as_os_str() == "-" => Target::Stdout,
            Some(p) => Target::Path(p),
            None => Target::Default,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 9, 29, 19, 4, 5).unwrap()
    }

    #[test]
    fn default_template() {
        let name = file_name("Screenshot %Y-%m-%d at %H.%M.%S.png", &now(), None).unwrap();
        assert_eq!(name, "Screenshot 2026-09-29 at 19.04.05.png");
    }

    #[test]
    fn output_suffix() {
        let name = file_name("shot.png", &now(), Some("HDMI-A-1")).unwrap();
        assert_eq!(name, "shot (HDMI-A-1).png");
    }

    #[test]
    fn invalid_pattern_is_an_error_not_a_panic() {
        let err = file_name("shot %Q.png", &now(), None).unwrap_err();
        assert!(
            format!("{err:#}").contains("invalid strftime pattern"),
            "{err:#}"
        );
    }

    #[test]
    fn rejects_slashes_and_empty_names() {
        assert!(file_name("%Y/%m.png", &now(), None).is_err());
        assert!(file_name("", &now(), None).is_err());
    }

    #[test]
    fn suffix_placement() {
        assert_eq!(with_suffix("a.png", " (2)"), "a (2).png");
        assert_eq!(with_suffix("a.b.png", " (2)"), "a.b (2).png");
        assert_eq!(with_suffix("noext", " (2)"), "noext (2)");
        assert_eq!(with_suffix(".hidden", " (2)"), ".hidden (2)");
    }

    #[test]
    fn unique_path_counts_up() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(unique_path(dir.path(), "a.png"), dir.path().join("a.png"));
        fs::write(dir.path().join("a.png"), b"").unwrap();
        fs::write(dir.path().join("a (2).png"), b"").unwrap();
        assert_eq!(
            unique_path(dir.path(), "a.png"),
            dir.path().join("a (3).png")
        );
    }

    #[test]
    fn write_atomic_creates_dirs_and_leaves_no_temp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new/sub/a.png");
        write_atomic(&path, b"png").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"png");
        let names: Vec<_> = fs::read_dir(path.parent().unwrap()).unwrap().collect();
        assert_eq!(names.len(), 1);
    }

    #[test]
    fn write_atomic_writes_devices_directly() {
        // No temp file and rename for things that aren't regular files.
        write_atomic(Path::new("/dev/null"), b"png").unwrap();
    }

    #[test]
    fn write_atomic_reports_unwritable_dir() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file");
        fs::write(&file, b"").unwrap();
        let err = write_atomic(&file.join("a.png"), b"png").unwrap_err();
        assert!(format!("{err:#}").contains("could not create"), "{err:#}");
    }

    #[test]
    fn write_atomic_relative_path() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        write_atomic(Path::new("a.png"), b"png").unwrap();
        assert_eq!(fs::read(dir.path().join("a.png")).unwrap(), b"png");
    }

    #[test]
    fn png_round_trip() {
        let img = RgbaImage::from_raw(1, 1, vec![1, 2, 3, 255]).unwrap();
        let png = encode_png(&img).unwrap();
        let back = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(back, img);
    }

    #[test]
    fn targets() {
        use Target::*;
        assert_eq!(Target::from_args(None, false), Default);
        assert_eq!(Target::from_args(None, true), ClipboardOnly);
        assert_eq!(Target::from_args(Some("-".into()), false), Stdout);
        assert_eq!(
            Target::from_args(Some("a.png".into()), false),
            Path("a.png".into())
        );
    }
}
