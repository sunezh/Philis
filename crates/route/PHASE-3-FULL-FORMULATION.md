# Phase 3 — Full Formulation

**Goal:** Implement the remaining subsystems to realize the complete
signoff-first routing formulation: the pin access oracle, the exact-local
engine, in-loop parasitic extraction, EM/IR/antenna checking, full certificate
assembly with evidence, and ALT heuristics.

**Modules delivered:** `access/`, `engine/exact.rs`, `engine/alt.rs`,
`verify/extract.rs`, `verify/em.rs`, `certificate/` (full)

**Depends on:** Phase 2 (negotiated congestion, group routing, incremental DRC,
route state management)

**Unlocks:** Production-quality routing with signoff-grade certificates

---

## 1. What Phase 3 completes

After Phase 3, the router realizes the full formulation:

- Terminal access is a first-class subproblem with ranked candidates, a conflict
  graph, and fail-fast on impossible access
- The exact-local engine handles decision-dense clips (pin access, via/color
  legality, tightly coupled escapes) that negotiation and maze can't prove
- In-loop PEX estimates R, Cg, Cc for each committed net incrementally
- EM/IR checking sizes wires and vias against current-density limits
- Antenna checking accumulates metal area per gate and flags violations
- The certificate carries real evidence (internal proofs, counterexamples) and
  minimized infeasible cores for failures
- The ALT heuristic makes A* fast on the non-Euclidean cost graph

Phase 3 is not one deliverable — it's a collection of independently valuable
subsystems that can be built and shipped in any order after Phase 2 is stable.

---

## 2. Pin access oracle (`access/`)

### Why this matters now

Phase 1-2 use simple grid-snap: the closest on-grid point to each pin center.
This fails when:
- A pin shape spans multiple tracks and only some are legal (others violate
  via enclosure or obstruction spacing)
