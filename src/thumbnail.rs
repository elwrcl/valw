//! One preview thumbnail: its layer surface, pixels, animation and gestures.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use image::{RgbaImage, imageops};
use smithay_client_toolkit::{
    compositor::{CompositorState, FrameCallbackData, Region},
    shell::{
        WaylandSurface,
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerShell, LayerSurface},
    },
    shm::{
        Shm,
        slot::{Buffer, SlotPool},
    },
};
use wayland_client::{QueueHandle, protocol::wl_shm};
use wayland_protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
};

use crate::stack::{self, Anim, EDGE_MARGIN, Release};
use crate::wayland::{Output, State};

pub const NAMESPACE: &str = "valw-preview";

/// Premultiplied BGRA of the 1 px border: #e0e0e0 at 80% opacity.
const BORDER: [u8; 4] = [179, 179, 179, 204];

/// Loads the screenshot a thumbnail shows. The format comes from the file's
/// contents, not its name: save.filename and -o needn't end in .png.
pub fn load(path: &Path) -> Result<RgbaImage> {
    let read = || -> Result<RgbaImage> {
        Ok(image::ImageReader::open(path)?
            .with_guessed_format()?
            .decode()?
            .to_rgba8())
    };
    read().with_context(|| format!("could not read {}", path.display()))
}

/// The thumbnail image for `img` on an output with `scale` physical pixels
/// per logical pixel: returns the physical-size image and its logical size.
pub fn downscale(img: &RgbaImage, scale: f64) -> (RgbaImage, (u32, u32)) {
    let logical = stack::thumb_size(img.width(), img.height());
    let physical = (
        ((logical.0 as f64 * scale).round() as u32).max(1),
        ((logical.1 as f64 * scale).round() as u32).max(1),
    );
    let small = if physical == img.dimensions() {
        img.clone()
    } else {
        imageops::resize(
            img,
            physical.0,
            physical.1,
            imageops::FilterType::CatmullRom,
        )
    };
    (small, logical)
}

/// Paints `small` into a transparent ARGB8888 `canvas` that is `canvas_width`
/// px wide and as tall as `small`, starting at column `x0` and clipped to the
/// canvas, with a 1 px border on the image's edges.
pub fn paint(canvas: &mut [u8], canvas_width: u32, small: &RgbaImage, x0: i64) {
    canvas.fill(0);
    let (w, h) = small.dimensions();
    for y in 0..h {
        for x in 0..w {
            let cx = x0 + x as i64;
            if cx < 0 || cx >= canvas_width as i64 {
                continue;
            }
            let edge = x == 0 || y == 0 || x == w - 1 || y == h - 1;
            let px = if edge {
                BORDER
            } else {
                let [r, g, b, _] = small.get_pixel(x, y).0;
                [b, g, r, 255]
            };
            let i = ((y * canvas_width) as usize + cx as usize) * 4;
            canvas[i..i + 4].copy_from_slice(&px);
        }
    }
}

/// What the host should do after a pointer release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    OpenEditor,
}

pub struct Thumbnail {
    pub id: u64,
    pub output: String,
    pub path: PathBuf,
    layer: LayerSurface,
    viewport: WpViewport,
    pool: SlotPool,
    buffer: Option<Buffer>,
    small: RgbaImage,
    /// Logical size of the image (not the surface, which has padding).
    pub size: (u32, u32),
    scale: f64,
    /// How far right of its resting place the image is drawn, logical px.
    offset: f64,
    anim: Option<(Anim, Instant)>,
    drag: Option<(f64, f64)>,
    margin: u32,
    /// Input region while visible (the image) and while hidden (nothing).
    input: Region,
    no_input: Region,
    /// Hidden for a capture: drawn fully transparent, clicks pass through.
    hidden: bool,
    configured: bool,
    frame_pending: bool,
    started: bool,
    closing: bool,
    expired: bool,
}

/// The Wayland objects a new thumbnail needs.
pub struct Parts<'a> {
    pub compositor: &'a CompositorState,
    pub layer_shell: &'a LayerShell,
    pub viewporter: &'a WpViewporter,
    pub shm: &'a Shm,
}

