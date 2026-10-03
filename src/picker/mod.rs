//! `valw window`: an Alt+Tab-style window picker over the paint shader.
//! Cards for every window (most recently used first); the shader takes the
//! selected window's icon colours; Enter or a click captures that window.

pub mod icons;
pub mod layout;
pub mod state;

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use image::RgbaImage;
use niri_ipc::Window;
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
use tiny_skia::{FillRule, Paint as SkPaint, Pixmap, PixmapPaint, Transform};
use wayland_client::{Connection, QueueHandle, protocol::wl_surface::WlSurface};
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::{
    Shape, WpCursorShapeDeviceV1,
};
use wayland_protocols::wp::viewporter::client::wp_viewport::WpViewport;

use self::layout::Rect;
use crate::editor::text;
use crate::error::{Cancelled, HintExt};
use crate::theme::gl::{Frame, Paint};
use crate::theme::{Pacer, Palette, Step};
use crate::wayland::{Output, State, Wayland};

pub const NAMESPACE: &str = "valw-picker";
const ICON: f32 = 96.0;
const SPEED: f32 = 0.6;
const EASE: Duration = Duration::from_millis(300);

/// One window as a card.
struct Card {
    window: Window,
    icon: Option<Pixmap>,
    palette: Palette,
    title: String,
    app: String,
}

pub struct Picker {
    // Dropped first: the EGL surface goes before the wl_surface under it.
    paint: Option<Paint>,
    layer: LayerSurface,
    viewport: WpViewport,
    scale: f64,
    /// Logical size once configured.
    size: Option<(u32, u32)>,
    cards: Vec<Card>,
    selected: usize,
    motion: f32,
    start: Instant,
    from: Palette,
    changed: Instant,
    overlay_dirty: bool,
    /// One frame-callback chain, however often the surface is configured.
    pacer: Pacer,
    shift: bool,
    outcome: Option<Option<usize>>,
    error: Option<anyhow::Error>,
}

/// Shows the picker on `output` and returns the chosen window.
pub fn run(wl: &mut Wayland, output: &Output) -> Result<Window> {
    let windows = crate::niri::mru(crate::niri::windows()?);
    anyhow::ensure!(!windows.is_empty(), "there are no windows to capture");
    let px = (ICON as f64 * output.scale).round() as u32;
    let dirs = icons::Dirs::from_env();
    let cards: Vec<Card> = windows
        .into_iter()
        .map(|window| {
            let app_id = window.app_id.clone().unwrap_or_default();
            let image = icons::find(&app_id, &dirs).and_then(|p| icons::load(&p, px));
            let palette = image
                .as_ref()
                .map(|i| crate::theme::from_image(i, true))
                .unwrap_or(Palette::DEFAULT);
            Card {
                title: window.title.clone().unwrap_or_default(),
                app: app_id,
                icon: image.as_ref().and_then(to_pixmap),
                palette,
                window,
            }
        })
        .collect();

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
    let motion = state::next_motion(&state::default_path());
    let now = Instant::now();
    let first = cards[0].palette;
    wl.state.picker = Some(Picker {
        paint: None,
        layer,
        viewport,
        scale: output.scale,
        size: None,
        cards,
        selected: 0,
        motion,
        start: now,
        from: first,
        changed: now,
        overlay_dirty: true,
        pacer: Pacer::new(1),
        shift: false,
        outcome: None,
        error: None,
    });
    let result = wl.dispatch_blocking(|s| {
        s.picker
            .as_ref()
            .is_some_and(|p| p.outcome.is_some() || p.error.is_some())
    });
    // Keep only the answer; dropping the picker destroys its surface, which
    // must be gone before anything is captured.
    let ended = wl.state.picker.take().map(|p| {
        let chosen = p.outcome.flatten().map(|i| p.cards[i].window.clone());
        (chosen, p.error)
    });
    let _ = wl.conn.flush();
    let _ = wl.queue.roundtrip(&mut wl.state);
    result?;
    match ended {
        Some((_, Some(e))) => Err(e),
        Some((Some(window), None)) => Ok(window),
        _ => Err(Cancelled.into()),
    }
}

fn to_pixmap(img: &RgbaImage) -> Option<Pixmap> {
    let mut p = Pixmap::new(img.width(), img.height())?;
    for (dst, src) in p.pixels_mut().iter_mut().zip(img.pixels()) {
        let [r, g, b, a] = src.0;
        *dst = tiny_skia::ColorU8::from_rgba(r, g, b, a).premultiply();
    }
    Some(p)
}

