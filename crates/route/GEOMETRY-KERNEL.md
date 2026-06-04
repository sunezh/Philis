# Subsystem: Geometry Kernel

> **Shared crate:** This subsystem now lives in `crates/geom`
> (`philis_geom`). The geometry kernel is a shared crate used by both the
> placement and routing engines. This document describes what the shared
> crate provides. The route crate depends on `philis_geom` rather than
> implementing geometry types and spatial indexing internally.

**Crate:** `crates/geom` (`philis-geom`)

**Purpose:** Provide the shape types, spatial queries, and boolean operations
that every other subsystem depends on. This is the physical-world
representation layer — all coordinates are integer nanometers, all queries are
exact (no floating-point tolerance issues).

---

## 1. Design principles

### Integer coordinates only

All geometry is in `i64` nanometers. No `f64` anywhere in the geometry kernel.
This eliminates:
- Epsilon-comparison bugs in intersection tests
- Non-transitive equality (a == b and b == c but a != c due to rounding)
- Degenerate polygon topologies from floating-point snap

The tech model's `db_unit_nm` converts between internal coordinates and GDS
database units for I/O. All computation stays in nanometers.

### Axis-aligned only (Phase 0-2)

Routing geometry is overwhelmingly axis-aligned (rectilinear). The kernel
handles:
- Axis-aligned rectangles (the 95% case)
- Rectilinear polygons (L-shapes, T-shapes, min-area patches)

45-degree and arbitrary-angle geometry is out of scope until RF routing demands
it. This simplification makes boolean operations and spatial queries much
simpler and faster.

### Trait-based spatial index

The spatial index is behind a trait so the implementation can change without
touching callers:

```
trait SpatialIndex {
    fn insert(&mut self, shape: ShapeRef);
    fn remove(&mut self, id: ShapeId);
    fn query_point(&self, layer: LayerId, point: (i64, i64)) -> Vec<&ShapeRef>;
    fn query_window(&self, layer: LayerId, rect: &Rect) -> Vec<&ShapeRef>;
    fn query_nearest(&self, layer: LayerId, point: (i64, i64), k: usize) -> Vec<&ShapeRef>;
}
```

Phase 0-2: backed by R-tree. Phase 3+: optionally backed by corner stitching.

---

## 2. Shape types

### Rect

The core shape. Axis-aligned, closed (boundary included), non-degenerate
(x_lo < x_hi, y_lo < y_hi).

```
Rect {
    x_lo: i64,
    y_lo: i64,
    x_hi: i64,
    y_hi: i64,
}
```

Key operations:
- `contains(point) -> bool`
- `overlaps(other: &Rect) -> bool`
- `intersection(other: &Rect) -> Option<Rect>`
- `expand(dx: i64, dy: i64) -> Rect` — dilation (for spacing checks)
- `manhattan_distance(other: &Rect) -> i64` — min distance between closest edges
- `area() -> i64`
- `width() -> i64` / `height() -> i64`

### RectiPoly

A rectilinear polygon — all edges are axis-aligned, vertices are in CCW order.
Used for L-shapes, T-shapes, and min-area patches.

```
RectiPoly {
    vertices: Vec<(i64, i64)>,   // CCW, first vertex not repeated at end
}
```

Key operations:
- `bounding_box() -> Rect`
- `contains(point) -> bool` — ray-casting on the rectilinear grid
- `area() -> i64` — shoelace formula
- `decompose() -> Vec<Rect>` — decompose into non-overlapping rectangles
  (for spatial index insertion and DRC checking)

### ShapeRef

A shape with ownership metadata, stored in the spatial index:

```
ShapeRef {
    id: ShapeId,                  // unique, stable identifier
    layer: LayerId,
    owner: ShapeOwner,            // Net(NetId) | Obstacle | Pin(TerminalId) | Fill | Shield(NetId)
    rect: Rect,                   // bounding box (R-tree envelope)
    geometry: ShapeGeometry,      // Rect(Rect) | Poly(RectiPoly)
}
```

`ShapeId` is a monotonic counter — IDs are never reused, even after removal.
This prevents dangling-reference bugs.

---

## 3. Boolean operations

Boolean operations on rectilinear polygons are needed for:
- Merging overlapping same-net shapes into a single polygon
- Clipping shapes against a region (for local DRC checking)
- Subtracting obstacles from routing regions

### Algorithm: scanline sweep

For rectilinear polygons, boolean operations reduce to a 1D sweep:

1. Collect all y-coordinates from both polygons as events.
2. Sort events. Between consecutive y-values, the cross-section is a union
   of horizontal intervals.
