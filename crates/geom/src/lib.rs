//! Integer geometry kernel for the Philis analog layout engine.
//!
//! All coordinates are `i64` nanometers. No floating point.
//! Provides axis-aligned rectangles, rectilinear polygons, spatial indexing
//! (R-tree), boolean operations, orientation transforms, and a geometry store
//! with insert/remove/query.
//!
//! Currently implemented: [`Rect`] and the coordinate aliases in [`coord`].
//! The rest of the kernel (`RectiPoly`, orientation transforms, boolean ops,
//! spatial index, geometry store) is not yet implemented.
//!
//! See `crates/geom/PLAN.md` and `docs/Developer/geom/` for the full design.

pub mod coord;
mod rect;

pub use coord::{Area, Coord, Distance};
pub use rect::{InvalidRect, Rect};
