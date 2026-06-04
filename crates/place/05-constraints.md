# Constraints — Hard Intent Implementation

## Scope

This document covers the implementation of all six `IntentClass::Hard`
constraints from the `Constraint` enum:

1. `Symmetric(a, b)` — mirror symmetry between two devices
2. `SelfSymmetric(a)` — a device is symmetric about its own center
3. `SymmetricGroup(pairs)` — multiple pairs share a symmetry axis
4. `Order(a, b, axis)` — device a must be left/below device b
5. `Align(devices, axis)` — devices share a coordinate on one axis
6. `GuardRing(devices, ring_type)` — devices are enclosed by a guard ring

Plus the two `IntentClass::Bounded` constraints that interact with hard
constraints during placement:

7. `Matching(a, b)` — matched pair (equal W/L, proximity)
8. `DistanceConstraint(a, b, max_dist)` — maximum separation

## The IntentGraph

The constraint compiler transforms `Vec<Constraint>` into an `IntentGraph`:
a set of typed edges between device indices and device groups.

### `IntentGraph`

| Field | Type | Description |
|-------|------|-------------|
| `symmetric_pairs` | `Vec<SymmetricPair>` | All mirror-symmetry pairs |
| `symmetric_groups` | `Vec<SymmetricGroupData>` | Multi-pair symmetry groups sharing an axis |
| `self_symmetric` | `Vec<usize>` | Device indices that are self-symmetric |
| `order_edges` | `Vec<OrderEdge>` | Ordering constraints |
| `align_groups` | `Vec<AlignGroup>` | Alignment groups |
| `guard_rings` | `Vec<GuardRingSpec>` | Guard ring enclosures |
| `matched_pairs` | `Vec<MatchedPairData>` | Matching pairs (bounded) |
| `distance_edges` | `Vec<DistanceEdge>` | Max-distance constraints (bounded) |

### `SymmetricPair`

| Field | Type | Description |
|-------|------|-------------|
| `a` | `usize` | Device index of the first device |
| `b` | `usize` | Device index of the mirror partner |
| `axis` | `Axis` | Symmetry axis direction (X = vertical axis, Y = horizontal) |
| `group_id` | `Option<usize>` | If part of a SymmetricGroup, the group ID |

### `SymmetricGroupData`

| Field | Type | Description |
|-------|------|-------------|
| `pairs` | `Vec<(usize, usize)>` | All (a, b) pairs in the group |
| `axis` | `Axis` | Shared symmetry axis |
| `axis_position` | `Option<i64>` | Fixed axis position if determined (None = floating) |

### `OrderEdge`

| Field | Type | Description |
|-------|------|-------------|
| `before` | `usize` | Device that must come first |
| `after` | `usize` | Device that must come second |
| `axis` | `Axis` | X = left-right ordering, Y = bottom-top ordering |

### `AlignGroup`

| Field | Type | Description |
|-------|------|-------------|
| `devices` | `Vec<usize>` | Devices that share a coordinate |
| `axis` | `Axis` | X = same x-coordinate, Y = same y-coordinate |

### `GuardRingSpec`

| Field | Type | Description |
|-------|------|-------------|
| `enclosed` | `Vec<usize>` | Devices enclosed by the ring |
| `ring_type` | `String` | Guard ring type (e.g., "nwell_tap", "pwell_tap") |
| `ring_devices` | `Vec<usize>` | Device indices of generated ring segments |

## Compilation: `Vec<Constraint>` -> `IntentGraph`

### Step 1: Device name resolution

Map device names from constraints to device indices in the netlist. If a name
doesn't match any device, emit a `PlacementConflict` with
`ConflictClass::IntentViolation` and `DiagnosticScope::GlobalInfeasible`.

### Step 2: Symmetric pair extraction

For each `Constraint::Symmetric(a, b)`:
- Resolve device indices
- Infer axis from context: default is `Axis::X` (vertical symmetry axis,
  meaning the two devices are mirrored left-right)
- Create a `SymmetricPair`