impl Thumbnail {
    pub fn new(
        id: u64,
        path: PathBuf,
        image: &RgbaImage,
        output: &Output,
        parts: &Parts,
        qh: &QueueHandle<State>,
    ) -> Result<Thumbnail> {
        let (small, size) = downscale(image, output.scale);
        let surface = parts.compositor.create_surface(qh);
        let layer = parts.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Overlay,
            Some(NAMESPACE),
            Some(&output.wl),
        );
        layer.set_anchor(Anchor::BOTTOM | Anchor::RIGHT);
        // Never take keyboard focus away from what the user is typing in.
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_size(size.0 + EDGE_MARGIN, size.1);
        layer.set_margin(0, 0, EDGE_MARGIN as i32, 0);
        // Clicks on the transparent padding go through to what's below.
        let input = Region::new(parts.compositor).context("could not create an input region")?;
        input.add(0, 0, size.0 as i32, size.1 as i32);
        let no_input = Region::new(parts.compositor).context("could not create an input region")?;
        layer.wl_surface().set_input_region(Some(input.wl_region()));
        let viewport = parts.viewporter.get_viewport(layer.wl_surface(), qh, ());
        viewport.set_destination((size.0 + EDGE_MARGIN) as i32, size.1 as i32);
        // The first commit has no buffer; the compositor answers with a
        // configure, and the first draw starts the slide-in.
        layer.commit();

