use crate::frame::{OutputGeom, PixelRect};

/// Selections smaller than this (in physical pixels) count as a click.
pub const MIN_PIXELS: u32 = 2;

/// A point in global logical coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum Selection {
    #[default]
    Idle,
    Dragging {
        start: Point,
        end: Point,
    },
}

impl Selection {
    pub fn press(&mut self, p: Point) {
        *self = Selection::Dragging { start: p, end: p };
    }

    pub fn motion(&mut self, p: Point) {
        if let Selection::Dragging { end, .. } = self {
            *end = p;
        }
    }

    /// Ends the drag and returns its start and end. Back to `Idle` either way.
    pub fn release(&mut self, p: Point) -> Option<(Point, Point)> {
        match std::mem::take(self) {
            Selection::Dragging { start, .. } => Some((start, p)),
            Selection::Idle => None,
        }
    }
}

/// The part of the drag from `a` to `b` that falls on `out`, in the physical
/// pixels of a `frame_w` x `frame_h` frame. `None` if nothing is left.
///
/// Edges round outwards so the rectangle covers every pixel the user touched.
pub fn clip(a: Point, b: Point, out: &OutputGeom, frame_w: u32, frame_h: u32) -> Option<PixelRect> {
    let sx = frame_w as f64 / out.width as f64;
    let sy = frame_h as f64 / out.height as f64;
    let to_px = |v: f64, origin: i32, s: f64, max: u32, round: fn(f64) -> f64| {
        round((v - origin as f64) * s).clamp(0.0, max as f64) as u32
    };
    let x0 = to_px(a.x.min(b.x), out.x, sx, frame_w, f64::floor);
    let x1 = to_px(a.x.max(b.x), out.x, sx, frame_w, f64::ceil);
    let y0 = to_px(a.y.min(b.y), out.y, sy, frame_h, f64::floor);
    let y1 = to_px(a.y.max(b.y), out.y, sy, frame_h, f64::ceil);
    (x1 > x0 && y1 > y0).then(|| PixelRect {
        x: x0,
        y: y0,
        width: x1 - x0,
        height: y1 - y0,
    })
}

/// Like [`clip`], but treats anything under [`MIN_PIXELS`] as a click.
pub fn resolve(
    a: Point,
    b: Point,
    out: &OutputGeom,
    frame_w: u32,
    frame_h: u32,
) -> Option<PixelRect> {
    clip(a, b, out, frame_w, frame_h).filter(|r| r.width >= MIN_PIXELS && r.height >= MIN_PIXELS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(name: &str, x: i32, y: i32, width: i32, height: i32) -> OutputGeom {
        OutputGeom {
            name: name.into(),
            x,
            y,
            width,
            height,
        }
    }

    fn p(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    fn rect(x: u32, y: u32, width: u32, height: u32) -> PixelRect {
        PixelRect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn scale_one() {
        let o = out("A", 0, 0, 1920, 1080);
        assert_eq!(
            clip(p(10.0, 20.0), p(110.0, 70.0), &o, 1920, 1080),
            Some(rect(10, 20, 100, 50))
        );
    }

    #[test]
    fn drag_direction_does_not_matter() {
        let o = out("A", 0, 0, 1920, 1080);
        assert_eq!(
            clip(p(110.0, 70.0), p(10.0, 20.0), &o, 1920, 1080),
            clip(p(10.0, 20.0), p(110.0, 70.0), &o, 1920, 1080)
        );
    }

    #[test]
    fn fractional_scale_rounds_outwards() {
        // 1920x1080 at 1.25 is 1536x864 logical.
        let o = out("A", 0, 0, 1536, 864);
        // 10.5 * 1.25 = 13.125 -> 13; 20.3 * 1.25 = 25.375 -> 26.
        assert_eq!(
            clip(p(10.5, 0.0), p(20.3, 8.0), &o, 1920, 1080),
            Some(rect(13, 0, 13, 10))
        );
    }

    #[test]
    fn scale_one_and_a_half() {
        let o = out("A", 0, 0, 1280, 720);
        assert_eq!(
            clip(p(100.0, 100.0), p(200.0, 150.0), &o, 1920, 1080),
            Some(rect(150, 150, 150, 75))
        );
    }

    #[test]
    fn output_offsets_including_negative() {
        let left = out("L", -1366, 0, 1366, 768);
        let right = out("R", 0, 0, 1920, 1080);
        assert_eq!(
            clip(p(-1356.0, 10.0), p(-1256.0, 60.0), &left, 1366, 768),
            Some(rect(10, 10, 100, 50))
        );
        assert_eq!(
            clip(
                p(1930.0, 10.0),
                p(2030.0, 60.0),
                &out("R2", 1920, 0, 800, 600),
                800,
                600
            ),
            Some(rect(10, 10, 100, 50))
        );
        assert_eq!(
            clip(p(-10.0, 10.0), p(10.0, 20.0), &right, 1920, 1080),
            Some(rect(0, 10, 10, 10))
        );
    }

    #[test]
    fn drag_across_outputs_is_clipped_to_the_output() {
        let left = out("L", 0, 0, 1366, 768);
        // From (1300, 100) on the left output to (1500, 900) on the right one.
        assert_eq!(
            clip(p(1300.0, 100.0), p(1500.0, 900.0), &left, 1366, 768),
            Some(rect(1300, 100, 66, 668))
        );
    }

    #[test]
    fn nothing_left_on_other_output() {
        let o = out("A", 0, 0, 100, 100);
        assert_eq!(clip(p(200.0, 200.0), p(300.0, 300.0), &o, 100, 100), None);
    }

    #[test]
    fn click_is_not_a_selection() {
        let o = out("A", 0, 0, 1920, 1080);
        assert_eq!(resolve(p(10.0, 10.0), p(10.0, 10.0), &o, 1920, 1080), None);
        assert_eq!(resolve(p(10.0, 10.0), p(11.0, 30.0), &o, 1920, 1080), None);
        assert_eq!(
            resolve(p(10.0, 10.0), p(12.0, 12.0), &o, 1920, 1080),
            Some(rect(10, 10, 2, 2))
        );
    }

    #[test]
    fn state_machine() {
        let mut s = Selection::default();
        assert_eq!(s.release(p(1.0, 1.0)), None);

        s.press(p(1.0, 2.0));
        s.motion(p(5.0, 6.0));
        assert_eq!(
            s,
            Selection::Dragging {
                start: p(1.0, 2.0),
                end: p(5.0, 6.0)
            }
        );
        assert_eq!(s.release(p(7.0, 8.0)), Some((p(1.0, 2.0), p(7.0, 8.0))));
        assert_eq!(s, Selection::Idle);

        s.motion(p(9.0, 9.0));
        assert_eq!(s, Selection::Idle);
    }
}
