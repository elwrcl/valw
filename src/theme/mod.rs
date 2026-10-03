//! valw's colours: palettes for the paint shader and the overlays, from
//! Noctalia's theme, from an image, or a built-in default.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use image::RgbaImage;

pub mod gl;

/// Three colours, dark to light, as linear 0–1 RGB.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub base: [f32; 3],
    pub body: [f32; 3],
    pub highlight: [f32; 3],
}

impl Palette {
    pub const DEFAULT: Palette = Palette {
        base: [0.106, 0.122, 0.165],
        body: [0.180, 0.227, 0.322],
        highlight: [0.561, 0.651, 0.788],
    };

    pub fn lerp(&self, other: &Palette, t: f32) -> Palette {
        let t = t.clamp(0.0, 1.0);
        let mix = |a: [f32; 3], b: [f32; 3]| [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t);
        Palette {
            base: mix(self.base, other.base),
            body: mix(self.body, other.body),
            highlight: mix(self.highlight, other.highlight),
        }
    }
}

pub fn hex(s: &str) -> Option<[f32; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let v = |i: usize| {
        u8::from_str_radix(&s[i..i + 2], 16)
            .ok()
            .map(|v| v as f32 / 255.0)
    };
    Some([v(0)?, v(2)?, v(4)?])
}

/// A Noctalia community palette file at `mode`.
pub fn from_noctalia_file(text: &str, mode: &str) -> Option<Palette> {
    let json: serde_json::Value = serde_json::from_str(text).ok()?;
    let m = json.get(mode)?;
    let get = |k: &str| hex(m.get(k)?.as_str()?);
    Some(Palette {
        base: get("mSurface")?,
        body: get("mSecondary")?,
        highlight: get("mPrimary")?,
    })
}

/// The dominant colours of `img`, dark to light. Icons skip transparent and
/// near-grey pixels so the brand colour wins.
pub fn from_image(img: &RgbaImage, icon: bool) -> Palette {
    let step = ((img.width() * img.height()) as usize / 4096).max(1);
    let mut buckets = std::collections::HashMap::<(u8, u8, u8), (u32, [u64; 3])>::new();
    for p in img.pixels().step_by(step) {
        let [r, g, b, a] = p.0;
        if icon {
            let (max, min) = (r.max(g).max(b), r.min(g).min(b));
            if a < 128 || max - min < 24 {
                continue;
            }
        }
        let e = buckets.entry((r >> 5, g >> 5, b >> 5)).or_default();
        e.0 += 1;
        for (s, v) in e.1.iter_mut().zip([r, g, b]) {
            *s += v as u64;
        }
    }
    let mut groups: Vec<_> = buckets.into_values().collect();
    groups.sort_by_key(|g| std::cmp::Reverse(g.0));
    let mut pick: Vec<[f32; 3]> = Vec::new();
    for (n, sum) in groups {
        let c = sum.map(|s| s as f32 / n as f32 / 255.0);
        if pick
            .iter()
            .all(|p| (0..3).map(|i| (p[i] - c[i]).abs()).sum::<f32>() > 0.3)
        {
            pick.push(c);
        }
        if pick.len() == 3 {
            break;
        }
    }
    match pick.len() {
        0 => return Palette::DEFAULT,
        1 => {
            let c = pick[0];
            pick = vec![c.map(|v| v * 0.45), c, c.map(|v| v + (1.0 - v) * 0.5)];
        }
        2 => {
            let c = pick[1];
            pick.push(c.map(|v| v + (1.0 - v) * 0.5));
        }
        _ => {}
    }
    pick.sort_by(|a, b| (a[0] + a[1] + a[2]).total_cmp(&(b[0] + b[1] + b[2])));
    Palette {
        base: pick[0],
        body: pick[1],
        highlight: pick[2],
    }
}

/// `noctalia msg <args>`'s output, or None (not installed, failing, or
/// slower than a second).
fn noctalia(args: &[&str]) -> Option<String> {
    let mut child = Command::new("noctalia")
        .arg("msg")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
    Some(out.trim().to_string())
}

/// Noctalia's current palette, or the default.
pub fn current() -> Palette {
    noctalia_palette().unwrap_or_else(|| {
        tracing::debug!("using the default palette");
        Palette::DEFAULT
    })
}