- Two adjacent pins have conflicting access points (the "net-ordering
  pathology" — a greedy per-pin choice paints a later pin into a corner)
- A differential pair needs mirrored access points on symmetric tracks

The access oracle eliminates these failure modes by solving access as a
selection problem before routing begins.

### Candidate generation (`access/candidate.rs`)

For each terminal:
1. Get the pin shape (exact from PDK/cell, generated from device model, or
   heuristic anchor — record the `AccessConfidence` level).
2. Generate on-track candidates: for each routing layer, intersect the pin
   shape with the track grid. Each intersection that fits a via landing pad
   is a candidate.
3. Generate off-track candidates: if no on-track candidate exists, try
   half-pitch offsets. These have higher cost and lower confidence.
4. For each candidate, check local DRC: via enclosure, cut spacing, min-area
   of the landing pad, spacing to nearby obstacles.
5. Attach parasitic access cost: the resistance/capacitance from the pin
   center to the candidate point.
6. Rank candidates by the multi-objective ranking from Section 6 of the
   formulation: exact > heuristic, fewer vias > more, lower DRC risk > higher,
   lower parasitic cost > higher, symmetric counterpart available > not.

### Conflict graph (`access/conflict.rs`)

Build a graph where vertices are access candidates and edges connect pairs that
cannot coexist (a wire at candidate A would cause a DRC violation with a wire
at candidate B, typically because they're on adjacent tracks of different nets
with insufficient spacing).

### Compatible subset selection (`access/select.rs`)

Select one candidate per terminal such that no two selected candidates are in
conflict. This is a constraint satisfaction problem:

For small instances (common in analog — typically <100 terminals):
- Model as a SAT problem or an integer program
- Solve exactly — the result is a globally compatible access assignment

The exact solver here is engine 1 (the exact-local engine) applied to the
access selection problem. It's the first use case for the exact engine, and
it's the right one: the problem is small, decision-dense, and the penalty for
a wrong greedy choice is high.

If no compatible subset exists, the oracle returns the infeasible core: the
minimal set of terminals and conflict edges that make selection impossible.
This becomes `HardVector.missing_access` and a `DiagnosticClass::NoLegalAccess`
diagnostic.

### Analog access extensions

For grouped nets:
- Differential pair: candidates are generated in mirrored pairs across the
  symmetry axis. The selection picks a pair or neither.
- Matched array: candidates respect the centroid/interdigitation structure.
- Multiport terminals: a current-carrying net can claim a via array (multiple
  candidates on the same terminal), not just a single stack.

---

## 3. Exact-local engine (`engine/exact.rs`)

> **Shared crate:** The SAT and ILP solver abstractions come from
> `crates/solver` (`philis_solver`). The exact-local engine uses
> `philis_solver::SatSolver` for feasibility problems (access selection,
> color assignment) and `philis_solver::IlpSolver` for optimization
> problems (minimum-cost via assignment). Convenience builders
> (at-most-one, at-least-one) are also provided by `philis_solver`.

### Scope

The exact engine handles small, decision-dense clips where maze/negotiation
can't prove legality:

1. **Pin access selection** (see above): select one access candidate per
   terminal with no conflicts.
2. **Via/color/min-area legality**: given a set of route segments in a small
   region, assign via types, multipatterning colors, and min-area patches
   such that all DRC predicates hold.
3. **Tightly coupled escapes**: for a diff pair or common-centroid array
   exiting a dense pin region, enumerate the legal escape topologies and
   select the one that satisfies group constraints.

### Formulation

The exact engine formulates each clip as a constraint satisfaction / optimization
problem:

- **Variables**: binary (which candidate / via / color / patch to use)
- **Constraints**: DRC predicates (no-short, spacing, enclosure, area, color
  compatibility), group predicates (mirrored, equal topology)
- **Objective**: minimize access cost, then via count, then area-patch count

### Solver choice

The `philis_solver` crate provides the `SatSolver` and `IlpSolver` traits,
along with MUS/MCS extraction utilities. For the problem sizes the exact
engine sees (10-100 variables, 50-500 constraints), calls go through these
trait abstractions:

1. **SAT** via `philis_solver::SatSolver` for pure feasibility problems
   (access selection, color assignment).
2. **ILP** via `philis_solver::IlpSolver` when the objective matters
   (minimize cost).
3. **Exhaustive enumeration** for very small clips (<20 variables). The
   simplest correct approach; fast enough for pin-access on a single cell.

Start with exhaustive enumeration for access selection and SAT for color
assignment. Move to ILP only when objective optimization justifies it.

---

## 4. ALT heuristic for A* (`engine/alt.rs`)

### The problem with Manhattan distance

Phase 1-2 use Manhattan distance as the A* heuristic. This is admissible but
loose: it ignores via costs, per-layer resistance differences, history
surcharges, and blockages. A loose heuristic means A* explores too many nodes,
making it slow on congested designs.

### Landmarks

ALT (A*, Landmarks, Triangle inequality) precomputes exact distances from a
small set of landmark vertices to every other vertex in the graph. Then:

```
h(u, target) = max over all landmarks L of |dist(u, L) - dist(target, L)|
```

This is admissible because it uses real graph distances (not an embedding),
and it's tight around blockages and dense channels where maze routing is slow.

### Landmark selection

Choose 4-8 landmarks at the graph extremes (corners and midpoints of the
bounding box) plus 2-4 in congested regions (identified by high history cost).
Recompute only when the graph structure changes (new obstacles, not just new
routes — routes change edge costs, not graph topology).

### When to use

ALT is an optimization that matters when the maze router is the bottleneck
(many rip-up/reroute queries per iteration on large designs). On small designs
(<50 nets), the overhead of landmark precomputation may exceed the savings.
Gate ALT behind a heuristic: use it when `net_count * avg_path_length > threshold`.

---

## 5. In-loop PEX surrogate (`verify/extract.rs`)

> **Shared crate:** The PEX surrogate model and EM/antenna checking types
> come from `crates/verify` (`philis_verify`). The route crate calls the
> shared PEX surrogate API with route-specific geometry and queries the
> shared EM/antenna models for violation checking.

### What it estimates

For each committed net, the PEX surrogate computes:
- **R**: total wire resistance (sum of `length * sheet_R / width` per segment)
- **Cg**: ground capacitance (sum of `length * area_cap + 2 * length * fringe_cap`
  per segment, from the extraction parameters)
- **Cc**: coupling capacitance to adjacent nets (based on parallel run length,
  spacing, and the per-layer coupling model from tech)
- **Rvia**: total via resistance (count * per-via resistance)

These are *surrogates*, not signoff-quality extraction. The tech model provides
per-layer extraction parameters; the surrogate uses them with simple analytical
formulas, not a field solver.

### Incremental update

On commit(net): compute the net's R/Cg/Rvia from its segments. For Cc, query
the spatial index for shapes on the same layer within the coupling window
(`Cc_window` from the formulation) and compute parallel run length.

On ripup(net): subtract the net's contribution from the totals. For Cc, the
coupling to other nets changes — re-query and update.

### Feeding back into routing

PEX outliers (nets where R or Cc exceeds a threshold from the constraint or
net class) seed `scope(V)` for the repair loop. The repair reroutes the
outlier on a lower-resistance path (wider wire, fewer vias, less coupling).

This is a soft optimization — PEX outliers are `DiagnosticScope::LocalRepairable`
unless the constraint promotes them to hard.

---

## 6. EM / IR / antenna checking (`verify/em.rs`)

### Electromigration

For each segment carrying declared current I:
```
J = I / (width * thickness)
EM_margin = J_max / J - 1
```

If `EM_margin < 0`, the segment fails EM. Check the Blech filter first:
if `J * length < (J*L)_crit`, the segment is EM-immortal regardless.

EM failures seed scope(V) for re-sizing (wider wire or more vias). If the
constraint or PDK promotes EM to hard, failures appear in the `HardVector`.

### Static IR drop

For power/ground nets with declared current:
```
IR_drop = sum of (I * R_segment) along the worst-case path from source to load
```

This requires a declared current map (which loads draw how much from which
power pin). In Phase 3, this comes from the constraint (`Constraint::ChargeFlow`
or from the netlist's current annotations).

### Antenna

Accumulate metal area per gate oxide connection along the route. If the ratio
exceeds the antenna limit (from the tech model), insert a diode repair or
report a violation.

Antenna is not delta-safe (it accumulates along the full net, not a local
region), so it runs at net scope, not clip scope. The certificate records
this: `is_delta_safe: false` for the antenna predicate.

---

## 7. Full certificate assembly (`certificate/`)

### Evidence collection (`certificate/evidence.rs`)

For each hard gate, collect the strongest available evidence:

- **DRC gate**: if the incremental DRC checked all coded predicates and found
  zero violations, emit `Evidence::InternalProof`. If external checks are
  needed (rules classified `ExternalOnly`), emit the external report's hash.
  If DRC fails, emit `Evidence::Counterexample` with the violation details.

- **LVS gate**: if the union-find connectivity invariant held for all nets and
  a same-net/different-net check found no shorts, emit `InternalProof`. If
  full LVS requires external extraction, emit `ExternalReport`.

- **Connectivity**: record whether each net was certified by the invariant
  or by the exact ILP mode.

### Infeasible core minimization (`certificate/diagnosis.rs`)

> **Shared crate:** MUS/MCS extraction algorithms come from `crates/solver`
> (`philis_solver`). The diagnosis module uses `philis_solver` for the
> satisfiability checks underlying core minimization.

When the router cannot satisfy a hard gate, it preserves the minimal conflicting
set rather than reporting "routing failed":

1. Collect all `RouteDiagnostic` entries with `DiagnosticScope::GlobalInfeasible`.
2. For each, extract the implicated nets, predicates, and geometry.
3. Attempt to minimize: remove each element and re-check whether the infeasibility
   persists. If removing an element makes the problem feasible, it's in the core.
4. Store the minimized set in `RouteCertificate.infeasible_core`.

The cost of minimization is bounded by the core size times the cost of a
feasibility check on the clip. For small infeasible cores (the common case in
analog), this is fast.

### Heuristic-access tracking

Any net routed against a non-`ExactPin` access candidate is recorded in
`RouteCertificate.heuristic_access_nets`. This is the "not signoff-quality
until verified by exact device geometry" flag from the formulation.

---

## 8. Advanced DRC predicates

Phase 3 extends the incremental DRC engine with:

- **End-of-line (EOL) spacing**: a short wire tip requires extra spacing to
  nearby shapes. Check by querying the spatial index in the EOL extension zone
  beyond each wire endpoint.
- **Parallel run length (PRL) dependent spacing**: spacing increases when two
  wires run parallel for more than a threshold distance. Track parallel run
  length during the spacing check.
- **Min-area and area patches**: after routing, check that every shape on each
  layer meets the minimum area rule. If not, synthesize a patch rectangle
  (`patch[n,p]` in the formulation) and check that the patch is DRC-legal.
  Report patch count in `RouteQuality.min_area_patch_count`.
- **Density windows**: accumulate metal density per window and check against
  min/max limits. This is not delta-safe (a window can span many nets) and
  runs at tile scope.

### Multipatterning (advanced nodes only)

For technologies with double/triple patterning:
- Build the conflict graph (shapes within same-mask min-spacing are connected)
- 2-color: BFS for odd cycles. If found, attempt stitch insertion. If
  unstichable, report `DiagnosticClass::ColorCutMaskStitch`.
- 3-color: NP-complete; use heuristic coloring or exact on small clips.

This is gated by the tech model: if no multipatterning layers exist, the
entire subsystem is skipped.

---

## 9. Exit criteria

Phase 3 is feature-by-feature. Each subsystem has its own exit:

| Subsystem | Exit criterion |
|-----------|---------------|
| Access oracle | The 5-device diff-pair circuit from Phase 2 routes with exact-pin access and no `heuristic_access_nets` |
| Exact engine | Pin access selection for a dense cell finds a globally compatible assignment that greedy would miss |
| ALT heuristic | A* explores 30%+ fewer nodes on a congested 50-net design vs. Manhattan heuristic |
| PEX surrogate | R/Cg/Cc estimates are within 20% of an analytical reference on a simple RC benchmark |
| EM checking | A power net with declared current gets flagged when width is below EM limit, and passes after re-sizing |
| Full certificate | Every field of `RouteCertificate` is populated with real values; `is_signoff_clean()` returns `true` on a clean result with a `FullSignoff` PDK |
| Infeasible core | An impossible routing (contradictory constraints) returns a minimal conflict set, not a bare "failed" |

---

## 10. What remains beyond Phase 3

The formulation includes features that are beyond Phase 3:

- **Learning-guided priors** (Section 12 of Algorithms): needs training data
  from successful routes. Build the bias interface but don't train until the
  router produces enough layouts.
- **Rubber-band / topological sketch routing** (Section 11): for RF and
  off-grid analog routing. Requires a fundamentally different graph
  representation (homotopy classes instead of grid edges). Treat as a
  separate engine mode.
- **Full GeoSteiner**: exact RSMT for small high-value nets. FLUTE for degree
  <= 9 is trivial (a lookup table); GeoSteiner's branch-and-bound is a
  library dependency. Add when matched-trunk wirelength optimization needs it.
- **Hierarchical routing**: routing across hierarchy levels (feed-throughs,
  inter-block connections). Requires a hierarchical version of the route state
  and the resource graph.
- **External signoff integration**: calling Calibre/IC Validator/PEX tools
  from the loop. Requires file-based I/O (GDSII out, report in) and is
  process-specific. Build the adapter interface; implement for specific flows.
