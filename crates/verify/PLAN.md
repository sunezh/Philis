# Verification Predicates — Implementation Plan

**Status:** plan  
**Target crate:** `crates/verify` (crate name `philis_verify`)  
**Dependencies:** `philis-geom` (Rect, SpatialIndex, Coord), `philis-tech` (RuleSet, LayerId, ExtractModel)  
**Consumers:** `crates/place` (device-pair broad/narrow-phase loop), `crates/route` (commit-time incremental delta loop)  
**Execution model:** predicate library — stateless functions + lightweight state containers; checking loops stay in consumers

---

## 1. Overview

### What the crate owns

`philis-verify` provides the **verification predicates, types, and state containers** that both the placement and routing engines share:

- **DRC rule predicates** — pure functions that take two shapes (or a shape + rule parameters) and return a violation or `None`. No spatial indexing, no broad-phase; the caller owns those.
- **DRC violation types** — `DrcViolation`, `ViolationKind`, `DrcDelta`, `DrcRegion` — the canonical representation of design rule failures with all metadata needed for repair.
- **Influence radius computation** — `r_max` from a `RuleSet`, used by both consumers to size their broad-phase query windows.
- **LVS connectivity tracking** — `ConnectivityTracker` (union-find with net labels), `LvsResult` (opens/shorts/mismatches).
- **PEX lumped-element surrogates** — `NetParasitics`, wire/via/coupling models, incremental add/remove, outlier detection.
- **EM/IR/Antenna checking** — Black's equation predicates, resistive network IR drop, antenna ratio checks.
- **Coverage tracking** — per-rule-family counters for the constraint certificate.

### What stays in consumers

- **Place** owns the broad-phase R-tree loop that generates device-pair candidates and calls DRC predicates in its narrow-phase. Place also owns its own spatial index and device-pair iteration order.
- **Route** owns the incremental commit-time delta loop. When a wire segment is added or removed, route computes the affected `DrcRegion`, queries its spatial index for nearby geometry, and calls DRC predicates on each candidate pair. Route also owns its net-based union-find integration and parasitic accumulation across commit/rollback cycles.
- Both consumers own their domain-specific repair logic. The verify crate reports violations; the consumer decides how to fix them.

### Dependency graph

```
philis-geom ──┐
              ├──→ philis-verify
philis-tech ──┘       ↑          ↑
                       │          │
                 philis-place  philis-route
```

---

## 2. Module Map (Target `src/` Layout)

```
src/
  lib.rs                     — public facade, re-exports all public types
  drc/
    mod.rs                   — DrcViolation, ViolationKind, DrcDelta, DrcRegion, coverage
    predicates.rs            — check_spacing, check_width, check_enclosure, check_area,
                               check_eol_spacing, check_prl_spacing, check_overlap,
                               check_cut_spacing, check_same_net_notch, check_grid_snap,
                               check_well_spacing, check_well_enclosure,
                               check_implant_spacing, check_outline_exceed
    influence.rs             — r_max computation from RuleSet
  lvs/
    mod.rs                   — ConnectivityTracker, LvsResult
    uf.rs                    — union-find with net labels (rank + path compression)
  pex/
    mod.rs                   — NetParasitics, PexOutlier, incremental state
    wire.rs                  — wire resistance, ground capacitance
    coupling.rs              — same-layer coupling, interlayer coupling
    via.rs                   — via resistance from tech model
  em/
    mod.rs                   — EmViolation, AntennaViolation, IrDropResult
    black.rs                 — Black's equation, Blech filter, J_max check
    ir.rs                    — resistive network IR drop (sparse linear solve)
    antenna.rs               — metal_area / gate_oxide_area ratio check
  coverage.rs                — RuleFamilyCoverage tracker
```

---

## 3. DRC Module

### 3.1 ViolationKind Enum

```rust
/// Closed taxonomy of all DRC violation categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViolationKind {
    /// Two shapes on the same layer overlap when they should not.
    Overlap,
    /// Distance between two shapes is less than the required spacing.
    Spacing,
    /// A shape's width (or height) is below the layer's minimum width.
    Width,
    /// An enclosing shape does not extend far enough beyond the enclosed shape.
    Enclosure,
    /// A shape's area is below the layer's minimum area.
    Area,
    /// End-of-line spacing rule violated (spacing depends on run length at line end).
    EOL,
    /// Parallel run length (PRL) dependent spacing rule violated.
    PRL,
    /// Spacing between via cuts is below the minimum.
    CutSpacing,
    /// Same-net notch: a narrow channel between shapes on the same net.
    SameNetNotch,
    /// Shape edge does not land on the manufacturing grid.
    GridSnap,
    /// Shape extends beyond the outline / boundary of the cell.
    OutlineExceed,
    /// Well-to-well spacing violation (n-well to n-well or p-well to p-well).
    WellSpacing,
    /// Well enclosure of diffusion or implant is insufficient.
    WellEnclosure,
    /// Implant-to-implant spacing violation (e.g., NPlus to PPlus).
    ImplantSpacing,
}
```

### 3.2 DrcViolation Type

```rust
/// A single DRC violation with full metadata for diagnosis and repair.
#[derive(Debug, Clone)]
pub struct DrcViolation {
    /// Which rule predicate was violated.
    pub kind: ViolationKind,
    /// The rule identifier from the tech model (e.g., "M1.S.1" for M1 spacing rule 1).
    pub rule: RuleId,
    /// Layer on which the violation occurs.
    pub layer: LayerId,
    /// The two shapes (or shape + boundary) involved. Second is `None` for
    /// single-shape violations (Width, Area, GridSnap, OutlineExceed).
    pub shapes: (ShapeRef, Option<ShapeRef>),
    /// Bounding box of the violation region (for UI highlighting and repair scoping).
    pub region: Rect,
    /// The rule's required value (e.g., minimum spacing in nm).
    pub required: i64,
    /// The actual measured value.
    pub actual: i64,
    /// Whether the two shapes belong to the same net (affects some rule predicates).
    pub same_net: bool,
}

/// Opaque reference to a shape in the caller's geometry store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShapeRef(pub u64);

/// Opaque rule identifier from the tech model.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RuleId(pub String);
```

### 3.3 Rule Predicates

Each predicate is a pure function. It receives geometry and rule parameters, and returns `Option<DrcViolation>`. The caller is responsible for broad-phase filtering; predicates assume the candidate pair is plausible.

#### `check_spacing`

