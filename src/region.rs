use std::sync::Arc;

use anyhow::{Context, Result};
use smithay_client_toolkit::{
    compositor::FrameCallbackData,
    seat::{
        keyboard::Modifiers,
        pointer::{BTN_LEFT, BTN_RIGHT, PointerEvent, PointerEventKind},
    },
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
use crate::selection::{self, Mods, Point, Selection};
use crate::wayland::{Output, State, Wayland};

pub const NAMESPACE: &str = "valw-overlay";

/// What the user chose in the overlay.
#[derive(Debug, PartialEq)]
pub enum Choice {
    /// A rectangle in the frame of output `.0`.
    Region(usize, PixelRect),
    /// A rectangle over several outputs (or past one's edge), with the
    /// output the pointer was over at release (for the preview).
    Span(selection::Span, usize),
    /// Space before dragging: capture a window instead.
    Window,
}

/// One full-screen layer surface per output, showing the frozen frame.
pub struct Overlay {
    surfaces: Vec<Surface>,
    selection: Selection,
    /// Shift, Alt and Space as the keyboard last reported them.
    mods: Mods,
    /// The pointer in global logical coordinates. A drag's events keep
    /// arriving on the surface it started on, with positions past its
    /// edges, so this stays right across outputs.
    pointer: Option<Point>,
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
    /// The selection part and whether the label were last drawn here.
    shown: Option<PixelRect>,
    labelled: bool,
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

    // The theme's colour for the overlay outside the selection.
    let palette = crate::theme::current();
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
        surfaces.push(Surface::new(layer, viewport, frame, &palette, &s.shm)?);
    }

    tracing::debug!(
        "overlay buffers ready {:?} after start",
        crate::log::since_start()
    );
    wl.state.overlay = Some(Overlay {
        surfaces,
        selection: Selection::default(),
        mods: Mods::default(),
        pointer: None,
        outcome: None,
    });
    let result = wl.dispatch_blocking(|s| s.overlay.as_ref().is_some_and(|o| o.outcome.is_some()));
    let outcome = wl.state.overlay.take().and_then(|o| o.outcome);
    // Dropping the overlay destroys the surfaces; make that visible right away.
    let _ = wl.conn.flush();
    result?;
    outcome.flatten().ok_or_else(|| Cancelled.into())
}

/// What a finished drag from `a` to `b` captures: one output's crop as
/// before, or the whole rectangle to stitch; `None` for a click.
fn choose(
    a: Point,
    b: Point,
    outputs: &[(&OutputGeom, (u32, u32))],
    pointer: Point,
) -> Option<Choice> {
    let span = selection::span(a, b, outputs)?;
    Some(match span.single() {
        Some((i, rect)) => Choice::Region(i, rect),
        None => Choice::Span(span, selection::locate(pointer, outputs)),
    })
}

impl Surface {
    fn new(
        layer: LayerSurface,
        viewport: WpViewport,
        frame: &Frame,
        palette: &crate::theme::Palette,
        shm: &Shm,
    ) -> Result<Surface> {
        let (width, height) = frame.pixels.dimensions();
        let len = (width * height * 4) as usize;
        Ok(Surface {
            layer,
            viewport,
            geom: frame.output.clone(),
            width,
            height,
            dark: render::tint(frame.pixels.as_raw(), palette.base, render::TINT),
            bright: Arc::clone(&frame.pixels),
            // Room for two buffers so one can be on screen while we draw the next.
            pool: SlotPool::new(len * 2, shm).context("could not create shm pool")?,
            buffer: None,
            configured: false,
            frame_pending: false,
            dirty: true,
            shown: None,
            labelled: false,
        })
    }