        let buffer_len = (small.height() * canvas_width(size.0, output.scale) * 4) as usize;
        Ok(Thumbnail {
            id,
            output: output.geom.name.clone(),
            path,
            layer,
            viewport,
            pool: SlotPool::new(buffer_len * 2, parts.shm).context("could not create shm pool")?,
            buffer: None,
            small,
            size,
            scale: output.scale,
            offset: travel(size.0),
            anim: None,
            drag: None,
            margin: EDGE_MARGIN,
            input,
            no_input,
            hidden: false,
            configured: false,
            frame_pending: false,
            started: false,
            closing: false,
            expired: false,
        })
    }

    pub fn is(&self, surface: &wayland_client::protocol::wl_surface::WlSurface) -> bool {
        self.layer.wl_surface() == surface
    }

    pub fn is_layer(&self, layer: &LayerSurface) -> bool {
        self.layer.wl_surface() == layer.wl_surface()
    }

    pub fn is_closing(&self) -> bool {
        self.closing
    }

    /// The slide-out has finished (or is invisible anyway).
    pub fn is_finished(&self) -> bool {
        self.closing && (self.hidden || anim_over(self.anim, Instant::now()))
    }

    /// Makes the thumbnail invisible and click-through. The surface stays
    /// mapped: remapping would need a configure that niri doesn't send.
    pub fn hide(&mut self, qh: &QueueHandle<State>) {
        if self.hidden {
            return;
        }
        self.hidden = true;
        self.layer
            .wl_surface()
            .set_input_region(Some(self.no_input.wl_region()));
        self.draw(qh, true);
    }

    pub fn show(&mut self, qh: &QueueHandle<State>) {
        if !self.hidden {
            return;
        }
        self.hidden = false;
        self.layer
            .wl_surface()
            .set_input_region(Some(self.input.wl_region()));
        self.draw(qh, true);
    }

    pub fn set_margin(&mut self, margin: u32) {
        if margin == self.margin {
            return;
        }
        self.margin = margin;
        self.layer.set_margin(0, 0, margin as i32, 0);
        self.layer.commit();
    }

    pub fn configure(&mut self, qh: &QueueHandle<State>) {
        self.configured = true;
        if !self.started {
            self.started = true;
            self.anim = Some((Anim::slide_in(travel(self.size.0)), Instant::now()));
        }
        self.draw(qh, false);
    }

    pub fn frame_done(&mut self, qh: &QueueHandle<State>) {
        self.frame_pending = false;
        if self.anim.is_some() || self.drag.is_some() {
            self.draw(qh, false);
        }
    }

    /// The timeout fired: slide out, unless the user is dragging it.
    pub fn expire(&mut self, qh: &QueueHandle<State>) {
        self.expired = true;
        if self.drag.is_none() {
            self.slide_out(qh);
        }
    }

    pub fn slide_out(&mut self, qh: &QueueHandle<State>) {
        if self.closing {
            return;
        }
        self.closing = true;
        self.anim = Some((
            Anim::slide_out(self.offset, travel(self.size.0)),
            Instant::now(),
        ));
        self.draw(qh, false);
    }

    pub fn press(&mut self, x: f64, y: f64) {
        if !self.closing {
            self.anim = None;
            self.drag = Some((x, y));
        }
    }

    pub fn motion(&mut self, x: f64, qh: &QueueHandle<State>) {
        if let Some((x0, _)) = self.drag {
            self.offset = stack::drag_offset(x - x0);
            self.draw(qh, false);
        }
    }

    pub fn release(&mut self, x: f64, y: f64, qh: &QueueHandle<State>) -> Option<Action> {
        let (x0, y0) = self.drag.take()?;
        match stack::release(x - x0, y - y0, self.size.0 as f64) {
            Release::Click => {
                self.slide_out(qh);
                Some(Action::OpenEditor)
            }
            Release::Dismiss => {
                self.slide_out(qh);
                None
            }
            Release::SnapBack if self.expired => {
                self.slide_out(qh);
                None
            }
            Release::SnapBack => {
                self.anim = Some((Anim::snap_back(self.offset), Instant::now()));
                self.draw(qh, false);
                None
            }
        }
    }

    fn draw(&mut self, qh: &QueueHandle<State>, force: bool) {
        if let Some((anim, start)) = self.anim {
            let (offset, done) = anim.at(start.elapsed().as_secs_f64() * 1000.0);
            self.offset = offset;
            if done {
                self.anim = None;
            }
        }
        // Hide and show must reach the screen now, even mid-animation.
        if !self.configured || (self.frame_pending && !force) {
            return;
        }
        let width = canvas_width(self.size.0, self.scale);
        let height = self.small.height();
        let Ok((buffer, canvas)) = self.pool.create_buffer(
            width as i32,
            height as i32,
            width as i32 * 4,
            wl_shm::Format::Argb8888,
        ) else {
            tracing::warn!("could not allocate a thumbnail buffer");
            return;
        };
        if self.hidden {
            canvas.fill(0);
        } else {
            paint(
                canvas,
                width,
                &self.small,
                (self.offset * self.scale).round() as i64,
            );
        }
        let surface = self.layer.wl_surface();
        surface.damage_buffer(0, 0, width as i32, height as i32);
        // Keep drawing while something moves; motion that arrives while a
        // frame is pending is picked up by the next frame callback.
        if self.anim.is_some() || self.drag.is_some() {
            surface.frame(qh, FrameCallbackData(surface.clone()));
            self.frame_pending = true;
        }
        if buffer.attach_to(surface).is_err() {
            tracing::warn!("could not attach a thumbnail buffer");
            return;
        }
        self.layer.commit();
        self.buffer = Some(buffer);
    }
}

impl Drop for Thumbnail {
    fn drop(&mut self) {
        self.viewport.destroy();
    }
}

/// Whether `anim` has run its full course by `now`; no animation counts as
/// over. Judged by the clock, not by frames: a compositor that stops sending
/// frame callbacks (monitor asleep) must not keep a thumbnail alive.
fn anim_over(anim: Option<(Anim, Instant)>, now: Instant) -> bool {
    anim.is_none_or(|(anim, start)| {
        now.duration_since(start).as_millis() >= anim.duration_ms as u128
    })
}

/// How far right the image moves to be fully off-screen.
fn travel(image_width: u32) -> f64 {
    (image_width + EDGE_MARGIN) as f64
}