```rust
/// Basic spacing check between two rectangles on the same layer.
///
/// Inputs:
///   - `a`, `b`: axis-aligned rectangles (Rect from philis-geom)
///   - `a_ref`, `b_ref`: opaque shape references for violation reporting
///   - `layer`: the layer both shapes reside on
///   - `rule`: the rule identifier
///   - `min_spacing`: minimum required spacing in nm (i64)
///   - `same_net`: whether both shapes belong to the same net
///
/// Output: `Some(DrcViolation)` if the edge-to-edge distance between a and b
///   is less than `min_spacing` and they do not overlap. `None` if the spacing
///   is satisfied or the shapes overlap (overlap is a separate check).
///
/// Invariants:
///   - Symmetric: check_spacing(a, b, ...) == check_spacing(b, a, ...)
///   - Never fires on overlapping shapes (use check_overlap for that)
///   - Distance is the minimum of all four edge-pair distances
pub fn check_spacing(
    a: &Rect, b: &Rect,
    a_ref: ShapeRef, b_ref: ShapeRef,
    layer: LayerId, rule: RuleId,
    min_spacing: i64, same_net: bool,
) -> Option<DrcViolation>;
```

#### `check_width`

```rust
/// Minimum width check for a single rectangle.
///
/// Inputs:
///   - `shape`: the rectangle to check
///   - `shape_ref`: opaque reference for violation reporting
///   - `layer`: the layer the shape resides on
///   - `rule`: the rule identifier
///   - `min_width`: minimum required width in nm (i64)
///
/// Output: `Some(DrcViolation)` if `min(shape.width(), shape.height()) < min_width`.
///
/// Invariants:
///   - A square shape with side == min_width passes
///   - Width is always the smaller dimension (orientation-independent)
pub fn check_width(
    shape: &Rect, shape_ref: ShapeRef,
    layer: LayerId, rule: RuleId,
    min_width: i64,
) -> Option<DrcViolation>;
```

#### `check_enclosure`

```rust
/// Enclosure check: an outer shape must enclose an inner shape by at least
/// `min_enclosure` on all four sides.
///
/// Inputs:
///   - `outer`: the enclosing rectangle (e.g., metal over via, well over diffusion)
///   - `inner`: the enclosed rectangle
///   - `outer_ref`, `inner_ref`: opaque shape references
///   - `layer`: the layer for the violation report (typically the outer's layer)
///   - `rule`: the rule identifier
///   - `min_enclosure`: minimum required enclosure in nm (i64)
///
/// Output: `Some(DrcViolation)` if any edge of `inner` is closer than
///   `min_enclosure` to the corresponding edge of `outer`.
///
/// Invariants:
///   - If inner is entirely outside outer, this still fires (enclosure = negative)
///   - Enclosure is measured per-side: left, right, top, bottom
///   - The `actual` field in the violation is the minimum enclosure across all sides
pub fn check_enclosure(
    outer: &Rect, inner: &Rect,
    outer_ref: ShapeRef, inner_ref: ShapeRef,
    layer: LayerId, rule: RuleId,
    min_enclosure: i64,
) -> Option<DrcViolation>;
```

#### `check_area`

```rust
/// Minimum area check for a single rectangle.
///
/// Inputs:
///   - `shape`: the rectangle to check
///   - `shape_ref`: opaque reference for violation reporting
///   - `layer`: the layer the shape resides on
///   - `rule`: the rule identifier
///   - `min_area`: minimum required area in nm^2 (i64)
///
/// Output: `Some(DrcViolation)` if `shape.area() < min_area`.
///
/// Invariants:
///   - Area is always non-negative for valid rectangles
///   - A shape with area == min_area passes
pub fn check_area(
    shape: &Rect, shape_ref: ShapeRef,
    layer: LayerId, rule: RuleId,
    min_area: i64,
) -> Option<DrcViolation>;
```

#### `check_eol_spacing`

```rust
/// End-of-line (EOL) spacing check. When a metal edge terminates (its run
/// length is short), the required spacing to adjacent shapes increases.
///
/// Inputs:
///   - `a`, `b`: the two rectangles
///   - `a_ref`, `b_ref`: opaque shape references
///   - `layer`: the layer both shapes reside on
///   - `rule`: the rule identifier
///   - `eol_width`: if a shape's narrow dimension is <= eol_width, the shape
///     has an end-of-line edge
///   - `eol_space`: the enlarged spacing required near EOL edges
///   - `eol_within`: the search region extent perpendicular to the EOL edge
///   - `same_net`: whether both shapes belong to the same net
///
/// Output: `Some(DrcViolation)` if an EOL edge of `a` or `b` is within
///   `eol_within` of the other shape and the spacing is less than `eol_space`.
///
/// Invariants:
///   - Only fires when at least one shape has an EOL edge (narrow dim <= eol_width)
///   - If neither shape has an EOL edge, returns None (basic spacing check applies)
///   - Symmetric in a, b
pub fn check_eol_spacing(
    a: &Rect, b: &Rect,
    a_ref: ShapeRef, b_ref: ShapeRef,
    layer: LayerId, rule: RuleId,
    eol_width: i64, eol_space: i64, eol_within: i64,
    same_net: bool,
) -> Option<DrcViolation>;
```

#### `check_prl_spacing`

```rust
/// Parallel run length (PRL) dependent spacing check. When two shapes run
/// parallel for a long enough distance, the required spacing increases.
///
/// Inputs:
///   - `a`, `b`: the two rectangles
///   - `a_ref`, `b_ref`: opaque shape references
///   - `layer`: the layer both shapes reside on
///   - `rule`: the rule identifier
///   - `prl_threshold`: minimum parallel overlap length that triggers enlarged spacing
///   - `prl_spacing`: the enlarged spacing required when PRL >= prl_threshold
///   - `same_net`: whether both shapes belong to the same net
///
/// Output: `Some(DrcViolation)` if the parallel run length between `a` and `b`
///   is >= `prl_threshold` and the spacing is less than `prl_spacing`.
///
/// Invariants:
///   - PRL is the overlap of the two shapes' projections onto the axis parallel
///     to their closest edges
///   - If PRL < prl_threshold, returns None (basic spacing check applies)
///   - Symmetric in a, b
pub fn check_prl_spacing(
    a: &Rect, b: &Rect,
    a_ref: ShapeRef, b_ref: ShapeRef,
    layer: LayerId, rule: RuleId,
    prl_threshold: i64, prl_spacing: i64,
    same_net: bool,
) -> Option<DrcViolation>;
```

#### `check_overlap`

