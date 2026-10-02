//! Fitting the image into the canvas: image pixels ↔ egui points.

use crate::editor::shape::P;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    /// Points per image pixel.
    pub scale: f32,
    /// Where the image's top-left corner is, in points.
    pub offset: P,
}

impl Fit {
    /// Fits `image` into `area` (x, y, w, h in points), centred, never
    /// above 100 % (one image pixel per physical pixel).
    pub fn new(image: (u32, u32), area: (f32, f32, f32, f32), pixels_per_point: f32) -> Fit {
        let (iw, ih) = (image.0 as f32, image.1 as f32);
        let (x, y, w, h) = area;
        let scale = (w / iw).min(h / ih).min(1.0 / pixels_per_point);
        // Centred, then onto a physical pixel boundary so that at 100 % each
        // image pixel is exactly one screen pixel (no half-texel blur).
        let snap = |v: f32| (v * pixels_per_point).floor() / pixels_per_point;
        Fit {
            scale,
            offset: (
                snap(x + (w - iw * scale) / 2.0),
                snap(y + (h - ih * scale) / 2.0),
            ),
        }
    }

    pub fn to_screen(self, p: P) -> P {
        (
            self.offset.0 + p.0 * self.scale,
            self.offset.1 + p.1 * self.scale,
        )
    }

    pub fn to_image(self, p: P) -> P {
        (
            (p.0 - self.offset.0) / self.scale,
            (p.1 - self.offset.1) / self.scale,
        )
    }
}

/// `p` moved onto the image's area.
pub fn clamp(p: P, image: (u32, u32)) -> P {
    (
        p.0.clamp(0.0, image.0 as f32),
        p.1.clamp(0.0, image.1 as f32),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_images_are_not_upscaled() {
        let f = Fit::new((400, 300), (0.0, 50.0, 1000.0, 800.0), 1.0);
        assert_eq!(f.scale, 1.0);
        assert_eq!(f.offset, (300.0, 300.0), "centred");
    }

    #[test]
    fn large_images_fit_the_area() {
        let f = Fit::new((1920, 1080), (0.0, 40.0, 960.0, 1000.0), 1.0);
        assert_eq!(f.scale, 0.5);
        assert_eq!(f.offset, (0.0, 40.0 + (1000.0 - 540.0) / 2.0));
    }

    #[test]
    fn hundred_percent_means_one_image_pixel_per_physical_pixel() {
        let f = Fit::new((400, 300), (0.0, 0.0, 1000.0, 800.0), 2.0);
        assert_eq!(
            f.scale, 0.5,
            "0.5 points per image px at 2× = 1 physical px"
        );
    }

    #[test]
    fn the_image_sits_on_physical_pixels() {
        // 1001 - 400 is odd: centring alone lands on half a pixel.
        let f = Fit::new((400, 300), (0.0, 0.0, 1001.0, 801.0), 1.0);
        assert_eq!(f.offset, (300.0, 250.0));
        let g = Fit::new((400, 300), (0.0, 0.0, 1001.0, 801.0), 1.5);
        let px = (g.offset.0 * 1.5, g.offset.1 * 1.5);
        assert_eq!(px, (px.0.round(), px.1.round()), "{g:?}");
    }

    #[test]
    fn round_trip() {
        let f = Fit::new((1920, 1080), (10.0, 40.0, 1000.0, 700.0), 1.25);
        for p in [(0.0, 0.0), (1919.0, 1079.0), (333.5, 777.25)] {
            let q = f.to_image(f.to_screen(p));
            assert!(
                (q.0 - p.0).abs() < 1e-3 && (q.1 - p.1).abs() < 1e-3,
                "{p:?} {q:?}"
            );
        }
    }

    #[test]
    fn clamp_keeps_points_on_the_image() {
        assert_eq!(clamp((-5.0, 50.0), (100, 80)), (0.0, 50.0));
        assert_eq!(clamp((150.0, 90.0), (100, 80)), (100.0, 80.0));
    }
}
