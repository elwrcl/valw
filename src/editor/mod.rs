//! `valw edit`: a Markup-like editor window (eframe, OpenGL ES).

pub mod doc;
pub mod export;
pub mod shape;
pub mod text;
pub mod view;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use eframe::egui::{
    self, Color32, Key, Modifiers, Pos2, Rect, Sense, Stroke, StrokeKind, ViewportCommand,
};
use image::RgbaImage;

use crate::frame::PixelRect;

use self::doc::Doc;
use self::shape::{Drag, P, PALETTE, Prim, Shape, Style, Tool, WIDTHS, geometry, text_size};
use self::view::Fit;

const APP_ID: &str = "valw-editor";
const STATUS_FOR: Duration = Duration::from_secs(2);

pub fn run(path: &Path) -> Result<()> {
    let image = crate::thumbnail::load(path)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (w, h) = image.dimensions();
    let viewport = egui::ViewportBuilder::default()
        .with_title(format!("{name} — valw"))
        .with_app_id(APP_ID)
        .with_inner_size([
            (w as f32).clamp(480.0, 1600.0),
            (h as f32 + 48.0).clamp(320.0, 1000.0),
        ]);
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let path = path.to_path_buf();
    eframe::run_native(
        APP_ID,
        options,
        Box::new(move |cc| Ok(Box::new(Editor::new(&cc.egui_ctx, path, image)))),
    )
    .map_err(|e| anyhow!("the editor window failed: {e}"))
}

struct Editor {
    path: PathBuf,
    base: RgbaImage,
    /// The base with every finished shape, drawn by the export renderer so
    /// the canvas shows the saved pixels.
    texture: egui::TextureHandle,
    /// What the texture shows: the `Doc::revision` and the text being typed.
    texture_key: (u64, Option<Shape>),
    toolbar_height: f32,
    doc: Doc,
    tool: Tool,
    color: usize,
    width: usize,
    drag: Option<Drag>,
    typing: Option<Typing>,
    cropping: Option<CropEdit>,
    status: Option<(String, Instant)>,
    /// The unsaved-changes bar is showing.
    confirm_close: bool,
    /// The user chose to close; let the close through.
    closing: bool,
}

impl Editor {
    fn new(ctx: &egui::Context, path: PathBuf, base: RgbaImage) -> Editor {
        let size = [base.width() as usize, base.height() as usize];
        let pixels = egui::ColorImage::from_rgba_unmultiplied(size, base.as_raw());
        let texture = ctx.load_texture("image", pixels, egui::TextureOptions::LINEAR);
        Editor {
            path,
            base,
            texture,
            texture_key: (0, None),
            toolbar_height: 0.0,
            doc: Doc::new(),
            tool: Tool::Arrow,
            color: 0,
            width: 1,
            drag: None,
            typing: None,
            cropping: None,
            status: None,
            confirm_close: false,
            closing: false,
        }
    }

    fn style(&self) -> Style {
        Style {
            color: PALETTE[self.color],
            width: WIDTHS[self.width].1,
        }
    }

    fn say(&mut self, text: impl Into<String>) {
        self.status = Some((text.into(), Instant::now()));
    }

    fn png(&self) -> Result<Vec<u8>> {
        let image = export::render(&self.base, self.doc.shapes());
        crate::output::encode_png(&export::crop(image, self.doc.crop()))
    }

    fn save(&mut self, copy: bool) -> bool {
        let target = if copy {
            export::edited_path(&self.path)
        } else {
            self.path.clone()
        };
        match self
            .png()
            .and_then(|png| crate::output::write_atomic(&target, &png))
        {
            Ok(()) => {
                if !copy {
                    self.doc.mark_saved();
                }
                tracing::info!("saved {}", target.display());
                self.say(if copy { "Saved a copy" } else { "Saved" });
                true
            }
            Err(e) => {
                tracing::error!("save failed: {e:#}");
                self.say(format!("Could not save: {e}"));
                false
            }
        }
    }