```rust
/// Overlap check: two shapes on the same layer must not overlap unless they
/// belong to the same net (merge semantics).
///
/// Inputs:
///   - `a`, `b`: the two rectangles
///   - `a_ref`, `b_ref`: opaque shape references
///   - `layer`: the layer both shapes reside on
///   - `rule`: the rule identifier
///   - `same_net`: whether both shapes belong to the same net
///
/// Output: `Some(DrcViolation)` if `a` and `b` overlap and `same_net` is false.
///   Returns `None` if they do not overlap or if same_net is true (same-net
///   shapes are merged, not flagged).
///
/// Invariants:
///   - Overlap means intersection area > 0 (touching edges are not overlap)
///   - Symmetric in a, b
///   - The `actual` field is 0 (the shapes are at distance 0); `required` is
///     the minimum spacing that would be needed
pub fn check_overlap(
    a: &Rect, b: &Rect,
    a_ref: ShapeRef, b_ref: ShapeRef,
    layer: LayerId, rule: RuleId,
    same_net: bool,
) -> Option<DrcViolation>;
```

#### `check_cut_spacing`

```rust
/// Via cut spacing check. Minimum spacing between two via cut shapes.
///
/// Inputs:
///   - `a`, `b`: the two via cut rectangles
///   - `a_ref`, `b_ref`: opaque shape references
///   - `layer`: the cut layer
///   - `rule`: the rule identifier
///   - `min_cut_spacing`: minimum required spacing between cuts in nm (i64)
///   - `same_net`: whether both cuts belong to the same net
///
/// Output: `Some(DrcViolation)` if edge-to-edge distance < min_cut_spacing.
///
/// Invariants:
///   - Same semantics as check_spacing but on a cut layer
///   - Symmetric in a, b
pub fn check_cut_spacing(
    a: &Rect, b: &Rect,
    a_ref: ShapeRef, b_ref: ShapeRef,
    layer: LayerId, rule: RuleId,
    min_cut_spacing: i64, same_net: bool,
) -> Option<DrcViolation>;
```

#### `check_same_net_notch`

```rust
/// Same-net notch check. A narrow channel (notch) between two shapes on the
/// same net can cause manufacturing issues even though overlap is allowed.
///
/// Inputs:
///   - `a`, `b`: the two rectangles (must be same-net)
///   - `a_ref`, `b_ref`: opaque shape references
///   - `layer`: the layer both shapes reside on
///   - `rule`: the rule identifier
///   - `min_notch_width`: minimum allowed notch width in nm (i64)
///   - `max_notch_length`: maximum notch length below which the rule applies (i64)
///
/// Output: `Some(DrcViolation)` if the two shapes form a notch narrower than
///   `min_notch_width` and shorter than `max_notch_length`.
///
/// Invariants:
///   - Only meaningful for same-net shapes (caller must verify)
///   - A notch exists when shapes are close but not overlapping, and their
///     projections overlap on one axis
///   - Symmetric in a, b
pub fn check_same_net_notch(
    a: &Rect, b: &Rect,
    a_ref: ShapeRef, b_ref: ShapeRef,
    layer: LayerId, rule: RuleId,
    min_notch_width: i64, max_notch_length: i64,
) -> Option<DrcViolation>;
```

#### `check_grid_snap`

```rust
/// Grid snap check. All shape edges must land on the manufacturing grid.
///
/// Inputs:
///   - `shape`: the rectangle to check
///   - `shape_ref`: opaque reference for violation reporting
///   - `layer`: the layer the shape resides on
///   - `rule`: the rule identifier
///   - `grid_pitch`: the manufacturing grid pitch in nm (i64)
///
/// Output: `Some(DrcViolation)` if any edge coordinate of `shape` is not a
///   multiple of `grid_pitch`.
///
/// Invariants:
///   - Checks all four coordinates: x_lo, y_lo, x_hi, y_hi
///   - The `actual` field is the worst-case offset from the grid
///   - grid_pitch must be > 0 (panics otherwise in debug builds)
pub fn check_grid_snap(
    shape: &Rect, shape_ref: ShapeRef,
    layer: LayerId, rule: RuleId,
    grid_pitch: i64,
) -> Option<DrcViolation>;
```

#### `check_well_spacing`

```rust
/// Well-to-well spacing check. Minimum spacing between two well shapes
/// (e.g., two separate N-well regions).
///
/// Inputs:
///   - `a`, `b`: the two well rectangles
///   - `a_ref`, `b_ref`: opaque shape references
///   - `layer`: the well layer
///   - `rule`: the rule identifier
///   - `min_well_spacing`: minimum required spacing in nm (i64)
///
/// Output: `Some(DrcViolation)` if edge-to-edge distance < min_well_spacing.
///
/// Invariants:
///   - Same geometry math as check_spacing
///   - Symmetric in a, b
///   - Does not apply to overlapping wells (those are merged)
pub fn check_well_spacing(
    a: &Rect, b: &Rect,
    a_ref: ShapeRef, b_ref: ShapeRef,
    layer: LayerId, rule: RuleId,
    min_well_spacing: i64,
) -> Option<DrcViolation>;
```

#### `check_well_enclosure`

```rust
/// Well enclosure of diffusion. The well must extend beyond the diffusion
/// region by at least `min_well_enc` on all sides.
///
/// Inputs:
///   - `well`: the well rectangle (outer)
///   - `diffusion`: the diffusion rectangle (inner)
///   - `well_ref`, `diff_ref`: opaque shape references
///   - `layer`: the well layer (for violation reporting)
///   - `rule`: the rule identifier
///   - `min_well_enc`: minimum required enclosure in nm (i64)
///
/// Output: `Some(DrcViolation)` if any side's enclosure < min_well_enc.
///
/// Invariants:
///   - Delegates to the same enclosure math as check_enclosure
///   - Fires the WellEnclosure violation kind
pub fn check_well_enclosure(
    well: &Rect, diffusion: &Rect,
    well_ref: ShapeRef, diff_ref: ShapeRef,
    layer: LayerId, rule: RuleId,
    min_well_enc: i64,
) -> Option<DrcViolation>;
```

#### `check_implant_spacing`

