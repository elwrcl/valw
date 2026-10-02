//! The edit history: the shapes drawn so far, what undo took away, and
//! what was last saved.

use crate::editor::shape::Shape;

#[derive(Debug, Default)]
pub struct Doc {
    shapes: Vec<Shape>,
    redo: Vec<Shape>,
    saved: Vec<Shape>,
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
    }

    pub fn undo(&mut self) -> bool {
        let Some(shape) = self.shapes.pop() else {
            return false;
        };
        self.redo.push(shape);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(shape) = self.redo.pop() else {
            return false;
        };
        self.shapes.push(shape);
        true
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
