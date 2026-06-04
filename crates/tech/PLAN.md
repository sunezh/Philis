# Technology Compiler — Implementation Plan

**Status:** plan
**Target crate:** `crates/tech` (crate name `philis_tech`)
**First concrete PDK:** SKY130
**Next PDK:** IHP SG13G2
**Execution model:** batch compile, immutable result, `Send + Sync`
**Consumer targets:** `crates/constraints`, `crates/place`, `crates/route`

---

## 1. Overview

### What the crate owns

The `philis-tech` crate is the single owner of compiled PDK technology data in
the Philis engine. It reads PDK descriptions from external formats or
hand-authored schemas, validates structural invariants, classifies rule
coverage, and produces an immutable `CompiledTech` structure that every
downstream crate queries through typed accessors.

The crate owns:
- The Philis-native PDK schema (JSON format)
- The import pipeline (external format -> schema -> compiled structure)
- The `CompiledTech` data model: layer stacks, via tables, grid tables, DRC
  rule predicates, extraction parameters, coverage classification
- PDK hash computation (BLAKE3) for certificate integration
- `PdkState` derivation from coverage inventory
- SKY130 and future PDK concrete implementations

### Who consumes it

```
  crates/tech         (this crate — owns CompiledTech)
       ↑
  crates/constraints  (queries rule IDs, coverage inventory, PdkState)
  crates/place        (queries layer stack, grid, spacing/width/enclosure,
                       well rules, orientations, row pitch)
  crates/route        (queries layer stack, via table, grid, DRC predicates,
                       extraction model, coupling caps, EM)
```

Every consumer depends on `philis-tech`. No consumer depends on the import
pipeline or on any specific PDK backend — they see only the compiled
`CompiledTech` behind a `&` reference.

### Dependency direction

```
philis-geom  (coordinate types: i64 nm, Rect, Axis)
     ↓
philis-tech  (this crate)
     ↓
philis-constraints, philis-place, philis-route
```

The tech crate depends on `philis-geom` for `i64` nanometer coordinate types
and `Axis` (Horizontal / Vertical). It depends on `serde` / `serde_json` for
schema deserialization and `blake3` for hashing. It has no dependency on the
API crate, the constraint compiler, the placer, or the router.

### Design principles

1. **Immutable after construction.** `CompiledTech` is frozen once built.
   Consumers receive `&CompiledTech`. If the PDK changes, the entire
   `CompiledTech` is rebuilt and every downstream result is invalidated.

2. **Hot-switchable PDK.** New PDK = new importer. The compiler and all
   consumers are PDK-agnostic. SKY130 today, IHP SG13G2 next, commercial
   nodes later — no consumer code changes.

3. **Honest coverage.** Every rule predicate carries a `Coverage`
   classification. Missing rules are not silently omitted — they are
   classified as `Missing` and the `PdkState` reflects the gap. The
   certificate records exactly what was proven and what was not.

4. **Two-stage pipeline.** Import (external format -> Philis schema) is
   separated from compile (schema -> `CompiledTech`). The compiler never
   touches external format details. The schema is version-controlled,
   diffable, and hand-editable.

---

## 2. Module map

```
src/
  lib.rs                   — public facade, re-exports
  types.rs                 — LayerId, ViaId, LayerClass, LayerDirection,
                             Coverage, PdkState, RuleCoverageReport
  compiled.rs              — CompiledTech struct, accessors, Send + Sync
  layer.rs                 — LayerStack, LayerDef, LayerClass, LayerDirection
  via.rs                   — ViaTable, ViaDef
  grid.rs                  — GridTable, GridDef (per-layer track positions)
  rules.rs                 — RuleSet, dispatch methods (min_spacing, min_width,
                             via_enclosure, min_area, eol_spacing, prl_spacing)
  extract.rs               — ExtractModel, per-corner sheet R, cap tables,
                             coupling caps, via resistance
  coverage.rs              — Coverage enum, PdkState derivation,
                             RuleCoverageReport, lattice operations
  hash.rs                  — BLAKE3 PDK hash computation
  schema.rs                — Philis-native PDK schema (JSON serde types)
  validate.rs              — structural invariant checks
  compile.rs               — schema -> CompiledTech builder
  import/
    mod.rs                 — Importer trait, format detection
    lef.rs                 — generic Technology LEF importer
    sky130.rs              — targeted SKY130 importer (hardcoded data + LEF)
    json.rs                — passthrough for hand-authored JSON schemas

tests/
  schema_roundtrip.rs      — serialize/deserialize schema
  sky130_smoke.rs          — load SKY130, verify layer/via/rule counts
  coverage_lattice.rs      — lattice axioms (commutativity, associativity,
                             idempotence, conservative direction)
  pdk_state_derivation.rs  — PdkState classification from coverage buckets
  hash_stability.rs        — same input -> same hash across runs
  rule_dispatch.rs         — width-dependent spacing, PRL, EOL queries
  extract_model.rs         — per-corner extraction parameter lookups
  validation_errors.rs     — structural invariant violation detection
```

---

## 3. The Philis PDK schema (JSON format spec)

The schema is the interchange format between importers and the compiler. It is
a JSON document with the following top-level structure:

```json
{
  "schema_version": "1.0",
  "name": "sky130",
  "node_family": "130nm planar CMOS",
  "db_unit_nm": 1,
  "manufacturing_grid_nm": 5,

  "layers": [ ... ],
  "vias": [ ... ],
  "spacing_rules": [ ... ],
  "width_rules": [ ... ],
  "enclosure_rules": [ ... ],
  "area_rules": [ ... ],
  "eol_rules": [ ... ],

  "extraction": {
    "corners": ["tt", "ss", "ff"],
    "default_corner": "tt",
    "sheet_resistances": [ ... ],
    "layer_caps": [ ... ],
    "coupling_caps": [ ... ],
    "via_resistances": [ ... ]
  },

  "em_rules": [ ... ],
  "antenna_rules": [],
  "density_rules": [],
  "multipatterning": null,
  "external_decks": { ... },
  "notes": [ ... ]
}
```

### Layer entry

```json
{
  "name": "met1",
  "gds_layer": 68,
  "gds_datatype": 20,
  "class": "metal",
  "class_index": 1,
  "direction": "horizontal",
  "pitch_nm": 340,
  "offset_nm": 170,
  "min_width_nm": 140,
  "max_width_nm": null,
  "thickness_nm": 330,
  "sheet_resistance_ohm_sq": 0.125
}
```

`class` is one of: `diffusion`, `poly`, `local_interconnect`, `metal`, `via`,
`well`, `implant`, `marker`, `keepout`, `pad`, `passivation`.

`class_index` is the metal/via stack index (M1=1, V1=1, M2=2, V2=2, ...).
Required for `metal` and `via` classes; `null` for others.

`direction` is `horizontal`, `vertical`, or `any`. Required for routing
layers; `null` for non-routing layers.

### Via entry

```json
{
  "name": "mcon",
  "bottom": "li1",
  "top": "met1",
  "cut_size_nm": [170, 170],
  "cut_spacing_nm": [190, 190],
  "enclosure_bottom_nm": [60, 30],
  "enclosure_top_nm": [30, 60],
  "resistance_ohm": 9.3,
  "coverage": "coded"
}
```

### Spacing rule entry

```json
{
  "name": "met1.S.1",
  "layer": "met1",
  "min_spacing_nm": 140,
  "width_threshold_nm": null,
  "prl_threshold_nm": null,
  "eol_width_nm": null,
  "same_net": false,
  "coverage": "coded",
  "note": null
}
```

Width-dependent spacing: `width_threshold_nm` is non-null. The rule applies
when at least one shape is wider than the threshold.

PRL-dependent spacing: `prl_threshold_nm` is non-null. The rule applies when
the parallel run length exceeds the threshold.

EOL spacing: `eol_width_nm` is non-null. The rule applies to end-of-line
geometries narrower than the threshold.

### Width rule entry

```json
{
  "name": "met1.W.1",
  "layer": "met1",
  "min_width_nm": 140,
  "max_width_nm": null,
  "coverage": "coded"
}
```

### Enclosure rule entry

```json
{
  "name": "mcon.E.1",
  "via": "mcon",
  "layer": "li1",
  "side": "bottom",
  "min_enclosure_nm": [60, 30],
  "coverage": "coded"
}
```

