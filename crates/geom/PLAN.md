# Geometry Kernel — Implementation Plan

## 1. Overview

`crates/geom` (`philis-geom`) is the shared integer geometry kernel for the
Philis analog layout engine. It owns every shape type, spatial query, boolean
operation, and orientation transform that the placement and routing subsystems
need. By extracting the kernel into a single crate, both consumers operate on
identical geometry primitives with identical semantics — no duplication, no
drift.

### What it owns

- **Coordinate space.** All geometry is `i64` nanometers. No `f64` anywhere.
- **Shape types.** `Rect` (axis-aligned rectangle) and `RectiPoly`
  (rectilinear polygon with CCW winding).
- **Shape metadata.** `ShapeRef` (a shape annotated with ownership: net ID,
  layer, obstacle/pin/fill/shield role) and `ShapeId` (monotonic, never-reused
  identifier).
- **Orientation transforms.** The eight standard IC orientations (R0, R90,
  R180, R270, MX, MY, MXR90, MYR90) as a closed enum with `apply_point` and
  `apply_rect` methods.
- **Boolean operations.** Merge, clip (intersection), and difference for
  rectilinear geometry, with a rect-rect fast path and a general scanline sweep
  for `RectiPoly`.
- **Spatial index.** A layer-per-index R-tree architecture wrapping the `rstar`
  crate, behind a `SpatialIndex` trait so the backend can be swapped (corner
  stitching in Phase 3+).
- **Geometry store.** `GeometryStore` — the top-level container that dispatches
  insert/remove/query through the per-layer indices.

### Who consumes it

| Consumer | Uses |
|----------|------|
| `crates/place` | Bounding boxes, overlap checks, device R-tree (SA neighbor queries), GDS polygon emission, orientation transforms for placed devices |
| `crates/route` | Wire rects, spatial queries (DRC check regions), boolean ops for DRC region clipping, obstacle subtraction, ShapeRef ownership for net/obstacle tracking, corner-stitching readiness |
| `crates/constraints` | Geometry predicates for guard-ring enclosure, spacing checks, symmetry-axis mirror geometry |
| `crates/api` | Re-export of `Rect`, `Orientation`, coordinate types for the public contract |

### What it does NOT own

- Layer semantics (which layer is metal1, which is via) — that belongs to the
  technology compiler (`crates/tech` or `crates/route/src/tech/`).
- GDS I/O — `crates/api/src/io/` handles reading and writing GDS files.
- DRC rule predicates — `crates/route/src/verify/drc.rs` and
  `crates/place` oracle own the rule dispatch; the kernel provides the geometry
  queries they call.

---

## 2. Module Map

```
crates/geom/src/
  lib.rs              -- public re-exports, crate-level doc
  coord.rs            -- Point, coordinate aliases, AABB helpers
  rect.rs             -- Rect (axis-aligned rectangle)
  polygon.rs          -- RectiPoly (rectilinear polygon, CCW winding)
  orient.rs           -- Orientation enum + transform methods
  boolean.rs          -- merge, clip, difference (rect fast-path + scanline)
  scan.rs             -- scanline sweep internals (interval merge, event queue)
  shape.rs            -- ShapeId, ShapeOwner, ShapeGeometry, ShapeRef
  spatial.rs          -- SpatialIndex trait
  rtree.rs            -- R-tree backend (rstar wrapper, LayerIndex)
  store.rs            -- GeometryStore (top-level container)
```

---

## 3. Phase Plan

### Phase 0: Core shapes and transforms

**Goal:** `Rect`, `RectiPoly`, `Point`, `Orientation`, and their basic
operations. No spatial index yet.

**Modules:** `coord.rs`, `rect.rs`, `polygon.rs`, `orient.rs`

**Exit criteria:**
- `Rect` construction enforces `x_lo < x_hi, y_lo < y_hi`.
- `Rect::contains`, `overlaps`, `intersection`, `expand`, `manhattan_distance`,
  `area`, `width`, `height` pass unit and property-based tests.
