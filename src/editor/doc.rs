//! The edit history: the shapes drawn so far, what undo took away, and
//! what was last saved.

use crate::editor::shape::Shape;

#[derive(Debug, Default)]
pub struct Doc {
    shapes: Vec<Shape>,
    redo: Vec<Shape>,
    saved: Vec<Shape>,
    revision: u64,
}

impl Doc {
    pub fn new() -> Doc {
        Doc::default()
    }

    pub fn shapes(&self) -> &[Shape] {
        &self.shapes
    }

    pub fn add(&mut self, shape: Shape) {
        self.shapes.push(shape);
        self.redo.clear();
        self.revision += 1;
    }

    pub fn undo(&mut self) -> bool {
        let Some(shape) = self.shapes.pop() else {
            return false;
        };
        self.redo.push(shape);
        self.revision += 1;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(shape) = self.redo.pop() else {
            return false;
        };
        self.shapes.push(shape);
        self.revision += 1;
        true
    }

    /// Changes whenever the drawn shapes do.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn can_undo(&self) -> bool {
        !self.shapes.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Whether the shapes differ from what was last saved.
    pub fn is_dirty(&self) -> bool {
        self.shapes != self.saved
    }

    pub fn mark_saved(&mut self) {
        self.saved = self.shapes.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::shape::{Style, Tool};

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
        assert_eq!(d.shapes(), &[line(1.0)]);
        assert!(d.redo());
        assert_eq!(d.shapes(), &[line(1.0), line(2.0)]);
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
        assert_eq!(d.shapes(), &[line(3.0)]);
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
}