```rust
/// Implant-to-implant spacing check. Minimum spacing between two implant
/// regions (e.g., NPlus to PPlus).
///
/// Inputs:
///   - `a`, `b`: the two implant rectangles
///   - `a_ref`, `b_ref`: opaque shape references
///   - `layer`: the implant layer
///   - `rule`: the rule identifier
///   - `min_implant_spacing`: minimum required spacing in nm (i64)
///
/// Output: `Some(DrcViolation)` if edge-to-edge distance < min_implant_spacing.
///
/// Invariants:
///   - Same geometry math as check_spacing
///   - Symmetric in a, b
pub fn check_implant_spacing(
    a: &Rect, b: &Rect,
    a_ref: ShapeRef, b_ref: ShapeRef,
    layer: LayerId, rule: RuleId,
    min_implant_spacing: i64,
) -> Option<DrcViolation>;
```

#### `check_outline_exceed`

```rust
/// Outline exceedance check. A shape must lie entirely within the cell boundary.
///
/// Inputs:
///   - `shape`: the rectangle to check
///   - `shape_ref`: opaque reference for violation reporting
///   - `boundary`: the cell outline rectangle
///   - `layer`: the layer of the shape
///   - `rule`: the rule identifier
///
/// Output: `Some(DrcViolation)` if any part of `shape` extends beyond `boundary`.
///
/// Invariants:
///   - The `actual` field is the maximum exceedance distance on any side
///   - A shape touching the boundary edge (but not exceeding it) passes
pub fn check_outline_exceed(
    shape: &Rect, shape_ref: ShapeRef,
    boundary: &Rect,
    layer: LayerId, rule: RuleId,
) -> Option<DrcViolation>;
```

### 3.4 Incremental Checking Infrastructure

#### `DrcRegion`

```rust
/// A dilated check region: the bounding box of a modified shape expanded by
/// `r_max`. Any shape whose bounding box intersects this region is a candidate
/// for narrow-phase DRC checking against the modified shape.
///
/// The consumer (place or route) constructs a DrcRegion from the modified
/// shape and queries its spatial index for overlapping shapes.
#[derive(Debug, Clone)]
pub struct DrcRegion {
    /// The original shape's bounding box before dilation.
    pub origin: Rect,
    /// The dilated bounding box (origin expanded by r_max on all sides).
    pub dilated: Rect,
    /// The influence radius used for dilation.
    pub r_max: i64,
}

impl DrcRegion {
    /// Create a new DrcRegion by dilating `origin` by `r_max` on all sides.
    ///
    /// Invariants:
    ///   - dilated.x_lo == origin.x_lo - r_max
    ///   - dilated.y_lo == origin.y_lo - r_max
    ///   - dilated.x_hi == origin.x_hi + r_max
    ///   - dilated.y_hi == origin.y_hi + r_max
    pub fn new(origin: Rect, r_max: i64) -> Self;
}
```

#### `DrcDelta`

```rust
/// Diff of DRC violations between two states. Used by incremental checking:
/// when a shape is added or removed, the consumer runs narrow-phase checks
/// in the affected DrcRegion and produces a DrcDelta.
#[derive(Debug, Clone, Default)]
pub struct DrcDelta {
    /// Violations that appeared (new violations not present before the edit).
    pub added: Vec<DrcViolation>,
    /// Violations that disappeared (violations present before but resolved by the edit).
    pub removed: Vec<DrcViolation>,
}

impl DrcDelta {
    /// Merge another delta into this one.
    pub fn merge(&mut self, other: DrcDelta);

    /// Net change in violation count.
    pub fn net_change(&self) -> i64;

    /// True if this delta introduces no new violations.
    pub fn is_clean(&self) -> bool;
}
```

### 3.5 Influence Radius Computation

```rust
/// Compute the maximum influence radius from a RuleSet. This is the largest
/// distance at which a shape modification can create or resolve a DRC violation.
///
/// r_max = max(
///     max spacing rule value (including PRL-dependent and width-dependent),
///     max EOL within distance,
///     max enclosure rule value,
///     max well spacing,
///     max implant spacing,
/// )
///
/// Invariants:
///   - r_max >= 0
///   - r_max is computed once per technology, cached by the consumer
///   - If the RuleSet has no rules, r_max == 0
pub fn compute_r_max(rules: &RuleSet) -> i64;
```

### 3.6 Coverage Tracking per Rule Family

```rust
/// Tracks how many violations of each ViolationKind have been checked and found.
/// Used by the constraint certificate to report per-rule-family coverage.
#[derive(Debug, Clone, Default)]
pub struct RuleFamilyCoverage {
    /// Number of candidate pairs checked per violation kind.
    pub checked: HashMap<ViolationKind, u64>,
    /// Number of violations found per violation kind.
    pub violations: HashMap<ViolationKind, u64>,
}

impl RuleFamilyCoverage {
    /// Record a check (whether or not it produced a violation).
    pub fn record_check(&mut self, kind: ViolationKind, violation: bool);

    /// Total checks across all families.
    pub fn total_checked(&self) -> u64;

    /// Total violations across all families.
    pub fn total_violations(&self) -> u64;

    /// Rule families that have never been checked (potential coverage gaps).
    pub fn unchecked_families(&self) -> Vec<ViolationKind>;
}
```

---

## 4. LVS Module

### 4.1 ConnectivityTracker

A union-find data structure augmented with net labels. Each element (terminal, shape endpoint, via landing) belongs to a component. Components carry net labels assigned by the schematic.

```rust
/// Union-find with net labels for LVS connectivity extraction.
///
/// Elements are identified by `u32` indices. Each element may optionally carry
/// a `NetId` label from the schematic. The tracker supports union, find,
/// same-component queries, and invariant checking.
#[derive(Debug, Clone)]
pub struct ConnectivityTracker {
    parent: Vec<u32>,
    rank: Vec<u8>,
    labels: Vec<Option<NetId>>,
    count: u32,
}

/// Opaque net identifier from the schematic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NetId(pub u32);
```

### 4.2 Operations