- `RectiPoly` construction enforces CCW winding and axis-alignment.
- `RectiPoly::bounding_box`, `contains`, `area`, `decompose` pass tests.
- All 8 orientations round-trip correctly (`apply` then `inverse().apply`
  yields the original point).
- All types are `Send + Sync + Clone + Debug + PartialEq + Eq`.

### Phase 1: Boolean operations

**Goal:** Merge, clip, and difference for rect-rect (fast path) and
RectiPoly-RectiPoly (scanline sweep).

**Modules:** `boolean.rs`, `scan.rs`

**Exit criteria:**
- `Rect::intersection(Rect) -> Option<Rect>` matches the trivial formula.
- `Rect::union(Rect) -> Vec<Rect>` decomposes correctly (at most 3 rects if
  overlapping).
- `Rect::difference(Rect) -> Vec<Rect>` decomposes correctly (at most 4 rects).
- `RectiPoly` boolean operations preserve total area (merge area = A + B -
  intersection area).
- Scanline handles degenerate cases: touching edges (shared boundary, zero
  overlap), nested shapes, identical shapes.

### Phase 2: Spatial index and geometry store

**Goal:** `SpatialIndex` trait, R-tree backend, `ShapeRef`, `GeometryStore` with
insert/remove/query.

**Modules:** `shape.rs`, `spatial.rs`, `rtree.rs`, `store.rs`

**Exit criteria:**
- Insert N shapes, `query_window(bounding_box_of_all)` returns all N.
- `remove(id)` means subsequent queries do not return it.
- `query_point` returns only shapes containing the point.
- `query_within_distance` is equivalent to `query_window(rect.expand(d, d))`
  post-filtered by actual distance.
- `shapes_of(owner)` returns exactly the shapes with that owner.
- `ShapeId` values are never reused after removal.
- All public types are `Send + Sync`.
- Criterion benchmarks: insert 100k rects, window query on 10k-shape index,
  insert+remove cycle.

### Phase 3 (future): Corner stitching backend

**Goal:** Alternative `SpatialIndex` backend that tiles the plane with
non-overlapping rectangles (including explicit space tiles), enabling O(1)
empty-space queries.

**Decision gate:** Implement only if profiling shows that empty-space queries
(access oracle, detailed router) are a bottleneck with the R-tree. The
`SpatialIndex` trait boundary means the switch is local.

---

## 4. Detailed Module Specifications

### 4.1 `coord.rs` — Points and Coordinate Helpers

```rust
/// A 2D point in i64 nanometers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Point {
    pub x: i64,
    pub y: i64,
}

impl Point {
    pub const fn new(x: i64, y: i64) -> Self;
    pub const ORIGIN: Point = Point { x: 0, y: 0 };
    pub fn manhattan_distance(self, other: Point) -> i64;
    pub fn translate(self, dx: i64, dy: i64) -> Point;
}
```

**Invariants:** None beyond the type system. Points are unrestricted `i64`
pairs.

### 4.2 `rect.rs` — Axis-Aligned Rectangle

