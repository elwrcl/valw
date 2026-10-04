//! `valw backdrop`: the paint shader, slow and calm, behind niri's
//! overview (niri's `place-within-backdrop` layer rule), coloured from each
//! output's wallpaper. It draws only when niri asks for frames, so it costs
//! next to nothing while the overview is closed (niri then sends a hidden
//! surface about one frame a second).

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
use crate::theme::gl::{Frame, Paint};
use crate::theme::{Pacer, Palette, Step};
use crate::wayland::{State, Wayland};

pub const NAMESPACE: &str = "valw-backdrop";
const SPEED: f32 = 0.35;
/// Frame callbacks this far apart mean niri isn't showing the surface: it
/// throttles hidden surfaces to about one callback a second, and a shown one
/// gets one every frame.
const HIDDEN_GAP: Duration = Duration::from_millis(400);
const EASE: Duration = Duration::from_secs(1);
/// The paint is soft: drawn at this fraction of the output's resolution and
/// scaled up by the compositor, it looks the same at a fraction of the cost.
const RENDER_SCALE: f64 = 0.5;
/// Draw on every n-th frame callback (30 fps at 60 Hz): the flow is slow.
const DRAW_EVERY: u32 = 2;

/// Tells from the frame callbacks' rhythm when the overview shows the
/// surface again, the moment to re-read the wallpaper's colours.
#[derive(Debug, Default)]
pub struct Visibility {
    last_frame: Option<Instant>,
    hidden: bool,
}

impl Visibility {
    /// A frame callback at `now`: whether to re-read the wallpaper (the first
    /// frame, and the first fast one after a hidden spell).
    pub fn frame(&mut self, now: Instant) -> bool {
        let gap = self.last_frame.map(|t| now.saturating_duration_since(t));
        self.last_frame = Some(now);
        match gap {
            None => true,
            Some(gap) if gap > HIDDEN_GAP => {
                self.hidden = true;
                false
            }
            Some(_) => std::mem::take(&mut self.hidden),
        }
    }
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
    visibility: Visibility,
    from: Palette,
    to: Palette,
    changed: Instant,
    /// A wallpaper palette being worked out on a thread.
    incoming: Option<mpsc::Receiver<Option<Palette>>>,
    pacer: Pacer,
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
            visibility: Visibility::default(),
            from: palette,
            to: palette,
            changed: now,
            incoming: None,
            pacer: Pacer::new(DRAW_EVERY),
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
            ((w as f64 * s.scale * RENDER_SCALE).round() as u32).max(1),
            ((h as f64 * s.scale * RENDER_SCALE).round() as u32).max(1),
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
        let step = s.pacer.configure();
        s.step(step, qh);
    }

    pub fn frame_done(&mut self, surface: &WlSurface, qh: &QueueHandle<State>) {
        if let Some(i) = self.index(surface) {
            let s = &mut self.surfaces[i];
            let step = s.pacer.frame();
            s.step(step, qh);
        }
    }
}

impl Surface {
    /// Every configure and frame callback: notice the overview showing the
    /// surface again and pick up a new wallpaper palette, then draw or wait as
    /// the pacer says.
    fn step(&mut self, step: Step, qh: &QueueHandle<State>) {
        let now = Instant::now();
        // Every callback counts, drawn or not: skipped frames are not hidden ones.
        if self.visibility.frame(now) && self.incoming.is_none() {
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
        let surface = self.layer.wl_surface();
        match step {
            Step::Idle => {}
            Step::Wait => {
                surface.frame(qh, FrameCallbackData(surface.clone()));
                surface.commit();
            }
            Step::Draw => {
                let Some(paint) = &self.paint else {
                    return;
                };
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The refresh decisions for callbacks `gaps_ms` apart, after a first one.
    fn refreshes(gaps_ms: &[u64]) -> Vec<bool> {
        let mut now = Instant::now();
        let mut visibility = Visibility::default();
        let mut out = vec![visibility.frame(now)];
        for gap in gaps_ms {
            now += Duration::from_millis(*gap);
            out.push(visibility.frame(now));
        }
        out
    }

    #[test]
    fn refresh_on_the_first_frame_only_while_shown() {
        assert_eq!(refreshes(&[16, 16, 16]), [true, false, false, false]);
    }

    #[test]
    fn refresh_when_frames_speed_up_after_niri_throttled_them() {
        // Hidden, niri sends about one callback a second: no refresh then
        // (that would read the wallpaper every second), one when the
        // overview shows the surface again, and none after.
        assert_eq!(
            refreshes(&[16, 995, 995, 995, 600, 16, 16, 16]),
            [true, false, false, false, false, false, true, false, false]
        );
    }

    #[test]
    fn a_long_silence_also_counts_as_hidden() {
        assert_eq!(
            refreshes(&[16, 3000, 16, 16]),
            [true, false, false, true, false]
        );
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
