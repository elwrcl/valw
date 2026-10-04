//! Zoom mode: the frozen focused output, zoomed and panned on the GPU.

pub mod gl;
pub mod view;

use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use smithay_client_toolkit::{
    compositor::FrameCallbackData,
    seat::{
        keyboard::{Keysym, Modifiers},
        pointer::{BTN_LEFT, PointerEvent, PointerEventKind},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerSurface},
    },
};
use wayland_client::{QueueHandle, protocol::wl_surface::WlSurface};
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::{
    Shape, WpCursorShapeDeviceV1,
};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;

use self::view::{Animated, View};
use crate::config;
use crate::error::HintExt;
use crate::frame::{Frame, PixelRect};
use crate::wayland::{Output, State, Wayland};

pub const NAMESPACE: &str = "valw-zoom";

/// How a zoom session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// `c`: this part of the frame.
    Capture(PixelRect),
    /// Esc or `q`: nothing to save.
    Leave,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Leave,
    Capture,
    Flashlight,
    Reset,
}

pub fn command(key: Keysym) -> Option<Command> {
    match key {
        Keysym::Escape | Keysym::q => Some(Command::Leave),
        Keysym::c => Some(Command::Capture),
        Keysym::f => Some(Command::Flashlight),
        Keysym::_0 => Some(Command::Reset),
        _ => None,
    }
}

pub struct Zoom {
    // Dropped first: the EGL surface goes before the wl_surface under it.
    renderer: gl::Renderer,
    layer: LayerSurface,
    viewport: WpViewport,
    /// Frame size in physical px.
    size: (u32, u32),
    /// Buffer px per surface-local (logical) px.
    ratio: f64,
    view: Animated,
    step: f64,
    /// Pointer position in buffer px.
    pointer: (f64, f64),
    /// Last pointer position while the left button pans.
    panning: Option<(f64, f64)>,
    /// The latest pointer enter's serial: cursor shapes are set with it.
    enter_serial: Option<u32>,
    flashlight: bool,
    /// Flashlight radius in logical px.
    radius: f64,
    ctrl: bool,
    configured: bool,
    frame_pending: bool,
    dirty: bool,
    last_draw: Option<Instant>,
    outcome: Option<Outcome>,
    error: Option<anyhow::Error>,
}

/// Shows `frame` (the frozen `output`) until the user leaves or captures.
pub fn run(
    wl: &mut Wayland,
    output: &Output,
    frame: &Frame,
    config: &config::Zoom,
) -> Result<Outcome> {
    let qh = wl.queue.handle();
    let s = &wl.state;
    let layer_shell = s
        .layer_shell
        .as_ref()
        .context("compositor does not support wlr-layer-shell")
        .hint("run `valw doctor` to see what the compositor supports")?;
    let viewporter = s
        .viewporter
        .as_ref()
        .context("compositor does not support wp-viewporter")
        .hint("run `valw doctor` to see what the compositor supports")?;
    let surface = s.compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(
        &qh,
        surface,
        Layer::Overlay,
        Some(NAMESPACE),
        Some(&output.wl),
    );
    layer.set_anchor(Anchor::all());
    layer.set_exclusive_zone(-1);
    layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
    layer.commit();
    let viewport = viewporter.get_viewport(layer.wl_surface(), &qh, ());
    let pixels: &Arc<_> = &frame.pixels;
    let size = pixels.dimensions();
    let renderer = gl::Renderer::new(&wl.conn, layer.wl_surface(), size, pixels)
        .context("could not start OpenGL ES for zoom")
        .hint("run `valw doctor` to check EGL")?;

    wl.state.zoom = Some(Zoom {
        renderer,
        layer,
        viewport,
        size,
        ratio: 1.0,
        view: Animated::new(),
        step: config.scroll_step,
        pointer: (0.0, 0.0),
        panning: None,
        enter_serial: None,
        flashlight: false,
        radius: config.flashlight_radius,
        ctrl: false,
        configured: false,
        frame_pending: false,
        dirty: true,
        last_draw: None,
        outcome: None,
        error: None,
    });
    let result = wl.dispatch_blocking(|s| {
        s.zoom
            .as_ref()
            .is_some_and(|z| z.outcome.is_some() || z.error.is_some())
    });
    let ended = wl.state.zoom.take().map(|z| (z.outcome, z.error));
    // Dropping the zoom destroys its surface; make that visible right away.
    let _ = wl.conn.flush();
    result?;
    match ended {
        Some((_, Some(e))) => Err(e),
        Some((Some(outcome), None)) => Ok(outcome),
        _ => Ok(Outcome::Leave),
    }
}

