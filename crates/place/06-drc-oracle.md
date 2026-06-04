# DRC Oracle — Local Design Rule Checking

> **Shared crate:** DRC rule predicates and violation types (`DrcViolation`,
> `DrcViolationKind`, `RulePredicate`) come from `crates/verify`
> (`philis_verify`). The placement crate imports these types and uses the
> shared predicates for rule checking. The checking loop itself (broad-phase
> R-tree query, narrow-phase per-pair checking, SA cost integration) remains
> in `crates/place` because it is placement-specific.

## Purpose

The DRC oracle is the evaluator for the `AcceptanceGate::LocalDrc` gate. It
checks placement-owned design rule violations on a placement snapshot. It is
the cheapest high-yield rejector (runs before DeviceEquivalence in the realized
gate order) and one of only two repairable gates.

## Scope of "local DRC"

The DRC oracle checks rules that **placement owns** — rules that depend on
where devices are placed relative to each other and to the outline. It does
**not** check:
- Intra-device DRC (that's the primitive generator's responsibility — if the
  primitive is correctly generated, its internal geometry is DRC-clean)
- Routing DRC (the router owns that)
- Rules requiring extraction (PEX/EM/LDE — those are G5/R6)

### What the oracle checks

| Check | Rule family | Sky130 example |
|-------|------------|----------------|
| **Non-overlap** | No two devices overlap | Basic geometry |
| **Minimum spacing** | Edge-to-edge distance between same-layer shapes of different devices | diff.3 (270nm), poly.2 (210nm), li.3 (170nm) |
| **Outline containment** | All devices inside the host outline | If outline is specified |
| **Obstacle clearance** | No device overlaps a placement obstacle | User-specified obstacles |
| **Grid snap** | All device coordinates are multiples of the manufacturing grid | grid_nm = 5 for Sky130 |
| **Well spacing** | nwell-to-nwell distance between PFET devices in separate wells | nwell.1 (1270nm) |
| **Well enclosure** | psdm region enclosed by nwell for PFET | nwell.4 (180nm) |
| **Implant spacing** | nsdm-to-psdm spacing | Rule dependent on process |
| **Same-well grouping** | PFET devices in the same logical group share a well | Reduces well count |

## Architecture

### Input

| Field | Type | Description |
|-------|------|-------------|
| `placements` | `&[DevicePlacement]` | Current device positions and orientations |
| `primitives` | `&PrimitiveCatalog` | Device geometry templates |
| `technology` | `&CompiledTechnology` | Spacing/width/enclosure rules |
| `outline` | `Option<(i64, i64)>` | Host outline, if fixed |
| `obstacles` | `&[Rect]` | Forbidden regions |

### Output

A `GateEvaluation` with:
- `gate: AcceptanceGate::LocalDrc`
- `status: GateStatus::Pass | Fail | Degraded`
- `evidence: String` — human-readable violation list or "N violations" summary

Plus a `Vec<DrcViolation>` for the legalization pass to consume:

### `DrcViolation`

| Field | Type | Description |
|-------|------|-------------|
| `kind` | `DrcViolationKind` | What rule was violated |
| `device_a` | `usize` | First device index |
| `device_b` | `Option<usize>` | Second device (None for outline/grid violations) |
| `layer` | `u16` | Layer where the violation occurs |
| `actual_nm` | `i64` | Measured value (spacing, enclosure, etc.) |
| `required_nm` | `i64` | Rule minimum |
| `region` | `Rect` | Bounding box of the violation region |

### `DrcViolationKind`

```
enum DrcViolationKind {
    Overlap,          // two device shapes on the same layer overlap
    Spacing,          // spacing between shapes on the same layer is too small
    OutlineExceed,    // device extends beyond the outline
    ObstacleOverlap,  // device overlaps a placement obstacle
    GridSnap,         // coordinate is not a multiple of grid_nm
    WellSpacing,      // nwell-to-nwell distance too small
    WellEnclosure,    // psdm not enclosed by nwell
    ImplantSpacing,   // nsdm-to-psdm distance too small
}
```

## Checking algorithm

### Phase 1: Broad-phase (bounding box)

Use an R-tree spatial index to find candidate pairs. For each device, insert
its bounding box (expanded by the maximum same-layer spacing rule) into the
R-tree. Query for intersections.

```
for each device i:
    bbox_i = placement[i].bbox expanded by max_spacing
    candidates = rtree.query(bbox_i)
    for each candidate j where j > i:  // avoid double-checking
        run narrow-phase on (i, j)
```

The R-tree is rebuilt from scratch for each oracle invocation. For the SA
inner loop, we can use an incremental approach: maintain the R-tree and update
only the moved devices. But for V1, rebuild is fine — the device count in
analog blocks is small (10–100).

