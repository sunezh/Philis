# Phase 2 — Analog Quality

**Goal:** Transform the Phase 1 greedy maze router into a real analog routing
engine: rip-up-and-reroute with negotiated congestion, differential pair and
group routing, width-aware nets, and incremental in-loop DRC.

**Modules delivered:** `engine/negotiate.rs`, `group/` (diffpair, symmetric,
shield), `verify/drc.rs` (incremental), `state/` (commit/ripup, history,
ownership, union-find)

**Depends on:** Phase 1 (working maze router, grid graph, basic state)

**Unlocks:** Phase 3 (PEX, EM, full certificate, exact engine)

---

## 1. What "analog quality" means

A Phase 2 router can:
- Rip up and reroute nets when they block each other
- Converge using negotiated congestion (PathFinder) with history costs
- Route differential pairs jointly with enforced symmetry
- Route self-symmetric nets with axis-mirrored geometry
- Reserve and route shield tracks for sensitive nets
- Choose wire widths per net class (min width, wide power, etc.)
- Check DRC incrementally on each commit (not just post-route)
- Track connectivity incrementally via union-find
- Report the `HardVector` after every iteration, keeping the best-verified state

This is the quality jump that makes Philis useful for real analog blocks — a
10-50 device circuit with differential pairs, matched paths, power nets, and
sensitive signals.

---

## 2. Negotiated congestion (`engine/negotiate.rs`)

### The PathFinder loop

Replace the Phase 1 "route each net once and hope" with an iterative loop:

```
Iteration 0:  route all nets with h(e) = 0, p(e) = 1 (free sharing)
Iteration i:  rip up all nets, re-route in order with updated costs

cost(e) = (base(e) + history(e)) * present(e)
present(e) = 1 + max(0, occupancy(e) - capacity(e)) * pfac
history(e) += hfac * max(0, occupancy(e) - capacity(e))
```

Convergence: the loop terminates when no edge has `occupancy > capacity` (all
sharing resolved) or a budget fires (`max_iterations`, `max_runtime_ms`).

### History cost semantics

`history(e)` is monotone non-decreasing. An edge contested in iteration `i`
stays expensive in `i+1`. This is the convergence guard — it breaks symmetric
eviction cycles where net A and net B keep swapping the same edge.

The history cost is stored in `state/history.rs`:
```
HistoryCost
  costs: Vec<f64>                 -- one per edge, indexed by EdgeId
```

### Net ordering within an iteration

Priority: `NetClass` first (Critical > Sensitive > DifferentialPair > ... >
Ordinary), then by net-specific urgency (most-constrained first: fewest
legal routing channels, shortest slack). The order is fixed within an
iteration but may be perturbed across iterations (the seed is recorded for
determinism).

### Occupancy and capacity

Each edge has capacity 1 (it can be used by one net). Occupancy is the number
of nets currently using it. In Phase 2 the grid graph is exclusive — no
sharing. The `present(e)` penalty makes shared edges expensive during the free-
sharing iterations, pushing nets to find non-conflicting paths.

For global routing (Phase 3), capacity may be >1 per gcell edge, and the
occupancy model becomes the fractional MCF from the formulation.

### Budget and best-verified retention

The loop tracks the best `HardVector` seen across all iterations. On budget
exhaustion, it returns the routes from the best iteration (the one with the
lexicographically smallest hard vector), not the last one. This is the
"best-verified retention" property from the formulation.

```
BestState
  hard: HardVector
  routes: BTreeMap<NetId, NetRoute>
  iteration: usize
```

---

## 3. Rip-up and reroute (`state/commit.rs`)

### Transactional commit

The route state supports two operations:

**commit(net, path):**
1. Generate physical geometry for the path (wire rects + vias)
2. Insert shapes into the GeometryStore
3. Mark edges as owned by this net in the ownership map
4. Union the path's vertices in the connectivity union-find
5. Run incremental DRC on the new geometry (see Section 5)

**ripup(net):**
1. Remove all shapes owned by this net from the GeometryStore
2. Clear edge ownership for this net
3. Rebuild the connectivity component for affected nets (or mark for re-check)
4. The incremental DRC status for the removed region is invalidated

### Ownership map (`state/ownership.rs`)

```
OwnershipMap
  edge_owner: Vec<Option<NetId>>  -- one per edge
  net_edges: BTreeMap<NetId, Vec<EdgeId>>  -- reverse index
```

This is the core data structure that makes rip-up efficient: to rip up net N,
look up `net_edges[N]` and clear each entry.

### Connectivity invariant (`state/uf.rs`)

The union-find tracks connectivity per net:
```
ConnectivityTracker
  parent: Vec<usize>              -- standard union-find arrays
  rank: Vec<usize>
  net_of: Vec<Option<NetId>>      -- which net owns each vertex
```

On commit: union all vertices in the path with the net's existing component.
On ripup: re-build the component from surviving edges (this is the expensive
part — mitigated by only rebuilding for nets that shared a vertex with the
ripped-up net, which is rare in practice).

The connectivity invariant (Section 2.B of the formulation) is checked after
every commit: all selected access candidates of a net must be in one component,
and that component must touch no other net's component.

---

## 4. Group routing (`group/`)

### Differential pairs (`group/diffpair.rs`)

A differential pair (net_p, net_n) is routed jointly:

1. Determine the symmetry axis from the constraint (`Constraint::Symmetric`)
   and the pin positions.
