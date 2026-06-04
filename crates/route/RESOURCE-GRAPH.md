# Subsystem: Resource Graph

> **Shared crate:** Geometry types (`Rect`, `RectiPoly`, `GeometryStore`,
> spatial index queries) come from `crates/geom` (`philis_geom`).
> Technology types (`LayerStack`, `ViaTable`, `GridTable`) come from
> `crates/tech` (`philis_tech`). The resource graph module uses these
> shared types for obstacle integration, track generation, and edge
> attribute computation. The graph construction and representation
> themselves are route-specific.

**Module:** `crates/route/src/graph/`

**Purpose:** Build and maintain the directed resource graph
`G_R = (Q, E_R)` — the routing search space. Every vertex is a legal
point/patch/via-origin on a layer; every edge is a realizable wire segment or
via transition carrying process and extraction attributes.

---

## 1. Why the resource graph is hard

The formulation defines `G_R` declaratively: vertices are legal points, edges
have `attr(e)`. But *constructing* it from a technology model, placement, and
obstacle set is the single largest piece of engineering in the router. The
graph must:

- Represent every legal routing option (tracks, jogs, vias) without
  exploding in size
- Exclude illegal options (blocked by obstacles, violating spacing) without
  being over-conservative (blocking legal routes)
- Support incremental modification (edges become blocked/unblocked as nets
  are committed/ripped up)
- Be compact and cache-friendly enough for A* to run fast

---

## 2. Two-level graph structure

The graph has two levels that serve different purposes:

### Global routing graph (gcells)

A coarse grid that partitions the routing area into rectangular tiles (gcells).
Each gcell edge represents a *channel* between two tiles and carries a capacity
(how many tracks can cross the boundary).

```
GCell
  bbox: Rect                      -- the tile's bounding box
  layer: LayerId
  capacity: u16                   -- number of available tracks through this cell
  demand: u16                     -- current occupancy (nets routed through)

GCellEdge
  from: GCellId
  to: GCellId
  direction: Direction            -- Horizontal | Vertical
  capacity: u16
  demand: u16
```

The global graph is used by the negotiated congestion engine (Phase 2) for
capacity estimation, net ordering, and layer assignment. It is NOT used for
final path selection — that's the detailed graph.

**GCell sizing:** typically 5-10 track pitches per side. Too fine = the global
graph is as large as the detailed graph. Too coarse = congestion estimates
are inaccurate. Default: 10 * max(track_pitch) per layer.

### Detailed routing graph (tracks and vias)

The fine-grained graph where A* and the detailed router actually search.

```
GridVertex
  x: i64                          -- track coordinate
  y: i64                          -- track coordinate
  layer: LayerId

GridEdge
  from: GridVertexId
  to: GridVertexId
  kind: EdgeKind                  -- Wire(direction) | Via(ViaId)
  attr: EdgeAttr
```

```
EdgeAttr
  layer: LayerId
  width_nm: i64                   -- wire width for this edge
  length_nm: i64                  -- 0 for vias
  base_cost: f64                  -- b(e) from the formulation
  resistance_ohm: f64             -- R = length * sheet_R / width
  cap_ff: f64                     -- C = length * area_cap + 2 * length * fringe_cap
  via_def: Option<ViaId>
  is_preferred_direction: bool
```

---

## 3. Grid construction

### Track generation

For each routing layer:
1. Get the pitch, offset, and direction from the tech model.
2. Generate track positions: `pos = offset + i * pitch` for
   `i = 0, 1, 2, ...` within the routing area's bounding box.
3. For horizontal layers, tracks run left-to-right at fixed y positions.
   For vertical layers, tracks run bottom-to-top at fixed x positions.

The track set for a layer defines one axis of the grid. The other axis comes
from the perpendicular layers' tracks.

### Vertex generation

A vertex exists at every intersection of a horizontal track and a vertical
track on the same or adjacent layers. Specifically:

- On layer L (horizontal), a vertex at (x, y) where y is a track on L and
  x is a track on any vertical layer that connects to L via a via.
- On layer L (vertical), a vertex at (x, y) where x is a track on L and
  y is a track on any horizontal layer connected to L.

This produces the minimal graph that includes every legal routing option.

### Edge generation

**Wire edges:** between adjacent grid vertices on the same layer, in the
layer's preferred direction. Each wire edge has length = pitch (the distance
between adjacent tracks in the perpendicular direction).

**Wrong-way edges:** wire edges in the non-preferred direction. These exist
but have a higher base cost (multiplied by a wrong-way penalty factor,
typically 2-3x) to discourage non-preferred routing.

**Via edges:** between the same (x, y) position on adjacent layers, if a
legal via definition exists for that layer pair. The via edge's cost is the
via resistance plus a via penalty (to discourage excessive via use).

### Obstacle integration

After generating the base grid, mark vertices and edges blocked by obstacles:

For each obstacle shape (device body, pre-placed geometry, blockage):
1. Expand the shape by `min_spacing / 2` on each side (the keepout zone).
2. Query: which grid vertices fall inside the expanded shape?
3. Mark those vertices as blocked.
4. For each edge whose wire segment (at the layer's min width) would overlap
   the expanded shape, mark the edge as blocked.

This is conservative: a vertex is blocked if *any* minimum-width wire through
it would violate spacing. Width-dependent blocking (wider wires need more
clearance) is handled in Phase 2 by re-computing edge availability per width
class.

---

## 4. Compact representation

### Implicit grid

For a regular track grid, most of the graph structure is implicit in the
dimensions. A vertex can be identified by a triple `(x_idx, y_idx, layer_idx)`
and its neighbors computed by index arithmetic:

```
vertex_id = x_idx + y_idx * nx + layer_idx * nx * ny

neighbors:
  +1, -1                          -- x-adjacent (same layer)
  +nx, -nx                        -- y-adjacent (same layer)
  +nx*ny, -nx*ny                  -- layer-adjacent (via)
```

This representation uses O(1) per vertex (just a blocked/unblocked bit) and
O(1) per edge (blocked bit + cost, stored in a parallel array indexed the
same way).

For a typical analog design:
- 5 routing layers, 100 tracks per layer, 100 tracks perpendicular
- = 5 * 100 * 100 = 50k vertices
- ~6 edges per vertex = 300k edges
- At 16 bytes per edge (cost + flags): ~5 MB

This fits in L3 cache on most machines, making A* very fast.

### Non-uniform grids

Some designs have non-uniform track patterns (different pitch in different
regions, or half-pitch tracks near pins). The implicit grid doesn't handle
this directly.

Solution: use the uniform grid for the bulk of the design. For non-uniform
regions (pin access zones, dense local areas), add explicit vertices and edges
that connect to the regular grid at boundary points. These are stored in a
supplementary adjacency list.

```
GridGraph
  regular: RegularGrid            -- the implicit grid
  supplements: Vec<(GridVertexId, Vec<(GridVertexId, EdgeAttr)>)>
```

A* checks both the implicit neighbors and the supplement list.

---

## 5. Incremental updates

### Edge blocking/unblocking

When a net is committed, its wire segments become obstacles for other nets:
- Compute the keepout zone around each committed segment
- Mark edges within the keepout as blocked (for different-net routing)
- Same-net edges remain unblocked (a net can cross its own routes)

When a net is ripped up, reverse the process:
- Unblock edges that were blocked solely by this net's segments
- Edges blocked by multiple nets (the ripped-up net + another) stay blocked

This requires tracking *why* each edge is blocked:

```
BlockReason
  obstacles: bool                 -- blocked by fixed obstacles
  net_count: u16                  -- number of committed nets blocking this edge
```

An edge is available iff `!obstacles && net_count == 0` (for a different net)
or `!obstacles` (for same-net re-routing).

### Cost updates

When the negotiated congestion loop updates history costs, only the cost
array changes — the graph topology is stable within a routing iteration.
The cost array is a flat `Vec<f64>` indexed by edge ID, so updates are O(1).

---

## 6. Pin connection to the grid

Each pin (a physical shape on one or more layers) must connect to the grid.
In Phase 1, this is simple: find the nearest on-grid vertex to the pin center
on the pin's layer.

In Phase 3, the access oracle generates multiple candidates per pin, each
connecting to a different grid vertex. The grid builder adds explicit edges
from the pin shape to each candidate vertex, with costs reflecting the access
distance and via stack.

```
PinAccess
  terminal: TerminalId
  layer: LayerId
  point: (i64, i64)              -- the access point (on-grid or near-grid)
  grid_vertex: GridVertexId      -- the nearest grid vertex
  access_edges: Vec<(GridVertexId, EdgeAttr)>  -- edges from pin to grid
```

---

## 7. Global routing

Global routing assigns each net to a set of gcells (a coarse path through the
global graph) before detailed routing commits specific tracks. Its purpose is:

1. **Congestion estimation:** which regions are oversubscribed?
2. **Layer assignment:** which layers should each net use?
3. **Net ordering:** route the most-constrained nets first.

### Algorithm

For each net, find the minimum-cost path through the global graph from source
gcell(s) to target gcell(s). The cost includes:
- Base cost (proportional to distance)
- Congestion cost (proportional to demand/capacity, using the negotiated
  congestion formula from the formulation)
- Layer preference (preferred-direction routing is cheaper)

Global routing runs once before detailed routing and is re-run when the
negotiated congestion loop detects that the global assignment is suboptimal
(too many congested regions).

### When to skip global routing

For small analog designs (<50 nets, uncongested), global routing adds overhead
without benefit. The detailed router can handle the full problem directly. Gate
global routing behind: `if net_count > GLOBAL_ROUTING_THRESHOLD`.

---

## 8. Open questions

### Should the detailed graph be track-based or point-based?

**Track-based:** vertices are track intersections, edges follow tracks. This is
the standard approach and matches the formulation's "grid" model. It works well
for digital and for analog designs with regular track patterns.

**Point-based:** vertices are arbitrary legal points, edges are legal wire
segments between them. This is more flexible (supports off-track routing, custom
widths, non-rectilinear jogs) but produces a much larger graph.

**Recommendation:** start track-based. Add point-based supplements for pin
access and off-grid routing when needed. The supplement mechanism already
accommodates this.

### How do we handle multi-width nets on the same layer?

A wider wire on a track blocks adjacent tracks. Two approaches:
1. **Pre-compute per-width-class graphs:** for each width class, compute which
   edges are available (wider wires have more blockages). Store a blocked-bit
   vector per width class.
2. **Check at query time:** when expanding a node during A*, check whether the
   desired width at that edge would violate spacing. This is slower per query
   but avoids storing multiple copies of the blocked bits.

**Recommendation:** option 1 for Phase 2 (at most 2-3 width classes). If the
number of width classes grows, switch to option 2.
