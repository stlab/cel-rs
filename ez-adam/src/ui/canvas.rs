//! Canvas rendering, coordinates, and gesture handling. See
//! `docs/superpowers/specs/2026-08-26-ez-adam-ui-design.md` §4.

use crate::model::cell_node::CellNodeId;
use crate::model::conditional_group::ConditionalGroupId;
use crate::model::geometry::Point;
use crate::model::relationship_group::RelationshipGroupId;

/// Identifies a canvas node of any of the three kinds `ez-adam` places on
/// the canvas, for selection and drag-target tracking. UI-only — not part
/// of the Phase 1 `Document` model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeId {
    CellNode(CellNodeId),
    RelationshipGroup(RelationshipGroupId),
    ConditionalGroup(ConditionalGroupId),
}

/// The canvas's current pan offset (`x`, `y`) and uniform zoom scale (`k`).
/// UI-only state — `Document`'s stored positions are always canvas/world
/// space, unaffected by this transform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewTransform {
    pub x: f64,
    pub y: f64,
    pub k: f64,
}

impl ViewTransform {
    /// Returns the identity transform: no pan, 1:1 zoom.
    #[must_use]
    pub fn identity() -> Self {
        ViewTransform {
            x: 0.0,
            y: 0.0,
            k: 1.0,
        }
    }
}

/// Converts a screen-space point (e.g. a mouse event's client coordinates)
/// into canvas/world space, by inverting `transform`.
///
/// - Postcondition: `canvas_to_screen(transform, screen_to_canvas(transform, p)) == p`
///   (up to floating-point rounding).
#[must_use]
pub fn screen_to_canvas(transform: &ViewTransform, screen: Point) -> Point {
    Point::new(
        (screen.x - transform.x) / transform.k,
        (screen.y - transform.y) / transform.k,
    )
}

/// Converts a canvas/world-space point into screen space, applying
/// `transform`.
#[must_use]
pub fn canvas_to_screen(transform: &ViewTransform, canvas: Point) -> Point {
    Point::new(
        canvas.x * transform.k + transform.x,
        canvas.y * transform.k + transform.y,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_transform_is_a_no_op_both_ways() {
        let t = ViewTransform::identity();
        let p = Point::new(3.0, 4.0);
        assert_eq!(screen_to_canvas(&t, p), p);
        assert_eq!(canvas_to_screen(&t, p), p);
    }

    #[test]
    fn screen_to_canvas_and_back_round_trips() {
        let t = ViewTransform {
            x: 10.0,
            y: -5.0,
            k: 2.0,
        };
        let p = Point::new(100.0, 50.0);
        let canvas = screen_to_canvas(&t, p);
        let back = canvas_to_screen(&t, canvas);
        assert!((back.x - p.x).abs() < 1e-9);
        assert!((back.y - p.y).abs() < 1e-9);
    }

    #[test]
    fn screen_to_canvas_applies_pan_and_zoom_correctly() {
        // A view panned by (10, 20) and zoomed 2x: screen (30, 40) should
        // map to canvas ((30-10)/2, (40-20)/2) = (10, 10).
        let t = ViewTransform {
            x: 10.0,
            y: 20.0,
            k: 2.0,
        };
        let canvas = screen_to_canvas(&t, Point::new(30.0, 40.0));
        assert_eq!(canvas, Point::new(10.0, 10.0));
    }
}
