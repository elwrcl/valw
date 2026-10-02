//! The zoom view: which part of the frozen frame fills the screen.
//!
//! Everything is in the frame's physical pixels. The surface's buffer has
//! the frame's size, so at 1× one screen pixel is one frame pixel.

use crate::frame::PixelRect;

pub const MIN_SCALE: f64 = 1.0;
pub const MAX_SCALE: f64 = 32.0;
pub const MIN_RADIUS: f64 = 20.0;
pub const MAX_RADIUS: f64 = 2000.0;
/// Logical px of touchpad scrolling per zoom step.
const TOUCHPAD_PX_PER_STEP: f64 = 15.0;
/// Per second: 1 - e^(-RATE * 0.12) ≈ 0.9, 90 % of the way in 120 ms
/// (measured in the visible span, 1 / scale).
const RATE: f64 = 19.2;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub scale: f64,
    /// The frame pixel at the screen's top-left corner.
    pub origin: (f64, f64),
}

impl View {
    pub const IDENTITY: View = View {
        scale: 1.0,
        origin: (0.0, 0.0),
    };

    /// The frame point under screen point `p`.
    pub fn to_frame(self, p: (f64, f64)) -> (f64, f64) {
        (
            self.origin.0 + p.0 / self.scale,
            self.origin.1 + p.1 / self.scale,
        )
    }

    /// Zooms by `factor`, keeping the frame point under `p` in place.
    pub fn zoom_at(&self, p: (f64, f64), factor: f64, size: (f64, f64)) -> View {
        let f = self.to_frame(p);
        let scale = (self.scale * factor).clamp(MIN_SCALE, MAX_SCALE);
        View {
            scale,
            origin: (f.0 - p.0 / scale, f.1 - p.1 / scale),
        }
        .clamped(size)
    }

    /// Moves the image along with a pointer that moved `delta` screen px.
    pub fn pan(&self, delta: (f64, f64), size: (f64, f64)) -> View {
        View {
            scale: self.scale,
            origin: (
                self.origin.0 - delta.0 / self.scale,
                self.origin.1 - delta.1 / self.scale,
            ),
        }
        .clamped(size)
    }

    /// Keeps the view inside a frame of `size` px.
    pub fn clamped(self, size: (f64, f64)) -> View {
        let max = |len: f64| len - len / self.scale;
        View {
            scale: self.scale,
            origin: (
                self.origin.0.clamp(0.0, max(size.0)),
                self.origin.1.clamp(0.0, max(size.1)),
            ),
        }
    }

    /// The frame pixels on screen, rounded outwards and inside the frame.
    pub fn visible_rect(&self, size: (u32, u32)) -> PixelRect {
        let span = |origin: f64, len: u32| {
            let len = len as f64;
            let start = origin.floor().clamp(0.0, len - 1.0);
            let end = (origin + len / self.scale).ceil().clamp(start + 1.0, len);
            (start as u32, (end - start) as u32)
        };
        let (x, width) = span(self.origin.0, size.0);
        let (y, height) = span(self.origin.1, size.1);
        PixelRect {
            x,
            y,
            width,
            height,
        }
    }
}

/// The view on screen, easing towards the one asked for.
#[derive(Debug, Clone, Copy)]
pub struct Animated {
    pub shown: View,
    pub target: View,
}

impl Animated {
    pub fn new() -> Self {
        Animated {
            shown: View::IDENTITY,
            target: View::IDENTITY,
        }
    }

    /// Jumps to `view` without easing (panning).
    pub fn set(&mut self, view: View) {
        self.shown = view;
        self.target = view;
    }

    /// Advances by `dt` seconds. True while still moving.
    pub fn step(&mut self, dt: f64) -> bool {
        let k = 1.0 - (-RATE * dt).exp();
        let (s, t) = (&mut self.shown, self.target);
        let lerp = |a: f64, b: f64| a + (b - a) * k;
        // Easing the visible span (1 / scale) and the origin together keeps
        // the frame point under the pointer still the whole way.
        s.scale = 1.0 / lerp(1.0 / s.scale, 1.0 / t.scale);
        s.origin = (lerp(s.origin.0, t.origin.0), lerp(s.origin.1, t.origin.1));
        let settled = (s.scale - t.scale).abs() < 1e-3
            && (s.origin.0 - t.origin.0).abs() < 0.05
            && (s.origin.1 - t.origin.1).abs() < 0.05;
        if settled {
            self.shown = t;
        }
        !settled
    }
}

/// Zoom steps in one axis event: wheel notches (`value120`), or touchpad
/// pixels when there are none. Positive zooms in (scrolling up).
pub fn scroll_steps(value120: i32, absolute: f64) -> f64 {
    if value120 != 0 {
        -(value120 as f64) / 120.0
    } else {
        -absolute / TOUCHPAD_PX_PER_STEP
    }
}