fn noctalia_palette() -> Option<Palette> {
    // Noctalia keeps the live choice in its state settings (theme-mode-set
    // writes there); its config holds the initial one. Reading the files is
    // instant, unlike `noctalia msg`, and this runs before every overlay.
    let state = state_dir().join("noctalia/settings.toml");
    let config = crate::config::home().join(".config/noctalia/config.toml");
    let (source, name, mode) = [state, config]
        .iter()
        .find_map(|p| theme_choice(&std::fs::read_to_string(p).ok()?))?;
    if source != "community" {
        return None;
    }
    let file = state_dir()
        .join("noctalia/community-palettes")
        .join(format!("{}.json", name.replace(' ', "%20")));
    from_noctalia_file(&std::fs::read_to_string(file).ok()?, &mode)
}

fn state_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::config::home().join(".local/state"))
}

/// `(source, community palette, mode)` from a Noctalia settings file.
fn theme_choice(text: &str) -> Option<(String, String, String)> {
    let doc: toml::Table = text.parse().ok()?;
    let theme = doc.get("theme")?.as_table()?;
    let get = |k: &str| theme.get(k)?.as_str().map(str::to_string);
    Some((
        get("source")?,
        get("community_palette").unwrap_or_default(),
        get("mode").unwrap_or_else(|| "dark".into()),
    ))
}

/// The wallpaper Noctalia shows on `connector`.
pub fn wallpaper(connector: &str) -> Option<PathBuf> {
    noctalia(&["wallpaper-get", connector])
        .map(PathBuf::from)
        .filter(|p| p.exists())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    const FILE: &str = r##"{"dark":{"mPrimary":"#cabaaa","mSecondary":"#73685F","mSurface":"#1e1d1b"},
                           "light":{"mPrimary":"#262524","mSecondary":"#736B5E","mSurface":"#D4C1A8"}}"##;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 0.01)
    }

    #[test]
    fn hex_parses() {
        assert!(close(hex("#ff8000").unwrap(), [1.0, 0.502, 0.0]));
        assert!(close(hex("D4C1A8").unwrap(), [0.831, 0.757, 0.659]));
        assert_eq!(hex("#12"), None);
    }

    #[test]
    fn noctalia_palette_per_mode() {
        let light = from_noctalia_file(FILE, "light").unwrap();
        assert!(close(light.base, hex("#D4C1A8").unwrap()));
        assert!(close(light.body, hex("#736B5E").unwrap()));
        assert!(close(light.highlight, hex("#262524").unwrap()));
        let dark = from_noctalia_file(FILE, "dark").unwrap();
        assert!(close(dark.base, hex("#1e1d1b").unwrap()));
        assert_eq!(from_noctalia_file(FILE, "sepia"), None);
        assert_eq!(from_noctalia_file("{}", "light"), None);
        assert_eq!(from_noctalia_file("not json", "light"), None);
    }

    #[test]
    fn image_palette_is_dark_to_light() {
        let img = RgbaImage::from_fn(30, 30, |x, _| match x / 10 {
            0 => Rgba([250, 240, 20, 255]),
            1 => Rgba([20, 30, 160, 255]),
            _ => Rgba([200, 30, 30, 255]),
        });
        let p = from_image(&img, false);
        let lum = |c: [f32; 3]| c[0] + c[1] + c[2];
        assert!(
            lum(p.base) < lum(p.body) && lum(p.body) < lum(p.highlight),
            "{p:?}"
        );
    }

    #[test]
    fn icon_palettes_ignore_transparent_and_grey() {
        let img = RgbaImage::from_fn(30, 30, |x, y| {
            if x < 15 {
                Rgba([0, 0, 0, 0])
            } else if y < 10 {
                Rgba([128, 128, 128, 255])
            } else {
                Rgba([30, 160, 220, 255])
            }
        });
        let p = from_image(&img, true);
        for c in [p.base, p.body, p.highlight] {
            assert!(c[2] > c[0], "blue family, not grey or black: {p:?}");
        }
    }

    #[test]
    fn theme_choice_from_settings() {
        let text = "[bar]\nx = 1\n\n[theme]\ncommunity_palette = \"Kemuri Susu\"\nmode = \"dark\"\nsource = \"community\"\n\n[wallpaper]\n";
        assert_eq!(
            theme_choice(text),
            Some(("community".into(), "Kemuri Susu".into(), "dark".into()))
        );
        assert_eq!(
            theme_choice("[theme]\nmode = \"dark\"\n"),
            None,
            "no source"
        );
        assert_eq!(theme_choice("not toml ["), None);
    }

    #[test]
    fn lerp_eases() {
        let a = Palette::DEFAULT;
        let b = Palette {
            base: [1.0; 3],
            body: [1.0; 3],
            highlight: [1.0; 3],
        };
        assert_eq!(a.lerp(&b, 0.0), a);
        assert_eq!(a.lerp(&b, 1.0), b);
    }
}
