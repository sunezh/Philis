# Subsystem: Route State

**Module:** `crates/route/src/state/`

**Purpose:** Manage the mutable routing state — the committed geometry, net
ownership, connectivity tracking, history costs, and transactional
commit/ripup operations. This is the bookkeeping backbone that makes the
rip-up-and-reroute loop correct, efficient, and deterministic.

**Phase:** 1 (basic), 2 (full commit/ripup, ownership, history, union-find)

---

## 1. What the route state is

The route state is the single mutable data structure during routing. It records
everything about the current routing solution:

```
RouteState {
    // The technology and geometry foundation (immutable references)
    tech: &Tech,
    
    // The committed geometry (mutable)
    geom: GeometryStore,
    
    // Per-net routing data
    routes: BTreeMap<NetId, NetRoute>,
    
    // Edge ownership (which net uses which edge)
    ownership: OwnershipMap,
    
    // Connectivity tracking (union-find per net)
    connectivity: ConnectivityTracker,
    
    // Negotiated congestion state
    history: HistoryCosts,
    occupancy: OccupancyMap,
    
    // DRC state
    drc: DrcState,
    
    // Parasitic state (Phase 3)
    parasitics: ParasiticState,
    
    // The best-verified snapshot
    best: Option<BestSnapshot>,
    
    // Metrics
    iteration: usize,
    total_commits: usize,
    total_ripups: usize,
}
```

### NetRoute — a committed net's physical realization

```
NetRoute {
    net_id: NetId,
    edges: Vec<EdgeId>,            // the graph edges this net uses
    segments: Vec<Segment>,        // the physical wire rectangles
    vias: Vec<PlacedVia>,          // placed vias with positions
    shape_ids: Vec<ShapeId>,       // IDs in the GeometryStore
    length_nm: i64,                // total routed wire length
    via_count: usize,              // total vias
}

Segment {
    layer: LayerId,
    rect: Rect,
    width_nm: i64,
    edge: EdgeId,
}

PlacedVia {
    via_def: ViaId,
    position: (i64, i64),
    edge: EdgeId,
}
```

---

## 2. The commit operation

`commit(net, path)` is the central mutation. It takes a path from the search
engine and makes it physical:

### Step 1: Generate geometry

For each edge in the path:
- **Wire edge:** create a `Rect` centered on the track, with width from the
  net's width class, and length = edge length.
- **Via edge:** create a `PlacedVia` at the edge position, then generate the
  via's physical rectangles (cut shape, bottom enclosure, top enclosure).

### Step 2: Insert into GeometryStore

Insert each generated shape into the spatial index with the net as owner.
Record the returned `ShapeId`s in `NetRoute.shape_ids` for later ripup.

### Step 3: Update ownership

For each edge in the path, record `edge -> net` in the ownership map.

### Step 4: Update connectivity

Union all vertices in the path in the connectivity tracker. If the net already
has committed segments (from a previous partial route or a multi-pin
connection), union the new path's vertices with the existing component.

After unioning, verify the connectivity invariant: all pin vertices of this
net are in one component, and no other net's pin vertices are in the same
component (no short).

### Step 5: Update occupancy

For each edge in the path, increment the occupancy counter. If occupancy
exceeds capacity, the edge is oversubscribed (this is legal during the
free-sharing iterations of negotiated congestion, but creates overflow that
drives history cost).

### Step 6: Run incremental DRC

Call the DRC engine to check the new geometry against the dilated
neighborhood. Report any violations.

### Step 7: Update parasitics (Phase 3)

Compute the net's parasitic contribution and update coupling to neighbors.

---

## 3. The ripup operation

`ripup(net)` undoes a commit:

### Step 1: Remove geometry

Look up `NetRoute.shape_ids` and remove each shape from the GeometryStore.

### Step 2: Clear ownership

For each edge in `NetRoute.edges`, clear the ownership entry.

### Step 3: Update connectivity

Remove the net's vertices from the connectivity tracker. This is the expensive
part: the union-find doesn't natively support "split." Two approaches:

1. **Rebuild from scratch:** re-initialize the union-find for all remaining
   nets. O(total_committed_edges). Acceptable for small designs.
2. **Lazy rebuild:** mark the net as "dirty" and rebuild its component the
   next time connectivity is queried. Only rebuild the affected net, not all.

Phase 1: option 1 (small designs, fast enough).
Phase 2: option 2 (amortize the rebuild cost).

### Step 4: Update occupancy

For each edge in `NetRoute.edges`, decrement the occupancy counter.

### Step 5: Invalidate DRC

Remove violations involving this net's shapes. Note: removing a net may
*resolve* violations (the neighbor shapes are no longer too close) but may
also *reveal* new violations (shapes that were "shadowed" by the ripped-up
net). The conservative approach: clear violations involving the net, and
let the next commit's incremental check find any new ones.

### Step 6: Update parasitics (Phase 3)

Remove the net's parasitic contribution. Update coupling for neighbors.

### Step 7: Remove NetRoute

Delete the entry from `routes`.

---

## 4. Ownership tracking (`ownership.rs`)

```
OwnershipMap {
    // Forward: edge -> owner net
    edge_to_net: Vec<Option<NetId>>,     // indexed by EdgeId
    
    // Reverse: net -> edges
    net_to_edges: BTreeMap<NetId, Vec<EdgeId>>,
    
    // Blocking: edge -> how many nets use it (for multi-net overlap detection)
    edge_occupancy: Vec<u16>,            // indexed by EdgeId
}
```

The ownership map is the data structure that makes ripup O(edges_of_net)
instead of O(total_edges). Without the reverse index, ripup would require
scanning all edges to find the ones owned by the net.

### Conflict detection

Two nets on the same edge is a short. The ownership map detects this
immediately on commit:
```
fn commit_edge(edge: EdgeId, net: NetId) -> Result<(), ShortDetected> {
    if let Some(existing) = self.edge_to_net[edge] {
        if existing != net {
            return Err(ShortDetected { edge, nets: (existing, net) });
        }
    }
    self.edge_to_net[edge] = Some(net);
    self.net_to_edges.entry(net).or_default().push(edge);
    self.edge_occupancy[edge] += 1;
    Ok(())
}
```

During the free-sharing iterations of negotiated congestion, shorts are
*expected* (occupancy > capacity). The ownership map records the conflict;
the history cost penalizes the edge; subsequent iterations resolve the sharing.

---

## 5. Connectivity tracking (`uf.rs`)

> **Shared crate:** The union-find connectivity tracker uses
> `philis_verify::lvs::ConnectivityTracker` from `crates/verify`. The
> route state wraps this shared type and extends it with route-specific
> operations (commit/ripup lifecycle, short detection with net labels).

### Union-find with net labels

```
ConnectivityTracker {
    parent: Vec<usize>,
    rank: Vec<usize>,
    net_label: Vec<Option<NetId>>,   // which net each vertex belongs to
}
```

Operations:
- `union(v1, v2)`: standard union-find union with union-by-rank
- `find(v) -> usize`: standard path-compressed find
- `same_component(v1, v2) -> bool`: `find(v1) == find(v2)`
- `set_net(v, net)`: assign a vertex to a net
- `check_invariant(net, pin_vertices) -> bool`: all pin vertices are in the
  same component and no other net's pin vertices are in that component

### Checking for shorts

After committing a net, check: does any vertex in the newly committed path
belong to a component that contains a *different* net's vertices?

```
fn check_no_short(net: NetId, path_vertices: &[VertexId]) -> Option<ShortDiagnostic> {
    let root = find(path_vertices[0]);
    for &v in all_vertices_in_component(root) {
        if let Some(other_net) = net_label[v] {
            if other_net != net {
                return Some(ShortDiagnostic { net, other_net, vertex: v });
            }
        }
    }
    None
}
```

This is expensive if done naively (iterating all vertices in a component).
Optimization: maintain a per-component net set. On union, merge the sets.
Check that each component contains at most one net.

