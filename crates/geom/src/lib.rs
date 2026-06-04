//! Integer geometry kernel for the Philis analog layout engine.
//!
//! All coordinates are `i64` nanometers. No floating point.
//! Provides axis-aligned rectangles, rectilinear polygons, spatial indexing
//! (R-tree), boolean operations, orientation transforms, and a geometry store
//! with insert/remove/query.
//!
//! See `crates/geom/PLAN.md` and `docs/Developer/geom/` for the full design.
