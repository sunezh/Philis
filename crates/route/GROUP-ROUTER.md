# Subsystem: Group Router

**Module:** `crates/route/src/group/`

**Purpose:** Route nets that belong to analog groups — differential pairs,
self-symmetric nets, matched arrays, common-centroid escapes, shields, and
guard rings — under their joint constraints. The group-preserving invariant
(Section 7 of the formulation) is enforced here, not in the individual engines.

**Phase:** 2 (basic diff pair and symmetric), 3 (matched arrays, common-centroid,
advanced guard ring)

> **Note:** This module is route-specific. It uses geometry types from
> `philis_geom` and technology queries from `philis_tech`, but the group
> routing algorithms (topology selection, mirroring, skew compensation,
> scope expansion) are entirely owned by the route crate.

---

## 1. The group-preserving invariant

From the formulation:

```
INV_group: for every group g and every repair step,
  either all nets of g are simultaneously in scope(V) and rerouted under
  the group's joint objective, or none of g is rerouted.
```

This means:
- If net_p of a diff pair is implicated in a violation, net_n is also ripped
  up and both are rerouted together.
- A self-symmetric net's two halves are treated as a group of one.
- A matched array's elements are all in the same rip-up scope.
- A shield track is in the same scope as the net it shields.

The invariant is not a soft preference. It is a correctness requirement: routing
net_p independently of net_n can produce geometry that is individually DRC-clean
but violates the pair constraint (different topology, asymmetric parasitic
environment, skewed length).

### Scope computation

```
seed(V) = { owner(e) : e in imp(v), v in V }     // nets on violated edges
scope(V) = closure over grp(.) of seed(V)         // add all group members
```

`grp(n)` is defined by `NetClass::is_grouped()`:
- `DifferentialPair` -> the pair partner
- `SelfSymmetric` -> the net itself (self-closure)
- `MatchedArray` -> all nets in the array
- `ExactMatch` -> all nets in the match group

The scope is a set of `NetId`s. All nets in scope are ripped up and rerouted
in the same Engine 2 iteration step.

---

## 2. Differential pair routing (`diffpair.rs`)

### The algorithm

Given a diff pair (net_p, net_n) with a symmetry axis:

**Phase A — Topology selection:**
1. Determine the symmetry axis from the constraint and pin positions.
   For a horizontal pair: axis is the horizontal line midway between the
   P and N pins. For vertical: analogous.
2. Select access candidates for both nets (in Phase 3, from the access oracle;
   in Phase 2, grid-snap). Access must be symmetric: if net_p uses track T at
   distance d from the axis, net_n uses the track at distance d on the other
   side.
3. Plan the topology: both legs follow the same layer sequence, same turn
   pattern, mirrored about the axis. In Phase 2, this is implicit (route net_p,
   then mirror). In Phase 3, the rubber-band sketch gives an explicit topology
   template.

**Phase B — Route net_p:**
4. Route net_p using Engine 3 (A*) on the half-plane containing its pins,
   with the axis as a boundary.
