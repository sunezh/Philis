# Subsystem: Access Oracle

**Module:** `crates/route/src/access/`

**Purpose:** Generate, rank, and select legal terminal access candidates before
routing begins. Terminal access is a first-class subproblem — a path search
cannot repair a missing legal access point.

**Phase:** 3 (Phase 1-2 use simple grid-snap as a fallback)

---

## 1. Why access is a separate subproblem

The formulation (Section 6) explains the core issue: a pin is a *shape on one
or more layers*, not a point. The naive approach — "connect to the nearest grid
point" — fails when:

- The nearest grid point has a via enclosure violation
- Two adjacent pins compete for the same track (the net-ordering pathology)
- A differential pair needs mirrored access on symmetric tracks
- A current-carrying net needs a via array, not a single stack

Phase 1-2's grid-snap fallback works for simple cases. The access oracle
replaces it with a principled selection that eliminates these failure modes.

---

## 2. Candidate generation (`candidate.rs`)

For each terminal (a schematic pin bound to a physical shape at a location):

### Step 1: Get the pin shape

Four sources, in decreasing confidence:

1. **ExactPin:** the PDK or imported cell provides exact pin geometry (layer,
   shape, purpose). This is the gold standard.
2. **GeneratedPin:** a device generator (parameterized cell) computes pin
   geometry from device parameters (W, L, nf). Confidence depends on the
   generator's fidelity.
3. **InferredPin:** partial placement data provides the layer and approximate
   region but not exact geometry. Pin shape is estimated from device type and
   orientation.
4. **HeuristicPin:** no pin geometry at all — the access point is a heuristic
   anchor at the device center on a guessed layer.

Record the `AccessConfidence` level. Any net routed against a non-ExactPin
access is marked in the certificate's `heuristic_access_nets`.

### Step 2: Generate on-track candidates

For each routing layer that overlaps the pin shape's layer stack:
1. Compute track positions on this layer that cross the pin shape's bounding box.
2. For each track-pin intersection, compute the via stack needed to connect
   from the pin's native layer to the track layer.
3. Check that the via landing pad (cut + enclosures) fits entirely within
   the pin shape (for ExactPin) or the estimated pin region.
4. Each valid intersection is a candidate: `(layer, point, via_stack)`.

On-track candidates are preferred because they connect directly to the routing
grid without jogs.

### Step 3: Generate off-track candidates

If on-track candidates are insufficient (the pin is between tracks, or all
on-track candidates have DRC violations):
1. Try half-pitch offsets (midpoints between tracks).
2. Try via-in-pin: place the via inside the pin shape even if off-track,
   and add a short jog to reach the nearest track.
3. These have higher cost and lower ranking.

### Step 4: Local DRC check per candidate

For each candidate, run the tech model's rule predicates:
- Via enclosure: does the landing pad meet minimum enclosure on both layers?
- Cut spacing: is the cut far enough from other cuts (on the same or adjacent
  terminals)?
- Min-area: does the landing pad meet minimum area rules?
- Spacing to obstacles: is the access wire segment far enough from nearby
  fixed shapes?

Candidates that fail any predicate are rejected. The specific failed predicates
are recorded (for diagnostics if all candidates fail).

### Step 5: Attach costs and rank

Each surviving candidate gets:
- **Parasitic access cost:** estimated R and C from the pin center to the
  candidate point (via resistance + short wire resistance).
- **DRC margin:** how much clearance to the nearest potential violation
  (higher margin = more robust).
- **Symmetry class:** for grouped nets, which symmetry equivalence class
  the candidate belongs to.

**Ranking** (lexicographic, from the formulation):
1. Exact legal pin access > heuristic access
2. No-via or fewer-via access > stacked-via access
3. Higher DRC margin > lower margin
4. Lower parasitic access cost > higher
5. Symmetric counterpart available > isolated

---

## 3. Conflict graph (`conflict.rs`)

Build an undirected graph where:
- Vertices = access candidates (across all terminals)
- Edges = conflict pairs (two candidates that cannot coexist)

Two candidates A and B conflict if:
- They are on the same layer and their access wire segments (at minimum width)
  would violate minimum spacing
- Their via landing pads overlap or violate cut spacing
- They compete for the same track segment (occupancy > 1)

The conflict graph is precomputed once (after candidate generation) and does
not change during access selection.

### Efficient construction

Naive: check every pair of candidates — O(n^2). For analog designs (typically
<1000 candidates total), this is fast enough.

Optimization for larger designs: use the spatial index. For each candidate's
access region (dilated by min_spacing), query for other candidates' regions on
the same layer. Only check pairs that overlap the dilated region.

---

## 4. Compatible subset selection (`select.rs`)

