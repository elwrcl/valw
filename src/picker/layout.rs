//! Where the picker's cards go, in logical px of the output.

/// A card at rest: icon, title and app name.
pub const CARD: (f32, f32) = (200.0, 200.0);
pub const GAP: f32 = 24.0;
/// The selected card's size relative to the others.
pub const GROW: f32 = 1.1;

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

/// `n` cards centred on an output of `output` logical px: rows wrap inside
/// 90 % of the width, and the grid shrinks to fit 60 % of the height. The
/// `selected` card grows about its centre.
pub fn cards(n: usize, output: (f32, f32), selected: usize) -> Vec<Rect> {
    if n == 0 {
        return Vec::new();
    }
    let (max_w, max_h) = (output.0 * 0.9, output.1 * 0.6);
    // Shrink until the grid fits both ways.
    let mut scale = 1.0f32;
    let (per_row, rows) = loop {
        let (cw, g) = (CARD.0 * scale, GAP * scale);
        let per_row = (((max_w + g) / (cw + g)).floor() as usize).clamp(1, n);
        let rows = n.div_ceil(per_row);
        let grid_h = rows as f32 * CARD.1 * scale + (rows - 1) as f32 * g;
        if grid_h <= max_h || scale < 0.2 {
            break (per_row, rows);
        }
        scale *= 0.95;
    };
    let (cw, ch, g) = (CARD.0 * scale, CARD.1 * scale, GAP * scale);
    let grid_h = rows as f32 * ch + (rows - 1) as f32 * g;
    let top = (output.1 - grid_h) / 2.0;
    (0..n)
        .map(|i| {
            let (row, col) = (i / per_row, i % per_row);
            let in_row = per_row.min(n - row * per_row);
            let row_w = in_row as f32 * cw + (in_row - 1) as f32 * g;
            let r = Rect {
                x: (output.0 - row_w) / 2.0 + col as f32 * (cw + g),
                y: top + row as f32 * (ch + g),
                w: cw,
                h: ch,
            };
            if i == selected { grow(r) } else { r }
        })
        .collect()
}

fn grow(r: Rect) -> Rect {
    let (w, h) = (r.w * GROW, r.h * GROW);
    Rect {
        x: r.x - (w - r.w) / 2.0,
        y: r.y - (h - r.h) / 2.0,
        w,
        h,
    }
}

/// The card under `p`, the selected (grown) one first.
pub fn hit(cards: &[Rect], p: (f32, f32)) -> Option<usize> {
    let mut order: Vec<usize> = (0..cards.len()).collect();
    order.sort_by(|a, b| (cards[*b].w).total_cmp(&cards[*a].w));
    order.into_iter().find(|i| cards[*i].contains(p))
}

/// `selected` moved by `delta`, wrapping around `n` cards.
pub fn step(selected: usize, n: usize, delta: i32) -> usize {
    if n == 0 {
        return 0;
    }
    (selected as i64 + delta as i64).rem_euclid(n as i64) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlap(a: &Rect, b: &Rect) -> bool {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }

    #[test]
    fn one_card_is_centred() {
        let c = cards(1, (1920.0, 1080.0), 0);
        let r = c[0];
        assert!(
            ((r.x + r.w / 2.0) - 960.0).abs() < 0.01 && ((r.y + r.h / 2.0) - 540.0).abs() < 0.01
        );
    }

    #[test]
    fn five_cards_fit_one_row_on_1080p() {
        let c = cards(5, (1920.0, 1080.0), 2);
        let unselected: Vec<_> = c
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 2)
            .map(|(_, r)| r.y)
            .collect();
        assert!(unselected.windows(2).all(|w| w[0] == w[1]), "one row");
        for i in 0..5 {
            for j in i + 1..5 {
                assert!(!overlap(&c[i], &c[j]), "{i} and {j} overlap");
            }
        }
    }

    #[test]
    fn many_cards_wrap_and_shrink_to_fit() {
        let (w, h) = (1366.0, 768.0);
        let c = cards(20, (w, h), 0);
        assert_eq!(c.len(), 20);
        let left = c.iter().map(|r| r.x).fold(f32::MAX, f32::min);
        let right = c.iter().map(|r| r.x + r.w).fold(0.0, f32::max);
        let top = c.iter().skip(1).map(|r| r.y).fold(f32::MAX, f32::min);
        let bottom = c.iter().skip(1).map(|r| r.y + r.h).fold(0.0, f32::max);
        assert!(right - left <= w * 0.9 + 1.0, "{left}..{right}");
        assert!(bottom - top <= h * 0.6 + 1.0, "{top}..{bottom}");
        for i in 1..20 {
            for j in i + 1..20 {
                assert!(!overlap(&c[i], &c[j]), "{i} and {j} overlap");
            }
        }
    }

    #[test]
    fn the_selected_card_grows_about_its_centre() {
        let plain = cards(3, (1920.0, 1080.0), 0);
        let chosen = cards(3, (1920.0, 1080.0), 1);
        let (a, b) = (plain[1], chosen[1]);
        assert!((b.w - a.w * 1.1).abs() < 0.01);
        assert!(((a.x + a.w / 2.0) - (b.x + b.w / 2.0)).abs() < 0.01);
    }

    #[test]
    fn hits_and_gaps() {
        let c = cards(3, (1920.0, 1080.0), 0);
        let mid = |r: Rect| (r.x + r.w / 2.0, r.y + r.h / 2.0);
        assert_eq!(hit(&c, mid(c[2])), Some(2));
        assert_eq!(hit(&c, (5.0, 5.0)), None);
    }

    #[test]
    fn steps_wrap() {
        assert_eq!(step(4, 5, 1), 0);
        assert_eq!(step(0, 5, -1), 4);
        assert_eq!(step(2, 5, 1), 3);
        assert_eq!(step(0, 0, 1), 0);
    }
}
