# Subsystem: Search Engines

**Module:** `crates/route/src/engine/`

**Purpose:** The three cooperating search engines that find routes on the
resource graph: maze A* (engine 3, the workhorse), negotiated congestion
(engine 2, the convergence backbone), and exact-local (engine 1, the
precision tool for decision-dense clips).

---

## 1. Engine hierarchy and dispatch

The three engines are not alternatives — they cooperate:

```
Engine 1 (exact-local): solves small, decision-dense subproblems exactly
  - Pin access selection
  - Via/color/min-area legality in tight clips
  - Differential pair escape from dense pin regions

Engine 2 (negotiated congestion): drives the iterative rip-up-and-reroute loop
  - Controls history/present cost
  - Determines net ordering and iteration budget
  - Calls Engine 3 to route individual nets within each iteration

Engine 3 (maze A*): routes a single net on the detailed graph
  - Called by Engine 2 for each net in each iteration
  - Called directly in Phase 1 (no Engine 2 yet)
  - Uses Engine 1 results as pin access constraints
```

The dispatcher selects which engine handles what:

```
fn dispatch(problem) -> EngineAssignment:
  for each net:
    if net has a decision-dense clip (grouped escape, color, dense access):
      assign clip to Engine 1
    assign net's path search to Engine 3
  if net_count > 1 and congestion expected:
    wrap in Engine 2 iteration loop
```

---

## 2. Engine 3: Maze A* (`maze.rs`)

### Algorithm

Standard A* on the detailed routing graph:

```
fn route_net(
    graph: &GridGraph,
    sources: &[GridVertexId],      // access points for source pins
    targets: &[GridVertexId],      // access points for target pins
    costs: &CostMap,               // b(e) + h(e) for negotiation
    heuristic: &dyn Heuristic,     // Manhattan or ALT
) -> Option<Path>

Path = Vec<GridEdgeId>
```

**Open set:** binary min-heap keyed by `f(v) = g(v) + h(v)`.

**Closed set:** one bit per vertex (visited/unvisited). Reset between net
routings by keeping a "generation" counter and per-vertex generation stamp
(avoids O(|V|) clear).

**g(v):** accumulated cost from source to v through committed edges:
```
g(neighbor) = g(current) + cost(edge)
cost(edge) = (base(edge) + history(edge)) * present(edge)   // from Engine 2
```

In Phase 1, `cost(edge) = base(edge)`.

**h(v):** estimated remaining cost. Two implementations:

1. **Manhattan heuristic (Phase 1):**
   ```
   h(v) = manhattan_distance(v, nearest_target) * min_edge_cost
   ```
   Admissible, fast to compute, loose on non-Euclidean graphs.

2. **ALT heuristic (Phase 3):**
   ```
   h(v) = max over landmarks L of |dist(v, L) - dist(target, L)|
   ```
   Admissible, tight on non-Euclidean graphs, requires landmark precomputation.

### Multi-pin routing

For nets with >2 pins, route as a Steiner tree:

1. Start with the two most-distant pins. Route them with A*.
2. Add the routed path to the "tree" — all vertices on the path become
   additional targets (with cost 0 to reach).
3. Route the next unconnected pin to the nearest point on the tree.
4. Repeat until all pins are connected.

Pin ordering: by distance from the tree's centroid (outermost first). This
tends to produce shorter Steiner trees than random or nearest-first ordering.

### Bend penalties

Analog routing prefers fewer bends (each bend introduces parasitic
discontinuities). Add a bend penalty to the cost function:

```
bend_cost = penalty if the edge changes direction from the predecessor
```

This discourages zigzag paths and produces straighter wires.

### Wire width during search

In Phase 1, all nets route at minimum width. In Phase 2+, the net's width
class determines which edges are available (wider nets block more). The
graph provides per-width-class edge availability.

During A*, the width is fixed for the entire net (determined by its `NetClass`
and any explicit constraint). Width transitions within a net (e.g., tapering
from wide trunk to narrow branch) are a Phase 3 feature.

---

## 3. Engine 2: Negotiated Congestion (`negotiate.rs`)

### The PathFinder loop