`side` is `bottom` or `top` (relative to the via's bottom/top layers).
`min_enclosure_nm` is `[x, y]` (may be asymmetric).

### Area rule entry

```json
{
  "name": "met1.A.1",
  "layer": "met1",
  "min_area_nm2": 27200,
  "coverage": "coded"
}
```

### Extraction entries

Sheet resistance:
```json
{ "layer": "met1", "resistance_ohm_sq": 0.125, "corner": "tt" }
```

Layer capacitance:
```json
{
  "layer": "met1",
  "area_cap_ff_per_um2": 0.038,
  "fringe_cap_ff_per_um": 0.045,
  "corner": "tt"
}
```

Coupling capacitance:
```json
{
  "layer1": "met1",
  "layer2": "met1",
  "coupling_cap_ff_per_um": 0.052,
  "coupling_window_nm": 1000,
  "corner": "tt"
}
```

Via resistance:
```json
{ "via": "mcon", "resistance_ohm": 9.3, "corner": "tt" }
```

### Required vs. optional fields

**Required (compilation fails without):**
- `name`, `db_unit_nm`, `manufacturing_grid_nm`
- At least one routing layer (`class: "metal"` or `"local_interconnect"`)
- At least one via connecting two routing layers
- Basic spacing and width rules for each routing layer

**Optional (classified as `Missing` / `ExternalOnly` if absent):**
- Extraction parameters
- Antenna rules
- Density rules
- EM rules
- Multipatterning
- External deck paths (informational)

### Schema versioning

The `schema_version` field is a semver string. The compiler rejects schemas
with a major version it does not support. Minor version increments add
optional fields. The schema version is included in the PDK hash.

---

## 4. The CompiledTech data model

### Core structure

```rust
/// Immutable compiled PDK technology. Send + Sync.
/// Constructed once, shared via &CompiledTech.
pub struct CompiledTech {
    name: String,
    db_unit_nm: i64,
    manufacturing_grid_nm: i64,
    layers: LayerStack,
    vias: ViaTable,
    grids: GridTable,
    rules: RuleSet,
    extract: ExtractModel,
    coverage_report: RuleCoverageReport,
    pdk_state: PdkState,
    pdk_hash: [u8; 32],   // BLAKE3
}

// Thread-safe: all data is immutable after construction.
// Enforced by the type system: no interior mutability.
unsafe impl Send for CompiledTech {}
unsafe impl Sync for CompiledTech {}
```

### Top-level accessors

```rust
impl CompiledTech {
    pub fn name(&self) -> &str;
    pub fn db_unit_nm(&self) -> i64;
    pub fn manufacturing_grid_nm(&self) -> i64;
    pub fn layers(&self) -> &LayerStack;
    pub fn vias(&self) -> &ViaTable;
    pub fn grids(&self) -> &GridTable;
    pub fn rules(&self) -> &RuleSet;
    pub fn extract(&self) -> &ExtractModel;
    pub fn coverage_report(&self) -> &RuleCoverageReport;
    pub fn pdk_state(&self) -> PdkState;
    pub fn pdk_hash(&self) -> &[u8; 32];
}
```

### LayerStack

```rust
/// Ordered bottom-to-top. Random access by LayerId.
pub struct LayerStack {
    layers: Vec<LayerDef>,
    name_to_id: BTreeMap<String, LayerId>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct LayerId(pub u16);

pub struct LayerDef {
    pub id: LayerId,
    pub name: String,
    pub gds_layer: u16,
    pub gds_datatype: u16,
    pub class: LayerClass,
    pub direction: LayerDirection,
    pub min_width_nm: i64,
    pub max_width_nm: Option<i64>,
    pub thickness_nm: Option<i64>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayerClass {
    Diffusion,
    Poly,
    LocalInterconnect,
    Metal(u8),       // Metal(1) = M1, Metal(2) = M2, ...
    Via(u8),         // Via(1) = V1 (connects M1-M2), ...
    Well,            // nwell, pwell, dnwell
    Implant,         // nsdm, psdm, hvtp, lvtn
    Marker,
    Keepout,
    Pad,
    Passivation,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum LayerDirection {
    Horizontal,
    Vertical,
    Any,
}
```

`LayerStack` methods:
```rust
impl LayerStack {
    pub fn get(&self, id: LayerId) -> &LayerDef;
    pub fn by_name(&self, name: &str) -> Option<&LayerDef>;
    pub fn id_of(&self, name: &str) -> Option<LayerId>;
    pub fn routing_layers(&self) -> impl Iterator<Item = &LayerDef>;
    pub fn metals(&self) -> impl Iterator<Item = &LayerDef>;
    pub fn len(&self) -> usize;
    pub fn iter(&self) -> impl Iterator<Item = &LayerDef>;
}
```

### ViaTable

```rust
pub struct ViaTable {
    vias: Vec<ViaDef>,
    pair_to_vias: BTreeMap<(LayerId, LayerId), Vec<ViaId>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ViaId(pub u16);

pub struct ViaDef {
    pub id: ViaId,
    pub name: String,
    pub bottom: LayerId,
    pub top: LayerId,
    pub cut_size_nm: [i64; 2],        // [width, height]
    pub cut_spacing_nm: [i64; 2],     // for via arrays
    pub enclosure_bottom_nm: [i64; 2], // [x_enc, y_enc]
    pub enclosure_top_nm: [i64; 2],
    pub resistance_ohm: f64,
}
```

`ViaTable` methods:
```rust
impl ViaTable {
    pub fn get(&self, id: ViaId) -> &ViaDef;
    pub fn by_name(&self, name: &str) -> Option<&ViaDef>;
    pub fn between(&self, bottom: LayerId, top: LayerId) -> &[ViaId];
    pub fn len(&self) -> usize;
    pub fn iter(&self) -> impl Iterator<Item = &ViaDef>;
}
```

### GridTable

```rust
pub struct GridTable {
    grids: BTreeMap<LayerId, GridDef>,
}

pub struct GridDef {
    pub layer: LayerId,
    pub pitch_nm: i64,
    pub offset_nm: i64,
    pub min_width_nm: i64,
    pub max_width_nm: Option<i64>,
    pub direction: LayerDirection,
}
```

`GridTable` methods:
```rust
impl GridTable {
    pub fn get(&self, layer: LayerId) -> Option<&GridDef>;
    pub fn track_center(&self, layer: LayerId, index: i32) -> i64;
    // Returns offset + index * pitch
    pub fn snap_to_track(&self, layer: LayerId, coord: i64) -> i64;
    pub fn tracks_in_range(&self, layer: LayerId, lo: i64, hi: i64) -> Vec<i64>;
}
```

### RuleSet

```rust
pub struct RuleSet {
    spacing_rules: Vec<SpacingRule>,
    width_rules: Vec<WidthRule>,
    enclosure_rules: Vec<EnclosureRule>,
    area_rules: Vec<AreaRule>,
    eol_rules: Vec<EolRule>,
    // Indexes for fast lookup
    by_layer: BTreeMap<LayerId, Vec<usize>>,
}

pub struct SpacingRule {
    pub name: String,
    pub layer: LayerId,
    pub min_spacing_nm: i64,
    pub width_threshold_nm: Option<i64>,
    pub prl_threshold_nm: Option<i64>,
    pub same_net: bool,
    pub coverage: Coverage,
}

pub struct WidthRule {
    pub name: String,
    pub layer: LayerId,
    pub min_width_nm: i64,
    pub max_width_nm: Option<i64>,
    pub coverage: Coverage,
}

pub struct EnclosureRule {
    pub name: String,
    pub via: ViaId,
    pub layer: LayerId,
    pub min_enclosure_nm: [i64; 2],
    pub coverage: Coverage,
}

pub struct AreaRule {
    pub name: String,
    pub layer: LayerId,
    pub min_area_nm2: i64,
    pub coverage: Coverage,
}

pub struct EolRule {
    pub name: String,
    pub layer: LayerId,
    pub eol_width_nm: i64,
    pub eol_spacing_nm: i64,
    pub coverage: Coverage,
}
```

**Dispatch methods** (these are the primary consumer interface):

```rust
impl RuleSet {
    /// Returns the strictest applicable spacing for the given layer
    /// considering both shapes' widths.
    pub fn min_spacing(&self, layer: LayerId, width_a: i64, width_b: i64) -> i64;

    /// Returns the minimum width for the given layer.
    pub fn min_width(&self, layer: LayerId) -> i64;

    /// Returns the maximum width for the given layer, if constrained.
    pub fn max_width(&self, layer: LayerId) -> Option<i64>;

    /// Returns the (x, y) enclosure requirement for a via on the given layer.
    pub fn via_enclosure(&self, via: ViaId, layer: LayerId) -> [i64; 2];

    /// Returns the minimum area for the given layer, if constrained.
    pub fn min_area(&self, layer: LayerId) -> Option<i64>;

    /// Returns the EOL spacing for the given layer and eol width,
    /// or None if no EOL rule applies.
    pub fn eol_spacing(&self, layer: LayerId, width: i64) -> Option<i64>;

    /// Returns the spacing considering parallel run length.
    /// Falls back to base spacing if no PRL rule matches.
    pub fn prl_spacing(&self, layer: LayerId, width: i64, prl: i64) -> i64;
}
```

**Dispatch invariant.** Width-dependent spacing works by finding the rule
whose `width_threshold_nm` is the largest value <= `max(width_a, width_b)`.
If no width-dependent rule matches, the base (unconditional) spacing is
returned. PRL spacing works analogously with `prl_threshold_nm`. The returned
value is always the strictest applicable rule.

### ExtractModel

```rust
pub struct ExtractModel {
    pub corners: Vec<String>,
    pub default_corner: String,
    pub sheet_resistances: Vec<SheetResistance>,
    pub layer_caps: Vec<LayerCap>,
    pub coupling_caps: Vec<CouplingCap>,
    pub via_resistances: Vec<ViaResistance>,
}

pub struct SheetResistance {
    pub layer: LayerId,
    pub resistance_ohm_sq: f64,
    pub corner: String,
}

pub struct LayerCap {
    pub layer: LayerId,
    pub area_cap_ff_per_um2: f64,
    pub fringe_cap_ff_per_um: f64,
    pub corner: String,
}

pub struct CouplingCap {
    pub layer1: LayerId,
    pub layer2: LayerId,
    pub coupling_cap_ff_per_um: f64,
    pub coupling_window_nm: i64,
    pub corner: String,
}

pub struct ViaResistance {
    pub via: ViaId,
    pub resistance_ohm: f64,
    pub corner: String,
}
```

`ExtractModel` methods:
```rust
impl ExtractModel {
    pub fn sheet_r(&self, layer: LayerId, corner: &str) -> Option<f64>;
    pub fn area_cap(&self, layer: LayerId, corner: &str) -> Option<f64>;
    pub fn fringe_cap(&self, layer: LayerId, corner: &str) -> Option<f64>;
    pub fn coupling_cap(&self, l1: LayerId, l2: LayerId, corner: &str) -> Option<f64>;
    pub fn via_r(&self, via: ViaId, corner: &str) -> Option<f64>;

    /// Convenience: use default corner.
    pub fn sheet_r_default(&self, layer: LayerId) -> Option<f64>;
    pub fn area_cap_default(&self, layer: LayerId) -> Option<f64>;
    pub fn fringe_cap_default(&self, layer: LayerId) -> Option<f64>;
    pub fn via_r_default(&self, via: ViaId) -> Option<f64>;
}
```

---

## 5. The importer pipeline

### Two-stage design

```
External PDK files                  Philis-native schema         CompiledTech
(LEF, Sky130 files,         -->     (JSON file on disk     -->   (in-memory,
 hand-authored JSON)                 or in-memory)               immutable)

      IMPORT                              COMPILE
  (format-specific)                  (format-agnostic)
```

**Stage 1 — Import:** A format-specific importer reads external PDK files and
produces a Philis-native JSON schema. The importer is responsible for mapping
external layer names, rule formats, and parameter conventions into the Philis
schema vocabulary. The user can edit the resulting JSON before compilation.

**Stage 2 — Compile:** The compiler reads the Philis schema and builds a
`CompiledTech`. It validates structural invariants, classifies coverage, and
computes the PDK hash. The compiler is completely format-agnostic — it never
sees the original external files.

### Importer trait

```rust
pub trait Importer {
    /// Import from the given source and produce a Philis PDK schema.
    fn import(&self, source: &ImportSource) -> Result<PdkSchema, ImportError>;
}

pub enum ImportSource {
    /// Path to a Technology LEF file.
    Lef(PathBuf),
    /// Path to a Sky130 PDK root directory.
    Sky130(PathBuf),
    /// Path to a hand-authored JSON schema.
    Json(PathBuf),
    /// In-memory JSON string.
    JsonString(String),
}
```

### LEF importer

Reads Technology LEF (the process layer definition, not library cell LEF) and
extracts:
- Layer definitions: name, type, direction, pitch, width, spacing
- Via definitions: layers, cut geometry, enclosures
- Site definitions

Does NOT extract: parasitic models, antenna rules, density rules (these come
from other files or are hand-specified in the schema).

Coverage for LEF-imported rules: `Coded` for rules that are directly encoded
in the LEF. Everything not in the LEF is `Missing`.

### SKY130 importer

A targeted importer that produces a complete Philis schema for SKY130. Uses
a combination of:
- Hardcoded layer/via/rule data extracted from the SKY130 PDK documentation
  and DRC rule deck
- Hardcoded extraction parameters from published data

This is more complete than the generic LEF importer because SKY130 is a
primary development target and its structure is known.

### Hand-authored JSON

For PDKs without an importer, users write the JSON schema directly. The
compiler validates it and reports missing required fields. Coverage annotations
let the user honestly declare what they know and what they do not.

### Compilation entry point

```rust
impl CompiledTech {
    /// Compile from a Philis PDK schema.
    pub fn compile(schema: &PdkSchema) -> Result<CompiledTech, CompileError>;

    /// Convenience: import + compile in one call.
    pub fn from_source(source: &ImportSource) -> Result<CompiledTech, TechError>;

    /// Load from a JSON file and compile.
    pub fn from_json_file(path: &Path) -> Result<CompiledTech, TechError>;
}
```

---

## 6. Coverage classification and PdkState derivation

### The Coverage enum

```rust
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Coverage {
    /// Rule is fully coded with an internal predicate. The engine can
    /// check and prove compliance.
    Coded,
    /// Rule is partially coded. Some conditions are checked internally;
    /// others require external verification.
    Partial,
    /// Rule is coded but with known accuracy limitations (e.g., EM models
    /// with approximate current estimates).
    Approximate,
    /// Rule exists but can only be checked by an external signoff deck.
    ExternalOnly,
    /// Rule exists and requires human review (e.g., foundry guidelines
    /// without machine-checkable predicates).
    ManualReview,
    /// Rule is known to exist for this node but is not modeled at all.
    Missing,
    /// Rule is not applicable to this node (e.g., multipatterning for
    /// a mature single-patterning process).
    NotApplicable,
    /// Two or more sources provide contradictory data for this rule.
    Contradictory,
    /// Rule class is known but no predicate exists in the engine.
    Unsupported,
}
```

**Ordering.** The enum derives `Ord` with variants ordered from best (Coded)
to worst (Unsupported/Contradictory). The lattice meet operation returns the
worse of two values:

```rust
pub fn coverage_meet(a: Coverage, b: Coverage) -> Coverage {
    std::cmp::max(a, b)  // max because worse = higher ordinal
}
```

### RuleCoverageReport

```rust
pub struct RuleCoverageReport {
    pub coded: u32,
    pub partial: u32,
    pub approximate: u32,
    pub external_only: u32,
    pub manual_review: u32,
    pub missing: u32,
    pub not_applicable: u32,
    pub contradictory: u32,
    pub unsupported: u32,
    pub total: u32,
}

impl RuleCoverageReport {
    /// Fraction of rules that are Coded.
    pub fn coded_fraction(&self) -> f64;

    /// Fraction of rules that are at least Partial (Coded or Partial).
    pub fn at_least_partial_fraction(&self) -> f64;

    /// True if any rule is Contradictory.
    pub fn has_contradictions(&self) -> bool;
}
```

### PdkState derivation

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum PdkState {
    /// All required rules are Coded. Full signoff confidence.
    FullSignoff,
    /// Most required rules are at least Partial. Abstract/schematic-level
    /// placement and routing are possible with known gaps.
    AbstractComplete,
    /// Some required rules are present but coverage is sparse. Research
    /// and prototyping only.
    SparseResearch,
    /// At least one required rule is Missing or Contradictory.
    /// The compiled technology may produce incorrect results.
    ContradictoryOrMissing,
}
```

**Derivation algorithm:**

```rust
fn derive_pdk_state(report: &RuleCoverageReport, required: &RequiredRules) -> PdkState {
    // 1. If any required rule is Missing or Contradictory:
    //    -> ContradictoryOrMissing
    if required.any_missing_or_contradictory() {
        return PdkState::ContradictoryOrMissing;
    }

    // 2. If all required rules are Coded:
    //    -> FullSignoff
    if required.all_coded() {
        return PdkState::FullSignoff;
    }

    // 3. If most required rules are at least Partial:
    //    -> AbstractComplete
    if required.at_least_partial_fraction() >= 0.7 {
        return PdkState::AbstractComplete;
    }

    // 4. Otherwise:
    //    -> SparseResearch
    PdkState::SparseResearch
}
```

**"Required" rules** are the minimum set without which the engine cannot
produce legal geometry:
- Basic spacing for each routing layer (unconditional, same-layer)
- Basic width for each routing layer
- Via enclosure for each via in the stack
- Connectivity extraction parameters for each routing layer (sheet R)

---

## 7. Sky130 concrete implementation

### What SKY130 gives us

- **Routing layers:** li1 (local interconnect), met1-met5
- **Vias:** licon1, mcon, via, via2, via3, via4
- **Devices:** nfet_01v8, pfet_01v8 (and low/high-Vt variants), resistors,
  capacitors
- **Grid:** 5nm manufacturing grid, 460nm site height, 340nm M1 pitch (typical)
- **DRC rules:** ~200+ rules (spacing, width, enclosure, overlap, area,
  antenna, well, tap, guard ring)
- **Terminal order:** D, G, S, B for MOSFETs

### Coverage classification for SKY130

| Rule class | Coverage | Rationale |
|------------|----------|-----------|
| min-width (all routing layers) | `Coded` | directly from DRC deck |
| min-spacing (all routing layers, base) | `Coded` | directly from DRC deck |
| wide-metal spacing | `Coded` | width-dependent rules encoded |
| via enclosure | `Coded` | per-via enclosure from DRC deck |
| min-area | `Coded` | per-layer area rules from DRC deck |
| PRL spacing | `Partial` | some layers only |
| EOL spacing | `Partial` | some layers only |
| antenna | `ExternalOnly` | complex area-ratio rules, needs external check |
| density/fill | `ExternalOnly` | density rules exist but fill generation external |
| EM/IR drop | `Approximate` | resistivity present, current models approximate |
| LDE (LOD, WPE) | `Approximate` | models exist but accuracy varies |
| CMP/lithography | `Missing` | open PDK does not provide |
| multipatterning | `NotApplicable` | mature single-patterning node |
| ESD | `ManualReview` | rules exist as guidelines |
| latch-up/guard ring | `Partial` | tap/guard rules coded, spacing conservative |

**PdkState for SKY130 V1:** `SparseResearch` — enough for prototyping and
placement/routing development, not for signoff.

### Key layer data (hardcoded)

| Layer | GDS | Class | Direction | Pitch (nm) | Min width (nm) | Min spacing (nm) |
|-------|-----|-------|-----------|------------|----------------|-------------------|
| li1 | 67/20 | LocalInterconnect | Any | 460 | 170 | 170 |
| met1 | 68/20 | Metal(1) | Horizontal | 340 | 140 | 140 |
| met2 | 69/20 | Metal(2) | Vertical | 340 | 140 | 140 |
| met3 | 70/20 | Metal(3) | Horizontal | 680 | 300 | 300 |
| met4 | 71/20 | Metal(4) | Vertical | 680 | 300 | 300 |
| met5 | 72/20 | Metal(5) | Horizontal | 3400 | 1600 | 1600 |

### Key via data (hardcoded)

| Via | Bottom | Top | Cut size (nm) | Bottom enc (nm) | Top enc (nm) | R (ohm) |
|-----|--------|-----|---------------|-----------------|--------------|---------|
| mcon | li1 | met1 | 170x170 | 60,30 | 30,60 | 9.3 |
| via | met1 | met2 | 150x150 | 55,85 | 55,85 | 4.5 |
| via2 | met2 | met3 | 200x200 | 40,85 | 65,65 | 3.4 |
| via3 | met3 | met4 | 200x200 | 60,90 | 65,65 | 3.4 |
| via4 | met4 | met5 | 800x800 | 190,190 | 310,310 | 0.38 |

### Extraction data (tt corner, hardcoded)

| Layer | Sheet R (ohm/sq) | Area cap (fF/um2) | Fringe cap (fF/um) |
|-------|-------------------|--------------------|--------------------|
| li1 | 12.8 | 0.022 | 0.038 |
| met1 | 0.125 | 0.038 | 0.045 |
| met2 | 0.125 | 0.033 | 0.040 |
| met3 | 0.047 | 0.020 | 0.032 |
| met4 | 0.047 | 0.015 | 0.028 |
| met5 | 0.029 | 0.010 | 0.022 |

### Implementation approach

For V1, all SKY130 data is hardcoded in `import/sky130.rs` as Rust constants.
The importer produces a `PdkSchema` from these constants. No external file
parsing in V1.

```rust
pub struct Sky130Importer;

impl Importer for Sky130Importer {
    fn import(&self, _source: &ImportSource) -> Result<PdkSchema, ImportError> {
        Ok(PdkSchema {
            name: "sky130".into(),
            // ... hardcoded data from the tables above ...
        })
    }
}
```

---

## 8. Hashing for certificates

### What gets hashed

The PDK hash covers all data that affects the compiled technology:
- Schema version, name, grid/unit parameters
- All layer definitions (name, class, direction, dimensions)
- All via definitions (layers, cut size, enclosure, resistance)
- All DRC rules (spacing, width, enclosure, area, EOL)
- All extraction parameters (sheet R, caps, via R, per corner)
- The coverage classification of every rule
- The derived PdkState

### Hash algorithm

BLAKE3. Fast, deterministic, and collision-resistant.

### Hash computation

```rust
impl CompiledTech {
    /// Compute the BLAKE3 hash of all compiled data.
    /// Called once at construction time. Stored in `self.pdk_hash`.
    fn compute_hash(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(self.name.as_bytes());
        hasher.update(&self.db_unit_nm.to_le_bytes());
        hasher.update(&self.manufacturing_grid_nm.to_le_bytes());
        // ... hash all layers, vias, rules, extraction, coverage ...
        *hasher.finalize().as_bytes()
    }
}
```

### Stability invariant

Same input data must produce the same hash across Rust compiler versions,
platforms, and runs. This requires:
- Deterministic iteration order (BTreeMap, not HashMap, or sorted Vec)
- Canonical byte encoding (little-endian for integers, UTF-8 for strings)
- No floating-point in the hash input (f64 values are hashed as their
  `to_le_bytes()` representation — bit-exact, not value-exact)

### Certificate integration

The hash goes into the constraint certificate's `pdk_oracle_hash` field and
the route certificate's `pdk_oracle_hash` field. Any change to the compiled
technology produces a different hash, invalidating all cached results.

---

## 9. Phase plan

### Phase 0: Types and scaffolding (prerequisite)

**Files:** `types.rs`, `coverage.rs`, `lib.rs`

- Define `LayerId`, `ViaId`, `LayerClass`, `LayerDirection`, `Coverage`,
  `PdkState`, `RuleCoverageReport`
- Coverage lattice operations (meet, merge_required)
- PdkState derivation function
- Unit tests for lattice axioms

### Phase 1: Schema and compilation

**Files:** `schema.rs`, `validate.rs`, `compile.rs`, `compiled.rs`

- Define the `PdkSchema` serde types (JSON deserialization)
- Schema validation (structural invariants)
- `CompiledTech::compile(schema)` builder
- Typed validation errors

### Phase 2: Layer, via, grid, rule structures

**Files:** `layer.rs`, `via.rs`, `grid.rs`, `rules.rs`

- `LayerStack` with random access and name lookup
- `ViaTable` with layer-pair lookup
- `GridTable` with track position computation
- `RuleSet` with all dispatch methods (min_spacing, min_width,
  via_enclosure, min_area, eol_spacing, prl_spacing)

### Phase 3: Extraction model

**Files:** `extract.rs`

- `ExtractModel` with per-corner lookups
- Sheet resistance, area cap, fringe cap, coupling cap, via resistance

### Phase 4: SKY130 importer

**Files:** `import/mod.rs`, `import/sky130.rs`

- Hardcoded SKY130 data as Rust constants
- Produces a complete `PdkSchema`
- Smoke test: import -> compile -> verify layer/via/rule counts

### Phase 5: Hashing

**Files:** `hash.rs`

- BLAKE3 hash computation over all compiled data
- Stability test: same input -> same hash

### Phase 6: LEF importer

**Files:** `import/lef.rs`

- Parse Technology LEF (layer/via/site definitions)
- Produce Philis schema
- Coverage annotations for LEF-imported data

### Phase 7: Integration

**Files:** `import/json.rs`, integration tests

- JSON passthrough importer
- Schema round-trip tests
- Full pipeline tests (import -> compile -> query)
- Consumer integration: wire up constraints, place, route

---

## 10. Testing strategy

### Unit tests

| Test | Module | What it verifies |
|------|--------|------------------|
| Lattice axioms | `coverage.rs` | commutativity, associativity, idempotence |
| Conservative direction | `coverage.rs` | `meet(Coded, Missing) = Missing` |
| Universal Coded | `coverage.rs` | `merge_required([Coded, Coded]) = Coded` |
| Single degradation | `coverage.rs` | `merge_required([Coded, Approximate]) = Approximate` |
| PdkState derivation | `coverage.rs` | correct state for each coverage distribution |
| Schema round-trip | `schema.rs` | serialize -> deserialize -> equal |
| Schema validation | `validate.rs` | missing required fields, negative values, bad references |
| Layer lookup | `layer.rs` | by-name, by-id, routing filter |
| Via lookup | `via.rs` | by-name, by-layer-pair, multi-via pairs |
| Grid track positions | `grid.rs` | track_center, snap_to_track, tracks_in_range |
| Spacing dispatch | `rules.rs` | base, width-dependent, PRL, same-net |
| Width dispatch | `rules.rs` | min_width, max_width per layer |
| Enclosure dispatch | `rules.rs` | per-via, per-side |
| Area dispatch | `rules.rs` | min_area per layer |
| EOL dispatch | `rules.rs` | eol_spacing lookup |
| Extraction lookup | `extract.rs` | per-corner, default corner, missing corner |
| Hash stability | `hash.rs` | same input -> same hash, different input -> different hash |

### Integration tests

| Test | What it verifies |
|------|------------------|
| SKY130 smoke | Import SKY130, compile, verify expected layer/via/rule counts |
| SKY130 coverage | Verify expected Coverage values for key rule classes |
| SKY130 PdkState | Verify `SparseResearch` for V1 data |
| Full pipeline | Import -> validate -> compile -> query all dispatch methods |
| JSON round-trip | Import SKY130 -> export schema JSON -> re-import -> equal |

### Property tests (if using proptest)

- For any two Coverage values a, b: `meet(a, b) = meet(b, a)` (commutativity)
- For any three values: `meet(meet(a, b), c) = meet(a, meet(b, c))` (associativity)
- For any value: `meet(a, a) = a` (idempotence)
- For any valid schema: `compile(schema).is_ok()` implies all required fields present
- For any layer in compiled tech: `min_spacing(layer, 0, 0) > 0`

---

## 11. API surface (what is exported)

### Primary public types

```rust
// The compiled technology oracle
pub use compiled::CompiledTech;

// Identifiers
pub use types::{LayerId, ViaId};

// Enums
pub use layer::{LayerClass, LayerDirection};
pub use coverage::{Coverage, PdkState, RuleCoverageReport};

// Sub-structures (available via accessors on CompiledTech)
pub use layer::{LayerStack, LayerDef};
pub use via::{ViaTable, ViaDef};
pub use grid::{GridTable, GridDef};
pub use rules::{RuleSet, SpacingRule, WidthRule, EnclosureRule, AreaRule, EolRule};
pub use extract::{ExtractModel, SheetResistance, LayerCap, CouplingCap, ViaResistance};

// Schema types (for importers and serialization)
pub use schema::PdkSchema;

// Import pipeline
pub use import::{Importer, ImportSource, ImportError};
pub use compile::CompileError;
pub use validate::ValidationError;
```

### What each consumer needs

**crates/constraints:**
- `CompiledTech` (full access, for coverage inventory and rule predicates)
- `Coverage`, `PdkState`, `RuleCoverageReport` (for the coverage inventory
  that feeds the ConstraintCertificate)
- `LayerId` (for rule references in the IntentGraph)
- `pdk_hash()` (for the certificate's `pdk_oracle_hash`)

**crates/place:**
- `CompiledTech` (via `&` reference)
- `LayerStack`, `LayerDef`, `LayerClass` (for layer roles and dimensions)
- `GridTable`, `GridDef` (for track positions, row pitch, site width)
- `RuleSet` dispatch methods: `min_spacing()`, `min_width()`,
  `via_enclosure()` (for DRC oracle queries during placement)
- `Coverage`, `PdkState` (for the placement coverage report)
- `pdk_hash()` (for certificate)

**crates/route:**
- `CompiledTech` (via `&` reference)
- `LayerStack`, `LayerDef`, `LayerClass`, `LayerDirection` (for layer stack
  traversal and direction assignment)
- `ViaTable`, `ViaDef` (for via selection and via array generation)
- `GridTable`, `GridDef` (for track assignment and snapping)
- `RuleSet` dispatch methods: all of them (min_spacing, min_width,
  via_enclosure, min_area, eol_spacing, prl_spacing)
- `ExtractModel` (for in-loop PEX surrogate: wire resistance, cap estimation,
  coupling analysis, via resistance)
- `Coverage`, `PdkState`, `RuleCoverageReport` (for the route certificate)
- `pdk_hash()` (for the route certificate's `pdk_oracle_hash`)

### What is NOT exported

- Internal compilation intermediates (validation pass results, hash
  computation state)
- Importer internals (LEF parser state, SKY130 hardcoded constant arrays)
- Mutable builder state (the compiled tech is immutable)
- Any type from `philis-geom` that consumers can get directly from that crate

---

## Open questions

### 1. Should the schema support conditional rules beyond width/PRL/EOL?

Some PDK rules are multi-variable (e.g., spacing depends on width AND
parallel run length AND whether shapes are on the same net AND their
relative orientation). The current schema handles the common cases (width,
PRL, EOL, same-net) as structured fields.

**Recommendation:** Do not add an expression language yet. Model the common
cases. Classify everything else as `Partial` or `ExternalOnly`. The honest
coverage model means the certificate says exactly what is checked and what
is not.

### 2. Should the CompiledTech support overlays?

A user might want to override a coverage value ("I know antenna is actually
Coded because I have a deck"). The `CompiledTech` should support a
`with_overlay(overrides)` method that applies user-provided coverage upgrades
(only upgrades are automatic — downgrades to Missing/Contradictory are always
allowed, upgrades to Coded require authority).

**Recommendation:** Defer to phase 7. The overlay mechanism is important for
production use but not for initial development.

### 3. How to handle IHP SG13G2?

IHP SG13G2 (130nm SiGe BiCMOS) is the next target. It has a different layer
stack, different via definitions, and bipolar devices. The `LayerClass` enum
may need extension (e.g., `Bipolar`). The importer will be similar in
structure to the SKY130 importer.

**Recommendation:** Build the SKY130 importer first, then write the SG13G2
importer as validation that the schema and compiler are PDK-agnostic.
