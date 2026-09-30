//! Pure layout, gesture and animation rules for the preview thumbnails.

/// Long edge of a thumbnail, in logical pixels.
pub const LONG_EDGE: u32 = 220;
/// Distance between a thumbnail and the screen's right and bottom edges.
pub const EDGE_MARGIN: u32 = 16;
/// Vertical gap between stacked thumbnails.
pub const GAP: u32 = 12;
/// Thumbnails per output; a new one beyond this evicts the oldest.
pub const MAX: usize = 5;
/// Pointer movement below this (logical px) still counts as a click.
pub const CLICK_SLOP: f64 = 4.0;
/// A swipe of at least this fraction of the image width dismisses it.
pub const SWIPE_FRACTION: f64 = 0.4;
pub const SLIDE_IN_MS: u32 = 200;
pub const SLIDE_OUT_MS: u32 = 150;

/// Logical thumbnail size for an image of `width` x `height`: long edge
/// `LONG_EDGE`, aspect ratio kept, never scaled up.
pub fn thumb_size(width: u32, height: u32) -> (u32, u32) {
    let long = width.max(height);
    if long <= LONG_EDGE {
        return (width.max(1), height.max(1));
    }
    let scale = LONG_EDGE as f64 / long as f64;
    let fit = |v: u32| ((v as f64 * scale).round() as u32).max(1);
    (fit(width), fit(height))
}

/// Bottom margin of each thumbnail, given their heights newest first.
/// The newest sits `EDGE_MARGIN` above the screen edge, older ones stack up.
pub fn bottom_margins(heights_newest_first: &[u32]) -> Vec<u32> {
    let mut next = EDGE_MARGIN;
    heights_newest_first
        .iter()
        .map(|h| {
            let margin = next;
            next += h + GAP;
            margin
        })
        .collect()
}

/// How many of the oldest thumbnails to evict when there are `count`.
pub fn excess(count: usize) -> usize {
    count.saturating_sub(MAX)
}

/// What a pointer release on a thumbnail means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Release {
    /// Barely moved: open the editor.
    Click,
    /// Dragged far enough right: slide out and close.
    Dismiss,
    /// Dragged, but not far enough: animate back.
    SnapBack,
}

/// Classifies a release after a press-drag of (`dx`, `dy`) on an image
/// `width` logical px wide.
pub fn release(dx: f64, dy: f64, width: f64) -> Release {
    if dx.hypot(dy) < CLICK_SLOP {
        Release::Click
    } else if dx >= SWIPE_FRACTION * width {
        Release::Dismiss
    } else {
        Release::SnapBack
    }
}

/// Horizontal image offset while dragging: follows the pointer, right only.
pub fn drag_offset(dx: f64) -> f64 {
    dx.max(0.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ease {
    OutCubic,
    InCubic,
}

impl Ease {
    /// Maps linear progress `t` in 0..=1 to eased progress in 0..=1.
    pub fn apply(self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Ease::OutCubic => 1.0 - (1.0 - t).powi(3),
            Ease::InCubic => t.powi(3),
        }
    }
}

/// An animation of the image's horizontal offset.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Anim {
    pub from: f64,
    pub to: f64,
    pub duration_ms: u32,
    pub ease: Ease,
}

impl Anim {
    /// Slides in from fully off-screen (`travel` px to the right) to rest.
    pub fn slide_in(travel: f64) -> Anim {
        Anim {
            from: travel,
            to: 0.0,
            duration_ms: SLIDE_IN_MS,
            ease: Ease::OutCubic,
        }
    }

    /// Slides out from the current offset to fully off-screen.
    pub fn slide_out(from: f64, travel: f64) -> Anim {
        Anim {
            from,
            to: travel,
            duration_ms: SLIDE_OUT_MS,
            ease: Ease::InCubic,
        }
    }

    /// Animates back to rest after a short drag.
    pub fn snap_back(from: f64) -> Anim {
        Anim {
            from,
            to: 0.0,
            duration_ms: SLIDE_OUT_MS,
            ease: Ease::OutCubic,
        }
    }

    /// Offset after `elapsed_ms`, and whether the animation has finished.
    pub fn at(&self, elapsed_ms: f64) -> (f64, bool) {
        let t = elapsed_ms / self.duration_ms as f64;
        let offset = self.from + (self.to - self.from) * self.ease.apply(t);
        (offset, t >= 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn landscape_and_portrait_fit_the_long_edge() {
        assert_eq!(thumb_size(1920, 1080), (220, 124));
        assert_eq!(thumb_size(1080, 1920), (124, 220));
        assert_eq!(thumb_size(1366, 768), (220, 124));
    }

    #[test]
    fn small_images_are_not_scaled_up() {
        assert_eq!(thumb_size(100, 50), (100, 50));
        assert_eq!(thumb_size(220, 10), (220, 10));
    }

    #[test]
    fn extreme_aspect_ratio_keeps_one_pixel() {
        assert_eq!(thumb_size(10_000, 2), (220, 1));
    }

    #[test]
    fn stack_margins() {
        assert_eq!(bottom_margins(&[]), Vec::<u32>::new());
        assert_eq!(bottom_margins(&[124]), vec![16]);
        assert_eq!(bottom_margins(&[124, 124, 220]), vec![16, 152, 288]);
    }

    #[test]
    fn eviction_beyond_five() {
        assert_eq!(excess(5), 0);
        assert_eq!(excess(6), 1);
        assert_eq!(excess(0), 0);
    }

    #[test]
    fn release_decisions() {
        let w = 220.0;
        assert_eq!(release(0.0, 0.0, w), Release::Click);
        assert_eq!(release(2.0, 3.0, w), Release::Click);
        assert_eq!(release(0.39 * w, 0.0, w), Release::SnapBack);
        assert_eq!(release(0.40 * w, 0.0, w), Release::Dismiss);
        assert_eq!(release(-100.0, 0.0, w), Release::SnapBack);
        assert_eq!(release(0.0, 50.0, w), Release::SnapBack);
    }

    #[test]
    fn drag_follows_right_only() {
        assert_eq!(drag_offset(30.0), 30.0);
        assert_eq!(drag_offset(-30.0), 0.0);
    }

    #[test]
    fn easing_endpoints_and_monotonic() {
        for ease in [Ease::OutCubic, Ease::InCubic] {
            assert_eq!(ease.apply(0.0), 0.0);
            assert_eq!(ease.apply(1.0), 1.0);
            assert_eq!(ease.apply(2.0), 1.0, "clamped");
            let samples: Vec<f64> = (0..=10).map(|i| ease.apply(i as f64 / 10.0)).collect();
            assert!(
                samples.windows(2).all(|w| w[0] <= w[1]),
                "{ease:?} {samples:?}"
            );
        }
    }

    #[test]
    fn slide_in_runs_from_travel_to_rest() {
        let a = Anim::slide_in(236.0);
        assert_eq!(a.at(0.0), (236.0, false));
        assert_eq!(a.at(200.0), (0.0, true));
        assert_eq!(a.at(500.0), (0.0, true));
        let (mid, done) = a.at(100.0);
        assert!(mid > 0.0 && mid < 118.0, "ease-out is past halfway: {mid}");
        assert!(!done);
    }

    #[test]
    fn slide_out_and_snap_back() {
        let out = Anim::slide_out(50.0, 236.0);
        assert_eq!(out.at(0.0), (50.0, false));
        assert_eq!(out.at(150.0), (236.0, true));
        assert_eq!(Anim::snap_back(40.0).at(150.0), (0.0, true));
    }
}