    fn draw(
        &mut self,
        sel: Option<PixelRect>,
        label: Option<(PixelRect, (f64, f64))>,
        qh: &QueueHandle<State>,
    ) {
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
        if let Some((rect, (px, py))) = label {
            let scale = self.width as f64 / self.geom.width as f64;
            let text = render::size_text(rect);
            let size = (
                ((crate::toolbar::draw::measure(&text) + 12.0) as f64 * scale).ceil() as u32,
                ((crate::toolbar::draw::LABEL * 1.2 + 12.0) as f64 * scale).ceil() as u32,
            );
            // Square corners: sharp, like the rest of the shell.
            let pill = crate::toolbar::draw::pill(size, scale as f32, &text, 0.0);
            let at = render::label_origin(
                (px * scale, py * scale),
                (pill.width(), pill.height()),
                (self.width, self.height),
                16.0 * scale,
            );
            render::blend(canvas, self.width, &pill, at);
        }

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

    pub fn modifiers(&mut self, m: Modifiers, qh: &QueueHandle<State>) {
        self.mods.shift = m.shift;
        self.mods.alt = m.alt;
        self.apply_mods(qh);
    }

    /// Space before a drag switches to window mode; during one it moves
    /// the selection while held.
    pub fn space(&mut self, down: bool, qh: &QueueHandle<State>) {
        if down && self.selection.allows_window_switch() {
            self.switch_to_window();
            return;
        }
        self.mods.space = down;
        self.apply_mods(qh);
    }

    fn apply_mods(&mut self, qh: &QueueHandle<State>) {
        self.selection.set_mods(self.mods);
        self.redraw_changed(qh);
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

    fn outputs(&self) -> Vec<(&OutputGeom, (u32, u32))> {
        self.surfaces
            .iter()
            .map(|s| (&s.geom, (s.width, s.height)))
            .collect()
    }

    /// The selection's part on surface `i`.
    fn selection_on(&self, i: usize) -> Option<PixelRect> {
        let (start, end) = self.selection.corners()?;
        let s = &self.surfaces[i];
        selection::clip(start, end, &s.geom, s.width, s.height)
    }

    /// The size label on surface `i`: the whole shot's size next to the
    /// pointer, on the output the pointer is over.
    fn label_on(&self, i: usize) -> Option<(PixelRect, (f64, f64))> {
        let (start, end) = self.selection.corners()?;
        let p = self.pointer?;
        let outputs = self.outputs();
        if selection::locate(p, &outputs) != i {
            return None;
        }
        let (width, height) = selection::span(start, end, &outputs)?.size;
        let g = &self.surfaces[i].geom;
        Some((
            PixelRect {
                x: 0,
                y: 0,
                width,
                height,
            },
            (p.x - g.x as f64, p.y - g.y as f64),
        ))
    }

    fn redraw(&mut self, i: usize, qh: &QueueHandle<State>) {
        let sel = self.selection_on(i);
        let label = self.label_on(i);
        let s = &mut self.surfaces[i];
        s.shown = sel;
        s.labelled = label.is_some();
        s.dirty = true;
        if s.configured && !s.frame_pending {
            s.draw(sel, label, qh);
        }
    }

    /// Redraws the surfaces whose part of the selection or label changed.
    fn redraw_changed(&mut self, qh: &QueueHandle<State>) {
        for i in 0..self.surfaces.len() {
            let s = &self.surfaces[i];
            if self.selection_on(i) != s.shown || s.labelled || self.label_on(i).is_some() {
                self.redraw(i, qh);
            }
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
            self.pointer = Some(p);
            match event.kind {
                PointerEventKind::Enter { serial } => {
                    if let Some(cursor) = cursor {
                        cursor.set_shape(serial, Shape::Crosshair);
                    }
                }
                PointerEventKind::Press {
                    button: BTN_LEFT, ..
                } => {
                    // Modifiers already held apply from the start.
                    self.selection.press_with(p, self.mods);
                    self.redraw_changed(qh);
                }
                PointerEventKind::Press {
                    button: BTN_RIGHT, ..
                } => self.cancel(),
                PointerEventKind::Motion { .. } => {
                    self.selection.motion(p);
                    self.redraw_changed(qh);
                }
                PointerEventKind::Release {
                    button: BTN_LEFT, ..
                } => {
                    let Some((start, end)) = self.selection.release(p) else {
                        continue;
                    };
                    let choice = choose(start, end, &self.outputs(), p);
                    match choice {
                        Some(choice) => {
                            tracing::info!("selected {choice:?} (logical {start:?} to {end:?})");
                            self.outcome = Some(Some(choice));
                        }
                        // A click, not a drag: clear it and keep waiting.
                        None => self.redraw_changed(qh),
                    }
                }
                _ => {}
            }
        }
    }
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

    #[test]
    fn one_output_is_todays_region_and_several_a_span() {
        let laptop = out("LVDS-1", 0, 0, 1366, 768);
        let monitor = out("HDMI-A-1", -277, -1080, 1920, 1080);
        let outputs = [(&laptop, (1366, 768)), (&monitor, (1920, 1080))];
        assert_eq!(
            choose(p(100.0, 100.0), p(300.0, 250.0), &outputs, p(300.0, 250.0)),
            Some(Choice::Region(
                0,
                PixelRect {
                    x: 100,
                    y: 100,
                    width: 200,
                    height: 150
                }
            ))
        );
        let across = choose(
            p(100.0, 500.0),
            p(600.0, -300.0),
            &outputs,
            p(600.0, -300.0),
        );
        let Some(Choice::Span(span, on)) = across else {
            panic!("expected a span, got {across:?}");
        };
        assert_eq!(
            (span.size, span.parts.len(), on),
            ((500, 800), 2, 1),
            "preview on the monitor"
        );
        // A box touching only the laptop but reaching past its left edge.
        assert!(matches!(
            choose(p(-50.0, 100.0), p(200.0, 300.0), &outputs, p(200.0, 300.0)),
            Some(Choice::Span(_, 0))
        ));
        assert_eq!(
            choose(p(10.0, 10.0), p(11.0, 30.0), &outputs, p(11.0, 30.0)),
            None,
            "a click"
        );
    }
}