    fn copy(&mut self) {
        let result = self.png().and_then(|png| {
            use std::os::unix::process::CommandExt;
            let exe = std::env::current_exe().context("no path to valw")?;
            let mut command = std::process::Command::new(exe);
            command
                .arg("__clipboard")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            // SAFETY: setsid is async-signal-safe.
            unsafe {
                command.pre_exec(|| rustix::process::setsid().map(drop).map_err(Into::into));
            }
            let mut child = command
                .spawn()
                .context("could not start the clipboard server")?;
            child
                .stdin
                .take()
                .context("no stdin")?
                .write_all(&png)
                .context("could not hand over the image")?;
            Ok(())
        });
        match result {
            Ok(()) => self.say("Copied"),
            Err(e) => {
                tracing::error!("copy failed: {e:#}");
                self.say(format!("Could not copy: {e}"));
            }
        }
    }

    fn close(&mut self, ctx: &egui::Context) {
        self.closing = true;
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }

    fn keys(&mut self, ctx: &egui::Context) {
        if self.typing.is_some() {
            self.type_keys(ctx);
            return;
        }
        let shift_cmd = Modifiers::COMMAND | Modifiers::SHIFT;
        // Holding a key must not save, copy or close again and again; undo
        // and redo may repeat.
        let pressed = |m: Modifiers, k: Key| ctx.input_mut(|i| take_press(&mut i.events, m, k));
        let repeating = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_key(m, k));
        // Most specific first: extra Shift is ignored.
        if pressed(shift_cmd, Key::S) {
            self.save(true);
        } else if pressed(Modifiers::COMMAND, Key::S) && self.save(false) && self.confirm_close {
            self.close(ctx);
        }
        if repeating(shift_cmd, Key::Z) || repeating(Modifiers::COMMAND, Key::Y) {
            self.doc.redo();
        } else if repeating(Modifiers::COMMAND, Key::Z) {
            self.doc.undo();
        }
        if pressed(Modifiers::COMMAND, Key::C) {
            self.copy();
        }
        if self.cropping.is_some() {
            if pressed(Modifiers::NONE, Key::Enter) {
                self.apply_crop();
            } else if pressed(Modifiers::NONE, Key::Escape) {
                self.cropping = None;
            }
        }
        if pressed(Modifiers::NONE, Key::Escape) {
            if self.doc.is_dirty() && !self.confirm_close {
                self.confirm_close = true;
            } else {
                self.close(ctx);
            }
        }
        for (key, tool) in [
            (Key::A, Tool::Arrow),
            (Key::R, Tool::Rectangle),
            (Key::O, Tool::Ellipse),
            (Key::L, Tool::Line),
            (Key::P, Tool::Pen),
            (Key::H, Tool::Highlighter),
            (Key::T, Tool::Text),
            (Key::B, Tool::Pixelate),
            (Key::N, Tool::Number),
            (Key::C, Tool::Crop),
        ] {
            if pressed(Modifiers::NONE, key) {
                self.set_tool(tool);
            }
        }
    }

    /// The compositor asked to close: with unsaved changes, show the bar instead.
    fn guard_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && self.doc.is_dirty() && !self.closing {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.confirm_close = true;
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        // Wraps onto a second row in narrow windows (niri's half-width columns).
        ui.horizontal_wrapped(|ui| {
            for tool in Tool::ALL {
                if ui
                    .selectable_label(self.tool == tool, tool.label())
                    .clicked()
                {
                    self.set_tool(tool);
                }
            }
            ui.separator();
            for (i, [r, g, b]) in PALETTE.into_iter().enumerate() {
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(18.0, 18.0), Sense::click());
                ui.painter()
                    .rect_filled(rect, 4.0, Color32::from_rgb(r, g, b));
                let ring = if i == self.color {
                    ui.visuals().strong_text_color()
                } else {
                    ui.visuals().weak_text_color()
                };
                ui.painter().rect_stroke(
                    rect,
                    4.0,
                    Stroke::new(if i == self.color { 2.0 } else { 1.0 }, ring),
                    StrokeKind::Outside,
                );
                if response.clicked() {
                    self.color = i;
                }
            }
            ui.separator();
            for (i, (label, _)) in WIDTHS.into_iter().enumerate() {
                if ui.selectable_label(self.width == i, label).clicked() {
                    self.width = i;
                }
            }
            ui.separator();
            if ui
                .add_enabled(self.doc.can_undo(), egui::Button::new("Undo"))
                .clicked()
            {
                self.doc.undo();
            }
            if ui
                .add_enabled(self.doc.can_redo(), egui::Button::new("Redo"))
                .clicked()
            {
                self.doc.redo();
            }
            ui.separator();
            if ui.button("Copy").clicked() {
                self.copy();
            }
            if ui.button("Save").clicked() {
                self.save(false);
            }
            if let Some((text, at)) = &self.status
                && at.elapsed() < STATUS_FOR
            {
                ui.label(text.as_str());
                ui.ctx().request_repaint_after(STATUS_FOR);
            }
        });
    }

    fn canvas(&mut self, ui: &mut egui::Ui) {
        let area = ui.max_rect();
        let (response, painter) = ui.allocate_painter(area.size(), Sense::drag());
        let size = self.base.dimensions();
        let shown = self.doc.crop().unwrap_or(PixelRect {
            x: 0,
            y: 0,
            width: size.0,
            height: size.1,
        });
        let fit = Fit::with_crop(
            shown,
            (area.min.x, area.min.y, area.width(), area.height()),
            ui.ctx().pixels_per_point(),
        );
        let to_pos = |p: P| {
            let (x, y) = fit.to_screen(p);
            Pos2::new(x, y)
        };
        let (x0, y0) = (shown.x as f32, shown.y as f32);
        let (x1, y1) = (x0 + shown.width as f32, y0 + shown.height as f32);
        let image_rect = Rect::from_min_max(to_pos((x0, y0)), to_pos((x1, y1)));
        let (w, h) = (size.0 as f32, size.1 as f32);
        painter.image(
            self.texture.id(),
            image_rect,
            Rect::from_min_max(Pos2::new(x0 / w, y0 / h), Pos2::new(x1 / w, y1 / h)),
            Color32::WHITE,
        );

        // The texture: every finished shape plus the text being typed, all
        // drawn by the export renderer.
        let typed = self.typing.as_ref().map(|t| Shape {
            tool: Tool::Text,
            style: self.style(),
            points: vec![t.at],
            text: t.text.clone(),
        });
        let key = (self.doc.revision(), typed.clone());
        if key != self.texture_key {
            let composite = export::render(&self.base, self.doc.shapes().chain(typed.as_ref()));
            let size = [composite.width() as usize, composite.height() as usize];
            let pixels = egui::ColorImage::from_rgba_unmultiplied(size, composite.as_raw());
            self.texture.set(pixels, egui::TextureOptions::LINEAR);
            self.texture_key = key;
        }

        let shift = ui.input(|i| i.modifiers.shift);
        let screen = response.interact_pointer_pos();
        let pointer = screen.map(|p| view::clamp(fit.to_image((p.x, p.y)), shown));
        let crop_screen = |r: (f32, f32, f32, f32)| {
            let (a, b) = (to_pos((r.0, r.1)), to_pos((r.2, r.3)));
            (a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y))
        };

        if response.drag_started()
            && let (Some(p), Some(at)) = (pointer, screen)
            && (image_rect.contains(at) || self.tool == Tool::Crop)
        {
            if self.typing.is_some() {
                // A press elsewhere only finishes the text.
                self.finish_typing();
            } else {
                self.press(p, crop_screen, (at.x, at.y));
            }
        }
        if response.dragged()
            && let Some(p) = pointer
        {
            if let Some(drag) = &mut self.drag {
                drag.move_to(p);
            }
            if let Some(c) = &mut self.cropping {
                match c.grab {
                    Some(e) => {
                        if e.left {
                            c.rect.0 = p.0;
                        }
                        if e.right {
                            c.rect.2 = p.0;
                        }
                        if e.top {
                            c.rect.1 = p.1;
                        }
                        if e.bottom {
                            c.rect.3 = p.1;
                        }
                    }
                    None => (c.rect.2, c.rect.3) = p,
                }
            }
        }
        if response.drag_stopped() {
            if let Some(drag) = self.drag.take() {
                let shape = drag.shape(shift);
                if !shape.is_click() {
                    self.doc.add(shape);
                }
            }
            if let Some(c) = &mut self.cropping {
                c.grab = None;
                c.rect = normalized(c.rect);
                if c.rect.2 - c.rect.0 < 2.0 || c.rect.3 - c.rect.1 < 2.0 {
                    self.cropping = None;
                }
            }
        }

        // Finished shapes are in the texture; only the one being drawn, the
        // caret and the crop frame are painted here.
        if let Some(live) = self.drag.as_ref().map(|d| d.shape(shift)) {
            for prim in geometry(&live) {
                paint(&painter, &prim, fit.scale, to_pos);
            }
        }
        if let Some(t) = &self.typing {
            let size = text_size(self.style().width);
            let lines = text::layout(&t.text, size).lines;
            let row = lines.len().saturating_sub(1) as f32;
            let x = t.at.0 + lines.last().copied().unwrap_or(0.0);
            let y = t.at.1 + row * text::line_height(size);
            let on = (ui.input(|i| i.time) * 2.0) as i64 % 2 == 0;
            if on {
                let [r, g, b] = self.style().color;
                painter.line_segment(
                    [to_pos((x, y)), to_pos((x, y + text::line_height(size)))],
                    Stroke::new(1.5, Color32::from_rgb(r, g, b)),
                );
            }
            ui.ctx().request_repaint_after(CARET_BLINK);
        }
        if let Some(c) = &self.cropping {
            let (cx0, cy0, cx1, cy1) = crop_screen(c.rect);
            let inner = Rect::from_min_max(Pos2::new(cx0, cy0), Pos2::new(cx1, cy1));
            let dim = Color32::from_black_alpha(128);
            let outer = image_rect;
            for r in [
                Rect::from_min_max(outer.min, Pos2::new(outer.max.x, inner.min.y)),
                Rect::from_min_max(Pos2::new(outer.min.x, inner.max.y), outer.max),
                Rect::from_min_max(
                    Pos2::new(outer.min.x, inner.min.y),
                    Pos2::new(inner.min.x, inner.max.y),
                ),
                Rect::from_min_max(
                    Pos2::new(inner.max.x, inner.min.y),
                    Pos2::new(outer.max.x, inner.max.y),
                ),
            ] {
                painter.rect_filled(r, 0.0, dim);
            }
            painter.rect_stroke(
                inner,
                0.0,
                Stroke::new(1.0, Color32::WHITE),
                StrokeKind::Inside,
            );
            if let Some(hover) = response.hover_pos()
                && let Some(e) = crop_edges((cx0, cy0, cx1, cy1), (hover.x, hover.y), CROP_GRAB)
            {
                ui.ctx().set_cursor_icon(resize_icon(e));
            }
        }
    }

    /// A press on the canvas at image point `p` (screen point `at`).
    fn press(
        &mut self,
        p: P,
        crop_screen: impl Fn((f32, f32, f32, f32)) -> (f32, f32, f32, f32),
        at: P,
    ) {
        match self.tool {
            Tool::Text => {
                self.typing = Some(Typing {
                    at: p,
                    text: String::new(),
                });
            }
            Tool::Number => {
                let number = self.doc.next_number();
                self.doc.add(Shape {
                    tool: Tool::Number,
                    style: self.style(),
                    points: vec![p],
                    text: number.to_string(),
                });
            }
            Tool::Crop => {
                let grab = self
                    .cropping
                    .as_ref()
                    .and_then(|c| crop_edges(crop_screen(c.rect), at, CROP_GRAB));
                match (grab, &mut self.cropping) {
                    (Some(e), Some(c)) => c.grab = Some(e),
                    _ => {
                        self.cropping = Some(CropEdit {
                            rect: (p.0, p.1, p.0, p.1),
                            grab: None,
                        });
                    }
                }
            }
            _ => self.drag = Some(Drag::new(self.tool, self.style(), p)),
        }
    }

    fn set_tool(&mut self, tool: Tool) {
        if tool != self.tool {
            self.finish_typing();
            self.cropping = None;
        }
        self.tool = tool;
    }

    /// While typing every key edits the text; nothing else reacts.
    fn type_keys(&mut self, ctx: &egui::Context) {
        let events = ctx.input_mut(|i| std::mem::take(&mut i.events));
        for event in events {
            match event {
                egui::Event::Text(s) => {
                    if let Some(t) = &mut self.typing {
                        t.text.extend(s.chars().filter(|c| !c.is_control()));
                    }
                }
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => match key {
                    Key::Backspace => {
                        if let Some(t) = &mut self.typing {
                            t.text.pop();
                        }
                    }
                    Key::Enter if modifiers.shift => {
                        if let Some(t) = &mut self.typing {
                            t.text.push('\n');
                        }
                    }
                    Key::Enter => self.finish_typing(),
                    Key::Escape => self.typing = None,
                    _ => {}
                },
                _ => {}
            }
        }
    }

    fn finish_typing(&mut self) {
        if let Some(t) = self.typing.take()
            && !t.text.trim().is_empty()
        {
            self.doc.add(Shape {
                tool: Tool::Text,
                style: self.style(),
                points: vec![t.at],
                text: t.text,
            });
        }
    }

    fn apply_crop(&mut self) {
        let Some(c) = self.cropping.take() else {
            return;
        };
        let (x0, y0, x1, y1) = normalized(c.rect);
        let (x0, y0, x1, y1) = (x0.floor(), y0.floor(), x1.ceil(), y1.ceil());
        if x1 - x0 >= 2.0 && y1 - y0 >= 2.0 {
            self.doc.add_crop(PixelRect {
                x: x0 as u32,
                y: y0 as u32,
                width: (x1 - x0) as u32,
                height: (y1 - y0) as u32,
            });
        }
    }
}

