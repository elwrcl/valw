//! Where the toolbar's buttons and menu are, in logical px of the output,
//! and what a point hits.

use crate::toolbar::state::Mode;

pub const MARGIN_BOTTOM: f32 = 24.0;
pub const PADDING: f32 = 6.0;
pub const BUTTON_H: f32 = 32.0;
pub const BUTTON_PAD: f32 = 12.0;
pub const SEPARATOR_GAP: f32 = 8.0;
pub const MENU_GAP: f32 = 8.0;
pub const ROW_H: f32 = 28.0;
/// Room for the check or radio mark left of a menu label.
pub const MARK_W: f32 = 24.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, p: (f32, f32)) -> bool {
        p.0 >= self.x && p.0 < self.x + self.w && p.1 >= self.y && p.1 < self.y + self.h
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItem {
    Timer(u32),
    Cursor,
    Preview,
    Sound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Mode(Mode),
    Options,
    Menu(MenuItem),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub bar: Rect,
    pub buttons: Vec<(Target, Rect, &'static str)>,
    pub separators: Vec<Rect>,
    pub menu: Rect,
    pub rows: Vec<(MenuItem, Rect, &'static str)>,
}

/// `None` stands for a separator.
const BAR: [Option<(Target, &str)>; 7] = [
    Some((Target::Mode(Mode::Screen), "Screen")),
    Some((Target::Mode(Mode::Window), "Window")),
    Some((Target::Mode(Mode::Region), "Region")),
    None,
    Some((Target::Mode(Mode::Zoom), "Zoom")),
    None,
    Some((Target::Options, "Options ▾")),
];

const MENU: [(MenuItem, &str); 6] = [
    (MenuItem::Timer(0), "No timer"),
    (MenuItem::Timer(5), "5 second timer"),
    (MenuItem::Timer(10), "10 second timer"),
    (MenuItem::Cursor, "Show cursor"),
    (MenuItem::Preview, "Show preview"),
    (MenuItem::Sound, "Play sound"),
];

/// Lays the toolbar out on an output of `output` logical px; `measure`
/// gives a label's width in logical px.
pub fn layout(output: (f32, f32), measure: impl Fn(&str) -> f32) -> Layout {
    let widths: Vec<f32> = BAR
        .iter()
        .map(|item| match item {
            Some((_, label)) => measure(label) + 2.0 * BUTTON_PAD,
            None => 2.0 * SEPARATOR_GAP + 1.0,
        })
        .collect();
    let w = widths.iter().sum::<f32>() + 2.0 * PADDING;
    let h = BUTTON_H + 2.0 * PADDING;
    let bar = Rect {
        x: (output.0 - w) / 2.0,
        y: output.1 - MARGIN_BOTTOM - h,
        w,
        h,
    };

    let mut buttons = Vec::new();
    let mut separators = Vec::new();
    let mut x = bar.x + PADDING;
    for (item, width) in BAR.iter().zip(widths) {
        match item {
            Some((target, label)) => {
                buttons.push((
                    *target,
                    Rect {
                        x,
                        y: bar.y + PADDING,
                        w: width,
                        h: BUTTON_H,
                    },
                    *label,
                ));
            }
            None => separators.push(Rect {
                x: x + SEPARATOR_GAP,
                y: bar.y + PADDING + 6.0,
                w: 1.0,
                h: BUTTON_H - 12.0,
            }),
        }
        x += width;
    }

    let menu_w =
        MENU.iter().map(|(_, l)| measure(l)).fold(0.0, f32::max) + MARK_W + 2.0 * BUTTON_PAD;
    let menu_h = MENU.len() as f32 * ROW_H + 2.0 * PADDING;
    let options = buttons.last().expect("the bar has buttons").1;
    let menu_x = options.x.min(output.0 - menu_w).max(0.0);
    let menu = Rect {
        x: menu_x,
        y: bar.y - MENU_GAP - menu_h,
        w: menu_w,
        h: menu_h,
    };
    let rows = MENU
        .iter()
        .enumerate()
        .map(|(i, (item, label))| {
            (
                *item,
                Rect {
                    x: menu.x,
                    y: menu.y + PADDING + i as f32 * ROW_H,
                    w: menu.w,
                    h: ROW_H,
                },
                *label,
            )
        })
        .collect();
    Layout {
        bar,
        buttons,
        separators,
        menu,
        rows,
    }
}

impl Layout {
    pub fn hit(&self, p: (f32, f32), menu_open: bool) -> Option<Target> {
        if menu_open && let Some(row) = self.rows.iter().find(|r| r.1.contains(p)) {
            return Some(Target::Menu(row.0));
        }
        self.buttons.iter().find(|b| b.1.contains(p)).map(|b| b.0)
    }

    /// Whether `p` is on the bar or the open menu (a click elsewhere closes).
    pub fn inside(&self, p: (f32, f32), menu_open: bool) -> bool {
        self.bar.contains(p) || (menu_open && self.menu.contains(p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measure(s: &str) -> f32 {
        7.0 * s.chars().count() as f32
    }

    fn l() -> Layout {
        layout((1366.0, 768.0), measure)
    }

    fn centre(r: Rect) -> (f32, f32) {
        (r.x + r.w / 2.0, r.y + r.h / 2.0)
    }

    #[test]
    fn the_bar_is_centred_above_the_bottom() {
        let l = l();
        assert!(((l.bar.x + l.bar.w / 2.0) - 683.0).abs() < 0.01);
        assert_eq!(l.bar.y + l.bar.h, 768.0 - 24.0);
        assert_eq!(l.bar.h, 32.0 + 2.0 * 6.0);
    }

    #[test]
    fn buttons_are_in_order_and_do_not_overlap() {
        let l = l();
        let labels: Vec<_> = l.buttons.iter().map(|b| b.2).collect();
        assert_eq!(labels, ["Screen", "Window", "Region", "Zoom", "Options ▾"]);
        for pair in l.buttons.windows(2) {
            assert!(pair[0].1.x + pair[0].1.w <= pair[1].1.x);
        }
        // "Screen": 6 chars × 7 + 2 × 12 padding.
        assert_eq!(l.buttons[0].1.w, 42.0 + 24.0);
        assert_eq!(l.separators.len(), 2);
    }

    #[test]
    fn each_button_hits_its_target() {
        let l = l();
        for (target, rect, _) in &l.buttons {
            assert_eq!(l.hit(centre(*rect), false), Some(*target));
        }
    }

    #[test]
    fn gaps_hit_nothing() {
        let l = l();
        for sep in &l.separators {
            assert_eq!(l.hit(centre(*sep), false), None);
        }
        assert_eq!(
            l.hit((l.bar.x + 2.0, l.bar.y + 2.0), false),
            None,
            "padding"
        );
        assert_eq!(l.hit((10.0, 10.0), false), None, "outside");
        assert!(l.inside((l.bar.x + 2.0, l.bar.y + 2.0), false));
        assert!(!l.inside((10.0, 10.0), false));
    }

    #[test]
    fn the_menu_sits_above_options_and_only_counts_when_open() {
        let l = l();
        let options = l.buttons[4].1;
        assert_eq!(l.menu.y + l.menu.h, l.bar.y - 8.0);
        assert!(l.menu.x <= options.x + 0.01 || l.menu.x + l.menu.w <= 1366.0);
        let rows: Vec<_> = l.rows.iter().map(|r| r.0).collect();
        assert_eq!(
            rows,
            [
                MenuItem::Timer(0),
                MenuItem::Timer(5),
                MenuItem::Timer(10),
                MenuItem::Cursor,
                MenuItem::Preview,
                MenuItem::Sound
            ]
        );
        let cursor = centre(l.rows[3].1);
        assert_eq!(l.hit(cursor, true), Some(Target::Menu(MenuItem::Cursor)));
        assert_eq!(l.hit(cursor, false), None);
        assert!(l.inside(cursor, true) && !l.inside(cursor, false));
    }

    #[test]
    fn the_menu_stays_on_a_narrow_output() {
        let l = layout((400.0, 300.0), measure);
        assert!(l.menu.x >= 0.0 && l.menu.x + l.menu.w <= 400.0);
    }
}
