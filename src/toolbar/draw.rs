//! Painting the toolbar, its menu and the countdown pill with tiny-skia.

use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};

use crate::editor::text;
use crate::toolbar::layout::{Layout, MenuItem, Rect, Target};
use crate::toolbar::state::ToolbarState;

pub const LABEL: f32 = 13.0;
const RADIUS: f32 = 12.0;
const BACKGROUND: [u8; 4] = [0x1c, 0x1c, 0x1e, 230];
const SELECTED: [u8; 4] = [255, 255, 255, 46];
const HOVER: [u8; 4] = [255, 255, 255, 26];
const SEPARATOR: [u8; 4] = [255, 255, 255, 60];
const WHITE: [u8; 4] = [255, 255, 255, 255];

/// A label's width in logical px.
pub fn measure(label: &str) -> f32 {
    text::layout(label, LABEL).width
}

fn paint(c: [u8; 4]) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], c[3]);
    p.anti_alias = true;
    p
}

pub(crate) fn rounded(pixmap: &mut Pixmap, r: Rect, radius: f32, color: [u8; 4], scale: f32) {
    let (x, y, w, h, k) = (
        r.x * scale,
        r.y * scale,
        r.w * scale,
        r.h * scale,
        radius * scale,
    );
    let mut pb = PathBuilder::new();
    pb.move_to(x + k, y);
    pb.line_to(x + w - k, y);
    pb.quad_to(x + w, y, x + w, y + k);
    pb.line_to(x + w, y + h - k);
    pb.quad_to(x + w, y + h, x + w - k, y + h);
    pb.line_to(x + k, y + h);
    pb.quad_to(x, y + h, x, y + h - k);
    pb.line_to(x, y + k);
    pb.quad_to(x, y, x + k, y);
    pb.close();
    if let Some(path) = pb.finish() {
        pixmap.fill_path(
            &path,
            &paint(color),
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

/// `label` centred vertically in `r`, starting `left` logical px in.
fn label(pixmap: &mut Pixmap, r: Rect, left: f32, s: &str, scale: f32) {
    let size = LABEL * scale;
    let height = text::line_height(size);
    let origin = (
        (r.x + left) * scale,
        (r.y + r.h / 2.0) * scale - height / 2.0,
    );
    if let Some(path) = text::path(s, size, origin) {
        pixmap.fill_path(
            &path,
            &paint(WHITE),
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

/// The whole output: transparent but for the bar and, if open, the menu.
pub fn bar(
    layout: &Layout,
    size: (u32, u32),
    scale: f32,
    state: &ToolbarState,
    hover: Option<Target>,
    menu_open: bool,
) -> Pixmap {
    let mut p = Pixmap::new(size.0.max(1), size.1.max(1)).expect("the output has a size");
    rounded(&mut p, layout.bar, RADIUS, BACKGROUND, scale);
    for (target, r, text) in &layout.buttons {
        let fill =
            if *target == Target::Mode(state.mode) || (*target == Target::Options && menu_open) {
                Some(SELECTED)
            } else if hover == Some(*target) {
                Some(HOVER)
            } else {
                None
            };
        if let Some(fill) = fill {
            rounded(&mut p, *r, 8.0, fill, scale);
        }
        label(&mut p, *r, crate::toolbar::layout::BUTTON_PAD, text, scale);
    }
    for s in &layout.separators {
        rounded(&mut p, *s, 0.0, SEPARATOR, scale);
    }
    if menu_open {
        rounded(&mut p, layout.menu, RADIUS, BACKGROUND, scale);
        for (item, r, text) in &layout.rows {
            if hover == Some(Target::Menu(*item)) {
                rounded(&mut p, *r, 6.0, HOVER, scale);
            }
            let on = match item {
                MenuItem::Timer(t) => state.timer == *t,
                MenuItem::Cursor => state.cursor,
                MenuItem::Preview => state.preview,
                MenuItem::Sound => state.sound,
            };
            if on {
                label(&mut p, *r, crate::toolbar::layout::BUTTON_PAD, "✓", scale);
            }
            label(
                &mut p,
                *r,
                crate::toolbar::layout::BUTTON_PAD + crate::toolbar::layout::MARK_W,
                text,
                scale,
            );
        }
    }
    p
}

/// The countdown pill: a rounded background with `text` centred.
pub fn pill(size: (u32, u32), scale: f32, s: &str) -> Pixmap {
    let mut p = Pixmap::new(size.0.max(1), size.1.max(1)).expect("the pill has a size");
    let r = Rect {
        x: 0.0,
        y: 0.0,
        w: size.0 as f32 / scale,
        h: size.1 as f32 / scale,
    };
    rounded(&mut p, r, RADIUS, BACKGROUND, scale);
    let left = (r.w - measure(s)) / 2.0;
    label(&mut p, r, left, s, scale);
    p
}

/// Copies premultiplied RGBA into wl_shm ARGB8888 (bytes B, G, R, A).
pub fn to_argb(pixmap: &Pixmap, out: &mut [u8]) {
    for (dst, src) in out.as_chunks_mut::<4>().0.iter_mut().zip(pixmap.pixels()) {
        *dst = [src.blue(), src.green(), src.red(), src.alpha()];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolbar::layout::layout;
    use crate::toolbar::state::Mode;

    const STATE: ToolbarState = ToolbarState {
        mode: Mode::Region,
        timer: 0,
        cursor: false,
        preview: true,
        sound: true,
    };

    fn alpha(p: &Pixmap, x: f32, y: f32, scale: f32) -> u8 {
        p.pixel((x * scale) as u32, (y * scale) as u32)
            .unwrap()
            .alpha()
    }

    #[test]
    fn the_bar_is_opaque_and_the_rest_transparent() {
        let l = layout((800.0, 600.0), measure);
        let p = bar(&l, (800, 600), 1.0, &STATE, None, false);
        assert_eq!(alpha(&p, 5.0, 5.0, 1.0), 0);
        assert!(alpha(&p, l.bar.x + l.bar.w / 2.0, l.bar.y + 3.0, 1.0) >= 230);
        assert_eq!(
            alpha(&p, l.menu.x + 5.0, l.menu.y + 5.0, 1.0),
            0,
            "menu closed"
        );
        let open = bar(&l, (800, 600), 1.0, &STATE, None, true);
        assert!(alpha(&open, l.menu.x + l.menu.w / 2.0, l.menu.y + 3.0, 1.0) >= 230);
    }

    #[test]
    fn the_selected_mode_is_lighter() {
        let l = layout((800.0, 600.0), measure);
        let p = bar(&l, (800, 600), 1.0, &STATE, None, false);
        let lum = |r: crate::toolbar::layout::Rect| {
            p.pixel((r.x + 3.0) as u32, (r.y + 3.0) as u32)
                .unwrap()
                .demultiply()
                .red()
        };
        let region = l.buttons[2].1;
        let screen = l.buttons[0].1;
        assert!(lum(region) > lum(screen));
    }

    #[test]
    fn scales() {
        let l = layout((800.0, 600.0), measure);
        let p = bar(&l, (1000, 750), 1.25, &STATE, None, false);
        assert_eq!((p.width(), p.height()), (1000, 750));
        assert!(alpha(&p, l.bar.x + l.bar.w / 2.0, l.bar.y + 3.0, 1.25) >= 230);
        assert_eq!(alpha(&p, l.bar.x - 3.0, l.bar.y + 3.0, 1.25), 0);
    }

    #[test]
    fn argb_swaps_red_and_blue() {
        let mut p = Pixmap::new(1, 1).unwrap();
        p.fill(tiny_skia::Color::from_rgba8(10, 20, 30, 255));
        let mut out = [0u8; 4];
        to_argb(&p, &mut out);
        assert_eq!(out, [30, 20, 10, 255]);
    }

    #[test]
    fn the_pill_shows_its_text() {
        let p = pill((72, 40), 1.0, "5");
        let lit = p
            .pixels()
            .iter()
            .filter(|c| c.demultiply().red() > 200 && c.alpha() > 200)
            .count();
        assert!(lit > 10, "white digit pixels: {lit}");
    }

    #[test]
    fn the_font_has_the_marks() {
        use ab_glyph::Font;
        for c in ['✓', '▾'] {
            assert_ne!(text::FONT.glyph_id(c).0, 0, "{c}");
        }
    }
}
