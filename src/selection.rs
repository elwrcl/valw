use crate::frame::{OutputGeom, PixelRect};

/// Selections smaller than this (in physical pixels) count as a click.
pub const MIN_PIXELS: u32 = 2;

/// A point in global logical coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// Modifier keys that shape a selection while dragging.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub alt: bool,
    pub space: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Axis {
    X,
    Y,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drag {
    /// The press point (the centre with Alt); Space moves it.
    start: Point,
    /// The far corner before Alt is applied.
    end: Point,
    /// The last pointer position.
    last: Point,
    /// Added to the pointer to get `end` (non-zero after a Space move).
    offset: (f64, f64),
    mods: Mods,
    /// Shift: `end` when Shift went down, and the axis that follows.
    lock: Option<(Point, Option<Axis>)>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum Selection {
    #[default]
    Idle,
    Dragging(Drag),
}

impl Selection {
    /// A press with no modifiers held (tests use it).
    #[cfg(test)]
    pub fn press(&mut self, p: Point) {
        self.press_with(p, Mods::default());
    }

    /// Starts a drag with `mods` already held. A held Shift doesn't lock an
    /// axis yet: there is nothing to keep until the selection has a size.
    pub fn press_with(&mut self, p: Point, mods: Mods) {
        *self = Selection::Dragging(Drag {
            start: p,
            end: p,
            last: p,
            offset: (0.0, 0.0),
            mods,
            lock: None,
        });
    }

    pub fn motion(&mut self, p: Point) {
        let Selection::Dragging(d) = self else { return };
        if d.mods.space {
            let (dx, dy) = (p.x - d.last.x, p.y - d.last.y);
            d.start = Point {
                x: d.start.x + dx,
                y: d.start.y + dy,
            };
            // A Shift lock moves along, or releasing Space would snap back.
            if let Some((frozen, _)) = &mut d.lock {
                *frozen = Point {
                    x: frozen.x + dx,
                    y: frozen.y + dy,
                };
            }
            d.end = Point {
                x: d.end.x + dx,
                y: d.end.y + dy,
            };
        } else {
            let target = Point {
                x: p.x + d.offset.0,
                y: p.y + d.offset.1,
            };
            d.end = match &mut d.lock {
                Some((frozen, axis)) => {
                    let axis = *axis.get_or_insert_with(|| {
                        if (target.x - frozen.x).abs() >= (target.y - frozen.y).abs() {
                            Axis::X
                        } else {
                            Axis::Y
                        }
                    });
                    match axis {
                        Axis::X => Point {
                            x: target.x,
                            y: frozen.y,
                        },
                        Axis::Y => Point {
                            x: frozen.x,
                            y: target.y,
                        },
                    }
                }
                None => target,
            };
        }
        d.last = p;
    }

    /// The keyboard's modifiers changed (also while idle, for the next drag).
    pub fn set_mods(&mut self, mods: Mods) {
        let Selection::Dragging(d) = self else { return };
        if mods.shift && !d.mods.shift {
            d.lock = Some((d.end, None));
        } else if !mods.shift {
            d.lock = None;
        }
        if d.mods.space && !mods.space {
            // Resize again from where the corner is, not where the pointer is.
            d.offset = (d.end.x - d.last.x, d.end.y - d.last.y);
        }
        d.mods = mods;
    }

    /// The selection's corners in global logical coordinates.
    pub fn corners(&self) -> Option<(Point, Point)> {
        let Selection::Dragging(d) = self else {
            return None;
        };
        if d.mods.alt {
            let opposite = Point {
                x: 2.0 * d.start.x - d.end.x,
                y: 2.0 * d.start.y - d.end.y,
            };
            Some((opposite, d.end))
        } else {
            Some((d.start, d.end))
        }
    }

    /// Ends the drag at `p` and returns its corners. Back to `Idle` either way.
    pub fn release(&mut self, p: Point) -> Option<(Point, Point)> {
        self.motion(p);
        let corners = self.corners();
        *self = Selection::Idle;
        corners
    }

    /// Space switches to window mode only before a drag starts.
    pub fn allows_window_switch(&self) -> bool {
        *self == Selection::Idle
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

/// One output's share of a selection and where it goes in the whole image.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub output: usize,
    /// In that output's frame, physical pixels.
    pub rect: PixelRect,
    /// Its top-left corner and size in the whole image (a corner can sit a
    /// pixel left of or above the image when scales differ).
    pub at: (i64, i64),
    pub size: (u32, u32),
}

/// A selection over one or more outputs: its parts and the whole image size.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub parts: Vec<Part>,
    pub size: (u32, u32),
}

impl Span {
    /// The output and rectangle when the selection is a plain crop of one
    /// output (today's region shot); `None` when it needs stitching.
    pub fn single(&self) -> Option<(usize, PixelRect)> {
        match self.parts.as_slice() {
            [p] if p.at == (0, 0)
                && p.size == self.size
                && (p.rect.width, p.rect.height) == self.size =>
            {
                Some((p.output, p.rect))
            }
            _ => None,
        }
    }
}

/// The drag from `a` to `b` over `outputs` (each output's geometry and frame
/// size), at the largest scale among the outputs it covers. `None` for a
/// click (under [`MIN_PIXELS`]) or a rectangle over no output.
///
/// A box inside one output is that output's plain crop, pixel for pixel.
/// Otherwise the image lies on the physical pixel grid at that scale: its
/// edges are the corners rounded outwards, and each part sits where its own
/// pixels fall on that grid, so neighbouring parts meet without a gap or an
/// overlap even when the corners are fractional (as pointer positions are).
pub fn span(a: Point, b: Point, outputs: &[(&OutputGeom, (u32, u32))]) -> Option<Span> {
    let (x0, y0) = (a.x.min(b.x), a.y.min(b.y));
    let (x1, y1) = (a.x.max(b.x), a.y.max(b.y));
    let covered: Vec<(usize, PixelRect, f64)> = outputs
        .iter()
        .enumerate()
        .filter_map(|(i, (g, (w, h)))| {
            let r = clip(a, b, g, *w, *h)?;
            Some((i, r, *w as f64 / g.width as f64))
        })
        .collect();
    let inside = |g: &OutputGeom| {
        x0 >= g.x as f64
            && y0 >= g.y as f64
            && x1 <= (g.x + g.width) as f64
            && y1 <= (g.y + g.height) as f64
    };
    if let Some(&(i, rect, _)) = covered.iter().find(|c| inside(outputs[c.0].0)) {
        let size = (rect.width, rect.height);
        if size.0 < MIN_PIXELS || size.1 < MIN_PIXELS {
            return None;
        }
        let part = Part {
            output: i,
            rect,
            at: (0, 0),
            size,
        };
        return Some(Span {
            parts: vec![part],
            size,
        });
    }
    let scale = covered.iter().map(|c| c.2).fold(0.0, f64::max);
    if scale == 0.0 {
        return None;
    }
    let (left, top) = ((x0 * scale).floor(), (y0 * scale).floor());
    let size = (
        ((x1 * scale).ceil() - left) as u32,
        ((y1 * scale).ceil() - top) as u32,
    );
    if size.0 < MIN_PIXELS || size.1 < MIN_PIXELS {
        return None;
    }
    let parts = covered
        .into_iter()
        .map(|(i, rect, own)| {
            let g = outputs[i].0;
            let k = scale / own;
            let at = (
                (g.x as f64 * scale + rect.x as f64 * k).round() as i64 - left as i64,
                (g.y as f64 * scale + rect.y as f64 * k).round() as i64 - top as i64,
            );
            let size = (
                (rect.width as f64 * k).round() as u32,
                (rect.height as f64 * k).round() as u32,
            );
            Part {
                output: i,
                rect,
                at,
                size,
            }
        })
        .collect();
    Some(Span { parts, size })
}

/// The output a global point is over, or the nearest one (`outputs` is never
/// empty: there is one surface per output).
pub fn locate(p: Point, outputs: &[(&OutputGeom, (u32, u32))]) -> usize {
    let distance = |g: &OutputGeom| {
        let dx = (g.x as f64 - p.x)
            .max(p.x - (g.x + g.width) as f64)
            .max(0.0);
        let dy = (g.y as f64 - p.y)
            .max(p.y - (g.y + g.height) as f64)
            .max(0.0);
        dx * dx + dy * dy
    };
    (0..outputs.len())
        .min_by(|&i, &j| distance(outputs[i].0).total_cmp(&distance(outputs[j].0)))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_switch_only_before_a_drag() {
        let mut s = Selection::default();
        assert!(s.allows_window_switch());
        s.press(Point { x: 1.0, y: 1.0 });
        assert!(!s.allows_window_switch(), "Space during a drag is reserved");
        s.release(Point { x: 5.0, y: 5.0 });
        assert!(s.allows_window_switch());
    }

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
        let outputs = [(&o, (1920, 1080))];
        assert_eq!(span(p(10.0, 10.0), p(10.0, 10.0), &outputs), None);
        assert_eq!(span(p(10.0, 10.0), p(11.0, 30.0), &outputs), None);
        assert_eq!(
            span(p(10.0, 10.0), p(12.0, 12.0), &outputs).and_then(|s| s.single()),
            Some((0, rect(10, 10, 2, 2)))
        );
    }

    #[test]
    fn state_machine() {
        let mut s = Selection::default();
        assert_eq!(s.release(p(1.0, 1.0)), None);

        s.press(p(1.0, 2.0));
        s.motion(p(5.0, 6.0));
        assert_eq!(s.corners(), Some((p(1.0, 2.0), p(5.0, 6.0))));
        assert_eq!(s.release(p(7.0, 8.0)), Some((p(1.0, 2.0), p(7.0, 8.0))));
        assert_eq!(s, Selection::Idle);

        s.motion(p(9.0, 9.0));
        assert_eq!(s, Selection::Idle);
    }

    fn pt(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    const SHIFT: Mods = Mods {
        shift: true,
        alt: false,
        space: false,
    };
    const ALT: Mods = Mods {
        shift: false,
        alt: true,
        space: false,
    };
    const SPACE: Mods = Mods {
        shift: false,
        alt: false,
        space: true,
    };

    #[test]
    fn plain_drag_is_press_to_pointer() {
        let mut s = Selection::default();
        s.press(pt(10.0, 10.0));
        s.motion(pt(50.0, 30.0));
        assert_eq!(s.corners(), Some((pt(10.0, 10.0), pt(50.0, 30.0))));
    }

    #[test]
    fn shift_locks_the_dimension_that_moves_first() {
        let mut s = Selection::default();
        s.press(pt(0.0, 0.0));
        s.motion(pt(40.0, 30.0));
        s.set_mods(SHIFT);
        s.motion(pt(60.0, 32.0)); // mostly horizontal: width follows
        assert_eq!(s.corners(), Some((pt(0.0, 0.0), pt(60.0, 30.0))));
        s.motion(pt(70.0, 90.0)); // the axis stays decided
        assert_eq!(s.corners(), Some((pt(0.0, 0.0), pt(70.0, 30.0))));
        s.set_mods(Mods::default());
        s.motion(pt(70.0, 90.0));
        assert_eq!(
            s.corners(),
            Some((pt(0.0, 0.0), pt(70.0, 90.0))),
            "plain again"
        );

        let mut v = Selection::default();
        v.press(pt(0.0, 0.0));
        v.motion(pt(40.0, 30.0));
        v.set_mods(SHIFT);
        v.motion(pt(41.0, 80.0)); // mostly vertical: height follows
        assert_eq!(v.corners(), Some((pt(0.0, 0.0), pt(40.0, 80.0))));
    }

    #[test]
    fn alt_grows_from_the_centre() {
        let mut s = Selection::default();
        s.press(pt(100.0, 100.0));
        s.set_mods(ALT);
        s.motion(pt(130.0, 120.0));
        assert_eq!(s.corners(), Some((pt(70.0, 80.0), pt(130.0, 120.0))));
    }

    #[test]
    fn shift_and_alt_combine() {
        let mut s = Selection::default();
        s.press(pt(100.0, 100.0));
        s.motion(pt(120.0, 110.0));
        s.set_mods(Mods {
            shift: true,
            alt: true,
            space: false,
        });
        s.motion(pt(150.0, 112.0));
        assert_eq!(s.corners(), Some((pt(50.0, 90.0), pt(150.0, 110.0))));
    }

    #[test]
    fn space_moves_then_resizing_continues_without_a_jump() {
        let mut s = Selection::default();
        s.press(pt(0.0, 0.0));
        s.motion(pt(40.0, 30.0));
        s.set_mods(SPACE);
        s.motion(pt(50.0, 40.0));
        assert_eq!(s.corners(), Some((pt(10.0, 10.0), pt(50.0, 40.0))));
        s.motion(pt(60.0, 40.0));
        assert_eq!(s.corners(), Some((pt(20.0, 10.0), pt(60.0, 40.0))));
        s.set_mods(Mods::default());
        s.motion(pt(60.0, 40.0));
        assert_eq!(
            s.corners(),
            Some((pt(20.0, 10.0), pt(60.0, 40.0))),
            "no jump on release"
        );
        s.motion(pt(70.0, 45.0));
        assert_eq!(s.corners(), Some((pt(20.0, 10.0), pt(70.0, 45.0))));
    }

    #[test]
    fn space_with_a_pointer_away_from_the_corner_keeps_the_offset() {
        let mut s = Selection::default();
        s.press(pt(0.0, 0.0));
        s.motion(pt(40.0, 30.0));
        s.set_mods(SPACE);
        s.motion(pt(45.0, 30.0));
        s.set_mods(Mods::default());
        s.motion(pt(55.0, 40.0));
        assert_eq!(s.corners(), Some((pt(5.0, 0.0), pt(55.0, 40.0))));
    }

    #[test]
    fn moving_with_space_carries_the_shift_lock_along() {
        let mut s = Selection::default();
        s.press(pt(0.0, 0.0));
        s.motion(pt(40.0, 30.0));
        s.set_mods(SHIFT);
        s.motion(pt(60.0, 31.0)); // axis X
        s.set_mods(Mods {
            shift: true,
            alt: false,
            space: true,
        });
        s.motion(pt(60.0, 131.0)); // moved 100 down
        assert_eq!(s.corners(), Some((pt(0.0, 100.0), pt(60.0, 130.0))));
        s.set_mods(SHIFT);
        s.motion(pt(70.0, 131.0));
        assert_eq!(
            s.corners(),
            Some((pt(0.0, 100.0), pt(70.0, 130.0))),
            "no jump back"
        );
    }

    #[test]
    fn shift_held_at_the_press_does_not_freeze_the_press_point() {
        let mut s = Selection::default();
        s.press_with(pt(10.0, 10.0), SHIFT);
        s.set_mods(SHIFT);
        s.motion(pt(60.0, 40.0));
        assert_eq!(
            s.corners(),
            Some((pt(10.0, 10.0), pt(60.0, 40.0))),
            "a real rectangle"
        );
    }

    #[test]
    fn release_applies_the_held_modifiers() {
        let mut s = Selection::default();
        s.press(pt(100.0, 100.0));
        s.set_mods(ALT);
        assert_eq!(
            s.release(pt(110.0, 105.0)),
            Some((pt(90.0, 95.0), pt(110.0, 105.0)))
        );
        assert_eq!(s, Selection::Idle);
    }

    /// The user's layout: the laptop at the origin, the monitor above it,
    /// wider and 277 px to the left.
    fn layout() -> (OutputGeom, OutputGeom) {
        (
            out("LVDS-1", 0, 0, 1366, 768),
            out("HDMI-A-1", -277, -1080, 1920, 1080),
        )
    }

    #[test]
    fn a_span_inside_one_output_is_todays_clip() {
        let (laptop, monitor) = layout();
        let outputs = [(&laptop, (1366, 768)), (&monitor, (1920, 1080))];
        let s = span(p(100.0, 100.0), p(300.0, 250.0), &outputs).unwrap();
        assert_eq!(s.size, (200, 150));
        assert_eq!(s.single(), Some((0, rect(100, 100, 200, 150))));
    }

    #[test]
    fn a_span_from_the_laptop_up_onto_the_monitor() {
        let (laptop, monitor) = layout();
        let outputs = [(&laptop, (1366, 768)), (&monitor, (1920, 1080))];
        let s = span(p(100.0, 500.0), p(600.0, -300.0), &outputs).unwrap();
        assert_eq!(s.size, (500, 800));
        assert_eq!(
            s.parts,
            vec![
                Part {
                    output: 0,
                    rect: rect(100, 0, 500, 500),
                    at: (0, 300),
                    size: (500, 500)
                },
                Part {
                    output: 1,
                    rect: rect(377, 780, 500, 300),
                    at: (0, 0),
                    size: (500, 300)
                },
            ]
        );
        assert_eq!(s.single(), None);
        // The parts meet without a gap or an overlap.
        assert_eq!(s.parts[1].at.1 + s.parts[1].size.1 as i64, s.parts[0].at.1);
    }

    #[test]
    fn a_span_over_a_gap_keeps_the_whole_box() {
        let (laptop, monitor) = layout();
        let outputs = [(&laptop, (1366, 768)), (&monitor, (1920, 1080))];
        let s = span(p(-200.0, -200.0), p(300.0, 300.0), &outputs).unwrap();
        assert_eq!(s.size, (500, 500));
        assert_eq!(
            s.parts,
            vec![
                Part {
                    output: 0,
                    rect: rect(0, 0, 300, 300),
                    at: (200, 200),
                    size: (300, 300)
                },
                Part {
                    output: 1,
                    rect: rect(77, 880, 500, 200),
                    at: (0, 0),
                    size: (500, 200)
                },
            ]
        );
        // One output touched, but the box reaches past it: not a plain crop.
        let corner = span(p(-50.0, 100.0), p(200.0, 300.0), &outputs).unwrap();
        assert_eq!(corner.parts.len(), 1);
        assert_eq!(corner.single(), None);
    }

    #[test]
    fn a_span_takes_the_largest_scale() {
        let a = out("A", 0, 0, 100, 100);
        let b = out("B", 100, 0, 100, 100);
        let outputs = [(&a, (200, 200)), (&b, (100, 100))];
        let s = span(p(50.0, 10.0), p(150.0, 60.0), &outputs).unwrap();
        assert_eq!(s.size, (200, 100));
        assert_eq!(
            s.parts,
            vec![
                Part {
                    output: 0,
                    rect: rect(100, 20, 100, 100),
                    at: (0, 0),
                    size: (100, 100)
                },
                Part {
                    output: 1,
                    rect: rect(0, 10, 50, 50),
                    at: (100, 0),
                    size: (100, 100)
                },
            ]
        );
    }

    #[test]
    fn a_fractional_box_on_one_output_is_its_plain_crop() {
        let (laptop, monitor) = layout();
        let outputs = [(&laptop, (1366, 768)), (&monitor, (1920, 1080))];
        let (a, b) = (p(100.6, 100.6), p(300.2, 250.2));
        let s = span(a, b, &outputs).unwrap();
        let crop = clip(a, b, &laptop, 1366, 768).unwrap();
        assert_eq!(s.single(), Some((0, crop)));
        assert_eq!(s.size, (crop.width, crop.height));
        // A 1.25-scale output: the crop is what clip gives, not a resampled one.
        let o = out("A", 0, 0, 1000, 800);
        let scaled = [(&o, (1250, 1000))];
        let (a, b) = (p(101.0, 101.0), p(301.0, 251.0));
        assert_eq!(
            span(a, b, &scaled).unwrap().single(),
            Some((0, clip(a, b, &o, 1250, 1000).unwrap()))
        );
    }

    #[test]
    fn fractional_seams_meet_exactly_and_fill_the_image() {
        let (laptop, monitor) = layout();
        let outputs = [(&laptop, (1366, 768)), (&monitor, (1920, 1080))];
        for (top, bottom) in [(-300.4, 500.6), (-300.6, 500.6), (-300.5, 500.2)] {
            let s = span(p(100.3, bottom), p(600.7, top), &outputs).unwrap();
            let (laptop_part, monitor_part) = (&s.parts[0], &s.parts[1]);
            assert_eq!(monitor_part.at.1, 0, "{top}: the monitor starts the image");
            assert_eq!(
                monitor_part.at.1 + monitor_part.size.1 as i64,
                laptop_part.at.1,
                "{top}: no doubled or missing row at the seam"
            );
            assert_eq!(
                laptop_part.at.1 + laptop_part.size.1 as i64,
                s.size.1 as i64,
                "{top}: the last row is covered"
            );
            assert_eq!(laptop_part.size.0, s.size.0, "{top}: full width");
        }
        // Mixed scales with a fractional corner: the parts tile the width.
        let a = out("A", 0, 0, 100, 100);
        let b = out("B", 100, 0, 100, 100);
        let mixed = [(&a, (200, 200)), (&b, (100, 100))];
        let s = span(p(50.3, 10.0), p(150.7, 60.0), &mixed).unwrap();
        assert_eq!(s.parts[0].at.0, 0);
        assert_eq!(s.parts[0].at.0 + s.parts[0].size.0 as i64, s.parts[1].at.0);
        assert_eq!(s.parts[1].at.0 + s.parts[1].size.0 as i64, s.size.0 as i64);
    }

    #[test]
    fn a_tiny_span_is_a_click() {
        let (laptop, monitor) = layout();
        let outputs = [(&laptop, (1366, 768)), (&monitor, (1920, 1080))];
        assert_eq!(span(p(10.0, 10.0), p(11.0, 30.0), &outputs), None);
    }

    #[test]
    fn locate_finds_the_output_or_the_nearest() {
        let (laptop, monitor) = layout();
        let outputs = [(&laptop, (1366, 768)), (&monitor, (1920, 1080))];
        assert_eq!(locate(p(500.0, 500.0), &outputs), 0);
        assert_eq!(locate(p(500.0, -500.0), &outputs), 1);
        // Left of the laptop, under the monitor: the laptop is 100 px away.
        assert_eq!(locate(p(-100.0, 300.0), &outputs), 0);
    }
}