```
fn negotiate(
    graph: &mut GridGraph,
    nets: &[NetDescriptor],
    state: &mut RouteState,
    config: &NegotiateConfig,
) -> NegotiateResult

NegotiateConfig {
    max_iterations: usize,         // hard budget
    max_runtime: Duration,         // hard budget
    p_fac_initial: f64,            // present-sharing penalty factor
    p_fac_growth: f64,             // multiplier per iteration
    h_fac: f64,                    // history accumulation factor
    convergence_threshold: f64,    // stop when overflow < this
}
```

**Iteration structure:**

```
for iteration in 0..max_iterations:
    // Rip up all nets (iteration > 0)
    if iteration > 0:
        for net in nets:
            state.ripup(net)

    // Sort nets by priority (net class, then urgency)
    ordered = sort_nets(nets, state)

    // Route each net
    for net in ordered:
        path = engine3.route_net(graph, net.sources, net.targets, cost_map)
        if path is Some:
            state.commit(net, path)
            run_incremental_drc(state, path)
        else:
            record_failure(net)

    // Update costs
    for edge in graph.edges():
        overflow = max(0, state.occupancy(edge) - graph.capacity(edge))
        history[edge] += h_fac * overflow
        present[edge] = 1.0 + overflow as f64 * p_fac

    // Check convergence
    if total_overflow == 0:
        break
    
    // Track best state
    current_hard = compute_hard_vector(state)
    if current_hard.lex_le(&best_hard):
        best_hard = current_hard
        best_state = snapshot(state)

    // Increase pressure
    p_fac *= p_fac_growth

return best_state
```

### Net ordering

Within each iteration, nets are sorted by:
1. **Net class priority:** Critical > Sensitive > DifferentialPair > 
   ExactMatch > MatchedArray > SelfSymmetric > PowerGround > RfMmwave > 
   HighVoltage > GuardSubstrate > Ordinary
