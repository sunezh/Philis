//! Axis-aligned integer rectangles.

use std::fmt;

use crate::coord::{Area, Coord, Distance};

/// An axis-aligned rectangle in integer nanometer coordinates.
///
/// Invariant: `x_lo < x_hi` and `y_lo < y_hi`. Construct via [`Rect::new`],
/// which enforces this invariant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x_lo: Coord,
    pub y_lo: Coord,
    pub x_hi: Coord,
    pub y_hi: Coord,
}

/// The coordinates given to [`Rect::new`] did not satisfy `x_lo < x_hi` and
/// `y_lo < y_hi`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidRect {
    pub x_lo: Coord,
    pub y_lo: Coord,
    pub x_hi: Coord,
    pub y_hi: Coord,
}

impl fmt::Display for InvalidRect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid rect: ({}, {}) -> ({}, {}) (requires x_lo < x_hi and y_lo < y_hi)",
            self.x_lo, self.y_lo, self.x_hi, self.y_hi
        )
    }
}

impl std::error::Error for InvalidRect {}

impl Rect {
    /// Construct a rectangle, validating `x_lo < x_hi` and `y_lo < y_hi`.
    pub fn new(x_lo: Coord, y_lo: Coord, x_hi: Coord, y_hi: Coord) -> Result<Self, InvalidRect> {
        if x_lo < x_hi && y_lo < y_hi {
            Ok(Rect {
                x_lo,
                y_lo,
                x_hi,
                y_hi,
            })
        } else {
            Err(InvalidRect {
                x_lo,
                y_lo,
                x_hi,
                y_hi,
            })
        }
    }

    /// Width along the x axis.
    pub fn width(&self) -> Distance {
        self.x_hi - self.x_lo
    }

    /// Height along the y axis.
    pub fn height(&self) -> Distance {
        self.y_hi - self.y_lo
    }

    /// Area of the rectangle.
    pub fn area(&self) -> Area {
        self.width() * self.height()
    }

    /// True if `self` fully contains `other` (inclusive of touching edges).
    pub fn contains(&self, other: &Rect) -> bool {
        self.x_lo <= other.x_lo
            && self.y_lo <= other.y_lo
            && self.x_hi >= other.x_hi
            && self.y_hi >= other.y_hi
    }

    /// True if `self` and `other` overlap with positive area.
    ///
    /// Rectangles that only touch along an edge or at a corner do not
    /// overlap.
    pub fn overlaps(&self, other: &Rect) -> bool {
        self.x_lo < other.x_hi
            && other.x_lo < self.x_hi
            && self.y_lo < other.y_hi
            && other.y_lo < self.y_hi
    }

    /// The intersection of `self` and `other`, or `None` if they do not
    /// overlap with positive area.
    pub fn intersection(&self, other: &Rect) -> Option<Rect> {
        let x_lo = self.x_lo.max(other.x_lo);
        let y_lo = self.y_lo.max(other.y_lo);
        let x_hi = self.x_hi.min(other.x_hi);
        let y_hi = self.y_hi.min(other.y_hi);
        Rect::new(x_lo, y_lo, x_hi, y_hi).ok()
    }

    /// `self` expanded outward by `d` on all four sides.
    ///
    /// `d` may be negative to shrink the rectangle, but the result must
    /// still satisfy `x_lo < x_hi` and `y_lo < y_hi`.
    pub fn expand(&self, d: Distance) -> Result<Rect, InvalidRect> {
        Rect::new(self.x_lo - d, self.y_lo - d, self.x_hi + d, self.y_hi + d)
    }

    /// The Manhattan (L1) gap between `self` and `other`: the sum of the
    /// per-axis edge-to-edge gaps, each clamped to zero when the rectangles'
    /// projections overlap on that axis.
    ///
    /// Zero if the rectangles overlap or touch on both axes.
    pub fn manhattan_distance(&self, other: &Rect) -> Distance {
        let dx = if self.x_hi <= other.x_lo {
            other.x_lo - self.x_hi
        } else if other.x_hi <= self.x_lo {
            self.x_lo - other.x_hi
        } else {
            0
        };
        let dy = if self.y_hi <= other.y_lo {
            other.y_lo - self.y_hi
        } else if other.y_hi <= self.y_lo {
            self.y_lo - other.y_hi
        } else {
            0
        };
        dx + dy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_rejects_degenerate_and_inverted_rects() {
        assert!(Rect::new(0, 0, 10, 10).is_ok());
        assert!(Rect::new(0, 0, 0, 10).is_err());
        assert!(Rect::new(0, 0, 10, 0).is_err());
        assert!(Rect::new(10, 0, 0, 10).is_err());
    }

    #[test]
    fn width_height_area() {
        let r = Rect::new(0, 0, 10, 4).unwrap();
        assert_eq!(r.width(), 10);
        assert_eq!(r.height(), 4);
        assert_eq!(r.area(), 40);
    }

    #[test]
    fn contains() {
        let outer = Rect::new(0, 0, 10, 10).unwrap();
        let inner = Rect::new(2, 2, 8, 8).unwrap();
        let edge = Rect::new(0, 0, 10, 10).unwrap();
        let outside = Rect::new(5, 5, 15, 15).unwrap();
        assert!(outer.contains(&inner));
        assert!(outer.contains(&edge));
        assert!(!outer.contains(&outside));
        assert!(!inner.contains(&outer));
    }

    #[test]
    fn overlaps_excludes_touching() {
        let a = Rect::new(0, 0, 10, 10).unwrap();
        let touching = Rect::new(10, 0, 20, 10).unwrap();
        let overlapping = Rect::new(5, 5, 15, 15).unwrap();
        let disjoint = Rect::new(20, 20, 30, 30).unwrap();

        assert!(!a.overlaps(&touching));
        assert!(!touching.overlaps(&a));
        assert!(a.overlaps(&overlapping));
        assert!(overlapping.overlaps(&a));
        assert!(!a.overlaps(&disjoint));
    }

    #[test]
    fn intersection_matches_overlap() {
        let a = Rect::new(0, 0, 10, 10).unwrap();
        let b = Rect::new(5, 5, 15, 15).unwrap();
        let touching = Rect::new(10, 0, 20, 10).unwrap();

        assert_eq!(a.intersection(&b), Rect::new(5, 5, 10, 10).ok());
        assert_eq!(b.intersection(&a), Rect::new(5, 5, 10, 10).ok());
        assert_eq!(a.intersection(&touching), None);
    }

    #[test]
    fn expand_grows_and_shrinks() {
        let r = Rect::new(10, 10, 20, 20).unwrap();
        assert_eq!(r.expand(5), Rect::new(5, 5, 25, 25));
        assert_eq!(r.expand(-3), Rect::new(13, 13, 17, 17));
        // Shrinking past zero size is invalid.
        assert!(r.expand(-5).is_err());
    }

    #[test]
    fn manhattan_distance_symmetry_and_zero_when_touching() {
        let a = Rect::new(0, 0, 10, 10).unwrap();
        let b = Rect::new(20, 0, 30, 10).unwrap();
        let touching = Rect::new(10, 0, 20, 10).unwrap();
        let diagonal = Rect::new(15, 15, 25, 25).unwrap();

        assert_eq!(a.manhattan_distance(&b), 10);
        assert_eq!(b.manhattan_distance(&a), 10);
        assert_eq!(a.manhattan_distance(&touching), 0);
        assert_eq!(a.manhattan_distance(&diagonal), 10);
        assert_eq!(a.manhattan_distance(&a), 0);
    }
}