/// How close (screen px) a press must be to grab a crop edge.
const CROP_GRAB: f32 = 8.0;
const CARET_BLINK: Duration = Duration::from_millis(500);

/// A text being typed, at its top-left in image px.
struct Typing {
    at: P,
    text: String,
}

/// A crop being drawn or adjusted, in image px (x0, y0, x1, y1).
struct CropEdit {
    rect: (f32, f32, f32, f32),
    grab: Option<Edges>,
}

/// Which edges of the crop rectangle a press grabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Edges {
    left: bool,
    right: bool,
    top: bool,
    bottom: bool,
}

/// The edges of `r` (x0, y0, x1, y1 on screen) within `tolerance` of `p`;
/// `None` inside, outside, or far from every edge.
fn crop_edges(r: (f32, f32, f32, f32), p: P, tolerance: f32) -> Option<Edges> {
    let (x0, y0, x1, y1) = r;
    let within_x = p.0 > x0 - tolerance && p.0 < x1 + tolerance;
    let within_y = p.1 > y0 - tolerance && p.1 < y1 + tolerance;
    let e = Edges {
        left: within_y && (p.0 - x0).abs() <= tolerance,
        right: within_y && (p.0 - x1).abs() <= tolerance,
        top: within_x && (p.1 - y0).abs() <= tolerance,
        bottom: within_x && (p.1 - y1).abs() <= tolerance,
    };
    (e.left || e.right || e.top || e.bottom).then_some(e)
}

