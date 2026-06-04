# Phase 1 — Minimum Viable Router

**Goal:** Route nets on a gridded graph with A*, produce real geometry, populate
the `RoutingResult` contract with non-stub values.

**Modules delivered:** `graph/`, `engine/maze.rs`, `state/` (basic), `verify/`
(basic), `certificate/` (basic)

**Depends on:** Phase 0 (`tech/`, `geom/`)

**Unlocks:** Phase 2 (rip-up, negotiated congestion, group routing)

---

## 1. What "minimum viable" means

A Phase 1 router can:
- Accept a placement (device positions + pin locations) and a technology model
- Build a 3D routing graph
- Route each net one at a time using A*
- Check basic DRC (spacing, width) on the result
- Emit GDS geometry for routed segments
- Populate `RoutingResult` with real `routed_segments`, `drc_violation_count`,
  `lvs_clean`, and a certificate with `degraded_confidence = false` when
  DRC=0 and all nets routed

A Phase 1 router cannot:
- Handle congestion (if net A blocks net B, net B fails — no rip-up)
- Route differential pairs or matched groups
- Do in-loop PEX or EM checking
- Handle advanced DRC (EOL, min-area, density)
- Optimize net ordering

This is useful for: validating the entire pipeline end-to-end, testing on small
circuits (a few devices, <20 nets), and proving the architecture works before
investing in the hard stuff.

---

## 2. Resource graph (`graph/`)

### The 3D grid

The routing graph is a 3D grid: (x, y, layer). Vertices are grid crossings
where a wire segment or via can legally exist. Edges are wire segments (same
layer) and via transitions (adjacent layers).

```
RoutingGrid
  tech: &Tech
  x_coords: Vec<i64>             -- sorted unique x coordinates (track positions)
  y_coords: Vec<i64>             -- sorted unique y coordinates
  layers: Vec<LayerId>           -- routable layers in stack order
  obstacles: GeometryStore       -- pre-placed shapes that block routing
```

**Grid construction:**
1. For each routing layer, generate track positions from the layer's pitch and
   offset: `x = offset + i * pitch` for vertical layers, similarly for
   horizontal.
2. Collect all unique x and y coordinates across all layers — this is the
   Hanan-grid-like supergrid.
3. For each (x, y, layer), check whether a wire segment of minimum width
   centered at that point is free of obstacles. If not, the vertex is blocked.
4. Edges between adjacent grid points on the same layer are wire segments.
   Edges between the same (x, y) on adjacent layers are via transitions (if a
   legal via exists there).

**Edge attributes:**
```
EdgeAttr
  kind: EdgeKind                  -- Wire | Via
  layer: LayerId
  length_nm: i64                  -- 0 for vias
  width_nm: i64                   -- minimum width for this layer
  via_def: Option<ViaId>          -- which via, for Via edges
  base_cost: f64                  -- length * sheet_R + via_R, or similar
```

The `base_cost` is the `b(e)` from the formulation's `cost(e) = (b(e) + h(e)) * p(e)`.
In Phase 1, `h(e) = 0` and `p(e) = 1` — no negotiation yet.

### Obstacle integration

Before routing, all fixed shapes (device bodies, pin shapes, blockages,
pre-routed geometry) are inserted into the `GeometryStore`. The grid builder
queries the spatial index to mark blocked vertices and edges.

For each edge, the grid builder checks: "if I place a wire of width `w` along
this edge, does it violate minimum spacing to any obstacle?" This is a window
query on the spatial index, dilated by `min_spacing + w/2`.

### Graph representation

The grid graph is stored as a compressed structure, not an adjacency list of
all possible edges (which would be huge):

```
GridGraph
  dims: (usize, usize, usize)    -- (nx, ny, nlayers)
  blocked: BitVec                 -- one bit per vertex
  edge_blocked: BitVec            -- one bit per edge (6 directions per vertex)
  edge_attrs: Vec<EdgeAttr>       -- only for non-blocked edges
```

Vertex indexing: `v = x_idx + y_idx * nx + layer_idx * nx * ny`. Neighbor
enumeration is arithmetic (add/subtract 1, nx, or nx*ny), with bounds checks
and blocked-bit filtering.

This is compact and cache-friendly — critical for A* performance.

