# Phase 2 — Technology Compilation

> **Shared crate:** The `CompiledTech` type, `LayerStack`, `ViaTable`,
> `RuleSet`, `ExtractModel`, coverage classification, and `PdkState`
> derivation now live in `crates/tech` (`philis_tech`). The constraints
> crate's `technology/` module becomes a consumer of `philis_tech` rather
> than implementing compilation from scratch. The `CompiledTechnology`
> trait defined below is either replaced by or wraps
> `philis_tech::CompiledTech`, and the SKY130 concrete implementation
> delegates to `philis_tech`'s SKY130 support.

**Goal:** A technology consumer module that wraps `philis_tech::CompiledTech`
and populates the coverage inventory for constraint compilation.

**Depends on:** phase 1 (types, CoverageValue), `crates/tech` (`philis_tech`)  
**Unlocks:** phase 4 (intent compilation needs technology to classify coverage),
phase 6 (projections need rule IDs)

---

## 2.1 The `CompiledTechnology` trait — `src/technology/mod.rs`

This is the process-portability boundary. Every query the constraint compiler
makes about "what can the PDK check?" goes through this trait.

```rust
pub trait CompiledTechnology: Send + Sync {
    /// Unique identifier for this compiled technology (e.g. "sky130-1.0").
    fn id(&self) -> &str;

    /// Hash of the full compiled state — participates in certificate hashes.
    fn state_hash(&self) -> u64;

    /// The PDK readiness state, classified from the coverage inventory.
    fn pdk_state(&self) -> PdkState;

    // --- units & grid ---
    fn manufacturing_grid_nm(&self) -> i64;
    fn row_pitch_nm(&self) -> i64;
    fn site_width_nm(&self) -> i64;
    fn track_pitches_nm(&self) -> &[LayerTrackPitch];

    // --- layers ---
    fn layers(&self) -> &[LayerDef];
    fn layer_by_name(&self, name: &str) -> Option<&LayerDef>;

    // --- devices ---
    fn device_models(&self) -> &[DeviceModel];
    fn device_by_name(&self, name: &str) -> Option<&DeviceModel>;
    fn terminal_order(&self, device_model: &str) -> Option<&[String]>;

    // --- rules ---
    fn rule_ids(&self) -> &[RuleId];
    fn rule_coverage(&self, rule_id: &RuleId, key: &McmmKey) -> CoverageValue;
    fn rule_predicate(&self, rule_id: &RuleId) -> Option<&RulePredicate>;

    // --- coverage inventory ---
    fn coverage_inventory(&self) -> &CoverageInventory;
    fn coverage_report(&self) -> RuleCoverageReport;

    // --- extraction / analysis ---
    fn pex_corners(&self) -> &[PexCorner];
    fn em_models(&self) -> &[EmModel];
    fn deck_bindings(&self) -> &[DeckBinding];
}
```

### Supporting types

```rust
/// The six-dimensional MCMM key for coverage lookups.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct McmmKey {
    pub rule_id: RuleId,
    pub scope: String,       // device or layer name
    pub corner: String,      // PVT corner (e.g. "tt", "ss", "ff")
    pub mode: String,        // operating mode
    pub deck_id: String,     // analysis deck identifier
    pub host_template: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RuleId(pub String);  // stable across compilations

pub struct LayerDef {
    pub name: String,
    pub role: LayerRole,       // Active, Poly, Metal, Via, Well, Implant, ...
    pub index: u16,            // metal/via index (M1=0, V1=0, M2=1, ...)
    pub preferred_dir: Option<Axis>,
    pub min_width_nm: i64,
    pub min_spacing_nm: i64,
}

pub struct DeviceModel {
    pub name: String,          // e.g. "sky130_fd_pr__nfet_01v8"
    pub kind: DeviceKind,      // Nmos, Pmos, Resistor, Capacitor, Diode, Bipolar, ...
    pub terminals: Vec<String>,
    pub parameters: Vec<DeviceParam>,
}

pub struct RulePredicate {
    pub rule_id: RuleId,
    pub kind: RuleKind,        // Spacing, Width, Enclosure, Overlap, Antenna, ...
    pub layers: SmallVec<[String; 2]>,
    pub value_nm: Option<i64>,
    pub condition: Option<String>,  // conditional rule context
}

pub struct CoverageInventory {
    /// The full MCMM-keyed coverage map.
    entries: HashMap<McmmKey, CoverageValue>,
}
```

The `CoverageInventory` implements:
- `lookup(key: &McmmKey) -> CoverageValue` — returns `Missing` for absent keys.
- `merge_required(keys: &[McmmKey]) -> CoverageValue` — fold meet over the keys.
- `report() -> RuleCoverageReport` — aggregate counts per CoverageValue.
- `contradictions() -> Vec<McmmKey>` — keys with `Contradictory`.

---

## 2.2 SKY130 concrete implementation — `src/technology/sky130.rs`

> **Shared crate:** The core SKY130 technology data (layers, vias, rules,
> extraction parameters) is compiled by `philis_tech`. This module wraps
> the `philis_tech` SKY130 compilation and adds constraint-specific
> coverage inventory entries.

The first real technology. Built from the open-source SKY130 PDK data
via `philis_tech`.

### What SKY130 gives us