fn resize_icon(e: Edges) -> egui::CursorIcon {
    match (e.left || e.right, e.top || e.bottom) {
        (true, false) => egui::CursorIcon::ResizeHorizontal,
        (false, true) => egui::CursorIcon::ResizeVertical,
        _ if (e.left && e.top) || (e.right && e.bottom) => egui::CursorIcon::ResizeNwSe,
        _ => egui::CursorIcon::ResizeNeSw,
    }
}

/// `r` with x0 ≤ x1 and y0 ≤ y1.
fn normalized(r: (f32, f32, f32, f32)) -> (f32, f32, f32, f32) {
    (r.0.min(r.2), r.1.min(r.3), r.0.max(r.2), r.1.max(r.3))
}

/// Consumes presses of `key` with `modifiers` (extra Shift and Alt ignored,
/// like egui's `consume_key`). True only for a fresh press: key repeats are
/// swallowed without counting.
fn take_press(events: &mut Vec<egui::Event>, modifiers: Modifiers, key: Key) -> bool {
    let mut fresh = false;
    events.retain(|event| match event {
        egui::Event::Key {
            key: k,
            pressed: true,
            repeat,
            modifiers: m,
            ..
        } if *k == key && m.matches_logically(modifiers) => {
            fresh |= !repeat;
            false
        }
        _ => true,
    });
    fresh
}