```rust
impl ConnectivityTracker {
    /// Create a new tracker with `n` elements, each in its own component.
    pub fn new(n: u32) -> Self;

    /// Assign a net label to element `x`. Panics if `x` already has a
    /// different label (label conflicts are LVS errors, caught earlier).
    pub fn label(&mut self, x: u32, net: NetId);

    /// Union elements `x` and `y` into the same component.
    /// Uses union-by-rank with path compression.
    ///
    /// Invariants:
    ///   - If both components have labels, they must be the same NetId
    ///     (otherwise this is a short — recorded, not prevented)
    ///   - After union, find(x) == find(y)
    ///   - Component count decreases by 1 if x and y were in different components
    pub fn union(&mut self, x: u32, y: u32) -> UnionResult;

    /// Find the representative of element `x` with path compression.
    pub fn find(&mut self, x: u32) -> u32;

    /// Check whether `x` and `y` are in the same component.
    pub fn same_component(&mut self, x: u32, y: u32) -> bool;

    /// Return all elements in the same component as `x`.
    pub fn component_members(&mut self, x: u32) -> Vec<u32>;

    /// Return the net label of the component containing `x`, if any.
    pub fn component_net(&mut self, x: u32) -> Option<NetId>;

    /// Return all elements labeled with `net`.
    pub fn elements_with_net(&self, net: NetId) -> Vec<u32>;

    /// Invariant check: for every net N, all elements labeled N must be in
    /// the same component, and no element in that component may have a
    /// different label. Returns a list of violations.
    ///
    /// Violations:
    ///   - Open: elements with the same net label are in different components
    ///   - Short: elements with different net labels are in the same component
    pub fn invariant_check(&mut self) -> Vec<LvsError>;

    /// Number of distinct components.
    pub fn component_count(&self) -> u32;
}

/// Result of a union operation.
#[derive(Debug, Clone)]
pub enum UnionResult {
    /// Both elements were already in the same component.
    AlreadySame,
    /// Merged two components with compatible labels (or at least one unlabeled).
    Merged,
    /// Merged two components with conflicting labels — this is a short.
    Short { net_a: NetId, net_b: NetId },
}
```

### 4.3 LvsResult

```rust
/// Summary of LVS checking results.
#[derive(Debug, Clone, Default)]
pub struct LvsResult {
    /// Open circuits: net labels that span multiple disconnected components.
    pub opens: Vec<LvsOpen>,
    /// Short circuits: different net labels merged into one component.
    pub shorts: Vec<LvsShort>,
    /// Port mismatches: expected ports not found in the layout, or extra ports.
    pub mismatches: Vec<LvsMismatch>,
}

#[derive(Debug, Clone)]
pub struct LvsOpen {
    pub net: NetId,
    /// The distinct components that should have been one.
    pub components: Vec<Vec<u32>>,
}

#[derive(Debug, Clone)]
pub struct LvsShort {
    /// The nets that were incorrectly merged.
    pub nets: Vec<NetId>,
    /// The component representative where they merged.
    pub component: u32,
}

#[derive(Debug, Clone)]
pub struct LvsMismatch {
    pub kind: MismatchKind,
    pub net: NetId,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MismatchKind {
    MissingPort,
    ExtraPort,
    DeviceCountMismatch,
    DeviceTypeMismatch,
}

impl LvsResult {
    /// True if there are no opens, shorts, or mismatches.
    pub fn is_clean(&self) -> bool;

    /// Total number of errors.
    pub fn error_count(&self) -> usize;
}
```

### 4.4 Connectivity Extraction from Geometry

```rust
/// Extract connectivity from a set of shapes on multiple layers.
///
/// Algorithm:
///   1. For each layer, find all overlapping or touching shape pairs → union
///   2. For each via, union the shapes on the upper and lower layers that
///      the via connects (via landing overlap check)
///   3. Assign net labels from port/pin shapes
///   4. Run invariant_check to detect opens and shorts
///
/// Inputs:
///   - `shapes`: iterator of (ShapeRef, LayerId, Rect, Option<NetId>)
///   - `vias`: iterator of (ShapeRef, cut_layer: LayerId, Rect)
///   - `via_connects`: mapping from cut_layer to (lower_layer, upper_layer)
///
/// Output: `(ConnectivityTracker, LvsResult)`
pub fn extract_connectivity(
    shapes: impl Iterator<Item = (ShapeRef, LayerId, Rect, Option<NetId>)>,
    vias: impl Iterator<Item = (ShapeRef, LayerId, Rect)>,
    via_connects: &HashMap<LayerId, (LayerId, LayerId)>,
) -> (ConnectivityTracker, LvsResult);
```

---

## 5. PEX Module

### 5.1 NetParasitics

```rust
/// Lumped parasitic values for a single net.
///
/// These are first-order estimates suitable for driving placement/routing
/// quality metrics and repair loops. They are NOT signoff-accurate — the
/// constraint certificate records them as approximate evidence.
#[derive(Debug, Clone, Default)]
pub struct NetParasitics {
    /// Total wire resistance (ohms), summed across all segments.
    pub total_r: f64,
    /// Total ground capacitance (femtofarads), summed across all segments.
    pub total_cg: f64,
    /// Total coupling capacitance (femtofarads) to all aggressors.
    pub total_cc: f64,
    /// Total via resistance (ohms), summed across all vias.
    pub total_rvia: f64,
    /// Per-segment breakdown for diagnosis and repair scoping.
    pub segments: Vec<SegmentParasitics>,
}

/// Parasitics for a single wire segment or via.
#[derive(Debug, Clone)]
pub struct SegmentParasitics {
    /// The shape this segment corresponds to.
    pub shape: ShapeRef,
    /// Layer of the segment.
    pub layer: LayerId,
    /// Wire resistance contribution (ohms).
    pub r: f64,
    /// Ground capacitance contribution (fF).
    pub cg: f64,
    /// Coupling capacitance contributions to specific aggressors (fF).
    pub cc: Vec<(NetId, f64)>,
    /// Via resistance contribution, if this is a via segment (ohms).
    pub rvia: f64,
}
```

### 5.2 Wire Resistance

```rust
/// Compute wire resistance for a rectangular segment.
///
/// Formula: R = (length / width) * sheet_resistance
///
/// Where:
///   - length = segment length along the current flow direction (nm)
///   - width = segment width perpendicular to current flow (nm)
///   - sheet_resistance = ohms per square for the layer (from ExtractModel)
///
/// Inputs:
///   - `length`: wire length in nm (i64)
///   - `width`: wire width in nm (i64)
///   - `sheet_r`: sheet resistance in ohms/square (f64)
///
/// Output: resistance in ohms (f64)
///
/// Invariants:
///   - width > 0, length >= 0 (panics in debug on width == 0)
///   - Result is non-negative
///   - Zero length → zero resistance
pub fn wire_resistance(length: i64, width: i64, sheet_r: f64) -> f64;
```

### 5.3 Ground Capacitance

