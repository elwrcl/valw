//! Markup tools and the geometry both painters share: the canvas (egui)
//! and the export (tiny-skia) draw the same primitives, in image pixels.

pub type P = (f32, f32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Arrow,
    Rectangle,
    Ellipse,
    Line,
    Pen,
    Highlighter,
}

impl Tool {
    pub const ALL: [Tool; 6] = [
        Tool::Arrow,
        Tool::Rectangle,
        Tool::Ellipse,
        Tool::Line,
        Tool::Pen,
        Tool::Highlighter,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Arrow => "Arrow",
            Tool::Rectangle => "Rectangle",
            Tool::Ellipse => "Ellipse",
            Tool::Line => "Line",
            Tool::Pen => "Pen",
            Tool::Highlighter => "Highlighter",
        }
    }

    fn freehand(self) -> bool {
        matches!(self, Tool::Pen | Tool::Highlighter)
    }
}

/// The toolbar's colours; the first is the default.
pub const PALETTE: [[u8; 3]; 7] = [
    [0xff, 0x3b, 0x30],
    [0xff, 0x95, 0x00],
    [0xff, 0xcc, 0x00],
    [0x34, 0xc7, 0x59],
    [0x00, 0x7a, 0xff],
    [0x00, 0x00, 0x00],
    [0xff, 0xff, 0xff],
];

/// S, M, L in image pixels; M is the default.
pub const WIDTHS: [(&str, f32); 3] = [("S", 2.0), ("M", 4.0), ("L", 8.0)];

const HIGHLIGHT_ALPHA: u8 = 102;
const CLICK: f32 = 2.0;
const ELLIPSE_POINTS: usize = 72;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    pub color: [u8; 3],
    pub width: f32,
}

/// One finished mark. Two-point tools hold [start, end]; Pen and
/// Highlighter hold every point.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub tool: Tool,
    pub style: Style,
    pub points: Vec<P>,
}

impl Shape {
    /// A press without a real drag. A Pen click still leaves a dot.
    pub fn is_click(&self) -> bool {
        if self.tool == Tool::Pen {
            return false;
        }
        let first = self.points[0];
        self.points
            .iter()
            .all(|p| (p.0 - first.0).hypot(p.1 - first.1) < CLICK)
    }
}

/// A shape being drawn.
#[derive(Debug, Clone)]
pub struct Drag {
    tool: Tool,
    style: Style,
    points: Vec<P>,
}

impl Drag {
    pub fn new(tool: Tool, style: Style, start: P) -> Drag {
        Drag {
            tool,
            style,
            points: vec![start],
        }
    }

    pub fn move_to(&mut self, p: P) {
        if self.tool.freehand() || self.points.len() == 1 {
            self.points.push(p);
        } else {
            self.points[1] = p;
        }
    }

    /// The shape so far; `shift` applies the tool's constraint.
    pub fn shape(&self, shift: bool) -> Shape {
        let mut points = self.points.clone();
        if !self.tool.freehand() && points.len() == 2 {
            points[1] = constrain(self.tool, points[0], points[1], shift);
        }
        Shape {
            tool: self.tool,
            style: self.style,
            points,
        }
    }
}

/// Shift: lines and arrows snap to 45° steps, rectangles and ellipses
/// become squares and circles.
pub fn constrain(tool: Tool, start: P, end: P, shift: bool) -> P {
    if !shift {
        return end;
    }
    let (dx, dy) = (end.0 - start.0, end.1 - start.1);
    match tool {
        Tool::Line | Tool::Arrow => {
            let step = std::f32::consts::FRAC_PI_4;
            let angle = (dy.atan2(dx) / step).round() * step;
            let len = dx.hypot(dy);
            (start.0 + len * angle.cos(), start.1 + len * angle.sin())
        }
        Tool::Rectangle | Tool::Ellipse => {
            let side = dx.abs().max(dy.abs());
            (start.0 + side.copysign(dx), start.1 + side.copysign(dy))
        }
        Tool::Pen | Tool::Highlighter => end,
    }
}

/// A primitive in image pixels. Colours are straight (not premultiplied) RGBA.
#[derive(Debug, Clone, PartialEq)]
pub enum Prim {
    Stroke {
        points: Vec<P>,
        width: f32,
        color: [u8; 4],
        closed: bool,
        /// Round caps and joins; false means butt caps.
        round: bool,
    },
    /// A convex polygon.
    Fill { points: Vec<P>, color: [u8; 4] },
}

