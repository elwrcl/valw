use std::sync::Arc;

use anyhow::{Context, Result};
use smithay_client_toolkit::{
    compositor::FrameCallbackData,
    seat::pointer::{BTN_LEFT, BTN_RIGHT, PointerEvent, PointerEventKind},
    shell::{
        WaylandSurface,
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerSurface},
    },
    shm::{
        Shm,
        slot::{Buffer, SlotPool},
    },
};
use wayland_client::{
    QueueHandle,
    protocol::{wl_shm, wl_surface::WlSurface},
};
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::{
    Shape, WpCursorShapeDeviceV1,
};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;

use crate::error::{Cancelled, HintExt};
use crate::frame::{Bgrx, Frame, OutputGeom, PixelRect};
use crate::render;
use crate::selection::{self, Point, Selection};
use crate::wayland::{Output, State, Wayland};

pub const NAMESPACE: &str = "valw-overlay";

/// What the user chose in the overlay.
#[derive(Debug, PartialEq)]
pub enum Choice {
    /// A rectangle in the frame of output `.0`.
    Region(usize, PixelRect),
    /// Space before dragging: capture a window instead.
    Window,
}

/// One full-screen layer surface per output, showing the frozen frame.
pub struct Overlay {
    surfaces: Vec<Surface>,
    selection: Selection,
    /// Output where the current drag started; the capture is clipped to it.
    anchor: Option<usize>,
    outcome: Option<Option<Choice>>,
}

struct Surface {
    layer: LayerSurface,
    viewport: WpViewport,
    geom: OutputGeom,
    width: u32,
    height: u32,
    dark: Vec<u8>,
    bright: Arc<Bgrx>,
    pool: SlotPool,
    buffer: Option<Buffer>,
    configured: bool,
    frame_pending: bool,
    dirty: bool,
}

/// Shows the overlay on every output and waits for a selection.
/// Returns `Cancelled` if the user presses Esc or right-clicks.
/// Space before dragging returns `Choice::Window`.
pub fn select(wl: &mut Wayland, outputs: &[Output], frames: &[Frame]) -> Result<Choice> {
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

    let mut surfaces = Vec::new();
    for (output, frame) in outputs.iter().zip(frames) {
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
        surfaces.push(Surface::new(layer, viewport, frame, &s.shm)?);
    }

    tracing::debug!(
        "overlay buffers ready {:?} after start",
        crate::log::since_start()
    );
    wl.state.overlay = Some(Overlay {
        surfaces,
        selection: Selection::default(),
        anchor: None,
        outcome: None,
    });
    let result = wl.dispatch_blocking(|s| s.overlay.as_ref().is_some_and(|o| o.outcome.is_some()));
    let outcome = wl.state.overlay.take().and_then(|o| o.outcome);
    // Dropping the overlay destroys the surfaces; make that visible right away.
    let _ = wl.conn.flush();
    result?;
    outcome.flatten().ok_or_else(|| Cancelled.into())
}

impl Surface {
    fn new(layer: LayerSurface, viewport: WpViewport, frame: &Frame, shm: &Shm) -> Result<Surface> {
        let (width, height) = frame.pixels.dimensions();
        let len = (width * height * 4) as usize;
        Ok(Surface {
            layer,
            viewport,
            geom: frame.output.clone(),
            width,
            height,
            dark: render::dim(frame.pixels.as_raw(), render::DIM),
            bright: Arc::clone(&frame.pixels),
            // Room for two buffers so one can be on screen while we draw the next.
            pool: SlotPool::new(len * 2, shm).context("could not create shm pool")?,
            buffer: None,
            configured: false,
            frame_pending: false,
            dirty: true,
        })
    }

    fn draw(&mut self, sel: Option<PixelRect>, qh: &QueueHandle<State>) {
        let stride = self.width as i32 * 4;
        let Ok((buffer, canvas)) = self.pool.create_buffer(
            self.width as i32,
            self.height as i32,
            stride,
            wl_shm::Format::Xrgb8888,
        ) else {
            tracing::warn!("could not allocate an overlay buffer");
            return;
        };
        render::compose(canvas, &self.dark, self.bright.as_raw(), self.width, sel);

        let surface = self.layer.wl_surface();
        surface.damage_buffer(0, 0, self.width as i32, self.height as i32);
        surface.frame(qh, FrameCallbackData(surface.clone()));
        if buffer.attach_to(surface).is_err() {
            tracing::warn!("could not attach the overlay buffer");
            return;
        }
        self.layer.commit();
        self.buffer = Some(buffer);
        self.frame_pending = true;
        self.dirty = false;
    }
}