```rust
/// Compute ground (substrate) capacitance for a rectangular segment.
///
/// Formula: Cg = length * width * area_cap + 2 * length * fringe_cap
///
/// Where:
///   - length, width in nm (converted to um internally for pF → fF)
///   - area_cap = areal capacitance (fF/um^2) from ExtractModel
///   - fringe_cap = fringe capacitance per unit length (fF/um) from ExtractModel
///   - The factor of 2 accounts for both edges of the wire
///
/// Inputs:
///   - `length`: wire length in nm (i64)
///   - `width`: wire width in nm (i64)
///   - `area_cap`: areal capacitance in fF/um^2 (f64)
///   - `fringe_cap`: fringe capacitance in fF/um (f64)
///
/// Output: ground capacitance in fF (f64)
///
/// Invariants:
///   - All inputs non-negative
///   - Result is non-negative
pub fn ground_capacitance(length: i64, width: i64, area_cap: f64, fringe_cap: f64) -> f64;
```

### 5.4 Coupling Capacitance

```rust
/// Compute same-layer coupling capacitance between two parallel wire segments.
///
/// Formula: Cc = prl * coupling_cap(layer, spacing)
///
/// Where:
///   - prl = parallel run length (nm) — the overlap of the two segments'
///     projections onto the axis along which they run
///   - coupling_cap(layer, spacing) = a lookup function from ExtractModel
///     that returns fF/um for the given layer and edge-to-edge spacing
///
/// Inputs:
///   - `prl`: parallel run length in nm (i64)
///   - `spacing`: edge-to-edge spacing in nm (i64)
///   - `coupling_fn`: closure/function returning fF/um for the given spacing
///
/// Output: coupling capacitance in fF (f64)
///
/// Invariants:
///   - prl >= 0, spacing > 0
///   - coupling_fn is monotonically decreasing with spacing
///   - Result is non-negative
pub fn same_layer_coupling(
    prl: i64, spacing: i64,
    coupling_fn: impl Fn(i64) -> f64,
) -> f64;

/// Compute interlayer (adjacent-layer) coupling capacitance.
///
/// Formula: Cc = overlap_area * interlayer_cap
///
/// Where:
///   - overlap_area = area of intersection of the two shapes' projections
///     onto the XY plane (nm^2)
///   - interlayer_cap = parallel-plate capacitance per unit area (fF/um^2)
///     from ExtractModel
///
/// Inputs:
///   - `overlap_area`: intersection area in nm^2 (i64)
///   - `interlayer_cap`: capacitance per unit area in fF/um^2 (f64)
///
/// Output: coupling capacitance in fF (f64)
///
/// Invariants:
///   - overlap_area >= 0
///   - interlayer_cap >= 0
///   - Result is non-negative
pub fn interlayer_coupling(overlap_area: i64, interlayer_cap: f64) -> f64;
```

### 5.5 Via Resistance

```rust
/// Look up via resistance from the tech model.
///
/// Inputs:
///   - `via_type`: identifier for the via type (from the tech via table)
///   - `num_cuts`: number of via cuts in the via array
///   - `r_per_cut`: resistance per single via cut in ohms (from ExtractModel)
///
/// Output: total via resistance in ohms (f64) = r_per_cut / num_cuts
///   (parallel via cuts reduce resistance)
///
/// Invariants:
///   - num_cuts >= 1 (panics in debug on 0)
///   - Result is non-negative
pub fn via_resistance(num_cuts: u32, r_per_cut: f64) -> f64;
```

### 5.6 Incremental Update

```rust
impl NetParasitics {
    /// Add the contribution of a new wire segment to the running totals.
    pub fn add_segment(&mut self, seg: SegmentParasitics);

    /// Remove the contribution of a wire segment from the running totals.
    /// The caller must provide the exact same SegmentParasitics that was
    /// previously added.
    ///
    /// Invariants:
    ///   - After remove, total_r >= 0 (floating point: may be epsilon-negative)
    ///   - The segment must exist in self.segments (matched by ShapeRef)
    pub fn remove_segment(&mut self, shape: ShapeRef);

    /// Recompute totals from the segment list. Called after a batch of
    /// add/remove operations to eliminate floating-point drift.
    pub fn recompute_totals(&mut self);
}
```

### 5.7 PexOutlier Detection

```rust
/// A detected parasitic outlier — a net whose parasitics exceed a threshold
/// or a matched group whose parasitic delta exceeds its budget.
#[derive(Debug, Clone)]
pub struct PexOutlier {
    pub kind: OutlierKind,
    pub net: NetId,
    /// The measured value that triggered the outlier.
    pub actual: f64,
    /// The threshold or budget it exceeded.
    pub threshold: f64,
    /// Unit of the measurement (ohms, fF, etc.).
    pub unit: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutlierKind {
    /// Total wire resistance exceeds threshold.
    HighResistance,
    /// Total coupling capacitance exceeds threshold.
    HighCoupling,
    /// Ground capacitance exceeds threshold.
    HighGroundCap,
    /// Via resistance exceeds threshold.
    HighViaResistance,
    /// Delta between matched-group members exceeds budget.
    MatchedGroupDelta,
}

/// Detect outliers in a set of net parasitics.
///
/// Inputs:
///   - `nets`: map from NetId to NetParasitics
///   - `r_threshold`: maximum acceptable total wire resistance (ohms)
///   - `cc_threshold`: maximum acceptable total coupling capacitance (fF)
///   - `matched_groups`: groups of nets that must have matched parasitics,
///     with the maximum allowed delta per metric
///
/// Output: list of detected outliers
pub fn detect_outliers(
    nets: &HashMap<NetId, NetParasitics>,
    r_threshold: f64,
    cc_threshold: f64,
    matched_groups: &[(Vec<NetId>, MatchedBudget)],
) -> Vec<PexOutlier>;

/// Budget for parasitic matching within a group.
#[derive(Debug, Clone)]
pub struct MatchedBudget {
    pub max_r_delta: f64,      // ohms
    pub max_cg_delta: f64,     // fF
    pub max_cc_delta: f64,     // fF
}
```

---

## 6. EM/IR/Antenna Module

### 6.1 EM: Black's Equation