/// Draws one primitive on the canvas.
fn paint(painter: &egui::Painter, prim: &Prim, scale: f32, to_pos: impl Fn(shape::P) -> Pos2) {
    let color = |c: &[u8; 4]| Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
    match prim {
        Prim::Fill { points, color: c } => {
            let points = points.iter().map(|&p| to_pos(p)).collect();
            painter.add(egui::Shape::convex_polygon(points, color(c), Stroke::NONE));
        }
        Prim::Stroke {
            points,
            width,
            color: c,
            closed,
            round,
        } => {
            let points: Vec<Pos2> = points.iter().map(|&p| to_pos(p)).collect();
            let stroke = Stroke::new(width * scale, color(c));
            if *closed {
                painter.add(egui::Shape::closed_line(points, stroke));
            } else {
                // egui lines have butt caps; add the round caps the export draws.
                if *round {
                    for end in [points[0], points[points.len() - 1]] {
                        painter.circle_filled(end, width * scale / 2.0, color(c));
                    }
                }
                painter.add(egui::Shape::line(points, stroke));
            }
        }
    }
}

impl eframe::App for Editor {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.keys(&ctx);
        self.guard_close(&ctx);
        let toolbar = egui::Panel::top("toolbar").show(ui, |ui| {
            self.toolbar(ui);
            ui.min_rect().height()
        });
        // The panel takes its height from the last pass: when the toolbar
        // wraps or unwraps, lay out again so no row is cut off.
        if toolbar.inner != self.toolbar_height {
            self.toolbar_height = toolbar.inner;
            ctx.request_discard("toolbar height changed");
        }
        if self.confirm_close && self.doc.is_dirty() {
            egui::Panel::bottom("unsaved").show(ui, |ui| {
                ui.label("Unsaved changes — Esc to discard, Ctrl+S to save");
            });
        }
        egui::CentralPanel::no_frame().show(ui, |ui| self.canvas(ui));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: Key, modifiers: Modifiers, repeat: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat,
            modifiers,
        }
    }

    #[test]
    fn held_keys_act_once() {
        let mut events = vec![
            key(Key::Escape, Modifiers::NONE, false),
            key(Key::Escape, Modifiers::NONE, true),
            key(Key::Escape, Modifiers::NONE, true),
        ];
        assert!(take_press(&mut events, Modifiers::NONE, Key::Escape));
        assert!(events.is_empty(), "repeats are swallowed too");
        let mut repeats = vec![key(Key::Escape, Modifiers::NONE, true)];
        assert!(!take_press(&mut repeats, Modifiers::NONE, Key::Escape));
    }

    #[test]
    fn modifiers_match_like_consume_key() {
        let mut events = vec![key(Key::S, Modifiers::COMMAND | Modifiers::SHIFT, false)];
        assert!(!take_press(
            &mut events,
            Modifiers::COMMAND | Modifiers::ALT,
            Key::S
        ));
        assert!(
            take_press(&mut events, Modifiers::COMMAND, Key::S),
            "extra Shift ignored"
        );
        let mut plain = vec![key(Key::S, Modifiers::NONE, false)];
        assert!(!take_press(&mut plain, Modifiers::COMMAND, Key::S));
        assert_eq!(plain.len(), 1, "other events stay");
    }

    #[test]
    fn crop_edges_near_edges_and_corners() {
        let r = (100.0, 100.0, 300.0, 200.0);
        let e = |l, r_, t, b| {
            Some(Edges {
                left: l,
                right: r_,
                top: t,
                bottom: b,
            })
        };
        assert_eq!(
            crop_edges(r, (102.0, 150.0), 8.0),
            e(true, false, false, false)
        );
        assert_eq!(
            crop_edges(r, (295.0, 150.0), 8.0),
            e(false, true, false, false)
        );
        assert_eq!(
            crop_edges(r, (200.0, 205.0), 8.0),
            e(false, false, false, true)
        );
        assert_eq!(
            crop_edges(r, (99.0, 101.0), 8.0),
            e(true, false, true, false)
        );
        assert_eq!(crop_edges(r, (200.0, 150.0), 8.0), None, "inside");
        assert_eq!(crop_edges(r, (50.0, 150.0), 8.0), None, "outside");
    }
}
