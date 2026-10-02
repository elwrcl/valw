//! Fitting the image (or its crop) into the canvas: image pixels ↔ egui
//! points.

use crate::editor::shape::P;
use crate::frame::PixelRect;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    /// Points per image pixel.
    pub scale: f32,
    /// Where `origin` is drawn, in points.
    pub offset: P,
    /// The image pixel at the top-left of what is shown (the crop's corner).
    pub origin: P,
}

impl Fit {
    /// Fits the whole image; see `with_crop`.
    #[cfg(test)]
    pub fn new(image: (u32, u32), area: (f32, f32, f32, f32), pixels_per_point: f32) -> Fit {
        let whole = PixelRect {
            x: 0,
            y: 0,
            width: image.0,
            height: image.1,
        };
        Fit::with_crop(whole, area, pixels_per_point)
    }

    /// Fits `crop` into `area` (x, y, w, h in points), centred, never above
    /// 100 % (one image pixel per physical pixel).
    pub fn with_crop(crop: PixelRect, area: (f32, f32, f32, f32), pixels_per_point: f32) -> Fit {
        let (iw, ih) = (crop.width as f32, crop.height as f32);
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
            origin: (crop.x as f32, crop.y as f32),
        }
    }

    pub fn to_screen(self, p: P) -> P {
        (
            self.offset.0 + (p.0 - self.origin.0) * self.scale,
            self.offset.1 + (p.1 - self.origin.1) * self.scale,
        )
    }

    pub fn to_image(self, p: P) -> P {
        (
            (p.0 - self.offset.0) / self.scale + self.origin.0,
            (p.1 - self.offset.1) / self.scale + self.origin.1,
        )
    }
}

/// `p` moved into `rect`.
pub fn clamp(p: P, rect: PixelRect) -> P {
    (
        p.0.clamp(rect.x as f32, (rect.x + rect.width) as f32),
        p.1.clamp(rect.y as f32, (rect.y + rect.height) as f32),
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
        let r = PixelRect {
            x: 0,
            y: 0,
            width: 100,
            height: 80,
        };
        assert_eq!(clamp((-5.0, 50.0), r), (0.0, 50.0));
        assert_eq!(clamp((150.0, 90.0), r), (100.0, 80.0));
    }

    #[test]
    fn a_crop_fills_the_area() {
        let crop = PixelRect {
            x: 100,
            y: 50,
            width: 200,
            height: 100,
        };
        let f = Fit::with_crop(crop, (0.0, 0.0, 400.0, 400.0), 1.0);
        assert_eq!(f.scale, 1.0);
        assert_eq!(
            f.to_screen((100.0, 50.0)),
            (100.0, 150.0),
            "the crop's corner is centred"
        );
    }

    #[test]
    fn round_trip_with_a_crop() {
        let crop = PixelRect {
            x: 640,
            y: 360,
            width: 640,
            height: 360,
        };
        let f = Fit::with_crop(crop, (0.0, 30.0, 500.0, 400.0), 1.0);
        for p in [(640.0, 360.0), (1279.0, 719.0), (800.5, 400.25)] {
            let q = f.to_image(f.to_screen(p));
            assert!(
                (q.0 - p.0).abs() < 1e-3 && (q.1 - p.1).abs() < 1e-3,
                "{p:?} {q:?}"
            );
        }
    }
}