pub fn geometry(shape: &Shape) -> Vec<Prim> {
    let [r, g, b] = shape.style.color;
    let opaque = [r, g, b, 255];
    let w = shape.style.width;
    let pts = &shape.points;
    let stroke = |points: Vec<P>, closed: bool| Prim::Stroke {
        points,
        width: w,
        color: opaque,
        closed,
        round: true,
    };
    match shape.tool {
        Tool::Line => vec![stroke(vec![pts[0], pts[1]], false)],
        Tool::Arrow => arrow(pts[0], pts[1], w, opaque),
        Tool::Rectangle => {
            let ((x0, y0), (x1, y1)) = (pts[0], pts[1]);
            vec![stroke(vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)], true)]
        }
        Tool::Ellipse => {
            let ((x0, y0), (x1, y1)) = (pts[0], pts[1]);
            let (cx, cy, rx, ry) = (
                (x0 + x1) / 2.0,
                (y0 + y1) / 2.0,
                (x1 - x0).abs() / 2.0,
                (y1 - y0).abs() / 2.0,
            );
            vec![stroke(polygon(cx, cy, rx, ry, ELLIPSE_POINTS), true)]
        }
        Tool::Pen if pts.len() == 1 => {
            let (x, y) = pts[0];
            vec![Prim::Fill {
                points: polygon(x, y, w / 2.0, w / 2.0, 24),
                color: opaque,
            }]
        }
        Tool::Pen => vec![stroke(pts.clone(), false)],
        Tool::Highlighter => vec![Prim::Stroke {
            points: pts.clone(),
            width: w * 4.0,
            color: [r, g, b, HIGHLIGHT_ALPHA],
            closed: false,
            round: false,
        }],
    }
}

/// A line to the base of a filled head whose tip is at `end`.
fn arrow(start: P, end: P, width: f32, color: [u8; 4]) -> Vec<Prim> {
    let (dx, dy) = (end.0 - start.0, end.1 - start.1);
    let len = dx.hypot(dy).max(f32::EPSILON);
    let (ux, uy) = (dx / len, dy / len);
    let head = (4.0 * width).max(12.0).min(len);
    let half = head * 30f32.to_radians().tan();
    let base = (end.0 - ux * head, end.1 - uy * head);
    vec![
        Prim::Stroke {
            points: vec![start, base],
            width,
            color,
            closed: false,
            round: true,
        },
        Prim::Fill {
            points: vec![
                end,
                (base.0 - uy * half, base.1 + ux * half),
                (base.0 + uy * half, base.1 - ux * half),
            ],
            color,
        },
    ]
}