```rust
/// Compute current density for a wire segment.
///
/// Formula: J = I / (width * thickness)
///
/// Where:
///   - I = current through the segment (amps)
///   - width = wire width in nm
///   - thickness = wire thickness in nm (from ExtractModel layer stack)
///
/// Inputs:
///   - `current`: current in amps (f64)
///   - `width`: wire width in nm (i64)
///   - `thickness`: wire thickness in nm (i64)
///
/// Output: current density in A/nm^2 (f64)
///
/// Invariants:
///   - width > 0, thickness > 0
pub fn current_density(current: f64, width: i64, thickness: i64) -> f64;

/// Check current density against the layer's J_max limit.
///
/// Black's equation gives mean time to failure:
///   MTF = A * (1/J^n) * exp(Ea / (k*T))
///
/// For layout checking, we simplify to: is J <= J_max for the layer?
///
/// Inputs:
///   - `j`: computed current density (from current_density())
///   - `j_max`: maximum allowed current density for the layer (A/nm^2, from tech)
///   - `blech_length`: minimum length below which short-line effects prevent
///     EM failure (nm). If the segment length <= blech_length, EM is not a concern.
///   - `segment_length`: length of the wire segment in nm
///
/// Output: `Some(EmViolation)` if j > j_max and segment_length > blech_length
pub fn check_em(
    j: f64, j_max: f64,
    segment_length: i64, blech_length: i64,
    shape: ShapeRef, layer: LayerId,
) -> Option<EmViolation>;
```

#### EmViolation

```rust
#[derive(Debug, Clone)]
pub struct EmViolation {
    pub shape: ShapeRef,
    pub layer: LayerId,
    /// Computed current density (A/nm^2).
    pub j_actual: f64,
    /// Maximum allowed current density (A/nm^2).
    pub j_max: f64,
    /// Ratio j_actual / j_max (> 1.0 means violation).
    pub ratio: f64,
    /// Segment length (nm).
    pub segment_length: i64,
    /// Blech length (nm). If segment_length <= blech_length, this violation
    /// was filtered by the Blech criterion and should not appear.
    pub blech_length: i64,
}
```

### 6.2 IR Drop

```rust
/// Compute static IR drop across a resistive network.
///
/// Model: The power/ground network is modeled as a resistive mesh. Each wire
/// segment is a resistor (from wire_resistance). Each via is a resistor (from
/// via_resistance). Current sources are placed at device terminals.
///
/// The system solves: G * V = I
///   where G is the conductance matrix (sparse), V is the node voltage vector,
///   and I is the current source vector.
///
/// Inputs:
///   - `nodes`: list of nodes in the resistive network
///   - `resistors`: list of (node_a, node_b, resistance_ohms)
///   - `current_sources`: list of (node, current_amps) — positive = current sink
///   - `vdd_nodes`: nodes connected to ideal VDD (voltage sources)
///   - `vdd_voltage`: the supply voltage
///
/// Output: IrDropResult with per-node voltage and worst-case drop
///
/// Solver: sparse Cholesky or iterative (CG) — the verify crate provides
/// the model setup; the actual sparse solve may delegate to philis-solver.
pub fn compute_ir_drop(
    nodes: &[u32],
    resistors: &[(u32, u32, f64)],
    current_sources: &[(u32, f64)],
    vdd_nodes: &[u32],
    vdd_voltage: f64,
) -> IrDropResult;

#[derive(Debug, Clone)]
pub struct IrDropResult {
    /// Voltage at each node (indexed by node id).
    pub node_voltages: Vec<(u32, f64)>,
    /// Worst-case voltage drop (max VDD - V_node across all non-VDD nodes).
    pub worst_drop: f64,
    /// Node with the worst drop.
    pub worst_node: u32,
    /// Whether the solve converged (for iterative solvers).
    pub converged: bool,
}
```

### 6.3 Antenna

```rust
/// Antenna ratio check for a single net at a single layer.
///
/// Formula: ratio = metal_area / gate_oxide_area
///
/// If ratio > max_ratio for the layer, this is an antenna violation.
/// The check is cumulative: metal area includes all metal on the checked
/// layer and below (or above, depending on the process direction).
///
/// Inputs:
///   - `metal_area`: total metal area connected to the gate on this layer (nm^2)
///   - `gate_oxide_area`: total gate oxide area connected to this net (nm^2)
///   - `max_ratio`: maximum allowed antenna ratio for the layer (from tech)
///   - `net`: the net being checked
///   - `layer`: the metal layer being checked
///
/// Output: `Some(AntennaViolation)` if ratio > max_ratio and gate_oxide_area > 0
pub fn check_antenna(
    metal_area: i64, gate_oxide_area: i64,
    max_ratio: f64,
    net: NetId, layer: LayerId,
) -> Option<AntennaViolation>;

#[derive(Debug, Clone)]
pub struct AntennaViolation {
    pub net: NetId,
    pub layer: LayerId,
    /// Computed antenna ratio.
    pub ratio: f64,
    /// Maximum allowed ratio.
    pub max_ratio: f64,
    /// Metal area (nm^2).
    pub metal_area: i64,
    /// Gate oxide area (nm^2).
    pub gate_oxide_area: i64,
}
```

---

## 7. Phase Plan (Build Order)

| Phase | Deliverable | Depends On | Estimated Complexity |
|-------|-------------|------------|---------------------|
| 1 | **DRC types & basic predicates** — `ViolationKind`, `DrcViolation`, `ShapeRef`, `RuleId`, `check_spacing`, `check_width`, `check_overlap`, `check_area`, `check_grid_snap`, `check_outline_exceed` | `philis-geom` Rect only | Low |
| 2 | **DRC advanced predicates** — `check_enclosure`, `check_eol_spacing`, `check_prl_spacing`, `check_cut_spacing`, `check_same_net_notch`, `check_well_spacing`, `check_well_enclosure`, `check_implant_spacing` | Phase 1 | Medium |
| 3 | **DRC incremental infra** — `DrcRegion`, `DrcDelta`, `compute_r_max`, `RuleFamilyCoverage` | Phases 1-2, `philis-tech` RuleSet | Low |
| 4 | **LVS union-find & types** — `ConnectivityTracker`, `NetId`, `UnionResult`, `LvsResult`, `LvsOpen`, `LvsShort`, `LvsMismatch` | None (self-contained data structure) | Low |
| 5 | **LVS connectivity extraction** — `extract_connectivity` function | Phase 4, `philis-geom` Rect, `philis-tech` LayerId | Medium |
| 6 | **PEX wire models** — `wire_resistance`, `ground_capacitance`, `via_resistance` | `philis-tech` ExtractModel | Low |
| 7 | **PEX coupling models** — `same_layer_coupling`, `interlayer_coupling` | Phase 6 | Medium |
| 8 | **PEX aggregation** — `NetParasitics`, `SegmentParasitics`, incremental add/remove, `detect_outliers`, `PexOutlier` | Phases 6-7 | Medium |
| 9 | **EM checking** — `current_density`, `check_em`, `EmViolation` | `philis-tech` ExtractModel | Low |
| 10 | **IR drop** — `compute_ir_drop`, `IrDropResult` | Phase 9, sparse solver (may depend on `philis-solver`) | High |
| 11 | **Antenna checking** — `check_antenna`, `AntennaViolation` | `philis-tech` for max_ratio lookup | Low |