```
ComponentNets {
    nets: BTreeMap<usize, BTreeSet<NetId>>,  // root -> nets in component
}
```

On union: merge the smaller set into the larger. If the merged set contains
>1 net, a short exists.

### The connectivity invariant from the formulation

The route-state invariant (Section 2.B):

> All selected access candidates of net n are in one class, and that class
> touches no other net's class.

This is checked after every commit by verifying:
1. All pin vertices of net n have the same `find()` root.
2. No other net's pin vertices have the same root.

If violated, emit `DiagnosticClass::DisconnectedNet` (open) or
`DiagnosticClass::DifferentNetShort` (short).

---

## 6. History costs (`history.rs`)

```
HistoryCosts {
    costs: Vec<f64>,              // indexed by EdgeId, monotone non-decreasing
}
```

Updated at the end of each negotiated congestion iteration:
```
fn update(edge: EdgeId, overflow: f64, h_fac: f64) {
    self.costs[edge] += h_fac * overflow;
    // Monotone: history never decreases
}
```

Queried during A*:
```
fn cost(edge: EdgeId, base: f64, present: f64) -> f64 {
    (base + self.costs[edge]) * present
}
```

History costs are NOT cleared between iterations — that's the convergence
guard. They ARE cleared at the start of a completely new routing attempt (if
the user re-runs the router with different parameters).

---

## 7. Best-verified snapshot

```
BestSnapshot {
    hard: HardVector,
    routes: BTreeMap<NetId, NetRoute>,
    drc_violations: Vec<DrcViolation>,
    iteration: usize,
}
```

After each iteration, compute the current `HardVector`. If it's
lexicographically better than `best.hard`, snapshot the current state:
```
fn maybe_update_best(&mut self) {
    let current = self.compute_hard_vector();
    if self.best.is_none() || current.lex_le(&self.best.as_ref().unwrap().hard) {
        self.best = Some(BestSnapshot {
            hard: current,
            routes: self.routes.clone(),
            drc_violations: self.drc.violations.clone(),
            iteration: self.iteration,
        });
    }
}
```

On budget exhaustion, the router returns `best`, not the current state. This
guarantees that the returned solution is the best the router found, even if
the last iteration was worse.

### Snapshot cost

Cloning `routes` is O(total_committed_edges). For a 100-net design with ~1000
edges per net: ~100k entries, ~1.6 MB. Acceptable — this happens once per
iteration (tens of times total), not per commit.

---

## 8. Determinism

Every data structure in the route state must support deterministic iteration:

- `routes`: `BTreeMap` (sorted by `NetId`)
- `ownership.net_to_edges`: `BTreeMap` with sorted `Vec<EdgeId>` values
- `drc.violations`: `Vec` sorted by `(rule, layer, shapes)`
- `connectivity`: deterministic by construction (union-find with deterministic
  tie-breaking)
- `history`: `Vec` indexed by `EdgeId` (deterministic by index)

No `HashMap` anywhere in the route state. The iteration order of every
collection is deterministic and reproducible.

If randomized net-order perturbation is used, the seed is recorded in the
certificate. Replaying with the same seed produces the same iteration order
and the same result.

---

## 9. Memory budget

For a typical analog design (50 nets, 5 layers, 100x100 grid):
- Grid graph: ~50k vertices, ~300k edges, ~5 MB
- GeometryStore: ~5k shapes, ~200 KB (R-tree overhead)
- Ownership: ~300k entries, ~2.4 MB
- Connectivity: ~50k entries, ~400 KB
- History: ~300k floats, ~2.4 MB
- Routes: ~50 nets * 100 edges * 80 bytes = ~400 KB
- Best snapshot: ~400 KB

Total: ~11 MB. Well within any modern system's memory.

For larger designs (500 nets, 10 layers, 500x500 grid), the grid graph
dominates at ~250M edges * 16 bytes = ~4 GB. At this scale, a sparse or
compressed graph representation becomes necessary. This is not Philis's
primary target, but the architecture should not preclude it.