/// The flashlight radius after `steps` scroll steps of `step` each.
pub fn radius(r: f64, steps: f64, step: f64) -> f64 {
    (r * step.powf(steps)).clamp(MIN_RADIUS, MAX_RADIUS)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: (f64, f64) = (1920.0, 1080.0);

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6
    }

    #[test]
    fn zoom_keeps_the_point_under_the_pointer() {
        for p in [(960.0, 540.0), (100.0, 900.0), (1500.0, 200.0)] {
            let v = View::IDENTITY.zoom_at(p, 2.0, SIZE);
            assert_eq!(v.scale, 2.0);
            assert!(close(v.to_frame(p), p), "{p:?} -> {v:?}");
            let w = v.zoom_at(p, 1.15, SIZE);
            assert!(close(w.to_frame(p), v.to_frame(p)), "{p:?} -> {w:?}");
        }
    }

    #[test]
    fn scale_is_clamped() {
        let v = View::IDENTITY.zoom_at((10.0, 10.0), 0.5, SIZE);
        assert_eq!(v, View::IDENTITY);
        let v = View::IDENTITY.zoom_at((10.0, 10.0), 1000.0, SIZE);
        assert_eq!(v.scale, MAX_SCALE);
    }

    #[test]
    fn zoom_out_near_an_edge_stays_inside() {
        let v = View::IDENTITY.zoom_at((1900.0, 1070.0), 4.0, SIZE);
        let out = v.zoom_at((0.0, 0.0), 0.5, SIZE);
        assert_eq!(out.scale, 2.0);
        assert!(
            out.origin.0 >= 0.0 && out.origin.0 <= 1920.0 - 960.0,
            "{out:?}"
        );
        assert!(
            out.origin.1 >= 0.0 && out.origin.1 <= 1080.0 - 540.0,
            "{out:?}"
        );
    }

    #[test]
    fn pan_moves_with_the_pointer_and_is_clamped() {
        let v = View::IDENTITY.zoom_at((960.0, 540.0), 2.0, SIZE);
        let p = v.pan((100.0, 0.0), SIZE);
        assert!(close(p.origin, (v.origin.0 - 50.0, v.origin.1)), "{p:?}");
        let far = v.pan((-1e6, 1e6), SIZE);
        assert!(close(far.origin, (960.0, 0.0)), "{far:?}");
        assert_eq!(View::IDENTITY.pan((50.0, 50.0), SIZE), View::IDENTITY);
    }

    #[test]
    fn visible_rect_at_identity_is_the_frame() {
        let r = View::IDENTITY.visible_rect((1920, 1080));
        assert_eq!((r.x, r.y, r.width, r.height), (0, 0, 1920, 1080));
    }

    #[test]
    fn visible_rect_rounds_outwards_inside_the_frame() {
        let v = View {
            scale: 4.0,
            origin: (100.5, 200.25),
        };
        let r = v.visible_rect((1920, 1080));
        assert_eq!((r.x, r.y, r.width, r.height), (100, 200, 481, 271));
        let edge = View {
            scale: 4.0,
            origin: (1440.0, 810.0),
        };
        let r = edge.visible_rect((1920, 1080));
        assert_eq!((r.x, r.y, r.width, r.height), (1440, 810, 480, 270));
    }

    #[test]
    fn animation_reaches_the_target_and_stops() {
        let mut a = Animated::new();
        a.target = View::IDENTITY.zoom_at((960.0, 540.0), 4.0, SIZE);
        let mut frames = 0;
        while a.step(1.0 / 60.0) {
            frames += 1;
            assert!(frames < 120, "never settles");
        }
        assert_eq!(a.shown, a.target);
        // About 90 % of the way after 120 ms, in the visible span (1 / scale).
        let mut b = Animated::new();
        b.target = View {
            scale: 11.0,
            origin: (0.0, 0.0),
        };
        b.step(0.12);
        let span = 1.0 / b.shown.scale;
        let expected = 1.0 + 0.9 * (1.0 / 11.0 - 1.0);
        assert!((span - expected).abs() < 0.02, "{:?}", b.shown);
    }

    #[test]
    fn animation_keeps_the_point_under_the_pointer() {
        for p in [(960.0, 540.0), (100.0, 900.0)] {
            let mut a = Animated::new();
            a.target = View::IDENTITY.zoom_at(p, 2.0, SIZE);
            let anchor = a.target.to_frame(p);
            for _ in 0..5 {
                a.step(0.02);
                assert!(close(a.shown.to_frame(p), anchor), "{p:?}: {:?}", a.shown);
            }
        }
    }

    #[test]
    fn set_jumps() {
        let mut a = Animated::new();
        let v = View {
            scale: 2.0,
            origin: (5.0, 5.0),
        };
        a.set(v);
        assert_eq!((a.shown, a.target), (v, v));
        assert!(!a.step(0.016));
    }

    #[test]
    fn scroll_steps_from_wheel_and_touchpad() {
        assert_eq!(scroll_steps(-120, -15.0), 1.0, "one notch up zooms in");
        assert_eq!(scroll_steps(240, 30.0), -2.0, "two notches down");
        assert_eq!(scroll_steps(0, -7.5), 0.5, "touchpad: 15 px per step");
        assert_eq!(scroll_steps(0, 0.0), 0.0);
    }

    #[test]
    fn radius_steps_and_bounds() {
        assert!((radius(180.0, 1.0, 1.15) - 207.0).abs() < 1e-9);
        assert_eq!(radius(25.0, -10.0, 1.15), MIN_RADIUS);
        assert_eq!(radius(1900.0, 10.0, 1.15), MAX_RADIUS);
    }
}
