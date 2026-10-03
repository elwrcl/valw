//! `valw backdrop`: the paint shader, slow and calm, behind niri's
//! overview (niri's `place-within-backdrop` layer rule), coloured from each
//! output's wallpaper. It draws only when niri asks for frames, so it costs
//! nothing while the overview is closed.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::Result;
use smithay_client_toolkit::{
    compositor::{FrameCallbackData, Region},
    shell::{
        WaylandSurface,
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerSurface},
    },
};
use wayland_client::{
    Connection, QueueHandle,
    protocol::{wl_output::WlOutput, wl_surface::WlSurface},
};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;

use crate::lock::Lock;
use crate::theme::Palette;
use crate::theme::gl::{Frame, Paint};
use crate::wayland::{State, Wayland};

pub const NAMESPACE: &str = "valw-backdrop";
const SPEED: f32 = 0.35;
/// A pause this long means the overview was closed: re-read the wallpaper.
const PAUSE: Duration = Duration::from_secs(2);
const EASE: Duration = Duration::from_secs(1);

/// Whether to re-read the wallpaper's colours before this frame.
pub fn should_refresh(last_frame: Option<Instant>, now: Instant) -> bool {
    last_frame.is_none_or(|t| now.saturating_duration_since(t) > PAUSE)
}

/// `from` easing into `to`, `since` after the change started.
pub fn ease(from: &Palette, to: &Palette, since: Duration) -> Palette {
    from.lerp(to, since.as_secs_f32() / EASE.as_secs_f32())
}