/// `n` points around an ellipse, starting at angle 0.
fn polygon(cx: f32, cy: f32, rx: f32, ry: f32, n: usize) -> Vec<P> {
    (0..n)
        .map(|i| {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            (cx + rx * a.cos(), cy + ry * a.sin())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Style = Style {
        color: [255, 59, 48],
        width: 4.0,
    };

    fn shape(tool: Tool, points: &[(f32, f32)]) -> Shape {
        Shape {
            tool,
            style: RED,
            points: points.to_vec(),
        }
    }

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3
    }

    #[test]
    fn shift_snaps_lines_to_45_degrees() {
        let end = constrain(Tool::Line, (0.0, 0.0), (100.0, 10.0), true);
        assert!(close(end, ((100f32.hypot(10.0)), 0.0)), "{end:?}");
        let end = constrain(Tool::Arrow, (0.0, 0.0), (50.0, 45.0), true);
        let d = 50f32.hypot(45.0) / 2f32.sqrt();
        assert!(close(end, (d, d)), "{end:?}");
        assert_eq!(
            constrain(Tool::Line, (0.0, 0.0), (100.0, 10.0), false),
            (100.0, 10.0)
        );
    }

    #[test]
    fn shift_makes_squares_and_circles() {
        assert_eq!(
            constrain(Tool::Rectangle, (10.0, 10.0), (40.0, 20.0), true),
            (40.0, 40.0)
        );
        assert_eq!(
            constrain(Tool::Ellipse, (10.0, 10.0), (0.0, 60.0), true),
            (-40.0, 60.0)
        );
    }

    #[test]
    fn clicks_are_discarded_but_a_pen_dot_is_kept() {
        assert!(shape(Tool::Arrow, &[(5.0, 5.0), (6.0, 6.0)]).is_click());
        assert!(!shape(Tool::Arrow, &[(5.0, 5.0), (8.0, 5.0)]).is_click());
        assert!(shape(Tool::Highlighter, &[(5.0, 5.0), (5.5, 5.0)]).is_click());
        assert!(!shape(Tool::Pen, &[(5.0, 5.0)]).is_click());
    }

    #[test]
    fn arrow_has_a_line_and_a_head_at_the_end() {
        let g = geometry(&shape(Tool::Arrow, &[(0.0, 0.0), (100.0, 0.0)]));
        assert_eq!(g.len(), 2);
        let Prim::Stroke { points, width, .. } = &g[0] else {
            panic!("{g:?}")
        };
        assert_eq!(*width, 4.0);
        assert!(
            close(points[0], (0.0, 0.0)) && close(points[1], (84.0, 0.0)),
            "{points:?}"
        );
        let Prim::Fill { points, .. } = &g[1] else {
            panic!("{g:?}")
        };
        // Head length max(12, 4 × 4) = 16, half-width 16 × tan 30°.
        let half = 16.0 * (30f32).to_radians().tan();
        assert!(close(points[0], (100.0, 0.0)), "{points:?}");
        assert!(
            close(points[1], (84.0, half)) && close(points[2], (84.0, -half)),
            "{points:?}"
        );
    }

    #[test]
    fn highlighter_is_wide_and_translucent() {
        let g = geometry(&shape(Tool::Highlighter, &[(0.0, 0.0), (50.0, 0.0)]));
        let Prim::Stroke {
            width,
            color,
            round,
            ..
        } = &g[0]
        else {
            panic!("{g:?}")
        };
        assert_eq!(*width, 16.0);
        assert_eq!(*color, [255, 59, 48, 102]);
        assert!(!round, "butt caps: overlapping caps would darken");
    }

    #[test]
    fn rectangle_and_ellipse_are_closed_outlines() {
        let g = geometry(&shape(Tool::Rectangle, &[(10.0, 10.0), (0.0, 20.0)]));
        let Prim::Stroke { points, closed, .. } = &g[0] else {
            panic!("{g:?}")
        };
        assert!(*closed);
        assert_eq!(
            points,
            &[(10.0, 10.0), (0.0, 10.0), (0.0, 20.0), (10.0, 20.0)]
        );
        let g = geometry(&shape(Tool::Ellipse, &[(0.0, 0.0), (20.0, 10.0)]));
        let Prim::Stroke { points, closed, .. } = &g[0] else {
            panic!("{g:?}")
        };
        assert!(*closed && points.len() == 72);
        assert!(
            points
                .iter()
                .all(|p| ((p.0 - 10.0) / 10.0).powi(2) + ((p.1 - 5.0) / 5.0).powi(2) - 1.0 < 1e-3)
        );
    }

    #[test]
    fn a_pen_dot_is_a_filled_circle() {
        let g = geometry(&shape(Tool::Pen, &[(5.0, 5.0)]));
        let Prim::Fill { points, color } = &g[0] else {
            panic!("{g:?}")
        };
        assert_eq!(*color, [255, 59, 48, 255]);
        assert!(
            points
                .iter()
                .all(|p| ((p.0 - 5.0).hypot(p.1 - 5.0) - 2.0).abs() < 1e-3)
        );
    }

    #[test]
    fn drag_collects_points_per_tool() {
        let mut d = Drag::new(Tool::Pen, RED, (0.0, 0.0));
        d.move_to((1.0, 1.0));
        d.move_to((2.0, 3.0));
        assert_eq!(
            d.shape(false).points,
            vec![(0.0, 0.0), (1.0, 1.0), (2.0, 3.0)]
        );
        let mut d = Drag::new(Tool::Rectangle, RED, (0.0, 0.0));
        d.move_to((5.0, 9.0));
        d.move_to((30.0, 10.0));
        assert_eq!(d.shape(false).points, vec![(0.0, 0.0), (30.0, 10.0)]);
        assert_eq!(d.shape(true).points, vec![(0.0, 0.0), (30.0, 30.0)]);
    }
}
