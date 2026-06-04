# Phase 0 — Technology and Geometry Foundation

> **Shared crates:** The `tech/` and `geom/` subsystems described in this
> document now live in `crates/tech` (`philis-tech`) and `crates/geom`
> (`philis-geom`) respectively. The route crate depends on these shared
> crates rather than implementing them internally. The design below
> describes what the shared crates provide; Phase 0 for the route crate
> reduces to wiring up the shared dependencies and verifying integration.

**Goal:** Integrate the shared technology model and geometry kernel that
every other phase stands on.

**Shared crates used:** `crates/tech` (`philis-tech`), `crates/geom` (`philis-geom`)

**Depends on:** `crates/tech` and `crates/geom` being available

**Unlocks:** Phase 1 (graph construction needs tech; DRC needs geometry)

---

## 1. Technology model (from `crates/tech`)

> **Shared crate:** This subsystem now lives in `crates/tech`
> (`philis_tech`). The `Tech`, `LayerStack`, `ViaTable`, `GridTable`,
> `RuleSet`, `ExtractModel`, and `PdkState` types are all provided by the
> shared crate. The route crate imports and queries them.

The formulation's `T = (L, G, V, R, E, X, M)` is the compiled technology. This
is a read-only, immutable data structure built once from PDK input and queried
by every downstream step.

### Data model

```
Tech
  layers: Vec<Layer>              -- ordered stack, bottom to top
  vias: Vec<ViaDef>               -- legal via transitions between layers
  grids: BTreeMap<LayerId, Grid>  -- per-layer routing grid
  rules: RuleSet                  -- DRC predicates, indexed by layer pair
  extract: ExtractParams          -- sheet R, cap tables per layer/pair
  metadata: TechMetadata          -- node family, db unit, corner variants
```

**Layer:**
```
Layer
  id: LayerId                     -- stable index into the stack
  name: String                    -- e.g. "M1", "VIA1", "POLY"
  class: LayerClass               -- Routing | Via | Cut | Diffusion | Well | Marker | ...
  direction: Direction            -- Horizontal | Vertical | Any
  pitch_nm: i64                   -- track pitch
  min_width_nm: i64
  max_width_nm: Option<i64>
  offset_nm: i64                  -- grid offset from origin
  sheet_resistance_ohm_sq: f64
  thickness_nm: i64
```

**ViaDef:**
```
ViaDef
  id: ViaId
  name: String
  bottom_layer: LayerId
  top_layer: LayerId
  cut_size: (i64, i64)           -- cut dimensions
  cut_spacing: (i64, i64)        -- min spacing between cuts
  enclosure_bottom: (i64, i64)   -- min enclosure on bottom layer
  enclosure_top: (i64, i64)      -- min enclosure on top layer
  resistance_ohm: f64
  array_rules: Option<ViaArrayRules>  -- for multi-cut vias
```

**RuleSet:** A collection of typed DRC predicates. Each predicate is a function
`(geometry_context, tech) -> bool` with metadata:
```
RulePredicate
  id: RuleId
  name: String                    -- e.g. "M1.S.1" (M1 min spacing rule 1)
  kind: RuleKind                  -- Spacing | Width | Enclosure | Area | EOL | ...
  layers: Vec<LayerId>            -- layers this rule applies to
  coverage: Coverage              -- from the canonical vocabulary
  influence_radius_nm: i64        -- max spatial extent (for incremental DRC)
  is_delta_safe: bool             -- can be checked incrementally
  parameters: RuleParams          -- the actual numbers (min_spacing, prl, etc.)
```

### PDK ingestion

The technology compiler reads from a Philis-native JSON/TOML schema. This is
not a "tolerant scanner" — it has a well-defined schema with required and
optional fields, and rejects malformed input with specific errors.

Importers (separate modules) translate from external formats:
- **LEF importer:** reads Technology LEF for layers, vias, sites, tracks
- **SKY130 importer:** direct translation from the SKY130 PDK structure
- **IHP SG13G2 importer:** for the IHP 130nm BiCMOS PDK

The importer writes the Philis schema; the compiler reads it. This two-stage
pipeline means the compiler never depends on external format details.

### Coverage classification

After compilation, the compiler classifies each predicate into the canonical
`Coverage` vocabulary and derives the overall `PdkState`:

- Walk every `RulePredicate`; if any required class is `Missing` or
  `Contradictory`, emit `PdkState::ContradictoryOrMissing`
- If any is `Unsupported` or `ManualReview`, cap at `AbstractComplete`
- If all required classes are `Coded`, emit `FullSignoff`
- Otherwise `SparseResearch`

This fills `RuleCoverageReport` and the certificate's `pdk_state` field.

