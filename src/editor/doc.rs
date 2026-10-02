//! The edit history: what was drawn and cropped so far, what undo took
//! away, and what was last saved.

use crate::editor::shape::{Shape, Tool};
use crate::frame::PixelRect;

/// One step of the history.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Shape(Shape),
    /// Show and export only this part of the image (image pixels).
    Crop(PixelRect),
}

#[derive(Debug, Default)]
pub struct Doc {
    items: Vec<Item>,
    redo: Vec<Item>,
    saved: Vec<Item>,
    revision: u64,
}

impl Doc {
    pub fn new() -> Doc {
        Doc::default()
    }

    pub fn shapes(&self) -> impl Iterator<Item = &Shape> {
        self.items.iter().filter_map(|i| match i {
            Item::Shape(s) => Some(s),
            Item::Crop(_) => None,
        })
    }

    /// The last crop, if any.
    pub fn crop(&self) -> Option<PixelRect> {
        self.items.iter().rev().find_map(|i| match i {
            Item::Crop(r) => Some(*r),
            Item::Shape(_) => None,
        })
    }

    /// The number the next Number shape shows.
    pub fn next_number(&self) -> u32 {
        1 + self.shapes().filter(|s| s.tool == Tool::Number).count() as u32
    }

    pub fn add(&mut self, shape: Shape) {
        self.push(Item::Shape(shape));
    }

    pub fn add_crop(&mut self, rect: PixelRect) {
        self.push(Item::Crop(rect));
    }

    fn push(&mut self, item: Item) {
        self.items.push(item);
        self.redo.clear();
        self.revision += 1;
    }

    pub fn undo(&mut self) -> bool {
        let Some(item) = self.items.pop() else {
            return false;
        };
        self.redo.push(item);
        self.revision += 1;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(item) = self.redo.pop() else {
            return false;
        };
        self.items.push(item);
        self.revision += 1;
        true
    }

    /// Changes whenever the drawn shapes or the crop do.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn can_undo(&self) -> bool {
        !self.items.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Whether the history differs from what was last saved.
    pub fn is_dirty(&self) -> bool {
        self.items != self.saved
    }

    pub fn mark_saved(&mut self) {
        self.saved = self.items.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::shape::{Style, Tool};
    use crate::frame::PixelRect;

    fn line(x: f32) -> Shape {
        Shape {
            tool: Tool::Line,
            style: Style {
                color: [0, 0, 0],
                width: 2.0,
            },
            points: vec![(0.0, 0.0), (x, 0.0)],
            text: String::new(),
        }
    }

    #[test]
    fn undo_and_redo_move_shapes() {
        let mut d = Doc::new();
        assert!(!d.can_undo() && !d.can_redo());
        d.add(line(1.0));
        d.add(line(2.0));
        assert!(d.undo());
        assert_eq!(d.shapes().cloned().collect::<Vec<_>>(), [line(1.0)]);
        assert!(d.redo());
        assert_eq!(
            d.shapes().cloned().collect::<Vec<_>>(),
            [line(1.0), line(2.0)]
        );
        assert!(!d.redo());
    }

    #[test]
    fn revision_follows_the_shapes() {
        let mut d = Doc::new();
        let r0 = d.revision();
        d.add(line(1.0));
        let r1 = d.revision();
        assert_ne!(r1, r0);
        d.mark_saved();
        assert_eq!(d.revision(), r1, "saving doesn't change what is drawn");
        d.undo();
        assert_ne!(d.revision(), r1);
        let r2 = d.revision();
        assert!(!d.undo(), "nothing left");
        assert_eq!(d.revision(), r2);
        d.redo();
        assert_ne!(d.revision(), r2);
    }

    #[test]
    fn a_new_shape_clears_redo() {
        let mut d = Doc::new();
        d.add(line(1.0));
        d.undo();
        d.add(line(3.0));
        assert!(!d.can_redo());
        assert_eq!(d.shapes().cloned().collect::<Vec<_>>(), [line(3.0)]);
    }

    #[test]
    fn dirty_until_saved() {
        let mut d = Doc::new();
        assert!(!d.is_dirty());
        d.add(line(1.0));
        assert!(d.is_dirty());
        d.mark_saved();
        assert!(!d.is_dirty());
    }

    #[test]
    fn undo_past_a_save_is_dirty() {
        let mut d = Doc::new();
        d.add(line(1.0));
        d.mark_saved();
        d.undo();
        assert!(d.is_dirty());
        d.redo();
        assert!(!d.is_dirty(), "back to the saved content");
    }

    fn number(n: &str) -> Shape {
        Shape {
            tool: Tool::Number,
            style: Style {
                color: [0, 0, 0],
                width: 4.0,
            },
            points: vec![(1.0, 1.0)],
            text: n.into(),
        }
    }

    const R: PixelRect = PixelRect {
        x: 1,
        y: 2,
        width: 30,
        height: 20,
    };

    #[test]
    fn numbers_count_only_numbers_and_follow_undo() {
        let mut d = Doc::new();
        assert_eq!(d.next_number(), 1);
        d.add(number("1"));
        d.add(line(5.0));
        d.add(number("2"));
        assert_eq!(d.next_number(), 3);
        d.undo();
        assert_eq!(d.next_number(), 2);
    }

    #[test]
    fn crops_are_undoable_and_nest() {
        let mut d = Doc::new();
        assert_eq!(d.crop(), None);
        d.add_crop(R);
        let inner = PixelRect {
            x: 5,
            y: 5,
            width: 10,
            height: 10,
        };
        d.add_crop(inner);
        assert_eq!(d.crop(), Some(inner));
        d.undo();
        assert_eq!(d.crop(), Some(R));
        d.undo();
        assert_eq!(d.crop(), None);
        d.redo();
        assert_eq!(d.crop(), Some(R));
    }

    #[test]
    fn crops_make_the_doc_dirty_and_skip_shapes() {
        let mut d = Doc::new();
        d.add(line(1.0));
        d.mark_saved();
        d.add_crop(R);
        assert!(d.is_dirty());
        assert_eq!(d.shapes().count(), 1);
    }
}