impl Overlay {
    pub fn cancel(&mut self) {
        self.outcome.get_or_insert(None);
    }

    pub fn switch_to_window(&mut self) {
        if self.selection.allows_window_switch() {
            tracing::info!("switching to window mode");
            self.outcome.get_or_insert(Some(Choice::Window));
        }
    }

    fn index_of(&self, surface: &WlSurface) -> Option<usize> {
        self.surfaces
            .iter()
            .position(|s| s.layer.wl_surface() == surface)
    }

    /// The selection as drawn on surface `i`: only the anchor output shows it.
    fn selection_on(&self, i: usize) -> Option<PixelRect> {
        let Selection::Dragging { start, end } = self.selection else {
            return None;
        };
        let s = &self.surfaces[i];
        (self.anchor == Some(i))
            .then(|| selection::clip(start, end, &s.geom, s.width, s.height))
            .flatten()
    }

    fn redraw(&mut self, i: usize, qh: &QueueHandle<State>) {
        let sel = self.selection_on(i);
        let s = &mut self.surfaces[i];
        s.dirty = true;
        if s.configured && !s.frame_pending {
            s.draw(sel, qh);
        }
    }

    pub fn configure(&mut self, layer: &LayerSurface, (w, h): (u32, u32), qh: &QueueHandle<State>) {
        let Some(i) = self.index_of(layer.wl_surface()) else {
            return;
        };
        let s = &mut self.surfaces[i];
        // Show the physical-size buffer 1:1 on the logical-size surface.
        s.viewport.set_destination(w as i32, h as i32);
        if !s.configured {
            s.configured = true;
            let name = s.geom.name.clone();
            self.redraw(i, qh);
            tracing::info!(
                "overlay on {name} committed {:?} after start",
                crate::log::since_start()
            );
        }
    }

    pub fn frame_done(&mut self, surface: &WlSurface, qh: &QueueHandle<State>) {
        let Some(i) = self.index_of(surface) else {
            return;
        };
        self.surfaces[i].frame_pending = false;
        if self.surfaces[i].dirty {
            self.redraw(i, qh);
        }
    }

    pub fn pointer(
        &mut self,
        events: &[PointerEvent],
        cursor: Option<&WpCursorShapeDeviceV1>,
        qh: &QueueHandle<State>,
    ) {
        for event in events {
            let Some(i) = self.index_of(&event.surface) else {
                continue;
            };
            let g = &self.surfaces[i].geom;
            let p = Point {
                x: g.x as f64 + event.position.0,
                y: g.y as f64 + event.position.1,
            };
            match event.kind {
                PointerEventKind::Enter { serial } => {
                    if let Some(cursor) = cursor {
                        cursor.set_shape(serial, Shape::Crosshair);
                    }
                }
                PointerEventKind::Press {
                    button: BTN_LEFT, ..
                } => {
                    self.selection.press(p);
                    self.anchor = Some(i);
                }
                PointerEventKind::Press {
                    button: BTN_RIGHT, ..
                } => self.cancel(),
                PointerEventKind::Motion { .. } => {
                    self.selection.motion(p);
                    if let Some(a) = self.anchor {
                        self.redraw(a, qh);
                    }
                }
                PointerEventKind::Release {
                    button: BTN_LEFT, ..
                } => {
                    let (Some((start, end)), Some(a)) = (self.selection.release(p), self.anchor)
                    else {
                        continue;
                    };
                    let s = &self.surfaces[a];
                    match selection::resolve(start, end, &s.geom, s.width, s.height) {
                        Some(rect) => {
                            tracing::info!(
                                "selected {rect:?} on {} (logical {start:?} to {end:?})",
                                s.geom.name
                            );
                            self.outcome = Some(Some(Choice::Region(a, rect)));
                        }
                        // A click, not a drag: clear it and keep waiting.
                        None => self.redraw(a, qh),
                    }
                }
                _ => {}
            }
        }
    }
}