2. **Urgency:** nets that failed in the previous iteration are routed first
   (they're the most constrained)
3. **Bounding box:** smaller bounding box first (shorter nets are easier to
   reroute, so they should be committed last — giving longer nets priority)
4. **Tie-breaking:** by net ID (deterministic)

### Group routing integration

When a grouped net is encountered in the ordering, the entire group is routed
together:
1. All nets in the group are in `scope` — they're all ripped up.
2. The group router (see [GROUP-ROUTER.md](GROUP-ROUTER.md)) routes them
   jointly.
3. All are committed together.
4. The next net in the ordering is the first non-group net.

### Convergence monitoring

Track per iteration:
- Total overflow: `sum of max(0, occ(e) - cap(e))` over all edges
- Hard vector: `(drc_violations, unrouted_nets, ...)`
- Overflow trend: is total overflow decreasing?

If total overflow has not decreased for `STALL_ITERATIONS` (default 3)
consecutive iterations, and `p_fac` is already high, the loop is stalled.
Options:
1. Increase `h_fac` (make history stickier)
2. Perturb net ordering (random shuffle with recorded seed)
3. Give up and return best-verified state

### Budget enforcement

The loop always terminates:
- `max_iterations` caps the iteration count
- `max_runtime` caps wall-clock time (checked after each net routing)
- Memory budget: if the route state exceeds a threshold, stop and return

On budget exhaustion, the result is the best-verified state (not the last
state, which may be worse). The certificate records `DiagnosticClass::Timeout`
for any remaining violations.

---

## 4. Engine 1: Exact Local (`exact.rs`)

> **Shared crate:** The exact-local engine's SAT and ILP calls go through
> `crates/solver` (`philis_solver`). SAT feasibility checks use
> `philis_solver::SatSolver`, ILP optimization uses
> `philis_solver::IlpSolver`, and convenience builders (at-most-one,
> at-least-one clauses) are provided by `philis_solver`.

### Scope

Engine 1 solves small subproblems exactly. It is NOT a full-chip router — it
handles clips of 10-100 variables that Engine 2/3 cannot prove.

### Subproblem types

**Pin access selection:** (see [ACCESS-ORACLE.md](ACCESS-ORACLE.md))
Select one access candidate per terminal with no conflicts. Modeled as SAT.

**Via legality:** given a path on the detailed graph, assign specific via
definitions to each layer transition such that all enclosure, cut-spacing,
and stacking rules are satisfied. If multiple via types are legal, choose the
one with lowest resistance.

**Color/mask assignment:** for multipatterning layers, assign colors to
features in a clip such that the conflict graph is 2-colorable (or 3-colorable
for triple patterning). Modeled as graph coloring / SAT.

**Min-area patching:** after routing, check each shape for minimum area.
If a shape is too small, enumerate legal patch rectangles that bring it above
the threshold without causing new DRC violations. Select the minimum-cost set.

**Grouped escape:** for a differential pair or common-centroid array exiting a
dense pin region, enumerate the legal escape topologies (which sides of which
obstacles do the wires pass?) and select the one that satisfies group
constraints (symmetry, equal length).

### Solver architecture

All solver calls go through `philis_solver` trait abstractions:

1. **Exhaustive enumeration** for very small clips (<20 variables).
   Generate all legal assignments, evaluate each, return the best.

2. **SAT via `philis_solver::SatSolver`** for medium clips (20-100
   variables) and pure feasibility problems. The `philis_solver` crate
   provides convenience builders for at-most-one and at-least-one
   constraints, which are the common patterns in access selection and
   color assignment.

3. **ILP via `philis_solver::IlpSolver`** when objective optimization
   (not just feasibility) is needed, such as minimum-cost via assignment.

MUS/MCS extraction for infeasible core reporting is also available
through `philis_solver`.

### Integration with Engine 2/3

Engine 1 runs *before* Engine 3 for pin access and *after* Engine 3 for via
legality and color assignment:

```
Before each net's A*:
  Engine 1 -> access candidates (selected from oracle)

A* finds path:
  Engine 3 -> path on detailed graph

After path is found:
  Engine 1 -> via assignment along path
  Engine 1 -> color assignment for multipatterning layers
  Engine 1 -> min-area patch synthesis
```

If Engine 1 fails on a post-route problem (no legal via assignment for the
chosen path), the path is rejected and Engine 3 re-routes with the
problematic vias excluded.

---

## 5. Heuristic: ALT landmarks (`alt.rs`)

### Landmark precomputation

Select landmark vertices:
1. 4 at the graph corners (extreme x/y positions on the middle routing layer)
2. 2-4 at congested regions (identified by high history cost from Engine 2)

For each landmark L, run Dijkstra's algorithm from L to compute `dist(v, L)`
for all vertices v. Store as a flat `Vec<f64>` indexed by vertex ID — one
array per landmark.

Total storage: `landmarks * vertices * 8 bytes`. For 8 landmarks and 50k
vertices: 3.2 MB.

### Query

```
fn alt_heuristic(v: GridVertexId, target: GridVertexId) -> f64 {
    landmarks.iter()
        .map(|l| (dist[v][l] - dist[target][l]).abs())
        .max()
}
```

This is O(landmarks) per node expansion — ~8 subtractions and comparisons.
Negligible compared to the heap operations in A*.

### Recomputation

Landmarks are recomputed when:
- The graph structure changes (new permanent obstacles, not route commits)
- The congested regions shift significantly between negotiation iterations

They are NOT recomputed on every rip-up/reroute — the topology is stable,
only costs change (and ALT uses topology-based distances, not cost-based).

### When to skip ALT

If A* without ALT is fast enough (small designs, <50 nets), skip the landmark
precomputation. The overhead of 8 Dijkstra runs (8 * O(|E| log |V|)) is
wasted on a design where each A* query takes <1ms.

Gate: `if net_count * avg_bounding_box_area > ALT_THRESHOLD`.

---

## 6. Performance considerations

### A* memory

The closed set (visited bits) and g-cost array are the dominant memory users.
For a 50k-vertex graph: 50k bits (6 KB) + 50k * 8 bytes (400 KB) = ~406 KB.
Well within L2 cache.

For larger designs (1M vertices), the g-cost array grows to 8 MB — still
manageable, but A* becomes memory-bound. Profile before optimizing.

### Heap implementation

The standard binary heap from `std::collections::BinaryHeap` is fine for
Phase 1-2. If profiling shows heap operations are the bottleneck (unlikely
for analog), consider a bucket queue (constant-time insert for integer costs)
or a 4-ary heap (better cache behavior).

### Parallelism potential

Within a negotiated congestion iteration, independent nets (no shared edges,
no group relationship) can be routed in parallel. The route state needs
interior mutability (e.g., per-region locks or a concurrent spatial index) to
support this.

This is a Phase 3+ optimization. For Phase 1-2, route nets sequentially.