```rust
/// Axis-aligned, closed rectangle. Boundary is included.
/// Invariant: x_lo < x_hi AND y_lo < y_hi (non-degenerate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x_lo: i64,
    pub y_lo: i64,
    pub x_hi: i64,
    pub y_hi: i64,
}

impl Rect {
    /// Panics if degenerate (lo >= hi on either axis).
    pub fn new(x_lo: i64, y_lo: i64, x_hi: i64, y_hi: i64) -> Self;

    /// Returns None for degenerate inputs instead of panicking.
    pub fn try_new(x_lo: i64, y_lo: i64, x_hi: i64, y_hi: i64) -> Option<Self>;

    pub fn width(&self) -> i64;        // x_hi - x_lo
    pub fn height(&self) -> i64;       // y_hi - y_lo
    pub fn area(&self) -> i64;         // width * height
    pub fn center(&self) -> Point;     // ((x_lo+x_hi)/2, (y_lo+y_hi)/2)
    pub fn lo(&self) -> Point;         // (x_lo, y_lo)
    pub fn hi(&self) -> Point;         // (x_hi, y_hi)

    pub fn contains_point(&self, p: Point) -> bool;
    pub fn contains_rect(&self, other: &Rect) -> bool;
    pub fn overlaps(&self, other: &Rect) -> bool;

    /// Returns the intersection rectangle, or None if disjoint.
    pub fn intersection(&self, other: &Rect) -> Option<Rect>;

    /// Expand by `dx` on each side horizontally, `dy` vertically.
    /// Result may be larger than i64::MAX — caller's responsibility.
    pub fn expand(&self, dx: i64, dy: i64) -> Rect;

    /// Shrink by `dx`/`dy`. Returns None if the result would be degenerate.
    pub fn shrink(&self, dx: i64, dy: i64) -> Option<Rect>;

    /// Minimum edge-to-edge Manhattan distance. Returns 0 if overlapping.
    pub fn manhattan_distance(&self, other: &Rect) -> i64;

    /// Smallest Rect containing both self and other.
    pub fn bounding_union(&self, other: &Rect) -> Rect;

    /// Translate by (dx, dy).
    pub fn translate(&self, dx: i64, dy: i64) -> Rect;
}
```

**Invariants:**
- `x_lo < x_hi` and `y_lo < y_hi` — enforced at construction. Zero-area
  rectangles are forbidden because they create degenerate spatial-index entries
  and ambiguous containment queries.
- `area()` is always positive (`i64`, but guaranteed > 0 by the constructor).

**Testing:**
- Unit: construction rejects degenerate inputs, `contains`/`overlaps` boundary
  cases, `intersection` of disjoint/touching/nested/identical rects.
- Property-based: `intersection` is commutative; `bounding_union` contains both
  inputs; `expand(d).shrink(d) == Some(original)` for positive `d`;
  `manhattan_distance` is commutative and zero iff `overlaps`.

### 4.3 `polygon.rs` — Rectilinear Polygon

```rust
/// A rectilinear polygon: all edges axis-aligned, vertices in CCW order.
/// First vertex is NOT repeated at the end.
/// Invariant: >= 4 vertices, all edges axis-aligned, CCW winding, simple
/// (non-self-intersecting).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RectiPoly {
    vertices: Vec<Point>,
}

impl RectiPoly {
    /// Constructs from a vertex list. Returns Err if:
    /// - fewer than 4 vertices
    /// - any edge is not axis-aligned
    /// - winding is not CCW (will attempt to reverse if CW)
    /// - polygon is self-intersecting
    pub fn new(vertices: Vec<Point>) -> Result<Self, GeomError>;

    pub fn vertices(&self) -> &[Point];
    pub fn vertex_count(&self) -> usize;
    pub fn bounding_box(&self) -> Rect;
    pub fn contains(&self, p: Point) -> bool;

    /// Signed area via shoelace formula. Always positive for CCW.
    pub fn area(&self) -> i64;

    /// Decompose into non-overlapping axis-aligned rectangles.
    /// The decomposition is not unique; this uses a horizontal-sweep
    /// maximal-strip algorithm.
    pub fn decompose(&self) -> Vec<Rect>;

    /// Number of edges.
    pub fn edge_count(&self) -> usize;

    /// Iterator over edges as (Point, Point) pairs.
    pub fn edges(&self) -> impl Iterator<Item = (Point, Point)> + '_;
}
```

**Invariants:**
- All edges are axis-aligned (each edge is purely horizontal or purely
  vertical).
- Winding is CCW. The constructor checks the signed area and reverses if
  negative (CW input).
- Simple (non-self-intersecting). The constructor validates this by checking
  for edge crossings.
- At least 4 vertices (the minimum for a non-degenerate rectilinear polygon).

**Testing:**
- Unit: L-shape, T-shape, U-shape construction and `area`/`bounding_box`.
- Unit: `contains` for points inside, outside, on edge, on vertex.
- Unit: `decompose` area sum equals `area()`.
- Property-based: `decompose` rectangles are non-overlapping and their union
  equals the original polygon's area.