**Parallelism:** Phases 1-3 (DRC), 4-5 (LVS), 6-8 (PEX), and 9-11 (EM/IR/Antenna) can be developed concurrently by different contributors. The only cross-module dependency is on shared types (`ShapeRef`, `NetId`, `LayerId`) which are defined in phase 1 or imported from dependencies.

---

## 8. Public API Surface

The crate's `lib.rs` re-exports all public types and functions organized by module:

```rust
// lib.rs — public facade

// === DRC ===
pub mod drc;
pub use drc::{
    // Types
    ViolationKind, DrcViolation, ShapeRef, RuleId,
    DrcRegion, DrcDelta, RuleFamilyCoverage,
    // Predicates
    check_spacing, check_width, check_enclosure, check_area,
    check_eol_spacing, check_prl_spacing, check_overlap,
    check_cut_spacing, check_same_net_notch, check_grid_snap,
    check_well_spacing, check_well_enclosure, check_implant_spacing,
    check_outline_exceed,
    // Influence
    compute_r_max,
};

// === LVS ===
pub mod lvs;
pub use lvs::{
    ConnectivityTracker, NetId, UnionResult,
    LvsResult, LvsOpen, LvsShort, LvsMismatch, MismatchKind, LvsError,
    extract_connectivity,
};

// === PEX ===
pub mod pex;
pub use pex::{
    NetParasitics, SegmentParasitics,
    PexOutlier, OutlierKind, MatchedBudget,
    wire_resistance, ground_capacitance,
    same_layer_coupling, interlayer_coupling, via_resistance,
    detect_outliers,
};

// === EM / IR / Antenna ===
pub mod em;
pub use em::{
    current_density, check_em, EmViolation,
    compute_ir_drop, IrDropResult,
    check_antenna, AntennaViolation,
};

// === Coverage ===
pub mod coverage;
pub use coverage::RuleFamilyCoverage;
```

### Consumer usage patterns

**Place (device-pair broad/narrow-phase loop):**
```rust
use philis_verify::{check_spacing, check_overlap, check_well_spacing, ...};
use philis_verify::{DrcViolation, compute_r_max, DrcRegion};

// In the placement loop:
let r_max = compute_r_max(&tech.rule_set());
for (a, b) in broad_phase_candidates(rtree, r_max) {
    if let Some(v) = check_spacing(&a.rect, &b.rect, ...) {
        violations.push(v);
    }
    if let Some(v) = check_overlap(&a.rect, &b.rect, ...) {
        violations.push(v);
    }
    // ... other predicates based on layer type
}
```

**Route (incremental commit-time delta loop):**
```rust
use philis_verify::{check_spacing, check_eol_spacing, check_prl_spacing, ...};
use philis_verify::{DrcRegion, DrcDelta, compute_r_max};
use philis_verify::{wire_resistance, ground_capacitance, same_layer_coupling};
use philis_verify::{ConnectivityTracker, NetParasitics};

// On wire segment commit:
let region = DrcRegion::new(new_segment.bbox(), r_max);
let mut delta = DrcDelta::default();
for neighbor in spatial_index.query(&region.dilated) {
    // Run all applicable predicates
    if let Some(v) = check_spacing(...) { delta.added.push(v); }
    if let Some(v) = check_eol_spacing(...) { delta.added.push(v); }
    // ...
}
// Update parasitics
net_parasitics.add_segment(SegmentParasitics { ... });
// Update connectivity
connectivity.union(seg_start_node, seg_end_node);
```

---

## 9. Testing Strategy

### Unit Tests (per predicate)

Each DRC predicate gets a test battery:

- **Passing case:** two shapes exactly at the minimum distance — no violation.
- **Failing case:** two shapes closer than the minimum — violation with correct `actual`/`required`.
- **Edge case:** touching edges (distance = 0), identical shapes, zero-area shapes.
- **Symmetry:** `check_spacing(a, b)` == `check_spacing(b, a)` for all random inputs.
- **Orthogonality:** `check_spacing` does not fire on overlapping shapes (that is `check_overlap`'s job).

### LVS Tests

- **Simple net:** 3 shapes on one layer, all overlapping → one component.
- **Open:** 3 shapes, two overlapping + one isolated with the same net label → open detected.
- **Short:** 2 shapes on different nets connected by an accidental overlap → short detected.
- **Via connectivity:** shapes on M1 and M2 connected by a via → same component.
- **Invariant check:** fuzz with random shapes and nets, verify `invariant_check()` finds all opens/shorts that manual inspection finds.

### PEX Tests

- **Wire resistance:** known geometry + known sheet resistance → exact expected value.
- **Ground capacitance:** known geometry + known area/fringe cap → exact expected value.
- **Coupling:** two parallel wires at known spacing → expected coupling from model.
- **Incremental:** add segment, verify totals increase; remove it, verify totals return to original.
- **Outlier detection:** create a net with high resistance, verify it appears in outlier list.

### EM/IR/Antenna Tests

- **EM:** wire with known current/width/thickness → expected J; compare vs J_max.
- **Blech filter:** short segment below Blech length → no violation even if J > J_max.
- **IR drop:** simple 3-node resistive network → hand-computed voltage at each node.
- **Antenna:** known metal area / gate area → expected ratio; check vs threshold.

### Integration Tests

- **Place consumer mock:** simulate a placement loop calling DRC predicates on a grid of device pairs. Verify violation count matches hand analysis.
- **Route consumer mock:** simulate adding/removing wire segments incrementally. Verify DrcDelta correctly tracks added/removed violations.
- **Full stack:** combine DRC + LVS + PEX on a small test layout (e.g., two-transistor inverter). Verify all three produce expected results.

### Property Tests (proptest)

- **DRC predicate symmetry:** for all random rect pairs, `check_*(a, b) == check_*(b, a)`.
- **DRC predicate consistency:** if `check_spacing` fires, `check_overlap` does not fire on the same pair (and vice versa).
- **Union-find invariants:** for random union sequences, `find(x) == find(y)` iff `x` and `y` were unioned (transitively).
- **PEX non-negativity:** for all random inputs, all parasitic values are non-negative.
- **DrcDelta merge associativity:** `merge(merge(a, b), c)` produces the same violation set as `merge(a, merge(b, c))`.