For each `Constraint::SymmetricGroup(pairs)`:
- Resolve all pairs
- All pairs share a single axis (inferred or explicit)
- Create `SymmetricPair` for each pair, with `group_id` pointing to the group
- Create a `SymmetricGroupData`

For each `Constraint::SelfSymmetric(a)`:
- Resolve device index
- Record in `self_symmetric`

### Step 3: Consistency checking

- No device appears in two different symmetric pairs (unless they share a group)
- No circular ordering (Order(a,b,X) and Order(b,a,X))
- Alignment groups don't conflict with ordering (if A is aligned-Y with B and
  ordered-X before B, that's fine; if aligned-X with B and ordered-X, conflict)
- Guard ring enclosed devices exist

Conflicts produce `PlacementConflict` with `ConflictClass::IntentViolation`.

## Constraint enforcement in the solver

Each constraint type affects the SA solver in two ways:
1. **Cost penalty** — violated constraints add to the cost function
2. **Move generation** — constraint-aware moves maintain invariants

### 1. Symmetric(a, b)

**Invariant:** Device a and device b are mirrored about a common axis. For a
vertical axis (Axis::X): `x_a + x_b + w_b = 2 * axis_x` and `y_a = y_b`,
and orientation of b is `MY(orientation_a)`.

**In the sequence-pair:** Symmetry is encoded as a constraint on the positions
of a and b in Gamma+ and Gamma-. Specifically, for a vertical-axis mirror:
- In Gamma+: if a is at position p, then b must be at the "mirror" position
  such that the decoded x-coordinates satisfy the mirror equation.
- The simplest encoding: treat (a, b) as a single unit in the sequence-pair
  with width = 2 * device_width + spacing, and decode as a pair.

**Cost penalty:** If the mirror invariant is violated:
```
penalty = GAMMA * (|x_a + x_b + w_b - 2 * axis| + |y_a - y_b|) / grid_nm
```
Where `axis` is the current best axis position (the mean of `x_a + w_a/2`
and `x_b + w_b/2`).

**Move operator — Symmetric swap:** When moving device a in a sequence,
simultaneously move device b to the mirror position. The move is:
1. Pick a random position for a in Gamma+
2. Place b such that their decoded positions satisfy the mirror equation
3. Adjust Gamma- accordingly

**Move operator — Symmetric mirror:** Flip both devices' orientations:
`orient_a -> MY(orient_a)`, `orient_b -> MY(orient_b)`, and swap their
positions. This explores the other valid mirror configuration.

### 2. SelfSymmetric(a)

**Invariant:** Device a is placed on the symmetry axis. For a vertical axis:
the device center is on the axis.

**In the sequence-pair:** No constraint on the sequence. The invariant is
checked during cost evaluation.

**Cost penalty:**
```
penalty = GAMMA * |x_a + w_a/2 - axis_x| / grid_nm
```

### 3. SymmetricGroup(pairs)

**Invariant:** All pairs in the group share the same symmetry axis. The axis
position is a single variable, not per-pair.

**Implementation:** The group is treated as a set of `SymmetricPair`
constraints with a shared `axis_position`. The axis position is recomputed
after every move as the mean of all pair midpoints:
```
axis_x = mean(x_a + w_a/2, x_b + w_b/2) for all (a, b) in group
```

**Cost penalty:** Sum of per-pair penalties, all computed relative to the
shared axis.

**Move operator:** A group move picks one pair and applies a symmetric swap
to it, then adjusts the axis position.

### 4. Order(a, b, axis)

**Invariant:** On the specified axis, a comes before b.
- `Axis::X`: `x_a + w_a <= x_b` (a is left of b)
- `Axis::Y`: `y_a + h_a <= y_b` (a is below b)

**In the sequence-pair:** For `Axis::X` (a left of b), a must appear before b
in Gamma+. This is checked and repaired during move validation — if a move
would place b before a in Gamma+, swap them back.

**Cost penalty:**
```
if axis == X and x_a + w_a > x_b:
    penalty = GAMMA * (x_a + w_a - x_b) / grid_nm
```