### Phase 2: Narrow-phase (per-pair)

For each candidate pair (i, j):

1. **Overlap check:** For each layer present in both devices, check if any
   polygon of device i on that layer overlaps any polygon of device j on the
   same layer. Overlap = intersection area > 0.

   For axis-aligned rectangles, overlap is:
   ```
   overlap = max(0, min(x1_a, x1_b) - max(x0_a, x0_b))
           * max(0, min(y1_a, y1_b) - max(y0_a, y0_b))
   ```

2. **Spacing check:** For each layer, find the minimum edge-to-edge distance
   between polygons of device i and device j on that layer. Compare against
   the spacing rule for that layer.

   For axis-aligned rectangles, edge-to-edge distance is:
   ```
   dx = max(0, max(x0_a, x0_b) - min(x1_a, x1_b))
   dy = max(0, max(y0_a, y0_b) - min(y1_a, y1_b))
   distance = max(dx, dy)   // Manhattan distance for placement DRC
   ```

   (Using Manhattan distance, not Euclidean, because DRC spacing rules are
   typically Manhattan-measured in placement.)

3. **Well-specific checks:** If both devices are PFET (or both NFET with
   different wells), check nwell-to-nwell spacing. If they share a well,
   check that their nwell regions can merge (contiguous or overlapping).

### Phase 3: Global checks

1. **Grid snap:** For each device, check `x % grid_nm == 0` and
   `y % grid_nm == 0`. This is O(n) and doesn't need the R-tree.

2. **Outline containment:** For each device, check that its bounding box
   is inside `(0, 0, outline_w, outline_h)`. If no outline is specified,
   skip.

3. **Obstacle clearance:** For each obstacle, query the R-tree for devices
   whose bounding boxes intersect the obstacle.

## Incremental DRC for the SA inner loop

The full DRC check is O(n log n + k) where k is the number of candidate pairs.
For the SA inner loop, we only need to recheck the moved device(s):

1. After a move affecting device i:
   - Remove device i from the R-tree
   - Re-insert with new bbox
   - Query for new candidates involving i
   - Recheck only pairs involving i

2. After a symmetric move affecting (i, j):
   - Remove both, re-insert both, recheck pairs involving either

This gives O(log n + k_local) per move, where k_local is the number of
neighbors of the moved device.

### SA cost integration

For the SA cost function, the DRC oracle produces a scalar penalty:
```
drc_penalty = sum over all violations v:
    violation_weight(v.kind) * max(0, v.required_nm - v.actual_nm)
```

Where `violation_weight` is:
- Overlap: 10.0 (severe — overlaps are never legal)
- Spacing: 1.0 per nm of violation
- OutlineExceed: 5.0 per nm
- GridSnap: 100.0 (should be free to fix)

This penalty is multiplied by `gamma` in the SA cost function, so violations
are heavily penalized but the SA can still explore through them at high
temperature.

## Coverage classification

The DRC oracle's coverage report for V1 (Sky130):

| Rule family | Count coded | Count total (estimate) | Coverage |
|-------------|-------------|----------------------|----------|
| Same-layer spacing | ~8 (diff, poly, li, m1-m5) | ~40 | `Partial` |
| Well spacing | ~3 | ~10 | `Partial` |
| Well enclosure | ~2 | ~8 | `Partial` |
| Implant spacing | ~2 | ~6 | `Partial` |
| Overlap (all layers) | all | all | `Coded` |
| Grid snap | all | all | `Coded` |
| Density | 0 | ~5 | `Missing` |
| Antenna | 0 | ~10 | `Missing` |
| Via enclosure | 0 | ~15 | `Missing` (placement doesn't own vias) |
| Wide-wire / notch | 0 | ~20 | `Missing` |

Overall `GateStatus` for LocalDrc:
- If any Overlap or GridSnap violation: `Fail`
- If spacing violations: `Fail`
- If no violations but coverage is `Partial`: `Degraded`
  (we can't claim signoff because we haven't checked all rules)
- If no violations and coverage is `Coded`: `Pass`

For V1, the gate will be `Degraded` at best (not all rules coded). This is
honest and correct — the `degraded_confidence` field in the certificate
reflects it.

## Future: full DRC

The path to `GateStatus::Pass` on the LocalDrc gate:

1. Parse the Calibre/Klayout DRC deck for Sky130
2. Extract all placement-relevant rules into `SpacingRule` / etc.
3. Implement the missing rule families (wide-wire, notch, density)
4. Run the full check
5. When every coded rule passes, the gate can be `Pass`

The oracle never claims `Pass` for a rule family with `Coverage::Missing`.
That's the certificate's honesty guarantee.