### 4.4 `orient.rs` — Orientation Transforms

```rust
/// The eight standard IC orientations (GDS-II / OASIS convention).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Orientation {
    R0,      // identity
    R90,     // 90 degrees CCW
    R180,    // 180 degrees
    R270,    // 270 degrees CCW (= 90 CW)
    MX,      // mirror about X axis (y -> -y)
    MY,      // mirror about Y axis (x -> -x)
    MXR90,   // MX then R90
    MYR90,   // MY then R90
}

impl Orientation {
    /// Apply this transform to a point relative to the origin.
    pub fn apply_point(self, p: Point) -> Point;

    /// Apply this transform to a rect whose center is at the origin.
    /// Returns a new Rect with the transformed corners, re-normalized
    /// so that lo < hi.
    pub fn apply_rect(self, r: Rect) -> Rect;

    /// Apply this transform to a point relative to a given center.
    /// Equivalent to: translate(-center), apply_point, translate(+center).
    pub fn apply_about(self, p: Point, center: Point) -> Point;

    /// Apply to a RectiPoly about a given center.
    pub fn apply_poly(self, poly: &RectiPoly, center: Point) -> RectiPoly;

    /// The inverse orientation.
    pub fn inverse(self) -> Orientation;

    /// Compose two orientations: self followed by other.
    pub fn compose(self, other: Orientation) -> Orientation;

    /// Whether this orientation includes a reflection.
    pub fn is_reflected(self) -> bool;
}
```

**Transform table (on `(x, y)` relative to center):**

| Orientation | x' | y' |
|-------------|----|----|
| R0 | x | y |
| R90 | -y | x |
| R180 | -x | -y |
| R270 | y | -x |
| MX | x | -y |
| MY | -x | y |
| MXR90 | y | x |
| MYR90 | -y | -x |

**Invariants:**
- `orientation.compose(orientation.inverse()) == R0` for all orientations.
- The eight orientations form the dihedral group D4 under `compose`.
- `apply_rect` always produces a valid (non-degenerate) `Rect`.

**Testing:**
- Exhaustive: all 8 orientations round-trip (`apply` then `inverse().apply`).
- Exhaustive: all 64 `compose` pairs match the D4 multiplication table.
- Unit: `apply_rect` on a non-square rect produces correct dimensions (width
  and height swap for R90/R270/MXR90/MYR90).

### 4.5 `boolean.rs` — Boolean Operations

```rust
/// Boolean operation result. May be empty, a single Rect, or a set of Rects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BooleanResult {
    Empty,
    Single(Rect),
    Multi(Vec<Rect>),
}

impl BooleanResult {
    pub fn rects(&self) -> &[Rect];
    pub fn is_empty(&self) -> bool;
    pub fn total_area(&self) -> i64;
}

// ---- Rect-Rect fast path (inlined, no allocation for the common case) ----

/// Intersection of two rects. Delegates to Rect::intersection.
pub fn rect_intersect(a: &Rect, b: &Rect) -> Option<Rect>;

/// Union of two rects. Returns 1-3 non-overlapping rects covering both.
pub fn rect_union(a: &Rect, b: &Rect) -> BooleanResult;

/// Difference a - b. Returns 0-4 non-overlapping rects.
pub fn rect_difference(a: &Rect, b: &Rect) -> BooleanResult;

// ---- General rectilinear boolean ops (scanline) ----

/// Merge (union) two rectilinear regions.
pub fn merge(a: &[Rect], b: &[Rect]) -> Vec<Rect>;

/// Clip (intersection) of two rectilinear regions.
pub fn clip(a: &[Rect], b: &[Rect]) -> Vec<Rect>;

/// Difference of two rectilinear regions (a minus b).
pub fn difference(a: &[Rect], b: &[Rect]) -> Vec<Rect>;
```