3. Merge/intersect/subtract the interval sets using a sorted-interval algorithm.
4. Reconstruct the result polygon from the interval sequence.

Complexity: O((n + k) log n) where n is the total vertex count and k is the
number of intersection events. For the shapes routing produces (typically
<20 vertices), this is fast.

### Simplification: Rect-only fast path

Most boolean operations in routing involve two rectangles. Special-case this:
- `Rect::intersection(Rect) -> Option<Rect>` — trivial (max of lo, min of hi)
- `Rect::union(Rect) -> Vec<Rect>` — at most 3 rectangles if they overlap
- `Rect::difference(Rect) -> Vec<Rect>` — at most 4 rectangles

Only fall back to the general scanline when RectiPolys are involved.

---

## 4. Spatial index: R-tree

### Why R-tree first

- Proven, well-understood data structure for spatial queries
- The `rstar` crate provides a production-quality implementation
- Supports all needed query types (point, window, nearest-neighbor)
- Insert and remove are O(log n) amortized
- Good cache behavior for the query patterns routing uses

### Layer-per-index

One R-tree per routing layer. This is the right granularity because:
- DRC rules are overwhelmingly intra-layer or adjacent-layer
- Query results don't include cross-layer noise
- Each layer's shape count is manageable (thousands, not millions)
- Via layers are sparse and share an index with their cut layer

### The `GeometryStore`

The top-level container that routes everything through:

```
GeometryStore {
    layers: BTreeMap<LayerId, LayerIndex>,
    next_id: ShapeId,
}

LayerIndex {
    tree: RTree<ShapeRef>,
}
```

Operations:
- `insert(layer, owner, geometry) -> ShapeId`
- `remove(id: ShapeId)`
- `query_point(layer, point) -> impl Iterator<Item = &ShapeRef>`
- `query_window(layer, rect) -> impl Iterator<Item = &ShapeRef>`
- `query_within_distance(layer, rect, distance) -> impl Iterator<Item = &ShapeRef>`
  (equivalent to `query_window(layer, rect.expand(distance, distance))`)
- `shapes_of(owner: ShapeOwner) -> impl Iterator<Item = &ShapeRef>`
  (backed by a secondary BTreeMap<ShapeOwner, Vec<ShapeId>> index)

---

## 5. Corner stitching (Phase 3 option)

### What it gives you over an R-tree

Corner stitching tiles the entire plane with non-overlapping rectangles,
including explicit **space tiles** for empty regions. This makes one query
type nearly free that is expensive on an R-tree:

**"Is there legal empty space at this location?"**

The R-tree can answer "what shapes are here?" (and the absence of results
implies empty space), but it can't answer "what is the largest empty rectangle
containing this point?" without a separate computation. Corner stitching
answers this by returning the space tile directly.

This matters for the access oracle ("find a legal landing pad here") and for
the detailed router ("find a legal jog here").

### What it costs

- Dramatically more complex implementation (~5x the code of an R-tree wrapper)
- Insert/remove requires "stitch surgery" — splitting and merging tiles to
  maintain the maximal-horizontal-strip invariant
- No standard crate — must be implemented from scratch
- Harder to debug (the tile structure is an invariant, not just data)

### Decision

Use the R-tree through Phase 2. If profiling shows that empty-space queries
are a bottleneck in Phase 3 (specifically in the access oracle and detailed
router), implement corner stitching as an alternative `SpatialIndex` backend.
The trait boundary means the switch is local to the index, not a global
rewrite.

Do not implement corner stitching speculatively.

---

## 6. Testing strategy

### Property-based tests

Use `proptest` or `quickcheck` to verify:
- `Rect::intersection` is commutative and associative
- `Rect::union` preserves total area when shapes don't overlap
- `Rect::difference` + the intersection = the original shape
- `RectiPoly::area()` equals the sum of its `decompose()` rectangles' areas
- Spatial index: insert N shapes, then `query_window(bounding_box_of_all)`
  returns all N shapes
- Spatial index: `remove(id)` means `query_point` no longer returns it

### Regression tests

- Known DRC violation geometries: two rectangles at exactly `min_spacing - 1`
  apart, verified that `manhattan_distance` returns the right value
- Pin shapes from SKY130 device models: verify that the bounding box and
  area match expected values
- Boolean operation edge cases: touching rectangles (share an edge but don't
  overlap), zero-area intersection, nested rectangles

### Performance benchmarks

- Insert 100k rectangles into the spatial index, measure time
- Window query on a 10k-shape index with varying window sizes
- Insert + remove cycle (simulate rip-up-and-reroute)

These benchmarks establish the baseline that Phase 3's corner stitching
decision is measured against.