---

## 3. Maze router (`engine/maze.rs`)

### Single-net A*

The core algorithm is textbook A* on the grid graph:

```
Input:  source vertices (pin access points), target vertices, grid graph
Output: path (list of edges) or failure

Open set:  binary heap, keyed by f(v) = g(v) + h(v)
Closed set: visited array (one bit per vertex)
g(v): cost from source to v through committed edges
h(v): heuristic estimate of cost from v to nearest target
```

**Heuristic:** Manhattan distance on the grid, scaled by the minimum per-layer
cost. This is admissible (never overestimates) because the cheapest path between
two points is at least the Manhattan distance times the cheapest edge cost.

For Phase 1, this is sufficient. ALT landmarks (Section 7 of
Router-Algorithms.html) are a Phase 3 optimization.

**Multi-pin nets:** For nets with >2 pins, route as a Steiner tree using
iterative 2-pin connections. Route pin 1 to pin 2, then route pin 3 to the
nearest point on the existing tree, etc. Order pins by distance to minimize
total wirelength. This is a simple heuristic — not optimal, but correct and
easy to implement.

**Source/target handling:** Each pin maps to one or more grid vertices (the
closest on-grid points to the pin center, on the pin's layer). In Phase 1,
this is a simple nearest-grid-point lookup. The full access oracle (Phase 3)
replaces this with ranked, DRC-checked candidates.

### Net ordering

Phase 1 uses a simple static ordering: route shorter nets first (by Manhattan
distance between pins). This is a common heuristic that works reasonably well
for uncongested designs.

The formulation's net-class priority (Critical before Ordinary, etc.) is added
in Phase 2 when net classes are wired up.

### Failure handling

When A* fails to find a path (the target is unreachable due to obstacles or
previously-routed nets), Phase 1 reports the net as unrouted:
- `HardVector.unrouted_required_nets += 1`
- A `RouteDiagnostic` with `DiagnosticClass::DisconnectedNet` and
  `DiagnosticScope::LocalRepairable` is emitted

No rip-up in Phase 1. The user sees which nets failed and why.

---

## 4. Route state (`state/`)

### Committed geometry

When a net's path is found, the state records it:

```
NetRoute
  net_id: NetId
  edges: Vec<EdgeId>              -- the edges this net uses
  segments: Vec<(LayerId, Rect)>  -- the physical wire rectangles
  vias: Vec<(ViaId, (i64, i64))> -- placed vias with positions
```

```
RouteState
  tech: &Tech
  geom: GeometryStore             -- all shapes (obstacles + routed)
  routes: BTreeMap<NetId, NetRoute>
  blocked_edges: BitVec           -- edges used by committed nets
```

When net N is committed:
1. For each edge in N's path, generate the physical rectangle (centered on the
   track, width = layer min width, length = edge length).
2. Insert each rectangle into the `GeometryStore` with `ShapeOwner::Net(N)`.
3. Mark each edge as blocked in `blocked_edges` so subsequent nets can't use it.
4. For each rectangle, check spacing to neighbors (window query dilated by
   min_spacing) and mark additional edges as blocked if they would cause a DRC
   violation.

### Blockage propagation

After committing a net, the state must update which edges are now blocked for
future nets. This is the "design rule aware" part of Phase 1:

For each committed wire segment on layer L:
- Query the spatial index for all shapes within `min_spacing` distance
- For each grid edge that would place a wire within `min_spacing` of the
  committed segment, mark it blocked

This is conservative (it may over-block) but safe — it guarantees that any net
routed after the update will not violate spacing to already-routed nets.

---

## 5. Basic verification (using `crates/verify`)

> **Shared crate:** DRC rule predicates and violation types come from
> `crates/verify` (`philis_verify`). The post-route DRC check uses
> `philis_verify` predicates for spacing and width checking, and
> `philis_verify::DrcViolation` for violation reporting. The connectivity
> check uses `philis_verify::lvs::ConnectivityTracker` (union-find).

Phase 1 implements two checks:

### Post-route DRC

After all nets are routed, sweep all committed geometry and check using
`philis_verify` predicates:
- **Min width:** every wire segment is at least `layer.min_width_nm` wide
- **Min spacing:** every pair of shapes on the same layer with different owners
  is at least `min_spacing` apart