**Algorithm:**
1. **Rect-rect fast path.** When both operands are single `Rect`s, use direct
   coordinate comparisons. No heap allocation in the intersection case. Union
   and difference produce `SmallVec`-style inline results (at most 3-4 rects).
2. **General scanline.** For multi-rect or `RectiPoly` operands:
   - Collect all unique y-coordinates from both operand sets as sweep events.
   - Sort events. Between consecutive y-values, the cross-section is a set of
     horizontal intervals.
   - Apply the boolean operation (union/intersection/difference) to the interval
     sets using a sorted-interval merge algorithm.
   - Reconstruct the output as a set of maximal-height axis-aligned rectangles.
   - Complexity: `O((n + k) log n)` where `n` = total vertex/rect count, `k` =
     intersection events. For typical routing shapes (<20 vertices), this is
     fast.

**Testing:**
- Unit: all rect-rect cases (disjoint, touching edge, overlapping,
  contained, identical).
- Property-based: `area(A) + area(B) - area(intersect(A,B)) == area(merge(A,B))`.
- Property-based: `area(difference(A,B)) + area(clip(A,B)) == area(A)`.
- Regression: two rects at exactly 0 distance (shared edge, zero-area
  intersection).

### 4.6 `scan.rs` — Scanline Sweep Internals

```rust
/// A horizontal interval on a scanline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Interval {
    pub lo: i64,
    pub hi: i64,
}

/// An event in the y-sweep.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SweepEvent {
    pub y: i64,
    pub kind: EventKind, // Enter | Exit
    pub interval: Interval,
    pub operand: Operand,  // A | B
}

/// Merge a sorted slice of intervals into a minimal non-overlapping set.
pub(crate) fn merge_intervals(intervals: &mut Vec<Interval>);

/// Intersect two sorted, non-overlapping interval sets.
pub(crate) fn intersect_intervals(a: &[Interval], b: &[Interval]) -> Vec<Interval>;

/// Subtract interval set B from interval set A.
pub(crate) fn subtract_intervals(a: &[Interval], b: &[Interval]) -> Vec<Interval>;

/// Union two sorted, non-overlapping interval sets.
pub(crate) fn union_intervals(a: &[Interval], b: &[Interval]) -> Vec<Interval>;
```

This module is `pub(crate)` — not part of the public API. It provides the
interval-level primitives that `boolean.rs` composes.

### 4.7 `shape.rs` — Shape Metadata

```rust
/// A monotonic, never-reused shape identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ShapeId(pub u64);

/// Identifies a routing/placement layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LayerId(pub u16);

/// Identifies a net.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NetId(pub u32);

/// Identifies a terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TerminalId(pub u32);

/// Who owns a shape in the geometry store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ShapeOwner {
    Net(NetId),
    Obstacle,
    Pin(TerminalId),
    Fill,
    Shield(NetId),
}

/// The geometric content of a shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShapeGeometry {
    Rect(Rect),
    Poly(RectiPoly),
}

impl ShapeGeometry {
    pub fn bounding_box(&self) -> Rect;
}

/// A shape with full ownership metadata, stored in the spatial index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShapeRef {
    pub id: ShapeId,
    pub layer: LayerId,
    pub owner: ShapeOwner,
    pub bbox: Rect,               // cached bounding box (R-tree envelope)
    pub geometry: ShapeGeometry,
}
```

**Invariants:**
- `ShapeId` values are monotonically increasing and never reused, even after
  `remove`. This prevents dangling-reference bugs.
- `bbox` always equals `geometry.bounding_box()`. The store enforces this at
  insertion time.

### 4.8 `spatial.rs` — SpatialIndex Trait

```rust
/// Backend-agnostic spatial index interface.
///
/// Implementations must be `Send + Sync`.
pub trait SpatialIndex: Send + Sync {
    fn insert(&mut self, shape: ShapeRef);
    fn remove(&mut self, id: ShapeId) -> bool;
    fn query_point(&self, point: Point) -> Vec<&ShapeRef>;
    fn query_window(&self, rect: &Rect) -> Vec<&ShapeRef>;
    fn query_nearest(&self, point: Point, k: usize) -> Vec<&ShapeRef>;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool;
}
```

