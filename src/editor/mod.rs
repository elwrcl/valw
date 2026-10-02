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

use self::doc::Doc;
use self::shape::{Drag, PALETTE, Prim, Style, Tool, WIDTHS, geometry};
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
    /// The `Doc::revision` the texture shows.
    shown: u64,
    toolbar_height: f32,
    doc: Doc,
    tool: Tool,
    color: usize,
    width: usize,
    drag: Option<Drag>,
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
            shown: 0,
            toolbar_height: 0.0,
            doc: Doc::new(),
            tool: Tool::Arrow,
            color: 0,
            width: 1,
            drag: None,
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
        crate::output::encode_png(&export::render(&self.base, self.doc.shapes()))
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
        ] {
            if pressed(Modifiers::NONE, key) {
                self.tool = tool;
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
                    self.tool = tool;
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
        let fit = Fit::new(
            size,
            (area.min.x, area.min.y, area.width(), area.height()),
            ui.ctx().pixels_per_point(),
        );
        let to_pos = |p: shape::P| {
            let (x, y) = fit.to_screen(p);
            Pos2::new(x, y)
        };
        let image_rect =
            Rect::from_min_max(to_pos((0.0, 0.0)), to_pos((size.0 as f32, size.1 as f32)));
        painter.image(
            self.texture.id(),
            image_rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );

        if self.doc.revision() != self.shown {
            let composite = export::render(&self.base, self.doc.shapes());
            let size = [composite.width() as usize, composite.height() as usize];
            let pixels = egui::ColorImage::from_rgba_unmultiplied(size, composite.as_raw());
            self.texture.set(pixels, egui::TextureOptions::LINEAR);
            self.shown = self.doc.revision();
        }

        let shift = ui.input(|i| i.modifiers.shift);
        let pointer = response
            .interact_pointer_pos()
            .map(|p| fit.to_image((p.x, p.y)));
        if response.drag_started()
            && let Some(p) = pointer
            && image_rect.contains(response.interact_pointer_pos().unwrap_or_default())
        {
            self.drag = Some(Drag::new(self.tool, self.style(), p));
        }
        if let (Some(drag), Some(p)) = (&mut self.drag, pointer) {
            drag.move_to(view::clamp(p, size));
        }
        if response.drag_stopped()
            && let Some(drag) = self.drag.take()
        {
            let shape = drag.shape(shift);
            if !shape.is_click() {
                self.doc.add(shape);
            }
        }

        // Finished shapes are in the texture; only the one being drawn is
        // painted here.
        if let Some(live) = self.drag.as_ref().map(|d| d.shape(shift)) {
            for prim in geometry(&live) {
                paint(&painter, &prim, fit.scale, to_pos);
            }
        }
    }
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
}