impl Picker {
    fn is(&self, surface: &WlSurface) -> bool {
        self.layer.wl_surface() == surface
    }

    fn rects(&self) -> Vec<Rect> {
        let (w, h) = self.size.unwrap_or((1, 1));
        layout::cards(self.cards.len(), (w as f32, h as f32), self.selected)
    }

    fn palette(&self, now: Instant) -> Palette {
        let to = self.cards[self.selected].palette;
        self.from
            .lerp(&to, (now - self.changed).as_secs_f32() / EASE.as_secs_f32())
    }

    fn select(&mut self, i: usize) {
        if i != self.selected {
            let now = Instant::now();
            self.from = self.palette(now);
            self.changed = now;
            self.selected = i;
            self.overlay_dirty = true;
        }
    }

    pub fn configure(
        &mut self,
        conn: &Connection,
        layer: &LayerSurface,
        (w, h): (u32, u32),
        qh: &QueueHandle<State>,
    ) {
        if !self.is(layer.wl_surface()) || w == 0 || h == 0 {
            return;
        }
        self.size = Some((w, h));
        let px = (
            (w as f64 * self.scale).round() as u32,
            (h as f64 * self.scale).round() as u32,
        );
        self.viewport.set_destination(w as i32, h as i32);
        match &mut self.paint {
            Some(paint) => paint.resize(px),
            None => match Paint::new(conn, self.layer.wl_surface(), px) {
                Ok(paint) => self.paint = Some(paint),
                Err(e) => {
                    self.error = Some(e.context("could not start OpenGL ES for the picker"));
                    return;
                }
            },
        }
        self.overlay_dirty = true;
        if self.pacer.configure() == Step::Draw {
            self.draw(qh);
        }
    }

    pub fn closed(&mut self, layer: &LayerSurface) {
        if self.is(layer.wl_surface()) {
            tracing::info!("picker closed by the compositor");
            self.outcome.get_or_insert(None);
        }
    }

    pub fn frame_done(&mut self, surface: &WlSurface, qh: &QueueHandle<State>) {
        if self.is(surface) && self.pacer.frame() == Step::Draw {
            self.draw(qh);
        }
    }

    fn draw(&mut self, qh: &QueueHandle<State>) {
        let (Some((w, h)), true) = (self.size, self.paint.is_some()) else {
            return;
        };
        if self.overlay_dirty {
            let px = (
                (w as f64 * self.scale).round() as u32,
                (h as f64 * self.scale).round() as u32,
            );
            let overlay = self.overlay(px);
            if let Some(paint) = &mut self.paint {
                paint.set_overlay(overlay.as_ref());
            }
            self.overlay_dirty = false;
        }
        let now = Instant::now();
        let frame = Frame {
            time: (now - self.start).as_secs_f32(),
            motion: self.motion,
            speed: SPEED,
            scale: 1.0,
            palette: self.palette(now),
        };
        let surface = self.layer.wl_surface();
        surface.frame(qh, FrameCallbackData(surface.clone()));
        if let Some(paint) = &self.paint
            && let Err(e) = paint.draw(&frame)
        {
            self.error = Some(e);
        }
    }