/// The wallpaper's palette on `connector`, if Noctalia tells us one.
fn wallpaper_palette(connector: &str) -> Option<Palette> {
    let path = crate::theme::wallpaper(connector)?;
    let img = image::ImageReader::open(&path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?
        .thumbnail(256, 256)
        .to_rgba8();
    Some(crate::theme::from_image(&img, false))
}

struct Surface {
    // Dropped first: the EGL surface goes before the wl_surface under it.
    paint: Option<Paint>,
    layer: LayerSurface,
    viewport: WpViewport,
    output: WlOutput,
    name: String,
    scale: f64,
    start: Instant,
    last_frame: Option<Instant>,
    from: Palette,
    to: Palette,
    changed: Instant,
    /// A wallpaper palette being worked out on a thread.
    incoming: Option<mpsc::Receiver<Option<Palette>>>,
}

#[derive(Default)]
pub struct Backdrop {
    surfaces: Vec<Surface>,
}

/// Runs until the compositor goes away. A second instance exits at once.
pub fn run() -> Result<()> {
    let path = crate::lock::default_path().with_file_name("valw-backdrop.lock");
    let Ok(_lock) = Lock::acquire(&path) else {
        tracing::info!("another backdrop is running");
        return Ok(());
    };
    let mut wl = Wayland::connect()?;
    wl.state.backdrop = Some(Backdrop::default());
    let qh = wl.queue.handle();
    for output in wl.outputs() {
        add(&mut wl.state, &qh, &output.wl);
    }
    wl.dispatch_blocking(|_| false)
}

/// Puts a backdrop surface on `output`.
pub fn add(state: &mut State, qh: &QueueHandle<State>, output: &WlOutput) {
    let Some(info) = state.output_list().into_iter().find(|o| &o.wl == output) else {
        return;
    };
    let (Some(layer_shell), Some(viewporter)) = (&state.layer_shell, &state.viewporter) else {
        tracing::warn!("no layer-shell or viewporter: no backdrop");
        return;
    };
    let surface = state.compositor.create_surface(qh);
    let layer = layer_shell.create_layer_surface(
        qh,
        surface,
        Layer::Background,
        Some(NAMESPACE),
        Some(output),
    );
    layer.set_anchor(Anchor::all());
    layer.set_exclusive_zone(-1);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    // Clicks go to niri, not to the backdrop.
    if let Ok(empty) = Region::new(&state.compositor) {
        layer.wl_surface().set_input_region(Some(empty.wl_region()));
    }
    layer.commit();
    let viewport = viewporter.get_viewport(layer.wl_surface(), qh, ());
    let now = Instant::now();
    tracing::info!("backdrop on {}", info.geom.name);
    let palette = crate::theme::current();
    if let Some(b) = &mut state.backdrop {
        b.surfaces.push(Surface {
            paint: None,
            layer,
            viewport,
            output: output.clone(),
            name: info.geom.name,
            scale: info.scale,
            start: now,
            last_frame: None,
            from: palette,
            to: palette,
            changed: now,
            incoming: None,
        });
    }
}

impl Backdrop {
    fn index(&self, surface: &WlSurface) -> Option<usize> {
        self.surfaces
            .iter()
            .position(|s| s.layer.wl_surface() == surface)
    }

    pub fn remove(&mut self, output: &WlOutput) {
        self.surfaces.retain(|s| &s.output != output);
    }

    pub fn closed(&mut self, layer: &LayerSurface) {
        self.surfaces
            .retain(|s| s.layer.wl_surface() != layer.wl_surface());
    }

    pub fn configure(
        &mut self,
        conn: &Connection,
        layer: &LayerSurface,
        (w, h): (u32, u32),
        qh: &QueueHandle<State>,
    ) {
        let Some(i) = self.index(layer.wl_surface()) else {
            return;
        };
        let s = &mut self.surfaces[i];
        if w == 0 || h == 0 {
            return;
        }
        let px = (
            (w as f64 * s.scale).round() as u32,
            (h as f64 * s.scale).round() as u32,
        );
        s.viewport.set_destination(w as i32, h as i32);
        match &mut s.paint {
            Some(paint) => paint.resize(px),
            None => match Paint::new(conn, s.layer.wl_surface(), px) {
                Ok(paint) => s.paint = Some(paint),
                Err(e) => {
                    tracing::warn!("no backdrop on {}: {e:#}", s.name);
                    return;
                }
            },
        }
        s.draw(qh);
    }

    pub fn frame_done(&mut self, surface: &WlSurface, qh: &QueueHandle<State>) {
        if let Some(i) = self.index(surface) {
            self.surfaces[i].draw(qh);
        }
    }
}

impl Surface {
    fn draw(&mut self, qh: &QueueHandle<State>) {
        let now = Instant::now();
        if should_refresh(self.last_frame, now) && self.incoming.is_none() {
            let (tx, rx) = mpsc::channel();
            let name = self.name.clone();
            std::thread::spawn(move || {
                let _ = tx.send(wallpaper_palette(&name));
            });
            self.incoming = Some(rx);
        }
        if let Some(rx) = &self.incoming
            && let Ok(result) = rx.try_recv()
        {
            match &result {
                Some(p) => tracing::info!("backdrop on {}: wallpaper palette {p:?}", self.name),
                None => tracing::info!("backdrop on {}: no wallpaper palette", self.name),
            }
            if let Some(palette) = result {
                self.from = ease(&self.from, &self.to, now - self.changed);
                self.to = palette;
                self.changed = now;
            }
            self.incoming = None;
        }
        self.last_frame = Some(now);
        let Some(paint) = &self.paint else {
            return;
        };
        let surface = self.layer.wl_surface();
        surface.frame(qh, FrameCallbackData(surface.clone()));
        let frame = Frame {
            time: (now - self.start).as_secs_f32(),
            motion: 0.0,
            speed: SPEED,
            scale: 1.0,
            palette: ease(&self.from, &self.to, now - self.changed),
        };
        if let Err(e) = paint.draw(&frame) {
            tracing::warn!("backdrop on {}: {e:#}", self.name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_after_a_pause() {
        let now = Instant::now();
        assert!(should_refresh(None, now), "first frame");
        assert!(!should_refresh(Some(now - Duration::from_secs(1)), now));
        assert!(should_refresh(Some(now - Duration::from_secs(3)), now));
    }

    #[test]
    fn colours_ease_over_a_second() {
        let from = Palette::DEFAULT;
        let to = Palette {
            base: [1.0; 3],
            body: [1.0; 3],
            highlight: [1.0; 3],
        };
        assert_eq!(ease(&from, &to, Duration::ZERO), from);
        assert_eq!(
            ease(&from, &to, Duration::from_millis(500)),
            from.lerp(&to, 0.5)
        );
        assert_eq!(ease(&from, &to, Duration::from_secs(2)), to);
    }
}
