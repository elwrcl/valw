//! `valw toolbar`: a floating bar to pick a mode, then the optional
//! countdown. The choice runs through the normal capture.

pub mod draw;
pub mod layout;
pub mod state;

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use smithay_client_toolkit::{
    seat::{
        keyboard::Keysym,
        pointer::{BTN_LEFT, PointerEvent, PointerEventKind},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerSurface},
    },
    shm::slot::{Buffer, SlotPool},
};
use wayland_client::protocol::wl_shm;
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::{
    Shape, WpCursorShapeDeviceV1,
};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;

use self::layout::{Layout, MenuItem, Target};
use self::state::{Mode, ToolbarState};
use crate::config::Config;
use crate::error::{Cancelled, HintExt};
use crate::wayland::{Output, Wayland};

pub const NAMESPACE: &str = "valw-toolbar";
pub const COUNTDOWN_NAMESPACE: &str = "valw-countdown";
const PILL: (f32, f32) = (72.0, 40.0);

/// The mode and options the user picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Picked {
    pub mode: Mode,
    pub cursor: bool,
    pub preview: bool,
    pub sound: bool,
}

impl From<ToolbarState> for Picked {
    fn from(s: ToolbarState) -> Self {
        Picked {
            mode: s.mode,
            cursor: s.cursor,
            preview: s.preview,
            sound: s.sound,
        }
    }
}

/// Shows the bar, waits for a pick, saves it, runs the countdown.
pub fn run(config: &Config) -> Result<Picked> {
    let path = state::default_path();
    let mut wl = Wayland::connect()?;
    let outputs = wl.outputs();
    anyhow::ensure!(!outputs.is_empty(), "the compositor reported no outputs");
    let output = outputs[crate::focused_output(&outputs)].clone();
    let initial = state::load(&path, config);

    let picked = pick(&mut wl, &output, initial)?;
    if let Err(e) = state::save(&path, &picked.0) {
        tracing::warn!("could not remember the toolbar choice: {e:#}");
    }
    countdown(&mut wl, &output, picked.0.timer)?;
    Ok(Picked::from(picked.0))
}

/// The remembered state, with `mode` (if given) remembered as the new mode.
pub fn remember(path: &Path, config: &Config, mode: Option<Mode>) -> ToolbarState {
    let mut s = state::load(path, config);
    if let Some(mode) = mode {
        s.mode = mode;
        if let Err(e) = state::save(path, &s) {
            tracing::warn!("could not remember the toolbar choice: {e:#}");
        }
    }
    s
}

/// `valw toolbar --run`: the remembered options without the bar (for the
/// Noctalia plugin), then the countdown.
pub fn run_remembered(config: &Config, mode: Option<Mode>) -> Result<Picked> {
    let s = remember(&state::default_path(), config, mode);
    if s.timer > 0 {
        let mut wl = Wayland::connect()?;
        let outputs = wl.outputs();
        anyhow::ensure!(!outputs.is_empty(), "the compositor reported no outputs");
        let output = outputs[crate::focused_output(&outputs)].clone();
        countdown(&mut wl, &output, s.timer)?;
    }
    Ok(Picked::from(s))
}

pub struct Toolbar {
    layer: LayerSurface,
    viewport: WpViewport,
    pool: SlotPool,
    buffer: Option<Buffer>,
    scale: f64,
    /// Logical size once configured.
    size: Option<(u32, u32)>,
    layout: Option<Layout>,
    state: ToolbarState,
    hover: Option<Target>,
    menu_open: bool,
    outcome: Option<Option<Mode>>,
}

fn pick(wl: &mut Wayland, output: &Output, initial: ToolbarState) -> Result<(ToolbarState, Mode)> {
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
    let pool = SlotPool::new(4096, &s.shm).context("could not create shm pool")?;
    wl.state.toolbar = Some(Toolbar {
        layer,
        viewport,
        pool,
        buffer: None,
        scale: output.scale,
        size: None,
        layout: None,
        state: initial,
        hover: None,
        menu_open: false,
        outcome: None,
    });
    let result = wl.dispatch_blocking(|s| s.toolbar.as_ref().is_some_and(|t| t.outcome.is_some()));
    // Keep only the answer: dropping the toolbar destroys its surface, and
    // the bar must be gone (flushed and round-tripped) before anything is
    // captured.
    let ended = wl.state.toolbar.take().map(|t| (t.state, t.outcome));
    let _ = wl.conn.flush();
    let _ = wl.queue.roundtrip(&mut wl.state);
    result?;
    let (state, outcome) = ended.context("the toolbar vanished")?;
    match outcome.flatten() {
        Some(mode) => Ok((ToolbarState { mode, ..state }, mode)),
        None => Err(Cancelled.into()),
    }
}