2. Route net_p using A* (or whatever engine is active).
3. Mirror net_p's path across the symmetry axis to get net_n's path.
4. Verify that the mirrored path is legal (doesn't hit obstacles specific to
   net_n's side).
5. If the mirror fails, route net_n independently but constrain it to use the
   same topology (same layer sequence, same turn directions, mirrored).

**Width and spacing:** both legs use the same width. The inter-pair spacing
(the gap between the P and N legs running in parallel) is a routing parameter,
defaulting to minimum spacing.

**Skew:** after routing, measure the length difference between the two legs.
Report as `RouteQuality.max_diff_pair_skew_nm`. If skew exceeds a threshold
(from the constraint), add serpentine compensation to the shorter leg.

### Self-symmetric nets (`group/symmetric.rs`)

A self-symmetric net's geometry is mirrored about its axis. Route one half,
mirror to get the other, and connect at the axis crossing. The implementation
is similar to diff pairs but for a single net.

### Shield routing (`group/shield.rs`)

For `Constraint::NetShield(net)`:
1. After routing the sensitive net, determine its trunk segments.
2. On each layer where the net has a trunk, reserve the adjacent tracks on
   both sides.
3. Route the shield net (typically ground) on those reserved tracks.
4. If the adjacent tracks are unavailable, report a
   `RouteDiagnostic::ImpossibleConstraint`.

Shield reservation feeds back into the negotiated congestion loop: the reserved
edges have their capacity set to 0 for non-shield nets.

### The group-preserving invariant

When the negotiated congestion loop needs to rip up a net that belongs to a
group (diff pair, matched, symmetric), it must rip up the entire group:

```
scope(V) = closure over grp(.) of seed(V)
```

In practice: if net_p of a diff pair is implicated in a violation, both net_p
and net_n are ripped up and rerouted together. The group closure is computed
from the `NetClass` and the constraint graph.

This is already modeled by `NetClass::is_grouped()` in the existing types.

---

## 5. Incremental DRC (`verify/drc.rs`)

> **Shared crate:** DRC rule predicates, violation types (`DrcViolation`,
> `DrcViolationKind`), and the rule predicate dispatch come from
> `crates/verify` (`philis_verify`). The incremental delta checking loop
> (bounded-influence dilation, spatial query, pairwise re-check) stays in
> the route crate because it is tightly coupled to the route state's
> commit/ripup lifecycle.

### Bounded-influence delta

Instead of re-checking all geometry after every commit (O(N) per commit), Phase
2 checks only the region affected by the change.

When new geometry is committed in region Q:
1. Compute the dilation of Q by `r_max` (the maximum influence radius across
   all active DRC rules).
2. Query the spatial index for all shapes within the dilated region.
3. For each pair of shapes in the result, check the applicable DRC predicates.
4. Report any violations as `RouteDiagnostic::DrcViolation`.

**Which rules are checked:**
Phase 2 handles the routing-critical subset:
- Min width (per layer)
- Min spacing (per layer pair, width-dependent)
- Via enclosure (per via definition)
- Same-net notch/spacing (if the tech model includes it)

Advanced rules (EOL, min-area, PRL-dependent spacing, density) are Phase 3.
Rules not checked are reflected in the certificate as uncoded coverage.

### DRC status tracking

```
DrcStatus
  violations: Vec<DrcViolation>
  last_checked_region: Option<Rect>
```

```
DrcViolation
  rule: RuleId
  layer: LayerId
  shapes: (ShapeRef, ShapeRef)
  region: Rect                    -- bounding box of the violation
```

After each commit, new violations are appended. After a ripup, violations in
the ripped-up region are removed. The `HardVector.drc_violations` is the length
of the current violation list.

---

## 6. Width-aware routing

Phase 2 introduces width classes for nets:

```
WidthClass
  width_nm: i64
  spacing_nm: i64                 -- min spacing when at this width
```

The `NetClass` determines the default width:
- `Ordinary`, `Critical`, `Sensitive`: min width
- `PowerGround`: 2-4x min width (from constraint or default)
- `DifferentialPair`: specified width or min width

When routing a net with a non-minimum width, the graph builder generates edges
with the appropriate width, and the spacing check uses the width-dependent
spacing rule.

In the grid, wider nets effectively "use more tracks" — a 2x-width net on a
1-pitch grid blocks the adjacent track. The graph accounts for this by marking
edges within `(width/2 + spacing)` of the net center as blocked.

---

## 7. Integration with the contract

Phase 2 populates these additional fields honestly:

```
RouteQuality {
  max_diff_pair_skew_nm:     measured from committed diff pair geometry
  max_length_match_delta_nm: measured from matched groups (if any)
  via_count:                 total across all committed routes
  drc_violation_count:       from incremental DRC
  lvs_clean:                 from connectivity invariant
  routed_required_net_ratio: routed / total
  degraded_confidence:       true if PEX/EM not checked (always in Phase 2)
}
```

The certificate records `connectivity_by_invariant: true` (the union-find is
now the primary connectivity check, not a post-route sweep).

---

## 8. Exit criteria

Phase 2 is done when:

1. A 20-device circuit with 2 differential pairs, 1 shielded net, and power
   nets routes to DRC=0 on SKY130 within 10 iterations of the negotiated loop.
2. Differential pair skew is measured and reported correctly.
3. Rip-up-and-reroute resolves cases where Phase 1 would leave nets unrouted.
4. The negotiated congestion loop converges monotonically (hard vector improves
   or stays flat across iterations).
5. Incremental DRC catches the same violations as a full post-route check
   (validated by running both and comparing).
6. Determinism: same seed, same input = identical output.