- **5 metal layers** (li1, met1–met4, met5 optional), 1 local interconnect
- **Core devices:** nfet_01v8, pfet_01v8, nfet_01v8_lvt, pfet_01v8_hvl,
  nfet_g5v0d10v5 (3.3V/5V), pfet_g5v0d10v5, resistors, capacitors
- **DRC rules:** ~200+ rules covering spacing, width, enclosure, overlap, area,
  antenna, well, tap, guard ring
- **Terminal order:** D, G, S, B for MOSFETs
- **Grid:** 460nm site height, 480nm M1 pitch (typical values)

### Coverage classification for SKY130

| Rule class | SKY130 coverage | Rationale |
|------------|----------------|-----------|
| min-width, min-spacing (all layers) | `Coded` | fully encoded in DRC deck |
| enclosure, overlap, PRL | `Coded` | standard rules present |
| antenna | `ExternalOnly` | rule exists but complex area-ratio, needs external check |
| density / fill | `ExternalOnly` | density rules exist but fill generation is external |
| EM / IR drop | `Approximate` | resistivity data present, current models approximate |
| LDE (LOD, WPE) | `Approximate` | models exist but accuracy varies |
| CMP / lithography | `Missing` | open PDK does not provide |
| color / multipatterning | `Missing` | not applicable (mature node) |
| ESD | `ManualReview` | rules exist as guidelines |
| latch-up / guard ring | `Partial` | tap/guard rules coded, spacing conservative |

### Implementation approach

```rust
pub struct Sky130Technology {
    inner: philis_tech::CompiledTech,  // from crates/tech
    inventory: CoverageInventory,       // constraint-specific coverage
    state_hash: u64,
}

impl Sky130Technology {
    /// Build by calling philis_tech's SKY130 compiler, then layering
    /// constraint-specific coverage inventory on top.
    pub fn new() -> Self {
        let inner = philis_tech::sky130::compile();
        let inventory = build_constraint_coverage(&inner);
        // ...
    }
}

impl CompiledTechnology for Sky130Technology { ... }
```

The core PDK data (layers, vias, spacing rules) comes from `philis_tech`,
which provides the SKY130 compilation. The constraints crate adds its own
coverage inventory entries for constraint-specific concerns (e.g., which
rule classes affect which constraint kinds).

### Data encoding

Layer data, device data, and rule data are encoded as `const` arrays:

```rust
const SKY130_LAYERS: &[(&str, LayerRole, i64, i64)] = &[
    ("li1",  LayerRole::LocalInterconnect, 170, 170),
    ("met1", LayerRole::Metal, 140, 140),
    ("met2", LayerRole::Metal, 140, 140),
    ("met3", LayerRole::Metal, 300, 300),
    ("met4", LayerRole::Metal, 300, 300),
    ("met5", LayerRole::Metal, 1600, 1600),
    // ... wells, implants, poly, diffusion, etc.
];
```

Rules are similarly encoded. The total data is ~2–5 KB of const arrays.

---

## 2.3 Coverage lattice operations — `src/technology/coverage.rs`

The `CoverageValue` lattice from `types.rs` gets its meet and inventory
operations here:

```rust
/// Meet of two coverage values — conservative (worst) direction.
/// Already implemented via Ord on CoverageValue (min).
pub fn coverage_meet(a: CoverageValue, b: CoverageValue) -> CoverageValue {
    std::cmp::min(a, b)
}

/// Merge required views: fold meet. Coded iff all Coded.
pub fn merge_required(views: &[CoverageValue]) -> CoverageValue {
    views.iter().copied().fold(CoverageValue::Coded, coverage_meet)
}
```

### Testing

- **Lattice axioms:** commutativity, associativity, idempotence.
- **Conservative direction:** `meet(Coded, Missing) = Missing`.
- **Universal Coded:** `merge_required([Coded, Coded, Coded]) = Coded`.
- **Single degradation:** `merge_required([Coded, Coded, Approximate]) = Approximate`.
- **SKY130 inventory:** load Sky130Technology, verify rule counts match expected
  coverage distribution.
- **MCMM lookup:** absent key returns `Missing`.
- **Contradiction detection:** inject two conflicting entries, verify
  `Contradictory` cell.

---

## 2.4 Open questions

1. **How much SKY130 data to hardcode?** The full DRC deck has 200+ rules.
   Starting with the ~30 most common placement/routing-relevant rules
   (spacing, width, enclosure for each metal layer + well/tap) is sufficient
   for the first useful constraint compilation. Antenna, density, EM can
   follow.

2. **Should technology data be serializable?** Yes — the certificate needs to
   record a hash of the compiled technology. Serde on the inventory is
   sufficient; we don't need to serialize the full rule predicates for the
   certificate (just the hash + the coverage report).

3. **How to handle technology overlay?** A user might want to override a
   coverage value ("I know antenna is actually Coded because I have a deck").
   The `CoverageInventory` should support a `with_overlay(overrides)` method
   that applies user-provided coverage upgrades (only upgrades — downgrades
   to Missing/Contradictory are always allowed, upgrades to Coded require
   authority).

---

## 2.5 Deliverables checklist

- [ ] `src/technology/mod.rs` — `CompiledTechnology` trait, supporting types
- [ ] `src/technology/sky130.rs` — SKY130 concrete, all data encoded
- [ ] `src/technology/coverage.rs` — lattice operations, inventory
- [ ] Unit tests for lattice axioms
- [ ] SKY130 inventory test (expected coverage distribution)
- [ ] MCMM key lookup tests