> **Shared crate:** SAT solving for compatible subset selection uses
> `crates/solver` (`philis_solver`). The `philis_solver::SatSolver` trait
> handles the feasibility check, and convenience builders
> (`philis_solver::at_most_one`, `philis_solver::at_least_one`) encode the
> per-terminal selection constraints. MUS extraction for infeasible core
> reporting also uses `philis_solver`.

### The problem

Select exactly one candidate per required terminal such that no two selected
candidates are connected by a conflict edge.

This is the maximum weight independent set problem restricted to a structured
graph (one vertex per terminal must be selected). It's NP-hard in general but
tractable for the sizes and structures seen in analog routing.

### Algorithm: incremental SAT / constraint propagation

Model as a SAT problem:
- For each terminal t with candidates c1, c2, ..., ck:
  - At-least-one: `c1 OR c2 OR ... OR ck`
  - At-most-one: `NOT(ci AND cj)` for all i != j (pairwise exclusion)
- For each conflict edge (ca, cb):
  - `NOT(ca AND cb)`

Solve with a SAT solver. The at-most-one constraints are encoded using the
sequential counter encoding (O(k) clauses per terminal) or the ladder
encoding for small k.

For small instances (<100 terminals), a custom backtracking search with
constraint propagation (arc consistency on the candidate domains) may be
faster than a general SAT solver.

### Failure handling

If no compatible subset exists, the solver returns UNSAT. Extract the
unsatisfiable core: the minimal set of terminals and conflict edges that make
selection impossible.

This becomes:
- `HardVector.missing_access += terminals_in_core.len()`
- A `RouteDiagnostic` with `DiagnosticClass::NoLegalAccess` listing the
  conflicting terminals and the violated predicates

The router does not attempt to route nets whose terminals have no legal access.
The diagnostic tells the user *why* and *which terminals* — not a bare
"routing failed."

### Grouped net extensions

**Differential pair:** candidates are generated in symmetric pairs
`(cp, cn)` where cp is on net_p and cn is its mirror on net_n. The
selection constraint requires: `cp is selected IFF cn is selected`. This
is modeled as an equivalence clause in the SAT formulation.

**Matched array:** candidates respect the array's centroid structure. The
selection constraint ensures that the chosen access points for all elements
of the array are geometrically balanced (equal distances from centroid).

**Multiport terminals:** a single terminal can select multiple candidates
(for via arrays on current-carrying nets). The at-most-one constraint is
relaxed to at-most-k, where k is the multiport count.

---

## 5. Access in the routing loop

### Before route search

The selected access candidates become the source and target vertices for A*.
Each net's endpoints are no longer "nearest grid point to pin center" but
"the specific candidate selected by the oracle."

### During rip-up

When a net is ripped up, its access selection is *not* discarded by default.
The access candidates are stable across rip-up/reroute iterations — they
depend on pin geometry and obstacle proximity, not on other nets' routes.

Exception: if the route fails because the selected access point is unreachable
(all paths blocked by other nets), the oracle re-selects from the remaining
candidates for that terminal. If no unblocked candidate exists, the net is
diagnosed as `NoLegalAccess` and the blocking nets are identified.

### Incremental re-selection

When the obstacle set changes (new committed routes near a pin), some
candidates may become blocked. Rather than re-running the full conflict graph
and SAT solver, incrementally:
1. Check which candidates are newly blocked by the committed geometry.
2. If the selected candidate for a terminal is blocked, select the next-ranked
   candidate that is not in conflict with other selected candidates.
3. If no candidate remains, the terminal becomes `NoLegalAccess`.

---

## 6. Access confidence and the certificate

The access oracle populates the certificate:
- `RouteCertificate.heuristic_access_nets`: nets routed against non-ExactPin
  access (even if the route is DRC-clean, it's not signoff-quality because the
  pin geometry is uncertain)
- Each net's access confidence level contributes to the `degraded_confidence`
  flag: if any net has `HeuristicPin` access, the result is degraded unless
  the user explicitly accepts heuristic access

The access oracle also feeds the hard vector:
- `HardVector.missing_access`: count of terminals with no legal candidate

---

## 7. The Phase 1-2 fallback

Before the access oracle is implemented, the fallback is:

```
For each terminal:
  1. Get pin center (x, y) and pin layer.
  2. Find nearest grid vertex on pin layer.
  3. If the grid vertex is blocked, try the 4 next-nearest.
  4. If all are blocked, mark terminal as missing_access.
  5. Record AccessConfidence::HeuristicPin for all.
```

This always emits `degraded_confidence = true` for access (since everything
is heuristic), but it works well enough for simple designs where pins are on
routing layers and not densely packed.
