//! 2D positions for canvas node placement.

use serde::{Deserialize, Serialize};

/// A 2D position in canvas coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
///
/// # Examples
///
/// `Point::new(100.0, 50.0)` is a canvas-space position when stored on a node;
/// pass it through `canvas_to_screen` before rendering.
pub struct Point {
    /// The x-coordinate.
    pub x: f64,
    /// The y-coordinate.
    pub y: f64,
}

impl Point {
    /// Creates a point at the given coordinates.
    ///
    /// # Examples
    ///
    /// ```
    /// let point = ez_adam::model::geometry::Point::new(10.0, 20.0);
    /// assert_eq!((point.x, point.y), (10.0, 20.0));
    /// ```
    #[must_use]
    pub fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// Point construction preserves both coordinates.
    fn new_sets_x_and_y() {
        let p = Point::new(1.5, -2.0);
        assert_eq!(p.x, 1.5);
        assert_eq!(p.y, -2.0);
    }

    #[test]
    /// Finite coordinates survive a JSON round trip.
    fn round_trips_through_json() {
        let p = Point::new(3.0, 4.0);
        let json = serde_json::to_string(&p).unwrap();
        let back: Point = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }
}