The trait does NOT include a `layer` parameter — each `SpatialIndex` instance
is per-layer. The `GeometryStore` dispatches to the correct per-layer index.

### 4.9 `rtree.rs` — R-tree Backend

```rust
use rstar::{RTree, RTreeObject, AABB};

/// R-tree spatial index for one layer, wrapping `rstar`.
pub struct RTreeIndex {
    tree: RTree<ShapeRef>,
    by_id: BTreeMap<ShapeId, ShapeRef>,  // for remove-by-id
}

impl SpatialIndex for RTreeIndex { /* ... */ }
```

**Implementation notes:**
- `ShapeRef` implements `rstar::RTreeObject` with `Envelope = AABB<[i64; 2]>`,
  using `bbox` as the envelope. `rstar` 0.12 supports `i64` coordinates natively
  via `rstar::primitives`.
- `remove` is O(n) in the worst case with `rstar` (no indexed removal). The
  `by_id` map provides O(log n) lookup; removal rebuilds if the tree is large.
  This is acceptable for the shape counts in analog layout (thousands per layer,
  not millions). If profiling shows removal is a bottleneck, switch to a bulk
  rebuild strategy.
- `query_point` filters `tree.locate_all_at_point` by exact containment
  (`shape.geometry.contains(point)`), not just bounding-box intersection.

### 4.10 `store.rs` — GeometryStore

```rust
/// The top-level geometry container. One R-tree per layer, monotonic ShapeId
/// allocation, secondary owner index.
pub struct GeometryStore {
    layers: BTreeMap<LayerId, RTreeIndex>,
    next_id: u64,
    owner_index: BTreeMap<ShapeOwner, Vec<ShapeId>>,
}

impl GeometryStore {
    pub fn new() -> Self;

    /// Insert a shape. Returns the assigned ShapeId.
    pub fn insert(
        &mut self,
        layer: LayerId,
        owner: ShapeOwner,
        geometry: ShapeGeometry,
    ) -> ShapeId;

    /// Remove a shape by ID. Returns true if found.
    pub fn remove(&mut self, id: ShapeId) -> bool;

    /// All shapes on `layer` whose bounding box intersects `point`.
    /// Post-filtered by exact containment.
    pub fn query_point(
        &self,
        layer: LayerId,
        point: Point,
    ) -> Vec<&ShapeRef>;

    /// All shapes on `layer` whose bounding box intersects `rect`.
    pub fn query_window(
        &self,
        layer: LayerId,
        rect: &Rect,
    ) -> Vec<&ShapeRef>;

    /// All shapes on `layer` within `distance` of `rect`.
    /// Equivalent to query_window(layer, rect.expand(distance, distance))
    /// post-filtered by actual Manhattan distance.
    pub fn query_within_distance(
        &self,
        layer: LayerId,
        rect: &Rect,
        distance: i64,
    ) -> Vec<&ShapeRef>;

    /// All shapes owned by `owner`, across all layers.
    pub fn shapes_of(&self, owner: ShapeOwner) -> Vec<&ShapeRef>;

    /// All shapes on a given layer.
    pub fn shapes_on_layer(&self, layer: LayerId) -> Vec<&ShapeRef>;

    /// Number of shapes across all layers.
    pub fn len(&self) -> usize;

    pub fn is_empty(&self) -> bool;
}
```

**Invariants:**
- `next_id` only increments. IDs are never reused.
- `owner_index` stays in sync with the per-layer trees. `insert` adds to both;
  `remove` removes from both.
- A `LayerId` that has never had a shape inserted has no entry in `layers`.
  `query_*` on a missing layer returns an empty result, not a panic.

---

## 5. Public API Surface

`lib.rs` re-exports the following (everything else is `pub(crate)`):