impl Toolbar {
    fn is(&self, surface: &wayland_client::protocol::wl_surface::WlSurface) -> bool {
        self.layer.wl_surface() == surface
    }

    pub fn configure(&mut self, layer: &LayerSurface, (w, h): (u32, u32)) {
        if !self.is(layer.wl_surface()) || w == 0 || h == 0 {
            return;
        }
        self.size = Some((w, h));
        self.layout = Some(layout::layout((w as f32, h as f32), draw::measure));
        self.viewport.set_destination(w as i32, h as i32);
        self.draw();
    }

    pub fn closed(&mut self, layer: &LayerSurface) {
        if self.is(layer.wl_surface()) {
            self.outcome.get_or_insert(None);
        }
    }

    fn draw(&mut self) {
        let (Some((w, h)), Some(layout)) = (self.size, &self.layout) else {
            return;
        };
        let (pw, ph) = (
            (w as f64 * self.scale).round() as u32,
            (h as f64 * self.scale).round() as u32,
        );
        let pixmap = draw::bar(
            layout,
            (pw, ph),
            self.scale as f32,
            &self.state,
            self.hover,
            self.menu_open,
        );
        let Ok((buffer, canvas)) = self.pool.create_buffer(
            pw as i32,
            ph as i32,
            pw as i32 * 4,
            wl_shm::Format::Argb8888,
        ) else {
            tracing::warn!("could not allocate a toolbar buffer");
            return;
        };
        draw::to_argb(&pixmap, canvas);
        let surface = self.layer.wl_surface();
        surface.damage_buffer(0, 0, pw as i32, ph as i32);
        if buffer.attach_to(surface).is_err() {
            tracing::warn!("could not attach the toolbar buffer");
            return;
        }
        self.layer.commit();
        self.buffer = Some(buffer);
    }

    pub fn pointer(&mut self, events: &[PointerEvent], cursor: Option<&WpCursorShapeDeviceV1>) {
        let Some(layout) = self.layout.clone() else {
            return;
        };
        for event in events {
            if !self.is(&event.surface) {
                continue;
            }
            let p = (event.position.0 as f32, event.position.1 as f32);
            match event.kind {
                PointerEventKind::Enter { serial } => {
                    if let Some(cursor) = cursor {
                        cursor.set_shape(serial, Shape::Default);
                    }
                }
                PointerEventKind::Motion { .. } => {
                    let hover = layout.hit(p, self.menu_open);
                    if hover != self.hover {
                        self.hover = hover;
                        self.draw();
                    }
                }
                PointerEventKind::Press {
                    button: BTN_LEFT, ..
                } => match layout.hit(p, self.menu_open) {
                    Some(Target::Mode(mode)) => self.outcome = Some(Some(mode)),
                    Some(Target::Options) => {
                        self.menu_open = !self.menu_open;
                        self.draw();
                    }
                    Some(Target::Menu(item)) => {
                        match item {
                            MenuItem::Timer(t) => self.state.timer = t,
                            MenuItem::Cursor => self.state.cursor = !self.state.cursor,
                            MenuItem::Preview => self.state.preview = !self.state.preview,
                            MenuItem::Sound => self.state.sound = !self.state.sound,
                        }
                        self.draw();
                    }
                    None if !layout.inside(p, self.menu_open) => self.outcome = Some(None),
                    None => {}
                },
                _ => {}
            }
        }
    }

    pub fn key(&mut self, key: Keysym) {
        if key == Keysym::Escape {
            self.outcome.get_or_insert(None);
        }
    }
}

pub struct Pill {
    layer: LayerSurface,
    viewport: WpViewport,
    pool: SlotPool,
    buffer: Option<Buffer>,
    scale: f64,
    configured: bool,
    text: String,
    cancelled: bool,
}