/// Physical width of the surface: the image plus the edge padding.
fn canvas_width(image_width: u32, scale: f64) -> u32 {
    (((image_width + EDGE_MARGIN) as f64 * scale).round() as u32).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn slide_out_is_over_by_time_alone() {
        // No frame callback needed: a sleeping monitor must not keep a
        // closing thumbnail (and the host) around forever.
        let start = Instant::now();
        let anim = Some((Anim::slide_out(0.0, 236.0), start));
        assert!(!anim_over(anim, start));
        assert!(!anim_over(anim, start + Duration::from_millis(149)));
        assert!(anim_over(anim, start + Duration::from_millis(150)));
        assert!(anim_over(None, start));
    }

    #[test]
    fn loads_png_without_extension() {
        // save.filename and -o don't have to end in .png; the bytes always are PNG.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot");
        let img = RgbaImage::from_pixel(3, 2, image::Rgba([1, 2, 3, 255]));
        std::fs::write(&path, crate::output::encode_png(&img).unwrap()).unwrap();
        assert_eq!(load(&path).unwrap(), img);
    }

    #[test]
    fn downscale_fits_long_edge_at_scale_one() {
        let img = RgbaImage::new(1920, 1080);
        let (small, logical) = downscale(&img, 1.0);
        assert_eq!(logical, (220, 124));
        assert_eq!(small.dimensions(), (220, 124));
    }

    #[test]
    fn downscale_renders_physical_pixels_at_fractional_scale() {
        let img = RgbaImage::new(1920, 1080);
        let (small, logical) = downscale(&img, 1.25);
        assert_eq!(logical, (220, 124));
        assert_eq!(small.dimensions(), (275, 155));
    }

    #[test]
    fn downscale_leaves_small_images_alone() {
        let img = RgbaImage::from_pixel(100, 50, image::Rgba([1, 2, 3, 255]));
        let (small, logical) = downscale(&img, 1.0);
        assert_eq!(logical, (100, 50));
        assert_eq!(small, img);
    }

    fn pixel(canvas: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * width + x) * 4) as usize;
        canvas[i..i + 4].try_into().unwrap()
    }

    /// 4x3 red image, so the inner pixels are (1,1) and (2,1).
    fn red() -> RgbaImage {
        RgbaImage::from_pixel(4, 3, image::Rgba([255, 0, 0, 255]))
    }

    #[test]
    fn paint_at_rest_has_border_image_and_transparent_padding() {
        let mut canvas = vec![9u8; 6 * 3 * 4];
        paint(&mut canvas, 6, &red(), 0);
        assert_eq!(pixel(&canvas, 6, 0, 0), BORDER);
        assert_eq!(pixel(&canvas, 6, 1, 1), [0, 0, 255, 255], "BGRA red");
        assert_eq!(pixel(&canvas, 6, 3, 1), BORDER);
        assert_eq!(pixel(&canvas, 6, 4, 1), [0; 4], "padding is transparent");
        assert_eq!(pixel(&canvas, 6, 5, 2), [0; 4]);
    }

    #[test]
    fn paint_with_offset_shifts_and_clips() {
        let mut canvas = vec![0u8; 6 * 3 * 4];
        paint(&mut canvas, 6, &red(), 3);
        assert_eq!(pixel(&canvas, 6, 2, 1), [0; 4], "left of the image");
        assert_eq!(pixel(&canvas, 6, 3, 1), BORDER);
        assert_eq!(pixel(&canvas, 6, 4, 1), [0, 0, 255, 255]);
        assert_eq!(
            pixel(&canvas, 6, 5, 1),
            [0, 0, 255, 255],
            "clipped at the edge"
        );
    }

    #[test]
    fn paint_fully_off_screen_is_empty() {
        let mut canvas = vec![7u8; 6 * 3 * 4];
        paint(&mut canvas, 6, &red(), 6);
        assert!(canvas.iter().all(|&b| b == 0));
    }

    #[test]
    fn canvas_includes_padding() {
        assert_eq!(canvas_width(220, 1.0), 236);
        assert_eq!(canvas_width(220, 1.25), 295);
        assert_eq!(travel(220), 236.0);
    }
}
