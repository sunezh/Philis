# Legalization — Group-Preserving Snapping and Repair

## Purpose

After the SA solver produces a candidate placement, legalization cleans it up:
- Snaps all coordinates to the manufacturing grid
- Resolves residual overlaps that the SA couldn't eliminate
- Repairs spacing violations
- Re-mirrors symmetry pairs if disturbed by grid snapping

Legalization is **group-preserving**: it never moves one side of a matched
pair alone, and it never breaks an intent invariant that the SA satisfied.

## When legalization runs

Legalization runs once, after SA converges and before gate evaluation:

```
SA solver -> raw (x, y, orient) per device
                |
                v
         Legalization
                |
                v
         legalized (x, y, orient) per device
                |
                v
         Gate evaluation (LocalDrc, DeviceEquivalence, TerminalAccess,
                          Routability, IntentSatisfaction)
```

The gate evaluator calls `evaluate_gate_acceptance(gates, true)` (pre-
legalization) on the raw SA output to identify repairable failures, then
legalization runs, then `evaluate_gate_acceptance(gates, false)` (post-
legalization) on the final result.

Actually — the pre-legalization check is to decide whether to attempt
legalization at all. If a non-repairable gate fails (e.g., TechnologyCoverage,
DeviceEquivalence, TerminalAccess, Routability), legalization is skipped and
the failure goes straight to the conflict core. Only LocalDrc and
IntentSatisfaction failures trigger legalization.

## The legalization algorithm

### Phase 1: Grid snap

For each device i:
```
x[i] = round_to_grid(x[i], grid_nm)
y[i] = round_to_grid(y[i], grid_nm)
```

Where `round_to_grid(v, g) = ((v + g/2) / g) * g` (round to nearest multiple).

For symmetric pairs (a, b) with a vertical axis:
```
// Snap the axis position first, then derive device positions
axis = round_to_grid((x[a] + w[a]/2 + x[b] + w[b]/2) / 2, grid_nm)
x[a] = round_to_grid(axis - w[a]/2, grid_nm)  // may not be exact
x[b] = round_to_grid(2 * axis - x[a] - w[a], grid_nm)
y[a] = round_to_grid(y[a], grid_nm)
y[b] = y[a]  // symmetry requires same y
```

The key invariant: after grid snap, the symmetric pair may have a residual
axis error (the axis might not be exactly centered between the two devices
because both coordinates are snapped independently). This is tracked and
reported as a bounded violation if it exceeds the grid tolerance.

### Phase 2: Overlap removal