**Move operator:** The basic swap move skips swaps that would violate an
ordering constraint. If a swap in Gamma+ would place `after` before `before`,
the move is rejected (not penalized — just not generated).

### 5. Align(devices, axis)

**Invariant:** All devices in the group share a coordinate on the given axis.
- `Axis::X`: all devices have the same x-coordinate (vertical alignment)
- `Axis::Y`: all devices have the same y-coordinate (horizontal alignment,
  same row)

**In the sequence-pair:** Alignment on Y (same row) means all aligned devices
have no above/below relation in the sequence-pair — they differ only in
left/right order. This is enforced by requiring them to have the same relative
order in Gamma+ and Gamma- (all "left-of" relations, no "below" relations).

**Cost penalty:**
```
target = mean(coord[i] for i in devices)
penalty = GAMMA * sum(|coord[i] - target| for i in devices) / grid_nm
```
Where `coord` is x or y depending on the axis.

**Move operator — Group move:** All aligned devices are moved together. When
one member is swapped in a sequence, all other members maintain their relative
positions to it.

### 6. GuardRing(devices, ring_type)

**Invariant:** The specified devices are enclosed within a guard ring structure.
The ring is composed of tap devices placed around the perimeter of the enclosed
group's bounding box.

**Implementation (two phases):**

**Phase 1 — During constraint compilation:**
- Compute the bounding box of the enclosed devices (updated after each SA move)
- Generate ring segment primitives:
  - 4 edge segments (top, bottom, left, right) as tap devices
  - 4 corner segments
- Add the ring segments to the device list with fixed relative positions
  to the enclosed group

**Phase 2 — During cost evaluation:**
- Check that the ring bounding box encloses all enclosed devices plus the
  minimum enclosure spacing from the technology rules
- Check that ring segments don't overlap with non-enclosed devices

**Cost penalty:**
```
for each enclosed device d:
    if d is outside ring bounding box:
        penalty += GAMMA * distance_to_ring_interior
for each ring segment r:
    if r overlaps a non-enclosed device:
        penalty += GAMMA * overlap_area / grid_nm^2
```

**Move operator:** Ring segments are **not independent** — they move with the
enclosed group as a rigid body. A move that shifts one enclosed device must
adjust the ring accordingly.

**Simplification for V1:** Treat the guard ring as a minimum bounding box
constraint with margin. Generate the ring geometry as a post-processing step
after SA converges. This avoids making ring segments first-class devices in
the sequence-pair.

## Constraint validation

After SA converges and legalization completes, the IntentSatisfaction gate
evaluator checks every constraint:

For each constraint in the IntentGraph:
1. Compute the geometric invariant from the finalized positions
2. If violated, emit a `GateEvaluation` with `GateStatus::Fail` and evidence
   describing the violation
3. If satisfied, emit `GateStatus::Pass` with evidence (e.g., "Symmetric(M1, M2):
   axis at x=1500nm, max deviation 0nm")

A single violated hard constraint fails the IntentSatisfaction gate. The
evidence string lists all violations.

## Interaction between constraints

Constraints can interact:
- **Symmetric + Order:** If (A, A') are symmetric and Order(A, B, X) holds,
  then Order(A', B, X) is implied by the mirror. The compiler should detect
  this and not add a redundant order edge.
- **Symmetric + Align:** If (A, A') are symmetric about a vertical axis and
  Align({A, A'}, Y) is set, both are redundantly enforced (symmetry already
  implies same Y). The compiler should recognize this.
- **Align + Order:** Align({A, B}, Y) + Order(A, B, X) means A is left of B
  on the same row. Consistent and common.
- **GuardRing + any:** Guard ring adds devices to the problem. Constraints
  referring to ring-enclosed devices still apply.

The consistency checker in Step 3 of compilation detects true conflicts
(contradictory orderings, impossible alignments) and reports them as
`ConflictClass::IntentViolation` with `DiagnosticScope::GlobalInfeasible`.