impl Zoom {
    fn is(&self, surface: &WlSurface) -> bool {
        self.layer.wl_surface() == surface
    }

    fn sizef(&self) -> (f64, f64) {
        (self.size.0 as f64, self.size.1 as f64)
    }

    pub fn configure(&mut self, layer: &LayerSurface, (w, h): (u32, u32), qh: &QueueHandle<State>) {
        if !self.is(layer.wl_surface()) {
            return;
        }
        if w > 0 {
            self.ratio = self.size.0 as f64 / w as f64;
        }
        // Show the physical-size buffer 1:1 on the logical-size surface.
        self.viewport.set_destination(w as i32, h as i32);
        self.configured = true;
        self.request_draw(qh);
    }

    pub fn closed(&mut self, layer: &LayerSurface) {
        if self.is(layer.wl_surface()) {
            self.outcome.get_or_insert(Outcome::Leave);
        }
    }

    pub fn frame_done(&mut self, surface: &WlSurface, qh: &QueueHandle<State>) {
        if !self.is(surface) {
            return;
        }
        self.frame_pending = false;
        if self.dirty {
            self.draw(qh);
        }
    }

    fn request_draw(&mut self, qh: &QueueHandle<State>) {
        self.dirty = true;
        if self.configured && !self.frame_pending {
            self.draw(qh);
        }
    }

    fn draw(&mut self, qh: &QueueHandle<State>) {
        let now = Instant::now();
        let dt = self
            .last_draw
            .map_or(0.0, |t| now.duration_since(t).as_secs_f64().min(0.1));
        let moving = self.view.step(dt);
        self.last_draw = moving.then_some(now);
        let surface = self.layer.wl_surface();
        surface.frame(qh, FrameCallbackData(surface.clone()));
        let flashlight =
            self.flashlight
                .then_some((self.pointer.0, self.pointer.1, self.radius * self.ratio));
        if let Err(e) = self.renderer.draw(&self.view.shown, flashlight) {
            self.error = Some(e);
            return;
        }
        self.frame_pending = true;
        self.dirty = moving;
    }

    pub fn pointer(
        &mut self,
        events: &[PointerEvent],
        cursor: Option<&WpCursorShapeDeviceV1>,
        qh: &QueueHandle<State>,
    ) {
        for event in events {
            if !self.is(&event.surface) {
                continue;
            }
            let p = (event.position.0 * self.ratio, event.position.1 * self.ratio);
            if let (Some(device), Some((serial, shape))) = (
                cursor,
                cursor_change(self.enter_serial, &event.kind, self.panning.is_some()),
            ) {
                device.set_shape(serial, shape);
            }
            match event.kind {
                PointerEventKind::Enter { serial } => {
                    self.enter_serial = Some(serial);
                    self.pointer = p;
                }
                PointerEventKind::Press {
                    button: BTN_LEFT, ..
                } => {
                    self.panning = Some(p);
                }
                PointerEventKind::Release {
                    button: BTN_LEFT, ..
                } => {
                    self.panning = None;
                }
                PointerEventKind::Motion { .. } => {
                    self.pointer = p;
                    if let Some(last) = self.panning {
                        let size = self.sizef();
                        let moved = self.view.target.pan((p.0 - last.0, p.1 - last.1), size);
                        self.view.set(moved);
                        self.panning = Some(p);
                        self.request_draw(qh);
                    } else if self.flashlight {
                        self.request_draw(qh);
                    }
                }
                PointerEventKind::Axis { vertical, .. } => {
                    let steps = view::scroll_steps(vertical.value120, vertical.absolute);
                    if steps == 0.0 {
                        continue;
                    }
                    if self.ctrl {
                        self.radius = view::radius(self.radius, steps, self.step);
                    } else {
                        let size = self.sizef();
                        self.view.target = self.view.target.zoom_at(p, self.step.powf(steps), size);
                    }
                    self.request_draw(qh);
                }
                _ => {}
            }
        }
    }