After grid snapping, some devices may overlap (the SA cost function penalizes
overlap but doesn't guarantee zero overlap at convergence).

**Algorithm: greedy left-right shift**

1. Build a list of all overlap pairs from the DRC oracle
2. Sort by overlap area (largest first)
3. For each overlap pair (i, j):
   - Compute the minimum shift to resolve the overlap:
     ```
     shift_x = max(0, x0_i + w_i + spacing - x0_j)  // if i is left of j
     shift_y = max(0, y0_i + h_i + spacing - y0_j)  // if i is below j
     ```
   - Choose the axis with the smaller shift
   - Shift the device with the higher index (the one that's "later" in the
     canonical order) by that amount
   - If the shifted device is part of a symmetric pair, shift its partner
     by the mirror amount
   - If the shifted device is part of an alignment group, shift all members

4. Repeat until no overlaps remain or a max iteration count is reached

**Convergence guarantee:** Each iteration resolves at least one overlap and
creates at most one new overlap (the shifted device may now overlap a third
device). With n devices, at most O(n^2) iterations. For practical analog
circuits (n < 100), this converges quickly.

**If overlap removal fails:** After max iterations, report the remaining
overlaps as `DrcViolation` with `DrcViolationKind::Overlap` and let the
gate evaluator handle it as a `GateStatus::Fail` on LocalDrc.

### Phase 3: Spacing repair

After overlap removal, check all spacing violations:

1. Run the DRC oracle's spacing check
2. For each violation, compute the minimum shift to resolve it:
   ```
   needed = required_spacing - actual_spacing
   ```
3. Shift the device with the higher canonical index by `needed` (snapped to
   grid) along the axis of the violation
4. Group-preserve: shift symmetric partners and alignment group members

This phase may introduce new violations (shifting device j might violate
spacing with device k). Re-run the spacing check after all shifts. Iterate
up to max_repair_iterations (default: 5).

### Phase 4: Symmetry repair

After phases 2 and 3, symmetric pairs may have drifted. For each symmetric
pair (a, b):

1. Recompute the axis position:
   ```
   axis = (x[a] + w[a]/2 + x[b] + w[b]/2) / 2
   axis = round_to_grid(axis, grid_nm)
   ```
2. Adjust positions:
   ```
   x[a] = round_to_grid(axis - w[a]/2, grid_nm)
   x[b] = round_to_grid(2 * axis - x[a] - w[a], grid_nm)
   y[b] = y[a]
   ```
3. Verify orientation: b's orientation should be `MY(a's orientation)`. If
   not, fix it.

For symmetric groups, use the shared axis.

**If symmetry repair introduces new overlaps or spacing violations:** Run
phases 2 and 3 again. This is the "repair loop" — it alternates between
DRC repair and constraint repair until convergence.

### Phase 5: Alignment repair

For each alignment group:
1. Compute the median coordinate (more robust than mean for avoiding
   cascading shifts)
2. Snap to grid
3. Set all member coordinates to this value
4. Check for new overlaps/spacing violations

## The repair loop

The phases don't run in strict sequence — they iterate:

```
for round in 0..MAX_REPAIR_ROUNDS:
    snap_to_grid()
    violations = drc_oracle.check()
    if violations.is_empty():
        break
    remove_overlaps(violations.overlaps)
    repair_spacings(violations.spacings)
    repair_symmetry()
    repair_alignment()
```

`MAX_REPAIR_ROUNDS` = 10. If the loop doesn't converge, the remaining
violations are reported as gate failures.

## Output

### Success

- Legalized `(x, y, orient)` for each device
- All coordinates on grid
- No overlaps
- All spacing rules satisfied (within the coded rule set)
- All symmetric pairs properly mirrored
- All alignment groups properly aligned

The `solver_path` in the certificate becomes
`"sa-sequence-pair->legalize"`.

### Partial success

- Some spacing violations remain but overlaps are resolved
- Constraint invariants hold
- GateStatus: LocalDrc = Fail, IntentSatisfaction = Pass
- The conflict core lists the remaining spacing violations

### Failure

- Overlaps could not be resolved (devices are too tightly packed)
- OR: constraint repair creates an irreconcilable loop
- GateStatus: LocalDrc = Fail
- The conflict core lists all violations with their subjects and predicates
- The `infeasible_core` in the certificate is populated

## Group-preserving invariant

The critical invariant throughout legalization is:

> No legalization step may move one device of a constrained group without
> moving the other members to maintain the group's geometric invariant.

This means:
- Shifting one side of a symmetric pair → shift the other side by the
  mirrored amount
- Shifting one member of an alignment group → shift all members by the
  same amount (on the constrained axis)
- Shifting one member of an ordered pair → verify the order is still
  satisfied; if not, shift the other member too

The implementation achieves this by tagging each device with its group
memberships and looking them up before any shift.

### `DeviceGroups`

| Field | Type | Description |
|-------|------|-------------|
| `symmetric_partner` | `Option<usize>` | If in a symmetric pair, the partner's index |
| `symmetric_axis` | `Option<Axis>` | Symmetry axis direction |
| `align_groups` | `Vec<usize>` | Indices into IntentGraph.align_groups |
| `order_before` | `Vec<usize>` | Devices this one must precede |
| `order_after` | `Vec<usize>` | Devices this one must follow |
| `guard_ring` | `Option<usize>` | If enclosed by a guard ring, the ring ID |

Built once from the IntentGraph before legalization starts. Consulted on
every shift operation.
