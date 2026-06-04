# Subsystem: DRC Engine

> **Shared crate:** DRC rule predicates (`RulePredicate`, `RuleKind`,
> `RuleParams`) and violation types (`DrcViolation`, `ViolationScope`)
> come from `crates/verify` (`philis_verify`). The route crate imports
> these types and uses the shared rule predicate dispatch for checking.
> The incremental delta checking loop (bounded-influence dilation, spatial
> query integration, commit/ripup lifecycle) stays in the route crate
> because it is tightly coupled to the route state.

**Module:** `crates/route/src/verify/drc.rs`

**Purpose:** Check design rules incrementally during routing (on each commit)
and comprehensively after routing (final check). The DRC engine is the
mechanism that makes the formulation's "in-loop signoff" practical — it
localizes violations before clean routes are disturbed.

**Phase:** 1 (basic post-route), 2 (incremental delta), 3 (advanced rules)

---

## 1. Two modes of operation

### Incremental delta check (the primary mode)

After each `commit(net, path)`, re-check only the geometry affected by the
change. This is the bounded-influence property from the formulation (Section
7, soundness):

```
When geometry changes in region Q:
  check_region = Q.expand(r_max)     // dilation by max influence radius
  shapes = spatial_index.query_window(layer, check_region)
  for each pair (s1, s2) in shapes where s1.owner != s2.owner:
    check all applicable rules between s1 and s2
```

Soundness: the dilation by `r_max` guarantees that any shape whose legality
could have changed is included. No violation that a full check would find can
be missed by the delta, because every DRC predicate has a finite spatial
influence radius.

### Full check (the final verification)

After all routing is complete, sweep all geometry and check every rule. This
is the backstop: it catches anything the incremental mode might miss due to:
- Rules that are not delta-safe (their influence is unbounded or global)
- Accumulation effects (antenna, density) that depend on the full net, not a
  local region
- Bugs in the incremental implementation

The certificate records which predicates were proven at clip scope (incremental)
vs. full scope, so the soundness boundary is auditable.

---

## 2. Rule dispatch

### Rule classification

Each rule predicate from the tech model has metadata:

```
RulePredicate {
    id: RuleId,
    kind: RuleKind,
    layers: Vec<LayerId>,
    influence_radius_nm: i64,
    is_delta_safe: bool,
    coverage: Coverage,
    parameters: RuleParams,
}
```

**Delta-safe rules** (can be checked incrementally):
- Min width
- Min spacing (basic, width-dependent, PRL-dependent, EOL)
- Via enclosure
- Via cut spacing
- Same-net spacing and notch
- Min area (per shape, not per net)

**Not delta-safe** (must run at larger scope):
- Antenna (accumulates along a full net path)
- Density (accumulates over a window, typically 50-100 um)
- Connectivity (the union-find handles this, not the DRC engine)

### Rule evaluation

For a pair of shapes (s1, s2) on the same or adjacent layers:

```
fn check_pair(s1: &ShapeRef, s2: &ShapeRef, tech: &Tech) -> Vec<DrcViolation> {
    let mut violations = Vec::new();
    
    // Same layer, different owner: check spacing
    if s1.layer == s2.layer && s1.owner != s2.owner {
        let required = tech.rules.min_spacing(s1.layer, max(s1.width, s2.width));
        let actual = s1.rect.manhattan_distance(&s2.rect);
        if actual < required {
            violations.push(spacing_violation(s1, s2, required, actual));
        }
    }
    
    // Same layer, same net: check same-net spacing/notch
    if s1.layer == s2.layer && s1.owner == s2.owner {
        if let Some(notch_rule) = tech.rules.same_net_spacing(s1.layer) {
            // check notch between same-net shapes
        }
    }
    
    // Adjacent layers with via: check enclosure
    // ...
    
    violations
}
```

### Width-dependent spacing

The most common advanced rule: spacing increases when one of the wires is
wider than a threshold:

```
spacing_rules = [
    (width_threshold: 0,    min_spacing: 140),   // base
    (width_threshold: 3000, min_spacing: 280),   // wide metal
]

fn min_spacing(layer, width) -> i64:
    rules[layer]
        .filter(|r| width >= r.width_threshold)
        .max_by_key(|r| r.width_threshold)
        .min_spacing
```

### PRL-dependent spacing

Spacing increases when two wires run parallel for more than a threshold
distance:

```
fn spacing_with_prl(layer, width, parallel_run_length) -> i64:
    base = min_spacing(layer, width)
    for prl_rule in prl_rules[layer]:
        if parallel_run_length >= prl_rule.prl_threshold:
            base = max(base, prl_rule.min_spacing)
    base
```

Computing PRL between two rectangles: the parallel run length is the overlap
of their projections onto the axis perpendicular to their parallel edges.

### End-of-line (EOL) spacing

A short wire end (where the wire terminates) requires extra spacing to nearby
shapes. An edge is "end-of-line" if its length (in the wire direction) is
below the EOL width threshold.

```
fn eol_check(s1: &ShapeRef, s2: &ShapeRef, tech: &Tech) -> Option<DrcViolation>:
    for each end of s1:
        if end_length < eol_width_threshold:
            eol_extension = eol_extension_rule(layer)
            extended_region = extend end by eol_extension
            if s2 overlaps extended_region and spacing < eol_spacing:
                return Some(violation)
```

EOL is Phase 3 because it requires knowing which edges are wire endpoints,
which depends on the full route topology (not just local geometry).