Implementation: for each shape, do a window query (dilated by min_spacing) on
the same layer's spatial index (from `philis_geom`). Any returned shape with
a different owner that overlaps the dilated region is a spacing violation.

Report as `HardVector.drc_violations`.

### Connectivity check

After all nets are routed, verify that each net's pins are connected through
routed geometry using `philis_verify::lvs::ConnectivityTracker`:
- Build a union-find over all grid vertices
- For each committed edge, union its two endpoints
- For each net, check that all its pin vertices are in the same component

Any net with pins in different components is a `DiagnosticClass::DisconnectedNet`.
Report as `HardVector.lvs_mismatches` (an open is a connectivity mismatch).

---

## 6. GDS output

Phase 1 writes routed geometry to GDS via the existing `io::gds` module:

For each committed `NetRoute`:
- Each wire segment becomes a `BOUNDARY` rectangle on the appropriate GDS layer
- Each via becomes one or more `BOUNDARY` rectangles (the cut, bottom
  enclosure, top enclosure)

The GDS layer mapping comes from the technology model (each `Layer` has a
GDS layer/datatype pair).

---

## 7. Certificate population

Phase 1 fills the `RouteCertificate` with real values:

```
RouteCertificate {
  problem_hash:     hash of (tech, placement, netlist, obstacles)
  pdk_oracle_hash:  hash of the compiled Tech
  pdk_state:        from the tech compiler
  hard: HardVector {
    invalid_pdk:                  tech compiler's verdict
    missing_access:               0 (Phase 1 uses simple grid-snap, not the oracle)
    drc_violations:               from post-route DRC
    lvs_mismatches:               from connectivity check
    unrouted_required_nets:       count of A* failures
    unresolved_em_antenna_hard_errors: 0 (not checked in Phase 1)
  }
  coverage:         from the tech compiler's classification
  drc_evidence:     Evidence::InternalProof (the router did the check)
  lvs_evidence:     Evidence::InternalProof (connectivity check)
  connectivity_by_invariant: false (Phase 1 does post-route check, not incremental)
  degraded_confidence:
    false if hard.is_feasible() AND pdk_state is FullSignoff or AbstractComplete
    true otherwise
  ...
}
```

The certificate is honest: if the DRC check found violations, they're reported.
If the PDK is sparse, the confidence is degraded. Phase 1 never claims more
than it can prove.

---

## 8. The `route()` entry point

```
pub fn route(problem: &RouteProblem) -> RoutingResult

RouteProblem
  tech: &Tech
  netlist: netlist data (devices, nets, pins)
  placement: device positions and pin geometry
  obstacles: pre-existing geometry
  config: RouteConfig (max_layers, net_order_seed, ...)
```

The flow:
1. Build the grid graph from tech + placement + obstacles
2. Determine net ordering (shortest first)
3. For each net, run A* to find a path
4. Commit the path to the route state (update geometry + blockages)
5. After all nets, run post-route DRC and connectivity check
6. Assemble the RoutingResult (geometry summary + quality + certificate + diagnostics)

---

## 9. Exit criteria

Phase 1 is done when:

1. A 5-device, 10-net test circuit on SKY130 routes successfully with DRC=0
   and all nets connected.
2. The GDS output opens in KLayout and shows correct wire geometry.
3. The `RouteCertificate` fields are populated with real (not stub) values.
4. A deliberately impossible routing (too many nets, too narrow channel)
   returns `unrouted_required_nets > 0` with typed diagnostics, not a crash.
5. The router is deterministic: same input always produces same output (same
   net ordering, same paths, same certificate hash).

---

## 10. What Phase 1 explicitly does NOT do

- Rip-up and reroute (Phase 2)
- Differential pair or any group routing (Phase 2)
- Width-class selection — always routes at minimum width (Phase 2)
- In-loop DRC — checks only after all nets are committed (Phase 2)
- Pin access oracle — uses simple grid-snap (Phase 3)
- PEX, EM, IR, antenna (Phase 3)
- Negotiated congestion (Phase 2)
- ALT heuristic (Phase 3)
- Multi-width nets / power routing (Phase 2)

These are all tracked in the certificate as `degraded_confidence` causes.
