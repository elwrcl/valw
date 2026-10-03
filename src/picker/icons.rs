//! Finding and loading a window's app icon: app id → desktop file →
//! `Icon=` → the icon theme (with its `Inherits`), then hicolor.

use std::path::{Path, PathBuf};

use image::RgbaImage;

/// Where to look.
#[derive(Debug, Clone)]
pub struct Dirs {
    /// `$XDG_DATA_HOME` then `$XDG_DATA_DIRS`.
    pub data: Vec<PathBuf>,
    /// The icon theme's directory name.
    pub theme: String,
}

impl Dirs {
    pub fn from_env() -> Dirs {
        let home = crate::config::home();
        let mut data = vec![
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".local/share")),
        ];
        let dirs =
            std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
        data.extend(dirs.split(':').filter(|d| !d.is_empty()).map(PathBuf::from));
        let theme = std::fs::read_to_string(home.join(".config/gtk-3.0/settings.ini"))
            .ok()
            .and_then(|t| {
                t.lines().find_map(|l| {
                    let (k, v) = l.split_once('=')?;
                    (k.trim() == "gtk-icon-theme-name").then(|| v.trim().to_string())
                })
            })
            .unwrap_or_else(|| "hicolor".into());
        Dirs { data, theme }
    }
}

const SIZES: [&str; 6] = ["scalable", "256x256", "128x128", "96x96", "64x64", "48x48"];

/// The icon file for `app_id`, if any.
pub fn find(app_id: &str, dirs: &Dirs) -> Option<PathBuf> {
    let icon = desktop_icon(app_id, dirs).unwrap_or_else(|| app_id.to_string());
    let path = Path::new(&icon);
    if path.is_absolute() {
        return path.exists().then(|| path.to_path_buf());
    }
    themed(&icon, dirs)
}

/// `Icon=` from the app's desktop file.
fn desktop_icon(app_id: &str, dirs: &Dirs) -> Option<String> {
    let wanted = app_id.to_lowercase();
    let mut by_class = None;
    for dir in &dirs.data {
        let apps = dir.join("applications");
        let exact = apps.join(format!("{app_id}.desktop"));
        if let Some(icon) = icon_line(&exact) {
            return Some(icon);
        }
        let Ok(entries) = std::fs::read_dir(&apps) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if path.extension().is_some_and(|e| e == "desktop") {
                if stem == wanted {
                    if let Some(icon) = icon_line(&path) {
                        return Some(icon);
                    }
                } else if by_class.is_none() {
                    let text = std::fs::read_to_string(&path).unwrap_or_default();
                    let class = text.lines().find_map(|l| l.strip_prefix("StartupWMClass="));
                    if class.is_some_and(|c| c.trim().to_lowercase() == wanted) {
                        by_class = icon_line(&path);
                    }
                }
            }
        }
    }
    by_class
}

fn icon_line(desktop: &Path) -> Option<String> {
    let text = std::fs::read_to_string(desktop).ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix("Icon="))
        .map(|i| i.trim().to_string())
        .filter(|i| !i.is_empty())
}

/// `name` in the theme, its parents, hicolor, then pixmaps.
fn themed(name: &str, dirs: &Dirs) -> Option<PathBuf> {
    let mut themes = vec![dirs.theme.clone()];
    let mut i = 0;
    while i < themes.len() && i < 8 {
        for parent in inherits(&themes[i], dirs) {
            if !themes.contains(&parent) {
                themes.push(parent);
            }
        }
        i += 1;
    }
    if !themes.iter().any(|t| t == "hicolor") {
        themes.push("hicolor".into());
    }
    for theme in &themes {
        for dir in &dirs.data {
            let root = dir.join("icons").join(theme);
            if !root.is_dir() {
                continue;
            }
            for size in SIZES {
                for ext in ["svg", "png"] {
                    let p = root.join(size).join("apps").join(format!("{name}.{ext}"));
                    if p.exists() {
                        return Some(p);
                    }
                }
            }
            // Other layouts: any <size>/apps or apps/<size>.
            if let Ok(entries) = std::fs::read_dir(&root) {
                for entry in entries.flatten() {
                    for sub in [entry.path().join("apps"), entry.path()] {
                        for ext in ["svg", "png"] {
                            let p = sub.join(format!("{name}.{ext}"));
                            if p.exists() {
                                return Some(p);
                            }
                        }
                    }
                }
            }
        }
    }
    dirs.data.iter().find_map(|dir| {
        ["svg", "png"]
            .iter()
            .map(|ext| dir.join("pixmaps").join(format!("{name}.{ext}")))
            .find(|p| p.exists())
    })
}