```rust
// lib.rs
pub mod coord;
pub mod rect;
pub mod polygon;
pub mod orient;
pub mod boolean;
pub mod shape;
pub mod spatial;
pub mod rtree;
pub mod store;

// Convenience re-exports at crate root
pub use coord::Point;
pub use rect::Rect;
pub use polygon::RectiPoly;
pub use orient::Orientation;
pub use boolean::{BooleanResult, rect_intersect, rect_union, rect_difference,
                  merge, clip, difference};
pub use shape::{ShapeId, LayerId, NetId, TerminalId, ShapeOwner,
                ShapeGeometry, ShapeRef};
pub use spatial::SpatialIndex;
pub use rtree::RTreeIndex;
pub use store::GeometryStore;
```

The `scan` module is `pub(crate)` — internal to the boolean operations.

---

## 6. Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `rstar` | 0.12 | R-tree spatial index with `i64` coordinate support |

### Dev dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `proptest` | 1.x | Property-based testing for geometry invariants |
| `criterion` | 0.5 | Performance benchmarks (insert, query, boolean ops) |

### Explicitly NOT depended on

- No `f64` geometry crates (`geo`, `parry2d`, etc.) — integer coordinates only.
- No serialization (`serde`) in the kernel itself. Consumers serialize their own
  domain types and construct kernel types from them.
- No `crates/api` — the geom crate has zero intra-workspace dependencies. It is
  a leaf of the dependency graph.

---

## 7. Testing Strategy

### Unit tests (per-module `#[cfg(test)]`)

Every module has inline tests for:
- Construction: valid inputs succeed, invalid inputs return `Err` or panic as
  documented.
- Boundary cases: zero-distance, touching edges, contained shapes, identical
  shapes, degenerate coordinates.
- Correctness: operations produce the documented results.

### Property-based tests (`tests/properties.rs`)

Using `proptest`:

```rust
// Rect properties
proptest! {
    #[test]
    fn intersection_commutative(a: Rect, b: Rect) {
        assert_eq!(a.intersection(&b), b.intersection(&a));
    }

    #[test]
    fn bounding_union_contains_both(a: Rect, b: Rect) {
        let u = a.bounding_union(&b);
        assert!(u.contains_rect(&a));
        assert!(u.contains_rect(&b));
    }

    #[test]
    fn expand_shrink_roundtrip(r: Rect, d in 1i64..1000) {
        assert_eq!(r.expand(d, d).shrink(d, d), Some(r));
    }

    #[test]
    fn boolean_area_identity(a: Rect, b: Rect) {
        let inter = rect_intersect(&a, &b).map_or(0, |r| r.area());
        let union_area = rect_union(&a, &b).total_area();
        assert_eq!(a.area() + b.area() - inter, union_area);
    }

    #[test]
    fn orientation_roundtrip(o: Orientation, p: Point) {
        assert_eq!(o.inverse().apply_point(o.apply_point(p)), p);
    }
}
```

### Integration tests (`tests/store.rs`)

- Insert 1000 shapes across 10 layers, query each layer, verify counts.
- Insert then remove all shapes, verify store is empty.
- `query_within_distance` returns a superset of `query_window` for the
  non-expanded rect.

### Benchmarks (`benches/geom.rs`, using Criterion)

| Benchmark | Setup | Measurement |
|-----------|-------|-------------|
| `insert_100k` | Generate 100k random rects on one layer | Time to insert all |
| `query_window_10k` | 10k rects, varying window sizes | Time per query |
| `insert_remove_cycle` | 10k inserts, 5k removes, 5k inserts | Total cycle time |
| `boolean_rect_rect` | 10k random rect pairs | Time per `rect_difference` call |
| `boolean_scanline` | 100 random RectiPolys (8-20 vertices) | Time per `merge` call |
| `orientation_apply` | 100k points | Time per `apply_point` |

---

## 8. Performance Considerations

### Cache-friendly layout

- `Rect` is 32 bytes (4 x `i64`), fits in half a cache line. No pointer
  chasing for the common-case shape.