/// Shows `timer`, `timer - 1`, … 1, one second each; a click on the pill
/// cancels. The pill takes no keyboard focus, so the desktop stays usable.
fn countdown(wl: &mut Wayland, output: &Output, timer: u32) -> Result<()> {
    if timer == 0 {
        return Ok(());
    }
    let qh = wl.queue.handle();
    let s = &wl.state;
    let layer_shell = s
        .layer_shell
        .as_ref()
        .context("compositor does not support wlr-layer-shell")?;
    let viewporter = s
        .viewporter
        .as_ref()
        .context("compositor does not support wp-viewporter")?;
    let surface = s.compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(
        &qh,
        surface,
        Layer::Overlay,
        Some(COUNTDOWN_NAMESPACE),
        Some(&output.wl),
    );
    layer.set_anchor(Anchor::BOTTOM);
    layer.set_margin(0, 0, layout::MARGIN_BOTTOM as i32, 0);
    layer.set_size(PILL.0 as u32, PILL.1 as u32);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.commit();
    let viewport = viewporter.get_viewport(layer.wl_surface(), &qh, ());
    let pool = SlotPool::new(4096, &s.shm).context("could not create shm pool")?;
    wl.state.pill = Some(Pill {
        layer,
        viewport,
        pool,
        buffer: None,
        scale: output.scale,
        configured: false,
        text: timer.to_string(),
        cancelled: false,
    });

    let start = Instant::now();
    let mut result = Ok(());
    for left in (1..=timer).rev() {
        if let Some(pill) = &mut wl.state.pill {
            pill.set_text(&left.to_string());
        }
        let until = start + Duration::from_secs((timer - left + 1) as u64);
        // dispatch_until fails when the deadline passes: that is the tick.
        let _ = wl.dispatch_until(until, |s| s.pill.as_ref().is_some_and(|p| p.cancelled));
        if wl.state.pill.as_ref().is_some_and(|p| p.cancelled) {
            result = Err(Cancelled.into());
            break;
        }
    }
    wl.state.pill = None;
    // The pill must be gone before anything is captured.
    let _ = wl.conn.flush();
    let _ = wl.queue.roundtrip(&mut wl.state);
    result
}

impl Pill {
    fn is(&self, surface: &wayland_client::protocol::wl_surface::WlSurface) -> bool {
        self.layer.wl_surface() == surface
    }

    pub fn configure(&mut self, layer: &LayerSurface) {
        if self.is(layer.wl_surface()) {
            self.configured = true;
            self.viewport.set_destination(PILL.0 as i32, PILL.1 as i32);
            self.draw();
        }
    }

    fn set_text(&mut self, text: &str) {
        self.text = text.to_string();
        self.draw();
    }

    fn draw(&mut self) {
        if !self.configured {
            return;
        }
        let (pw, ph) = (
            (PILL.0 as f64 * self.scale).round() as u32,
            (PILL.1 as f64 * self.scale).round() as u32,
        );
        let pixmap = draw::pill((pw, ph), self.scale as f32, &self.text, draw::RADIUS);
        let Ok((buffer, canvas)) = self.pool.create_buffer(
            pw as i32,
            ph as i32,
            pw as i32 * 4,
            wl_shm::Format::Argb8888,
        ) else {
            return;
        };
        draw::to_argb(&pixmap, canvas);
        let surface = self.layer.wl_surface();
        surface.damage_buffer(0, 0, pw as i32, ph as i32);
        if buffer.attach_to(surface).is_ok() {
            self.layer.commit();
            self.buffer = Some(buffer);
        }
    }

    pub fn pointer(&mut self, events: &[PointerEvent]) {
        for event in events {
            if self.is(&event.surface)
                && matches!(
                    event.kind,
                    PointerEventKind::Press {
                        button: BTN_LEFT,
                        ..
                    }
                )
            {
                self.cancelled = true;
            }
        }
    }

    pub fn closed(&mut self, layer: &LayerSurface) {
        if self.is(layer.wl_surface()) {
            self.cancelled = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pick_carries_the_state_options() {
        let state = ToolbarState {
            mode: Mode::Window,
            timer: 5,
            cursor: true,
            preview: false,
            sound: true,
        };
        assert_eq!(
            Picked::from(state),
            Picked {
                mode: Mode::Window,
                cursor: true,
                preview: false,
                sound: true,
            }
        );
    }

    #[test]
    fn run_remembers_a_given_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("toolbar.toml");
        let config = Config::default();
        state::save(
            &path,
            &ToolbarState {
                timer: 5,
                ..state::defaults(&config)
            },
        )
        .unwrap();

        let state = remember(&path, &config, Some(Mode::Zoom));
        assert_eq!(
            (state.mode, state.timer),
            (Mode::Zoom, 5),
            "the options stay"
        );
        assert_eq!(state::load(&path, &config).mode, Mode::Zoom, "saved");

        assert_eq!(
            remember(&path, &config, None).mode,
            Mode::Zoom,
            "no mode: the remembered one"
        );
    }

    #[test]
    fn run_without_a_mode_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("toolbar.toml");
        let config = Config::default();
        assert_eq!(remember(&path, &config, None), state::defaults(&config));
        assert!(!path.exists());
    }
}