fn inherits(theme: &str, dirs: &Dirs) -> Vec<String> {
    dirs.data
        .iter()
        .find_map(|d| std::fs::read_to_string(d.join("icons").join(theme).join("index.theme")).ok())
        .and_then(|t| {
            t.lines()
                .find_map(|l| l.strip_prefix("Inherits=").map(str::to_string))
        })
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// The icon at `size` × `size` px (straight alpha).
pub fn load(path: &Path, size: u32) -> Option<RgbaImage> {
    if path.extension().is_some_and(|e| e == "svg") {
        let data = std::fs::read(path).ok()?;
        let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).ok()?;
        let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size)?;
        let s = tree.size();
        let k = size as f32 / s.width().max(s.height());
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(k, k),
            &mut pixmap.as_mut(),
        );
        let mut img = RgbaImage::new(size, size);
        for (dst, src) in img.pixels_mut().zip(pixmap.pixels()) {
            let c = src.demultiply();
            dst.0 = [c.red(), c.green(), c.blue(), c.alpha()];
        }
        Some(img)
    } else {
        let img = image::ImageReader::open(path)
            .ok()?
            .with_guessed_format()
            .ok()?
            .decode()
            .ok()?;
        Some(image::imageops::resize(
            &img.to_rgba8(),
            size,
            size,
            image::imageops::FilterType::Triangle,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn png(path: &std::path::Path, rgba: [u8; 4]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        image::RgbaImage::from_pixel(8, 8, image::Rgba(rgba))
            .save(path)
            .unwrap();
    }

    fn write(path: &std::path::Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#ff0000"/></svg>"##;

    #[test]
    fn finds_icons_through_desktop_files_and_themes() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("share");
        let dirs = Dirs {
            data: vec![data.clone()],
            theme: "mytheme".into(),
        };
        // By desktop file name, icon in hicolor.
        write(
            &data.join("applications/org.telegram.desktop.desktop"),
            "[Desktop Entry]\nName=Telegram\nIcon=telegram\n",
        );
        png(
            &data.join("icons/hicolor/256x256/apps/telegram.png"),
            [0, 0, 255, 255],
        );
        assert_eq!(
            find("org.telegram.desktop", &dirs),
            Some(data.join("icons/hicolor/256x256/apps/telegram.png"))
        );
        // By StartupWMClass, icon in a theme that inherits hicolor.
        write(
            &data.join("applications/foo.desktop"),
            "[Desktop Entry]\nIcon=foo-icon\nStartupWMClass=FooApp\n",
        );
        write(
            &data.join("icons/mytheme/index.theme"),
            "[Icon Theme]\nName=My\nInherits=hicolor\n",
        );
        png(
            &data.join("icons/hicolor/48x48/apps/foo-icon.png"),
            [0, 255, 0, 255],
        );
        assert_eq!(
            find("FooApp", &dirs),
            Some(data.join("icons/hicolor/48x48/apps/foo-icon.png"))
        );
        // The theme wins over hicolor, scalable over bitmaps.
        write(&data.join("icons/mytheme/scalable/apps/telegram.svg"), SVG);
        assert_eq!(
            find("org.telegram.desktop", &dirs),
            Some(data.join("icons/mytheme/scalable/apps/telegram.svg"))
        );
        // An absolute Icon= path.
        let abs = tmp.path().join("abs.png");
        png(&abs, [1, 2, 3, 255]);
        write(
            &data.join("applications/bar.desktop"),
            &format!("[Desktop Entry]\nIcon={}\n", abs.display()),
        );
        assert_eq!(find("bar", &dirs), Some(abs));
        // No desktop file: the app id itself as the icon name.
        png(
            &data.join("icons/hicolor/64x64/apps/kitty.png"),
            [9, 9, 9, 255],
        );
        assert_eq!(
            find("kitty", &dirs),
            Some(data.join("icons/hicolor/64x64/apps/kitty.png"))
        );
        assert_eq!(find("unknown.app", &dirs), None);
    }

    #[test]
    fn loads_svg_and_png_at_the_asked_size() {
        let tmp = tempfile::tempdir().unwrap();
        let svg = tmp.path().join("x.svg");
        fs::write(&svg, SVG).unwrap();
        let img = load(&svg, 48).unwrap();
        assert_eq!(img.dimensions(), (48, 48));
        assert_eq!(img.get_pixel(24, 24).0, [255, 0, 0, 255]);
        let p = tmp.path().join("x.png");
        png(&p, [0, 0, 255, 255]);
        let img = load(&p, 32).unwrap();
        assert_eq!(img.dimensions(), (32, 32));
        assert_eq!(img.get_pixel(5, 5).0, [0, 0, 255, 255]);
        assert!(load(&tmp.path().join("none.png"), 32).is_none());
    }
}