- `ShapeRef` stores `bbox: Rect` inline (not behind a pointer) so the R-tree
  envelope check does not chase a pointer.
- `Point` is 16 bytes (2 x `i64`), `Copy`. No heap allocation for point
  operations.

### Arena allocation readiness

The `GeometryStore` currently uses `BTreeMap` and `Vec` for storage. The design
is compatible with arena allocation if profiling shows that allocation pressure
is a bottleneck:

- `ShapeId` is a plain `u64` index, directly usable as an arena slot key.
- `ShapeRef` is `Clone` — it can be stored in a typed arena (`bumpalo` or
  `typed-arena`) and referenced by `ShapeId`.
- The `SpatialIndex` trait does not require `&mut self` to return references —
  the arena owns the data, the index owns the `ShapeId` keys.

This is a Phase 3+ optimization. Do not add arena allocation before profiling
demonstrates the need.

### Determinism

- All iteration over `BTreeMap` is sorted by key (layer ID, owner). No
  `HashMap` in the public API or in the store internals.
- `ShapeId` allocation is monotonic and deterministic given the same insertion
  sequence.
- Boolean operations produce deterministic output for deterministic input (the
  scanline sweep order is sorted, not hash-dependent).

### R-tree bulk loading

When the initial shape set is known upfront (e.g., loading a placed design
before routing), use `RTree::bulk_load` instead of repeated `insert`. This
produces a better-balanced tree and is O(n log n) vs. O(n log^2 n) for
sequential insertion. The `GeometryStore` should expose a `bulk_insert` method
for this case.

### Integer overflow

`i64` nanometers can represent coordinates up to +/- 9.2 x 10^18 nm = 9.2 x
10^9 meters. Overflow is not a practical concern for IC layout. However,
`area()` computes `width * height` which can overflow for extremely large
rectangles. The implementation should use checked or widening multiplication
(cast to `i128`) for the area computation and document the overflow boundary.

---

## Appendix A: Consumer Use-Case Map

### Place crate (`crates/place`)

| Operation | Geometry kernel call | Where in place |
|-----------|---------------------|----------------|
| Device bounding box | `Rect::new`, `Rect::bounding_union` | `primitive/` catalog |
| Overlap check (SA) | `Rect::overlaps` | `solver/` cost function |
| DRC spacing | `Rect::manhattan_distance`, `GeometryStore::query_within_distance` | `oracle/` DRC evaluator |
| Orientation transform | `Orientation::apply_rect`, `apply_poly` | `geometry/` GDS emission |
| R-tree neighbor query | `GeometryStore::query_window` | `spatial/` SA neighbor lookup |
| GDS polygon output | `Rect`, `RectiPoly` → coordinate vectors | `geometry/` emit |

### Route crate (`crates/route`)

| Operation | Geometry kernel call | Where in route |
|-----------|---------------------|----------------|
| Wire shape insertion | `GeometryStore::insert` | `state/commit.rs` |
| Wire shape removal (rip-up) | `GeometryStore::remove` | `state/commit.rs` |
| DRC check region | `GeometryStore::query_window`, `Rect::expand` | `verify/drc.rs` |
| Obstacle subtraction | `difference(routing_region, obstacles)` | `graph/builder.rs` |
| Same-net merge | `merge(net_shapes_on_layer)` | `verify/lvs.rs` |
| Net ownership lookup | `GeometryStore::shapes_of(Net(id))` | `state/ownership.rs` |
| Access point geometry | `Rect::contains_point`, `GeometryStore::query_point` | `access/candidate.rs` |

### Constraints crate (`crates/constraints`)

| Operation | Geometry kernel call | Where in constraints |
|-----------|---------------------|----------------------|
| Guard-ring enclosure | `Rect::contains_rect`, `Rect::expand` | `reconcile/` geometry checks |
| Spacing predicates | `Rect::manhattan_distance` | intent predicates |
| Symmetry-axis mirror | `Orientation::MY`, `apply_about` | intent compilation |