    pub fn key(&mut self, key: Keysym, qh: &QueueHandle<State>) {
        match command(key) {
            Some(Command::Leave) => {
                self.outcome.get_or_insert(Outcome::Leave);
            }
            Some(Command::Capture) => {
                let rect = self.view.target.visible_rect(self.size);
                tracing::info!("zoom capture {rect:?} at {:.2}x", self.view.target.scale);
                self.outcome.get_or_insert(Outcome::Capture(rect));
            }
            Some(Command::Flashlight) => {
                self.flashlight = !self.flashlight;
                self.request_draw(qh);
            }
            Some(Command::Reset) => {
                self.view.target = View::IDENTITY;
                self.request_draw(qh);
            }
            None => {}
        }
    }

    pub fn modifiers(&mut self, modifiers: Modifiers) {
        self.ctrl = modifiers.ctrl;
    }
}

/// The cursor shape a pointer event calls for, with the serial to set it
/// with. The protocol wants the latest enter's serial: niri ignores a
/// release's, and the hand would stay after a drag.
fn cursor_change(
    enter: Option<u32>,
    kind: &PointerEventKind,
    panning: bool,
) -> Option<(u32, Shape)> {
    match kind {
        PointerEventKind::Enter { serial } => Some((*serial, cursor(panning))),
        PointerEventKind::Press {
            button: BTN_LEFT, ..
        } => Some((enter?, cursor(true))),
        PointerEventKind::Release {
            button: BTN_LEFT, ..
        } => Some((enter?, cursor(false))),
        _ => None,
    }
}

/// The pointer over a zoom: a magnifier, so it reads as zoom mode, and a
/// grabbing hand while dragging the view.
fn cursor(panning: bool) -> Shape {
    if panning {
        Shape::Grabbing
    } else {
        Shape::ZoomIn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_changes_use_the_enter_serial() {
        let enter = PointerEventKind::Enter { serial: 5 };
        let press = PointerEventKind::Press {
            time: 0,
            button: BTN_LEFT,
            serial: 9,
        };
        let release = PointerEventKind::Release {
            time: 0,
            button: BTN_LEFT,
            serial: 12,
        };
        assert_eq!(cursor_change(None, &enter, false), Some((5, Shape::ZoomIn)));
        assert_eq!(
            cursor_change(Some(5), &press, true),
            Some((5, Shape::Grabbing))
        );
        assert_eq!(
            cursor_change(Some(5), &release, false),
            Some((5, Shape::ZoomIn)),
            "a release's serial is ignored by the compositor"
        );
        assert_eq!(cursor_change(None, &press, true), None, "no enter yet");
    }

    #[test]
    fn the_cursor_says_zoom_and_grabs_while_dragging() {
        assert_eq!(cursor(false), Shape::ZoomIn);
        assert_eq!(cursor(true), Shape::Grabbing);
    }

    #[test]
    fn keys() {
        assert_eq!(command(Keysym::Escape), Some(Command::Leave));
        assert_eq!(command(Keysym::q), Some(Command::Leave));
        assert_eq!(command(Keysym::c), Some(Command::Capture));
        assert_eq!(command(Keysym::f), Some(Command::Flashlight));
        assert_eq!(command(Keysym::_0), Some(Command::Reset));
        assert_eq!(command(Keysym::a), None);
    }
}