5. For each segment of net_p's path, compute the mirrored segment for net_n.
6. Check that each mirrored segment is legal (not blocked by obstacles
   specific to net_n's side).

**Phase C — Resolve asymmetries:**
7. If the mirror is clean, commit both paths.
8. If the mirror fails at some point (an obstacle blocks net_n's mirrored
   path), try:
   a. Route net_n independently but constrained to the same topology (same
      layer sequence, turns at symmetric positions).
   b. If that fails, re-route net_p with a different topology and re-mirror.
   c. If both fail, report `ImpossibleConstraint` with the blocking obstacle.

**Phase D — Skew measurement and compensation:**
9. Measure total routed length of each leg.
10. Compute skew = |length_p - length_n|.
11. If skew > threshold (from constraint or default), add serpentine
    compensation to the shorter leg:
    - Find a straight segment on the shorter leg long enough to accommodate
      a serpentine meander.
    - Replace the straight segment with a zigzag of the required extra length.
    - Verify the zigzag is DRC-clean.
12. Record `RouteQuality.max_diff_pair_skew_nm`.

### Inter-pair spacing

The two legs run in parallel on the same layer, separated by the inter-pair
gap. This gap is determined by:
1. The minimum spacing for the wire width (from the tech model)
2. The coupling constraint (if the pair wants controlled coupling, the gap
   is set to achieve the target Cc)
3. Default: 2 * min_spacing (one empty track between the legs)

The inter-pair gap is a routing parameter, not a DRC rule — it's in the soft
objective.

### What makes diff pair routing hard

The difficulty is not the mirroring — it's the *constraints on access and
escape*. When the P and N pins are surrounded by other devices, finding a
symmetric escape path that doesn't violate DRC and doesn't create asymmetric
coupling is the hard part. This is why the access oracle (Phase 3) matters for
diff pairs more than for ordinary nets.

---

## 3. Self-symmetric nets (`symmetric.rs`)

A self-symmetric net has geometry that is its own mirror about a symmetry axis.
This arises in current mirrors, cross-coupled structures, and common-mode paths.

### Algorithm

1. Identify the symmetry axis from the constraint.
2. Classify the net's pins into three sets:
   - **Axis pins:** on the axis (or close enough to snap to it)
   - **P-side pins:** on one side of the axis
   - **N-side pins:** the symmetric counterparts on the other side
3. Route each P-side pin to the axis, mirror to connect the N-side pin.
4. Route axis pins directly on the axis.
5. Connect everything at the axis crossing.

The key constraint: every segment on the P-side has a mirrored segment on the
N-side. Via counts and layer transitions are equal on both sides.

### Difficulty

Self-symmetric routing is harder than diff pairs when the net has high fanout
(connects to many pins on both sides). The Steiner tree for a self-symmetric
net must itself be symmetric, which constrains the topology more than a
regular Steiner tree.

For Phase 2, handle the common cases: 2-pin and 4-pin self-symmetric nets.
Higher-fanout cases can fall back to unconstrained routing with a degraded
confidence marker.

---

## 4. Matched arrays and common-centroid (`matched.rs`)

### Matched length / matched parasitics

A `MatchedArray` or `ExactMatch` group requires multiple nets to have
(approximately) equal routed length, equal via count, equal per-layer length,
and/or equal parasitic environment.

**Algorithm:**
1. Route all nets in the group independently.
2. Measure the length, via count, and per-layer breakdown of each.
3. Compute the maximum delta (the worst-case mismatch).
4. If the delta exceeds the threshold:
   a. Identify the shortest and longest nets.
   b. Add serpentine compensation to the shorter nets.
   c. Remove vias from the longer nets (route on fewer layers) or add
      vias to the shorter nets (equalize via count).
5. Iterate until the delta is within tolerance or a budget fires.

This is a post-route equalization pass. A better approach (Phase 3) is to
route the group jointly with a shared topology template, but that requires
the rubber-band sketch or a joint A* formulation.

### Common-centroid escape

A common-centroid array (e.g., a capacitor bank or transistor array) has
devices arranged symmetrically around a centroid. The escape routing must
preserve the centroid property: the total wire from each element to the
common node must be balanced.

**Algorithm:**
1. Classify elements by their position relative to the centroid.
2. Route pairs of symmetric elements together (like diff pairs, with the
   centroid as the axis).
3. The central element (if any) routes directly.
4. Equalize lengths across all elements.

This is complex and Phase 3 scope. Phase 2 handles matched arrays only
through post-route equalization (step 1-5 above).

---

## 5. Shield routing (`shield.rs`)

For `Constraint::NetShield(net)`:

### Algorithm

1. Route the sensitive net first (it gets priority in net ordering).
2. After the sensitive net is committed, identify its trunk segments
   (long straight runs on routing layers).
3. For each trunk segment on layer L:
   a. Identify the adjacent tracks on both sides of the trunk.
   b. Reserve those tracks (set capacity to 0 for non-shield nets).
   c. Route the shield net (typically VSS/VDD) on the reserved tracks.
4. Connect the shield segments with vias to the shield net's power rail.

### When shielding is impossible

If the adjacent tracks are already occupied by other nets:
- If those nets are lower priority, rip them up (they'll re-route around).
- If those nets are higher priority or in a group, report the conflict as
  `DiagnosticClass::ImpossibleConstraint`.
- The certificate records whether each shielded net's shield is complete or
  partial.

### Shield as return-path provisioning

For RF nets, the shield serves a dual purpose: capacitive screening (reducing
Cc to aggressors) and return-path provisioning (providing a low-inductance
return current path). The formulation's `sh[n,e]` variable captures both.

In Phase 3, the PEX surrogate scores the shield's effectiveness — a partial
shield (gap in the return path) is worse than no shield for inductive coupling,
even though it's better for capacitive coupling.

---

## 6. Guard ring routing (`group/` general)

Guard rings are specialized structures that surround sensitive or noisy devices
to provide substrate isolation. Routing-wise, they have two aspects:

1. **The ring itself:** a continuous conductive loop (typically on a low metal
   layer) connected to a supply rail. The ring routing is straightforward —
   it follows the ring geometry from the placement.

2. **The connections:** the ring must be connected to the substrate through
   taps and to the supply rail through vias and metal. These connections
   must not break other routing.

Guard ring routing is a Phase 3 feature. In Phase 2, guard rings are treated
as obstacles (their geometry blocks routing) and the ring connectivity is
not verified by the router (classified `ExternalOnly` in the certificate).

---

## 7. Integration with Engine 2

The group router is called by Engine 2's negotiated congestion loop:

```
for net in ordered_nets:
    if net.net_class.is_grouped():
        group = get_group(net)
        if group not already routed this iteration:
            group_router.route_group(group, state, graph)
            mark group as routed this iteration
    else:
        engine3.route_net(net, state, graph)
```

The group router internally calls Engine 3 for individual path searches but
adds the group constraints (symmetry, mirroring, length matching).

### Rip-up scope

When Engine 2's violation analysis implicates a net that belongs to a group,
the scope expander pulls in the entire group:

```
fn expand_scope(seed_nets: &[NetId], groups: &GroupTable) -> Vec<NetId> {
    let mut scope = BTreeSet::from_iter(seed_nets);
    let mut changed = true;
    while changed {
        changed = false;
        for &net in scope.clone().iter() {
            if let Some(group) = groups.group_of(net) {
                for &member in group.members() {
                    changed |= scope.insert(member);
                }
            }
        }
    }
    scope.into_iter().collect()
}
```

This is a fixed-point computation that terminates because the group structure
is finite and acyclic (no net belongs to two groups).

---

## 8. Testing

### Diff pair tests

- Route a 2-pin diff pair on a clear channel: both legs should be mirrored,
  equal length, equal via count.
- Route a diff pair with an asymmetric obstacle: the router should find a
  symmetric detour or report the asymmetry.
- Skew measurement: manually create a pair with known skew and verify the
  measurement is correct.
- Serpentine compensation: create a pair where one leg is shorter by a known
  amount and verify the compensation adds the right length.

### Matched array tests

- Route a 4-net matched group: verify all lengths are within tolerance.
- Route a group where one net has a longer path: verify equalization adds
  serpentine to the shorter nets.

### Shield tests

- Route a shielded net: verify shield tracks are present on both sides.
- Route a shielded net where one side is blocked: verify the diagnostic
  reports partial shielding.

### Scope expansion tests

- Implicate one net of a diff pair: verify both are in scope.
- Implicate a net that shields another: verify both are in scope.
- Implicate an ordinary net: verify only that net is in scope.