### Decision: what about DRC rule decks?

Real foundry DRC decks (Calibre SVRF, IC Validator RS) are proprietary and
enormous (50k+ rules for advanced nodes). Philis does not parse them. Instead:

1. The Philis tech schema encodes the *routing-relevant* subset of DRC rules
   (spacing, width, enclosure, via, area, EOL — the rules a router can check
   in-loop).
2. Rules the router cannot check are classified `ExternalOnly` or `ManualReview`.
3. The certificate records which rules were proven internally and which require
   external signoff.

This is honest: Philis proves what it can, and tells you what it can't.

---

## 2. Geometry kernel (from `crates/geom`)

> **Shared crate:** This subsystem now lives in `crates/geom`
> (`philis_geom`). The `Rect`, `RectiPoly`, `ShapeRef`, `GeometryStore`,
> spatial index, and boolean operations are all provided by the shared
> crate. The route crate imports and uses them.

The geometry kernel provides shapes, spatial queries, and boolean operations.
Every other module depends on it.

### Shapes

Two primary shape types:

**Rect** — axis-aligned rectangle, the 95% case for routing geometry:
```
Rect
  x_lo, y_lo, x_hi, y_hi: i64   -- integer nanometer coordinates
```

Operations: contains_point, overlaps, intersection, expand (dilation), area,
perimeter, center. All integer arithmetic — no floating point in geometry.

**RectiPoly** — rectilinear polygon (all edges axis-aligned), for L-shapes,
T-shapes, and min-area patches:
```
RectiPoly
  vertices: Vec<(i64, i64)>      -- CCW-ordered vertices
```

Operations: bounding_box, contains_point, area, boolean ops (union,
intersection, difference with other RectiPolys).

### Spatial index

**Initial implementation: R-tree** using the `rstar` crate.

The R-tree supports:
- Point query: what shapes contain this point?
- Window query: what shapes overlap this rectangle?
- Nearest-neighbor: what is the closest shape to this point?
- Insert/remove: O(log n) amortized

This is sufficient for Phase 0-2. The formulation doc's corner stitching
(Section 6 of Router-Algorithms.html) is a Phase 3 optimization — see the
trade-off analysis in [GEOMETRY-KERNEL.md](GEOMETRY-KERNEL.md).

**The index stores `ShapeRef` handles, not shapes directly:**
```
ShapeRef
  layer: LayerId
  owner: ShapeOwner               -- Net(NetId) | Obstacle | Pin(TerminalId) | Fill
  rect: Rect                      -- bounding box (for R-tree envelope)
  shape: Shape                    -- Rect or RectiPoly
```

### Layer-per-index architecture

One spatial index per layer. This matches the physical reality (DRC rules
are overwhelmingly intra-layer or adjacent-layer) and keeps queries fast by
eliminating cross-layer noise.

```
LayerGeometry
  index: RTree<ShapeRef>
  shapes: Vec<ShapeRef>           -- all shapes on this layer
```

```
GeometryStore
  layers: BTreeMap<LayerId, LayerGeometry>
```

### Integer coordinates

All geometry is in integer nanometers. No floating point anywhere in the
geometry kernel. This eliminates an entire class of robustness bugs
(epsilon comparisons, non-transitive equality, degenerate intersections).

The database unit from the technology model converts between nanometers and
GDS database units for I/O.

---

## 3. Milestones and exit criteria

Phase 0 is done when:

1. A `Tech` can be compiled from a Philis JSON schema describing SKY130's
   routing layers (M1-M5), vias, and basic spacing/width rules.
2. The `PdkState` classifier correctly identifies SKY130 as
   `AbstractComplete` (some rules are external-only or approximate).
3. `Rect` and `RectiPoly` pass property-based tests for boolean operations.
4. The spatial index supports insert, remove, point query, and window query
   with correct results on a test dataset of 10k rectangles.
5. All types are `Send + Sync` (the geometry kernel must not prevent future
   parallelism).

---

## 4. Risks and mitigations

| Risk | Mitigation |
|------|------------|
| PDK schema design locks in wrong abstractions | Keep the schema minimal — only fields the router provably needs. Extend later. |
| Boolean operations on rectilinear polygons are subtle | Use a well-tested scanline algorithm; invest in property-based tests (commutativity, associativity, area conservation). |
| The R-tree becomes a bottleneck at scale | Profile first. If it does, the spatial index is behind a trait — swap in corner stitching without touching callers. |
| SKY130's rules are more complex than the initial schema can express | Accept `Partial` or `Approximate` coverage and report it honestly. Don't stretch the schema to accommodate edge cases before the core works. |