    /// The cards, drawn at buffer size `px`.
    fn overlay(&self, px: (u32, u32)) -> Option<Pixmap> {
        let mut p = Pixmap::new(px.0, px.1)?;
        let s = self.scale as f32;
        let highlight = self
            .palette(Instant::now() + EASE)
            .highlight
            .map(|c| (c * 255.0) as u8);
        for (i, (card, r)) in self.cards.iter().zip(self.rects()).enumerate() {
            let selected = i == self.selected;
            let k = r.w / layout::CARD.0;
            let panel = crate::toolbar::layout::Rect {
                x: r.x,
                y: r.y,
                w: r.w,
                h: r.h,
            };
            let border = if selected {
                [highlight[0], highlight[1], highlight[2], 255]
            } else {
                [255, 255, 255, 40]
            };
            let bw = if selected { 2.0 } else { 1.0 };
            crate::toolbar::draw::rounded(&mut p, panel, 18.0 * k, border, s);
            let inner = crate::toolbar::layout::Rect {
                x: r.x + bw,
                y: r.y + bw,
                w: r.w - 2.0 * bw,
                h: r.h - 2.0 * bw,
            };
            crate::toolbar::draw::rounded(
                &mut p,
                inner,
                18.0 * k - bw,
                [0, 0, 0, if selected { 140 } else { 90 }],
                s,
            );
            // The icon, centred near the top.
            let icon = ICON * k;
            let (ix, iy) = (r.x + (r.w - icon) / 2.0, r.y + 20.0 * k);
            match &card.icon {
                Some(img) => {
                    let t = Transform::from_scale(
                        icon * s / img.width() as f32,
                        icon * s / img.height() as f32,
                    )
                    .post_translate(ix * s, iy * s);
                    p.draw_pixmap(
                        0,
                        0,
                        img.as_ref(),
                        &PixmapPaint {
                            quality: tiny_skia::FilterQuality::Bilinear,
                            ..Default::default()
                        },
                        t,
                        None,
                    );
                }
                None => {
                    let tile = crate::toolbar::layout::Rect {
                        x: ix,
                        y: iy,
                        w: icon,
                        h: icon,
                    };
                    let c = card.palette.body.map(|v| (v * 255.0) as u8);
                    crate::toolbar::draw::rounded(
                        &mut p,
                        tile,
                        20.0 * k,
                        [c[0], c[1], c[2], 255],
                        s,
                    );
                    let initial: String = card
                        .app
                        .chars()
                        .next()
                        .map(|c| c.to_uppercase().collect())
                        .unwrap_or_default();
                    centred(
                        &mut p,
                        &initial,
                        48.0 * k * s,
                        (ix + icon / 2.0) * s,
                        (iy + icon / 2.0) * s,
                        [255, 255, 255, 255],
                    );
                }
            }
            let title = ellipsis(&card.title, 14.0 * k, r.w - 20.0 * k);
            centred(
                &mut p,
                &title,
                14.0 * k * s,
                (r.x + r.w / 2.0) * s,
                (iy + icon + 26.0 * k) * s,
                [255, 255, 255, 255],
            );
            let app = ellipsis(&card.app, 12.0 * k, r.w - 20.0 * k);
            centred(
                &mut p,
                &app,
                12.0 * k * s,
                (r.x + r.w / 2.0) * s,
                (iy + icon + 48.0 * k) * s,
                [255, 255, 255, 170],
            );
        }
        Some(p)
    }

    pub fn pointer(&mut self, events: &[PointerEvent], cursor: Option<&WpCursorShapeDeviceV1>) {
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
                    if let Some(i) = layout::hit(&self.rects(), p) {
                        self.select(i);
                    }
                }
                PointerEventKind::Press {
                    button: BTN_LEFT, ..
                } => {
                    let hit = layout::hit(&self.rects(), p);
                    tracing::info!("picker click at {p:?}: card {hit:?}");
                    self.outcome = Some(hit);
                }
                _ => {}
            }
        }
    }

    pub fn key(&mut self, key: Keysym) {
        let n = self.cards.len();
        match key {
            Keysym::Escape => {
                tracing::info!("picker: Esc");
                self.outcome = Some(None);
            }
            Keysym::Return | Keysym::KP_Enter => self.outcome = Some(Some(self.selected)),
            Keysym::Tab if self.shift => self.select(layout::step(self.selected, n, -1)),
            Keysym::ISO_Left_Tab | Keysym::Left | Keysym::Up => {
                self.select(layout::step(self.selected, n, -1))
            }
            Keysym::Tab | Keysym::Right | Keysym::Down => {
                self.select(layout::step(self.selected, n, 1))
            }
            _ => {}
        }
    }

    pub fn modifiers(&mut self, m: Modifiers) {
        self.shift = m.shift;
    }
}

/// `s` shortened with "…" to fit `width` logical px at `size`.
fn ellipsis(s: &str, size: f32, width: f32) -> String {
    if text::layout(s, size).width <= width {
        return s.to_string();
    }
    let mut chars: Vec<char> = s.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let t: String = chars.iter().collect::<String>() + "…";
        if text::layout(&t, size).width <= width {
            return t;
        }
    }
    String::new()
}

/// `s` centred on (`cx`, `cy`), in buffer px.
fn centred(p: &mut Pixmap, s: &str, size: f32, cx: f32, cy: f32, color: [u8; 4]) {
    let l = text::layout(s, size);
    let origin = (cx - l.width / 2.0, cy - l.height / 2.0);
    if let Some(path) = text::path(s, size, origin) {
        let mut paint = SkPaint::default();
        paint.set_color_rgba8(color[0], color[1], color[2], color[3]);
        paint.anti_alias = true;
        p.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}