---

## 3. Violation representation

```
DrcViolation {
    rule: RuleId,                  // which rule was violated
    layer: LayerId,
    shapes: (ShapeId, ShapeId),    // the two shapes involved
    region: Rect,                  // bounding box of the violation
    required: i64,                 // the required spacing/width/area
    actual: i64,                   // the measured value
    scope: ViolationScope,         // LocalRepairable | GlobalInfeasible
}
```

The violation carries enough information for the repair loop to:
1. Identify the nets involved (`ShapeRef.owner`)
2. Localize the problem (the `region`)
3. Understand the severity (how far below the required value)
4. Decide the repair strategy (widen the wire, move the route, rip up)

---

## 4. Integration with the route state

### On commit

```
fn on_commit(state: &mut RouteState, net: NetId, segments: &[Segment]) {
    for segment in segments {
        let check_region = segment.rect.expand(r_max, r_max);
        let neighbors = state.geom.query_window(segment.layer, &check_region);
        for neighbor in neighbors {
            if neighbor.id == segment.id { continue; }
            let violations = check_pair(segment, neighbor, state.tech);
            state.drc.add_violations(violations);
        }
    }
}
```

### On ripup

```
fn on_ripup(state: &mut RouteState, net: NetId) {
    // Remove violations involving this net's shapes
    let net_shapes: HashSet<ShapeId> = state.geom.shapes_of(ShapeOwner::Net(net))
        .map(|s| s.id)
        .collect();
    state.drc.remove_violations_involving(&net_shapes);
    
    // Also remove any violations that were BETWEEN this net and others,
    // since removing the net may have resolved them.
    // But: the neighbor shapes may now have NEW violations with other shapes
    // that were previously shadowed. This requires re-checking the neighbors.
    // In practice, this is conservative: only clear violations involving the
    // ripped-up net, and re-check will catch any new ones.
}
```

### Feeding violations into the repair loop

The DRC engine's violation list feeds directly into the negotiated congestion
loop's repair scope:

```
fn compute_repair_scope(violations: &[DrcViolation]) -> Vec<NetId> {
    let seed: BTreeSet<NetId> = violations.iter()
        .flat_map(|v| {
            let s1_owner = state.geom.shape(v.shapes.0).owner;
            let s2_owner = state.geom.shape(v.shapes.1).owner;
            [s1_owner.net(), s2_owner.net()]
        })
        .flatten()
        .collect();
    
    expand_scope(&seed, &groups)
}
```

---

## 5. Performance

### The r_max computation

`r_max` is the maximum influence radius across all active rules:

```
r_max = rules.iter()
    .filter(|r| r.is_delta_safe)
    .map(|r| r.influence_radius_nm)
    .max()
```

For most technologies, `r_max` is 2-3x the maximum spacing rule (a few
hundred nm to a few um). The dilated check region is therefore small relative
to the chip — the spatial query returns few shapes, and the pairwise checking
is fast.

### Amortization

For a net with K segments, the incremental check does K spatial queries, each
returning O(1) to O(10) neighbors (in typical analog layouts). The total cost
is O(K * neighbors) pairwise checks — typically O(K) to O(10K).

Compare to a full check: O(N^2) pairwise checks across all shapes. For a
layout with 1000 shapes, the full check does ~500k comparisons; the
incremental check for a 10-segment commit does ~100. This is the 5000x
speedup that makes in-loop DRC practical.

### Caching

Cache the set of shapes near each committed net (the "neighborhood"). When a
nearby net is ripped up or committed, invalidate the cache for affected
neighbors. This avoids redundant spatial queries.

---

## 6. The `r_max` soundness argument

The incremental delta is sound iff the dilation by `r_max` captures every
shape whose legality could change. This holds because:

1. Every DRC predicate has a finite spatial extent (the distance at which
   the predicate stops depending on a shape's presence).
2. `r_max` is at least as large as the largest such extent.
3. Therefore, the dilated region contains every shape that could participate
   in a new or resolved violation.

Rules with unbounded extent (density, antenna) are flagged `is_delta_safe = false`
and checked at larger scope. The certificate records: "these predicates were
proven at clip scope (sound); these were proven at tile/net scope (also sound
but more expensive)."

---

## 7. Phase progression

### Phase 1: post-route only

After all nets are routed, sweep all geometry and check:
- Min width (trivial — all wires are at layer min width)
- Min spacing between different-net shapes

No incremental checking. The full check is the baseline that Phase 2 must match.

### Phase 2: incremental delta

On each commit, check the dilated region for:
- Min spacing (basic and width-dependent)
- Via enclosure
- Same-net notch

The full check runs at the end as a safety net. Any violation found by the
full check but missed by the incremental engine is a bug in the incremental
engine (the dilation was too small or a rule was incorrectly classified as
delta-safe).

### Phase 3: advanced rules

Add to the incremental engine:
- EOL spacing
- PRL-dependent spacing
- Min-area checking (per shape) and patch synthesis
- Cut spacing (inter-via)

Add non-delta-safe rules:
- Antenna (net-scope check after each net is fully committed)
- Density (tile-scope check at configurable intervals or at the end)

### Beyond Phase 3: external DRC

For rules classified `ExternalOnly` or `ManualReview`, the router does not
check them. The certificate records their existence and coverage class. The
user runs the external DRC tool (Calibre, IC Validator, Magic) on the GDS
output and provides the report. The certificate can incorporate the external
report's hash.
