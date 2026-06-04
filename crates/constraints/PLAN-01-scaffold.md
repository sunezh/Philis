# Phase 1 — Scaffold & Core Types

**Goal:** The `philis_constraints` crate compiles, joins the workspace, and
exports the canonical type vocabulary with a working content-addressed ID
scheme.

**Depends on:** nothing  
**Unlocks:** every other phase

---

## 1.1 Crate setup

### Cargo.toml

```toml
[package]
name = "philis_constraints"
version = "0.1.0"
edition = "2021"
rust-version = "1.74"
description = "Constraint compiler for the Philis analog layout engine"
license = "MIT"

[dependencies]
blake3 = "1"           # content-addressed identity
serde = { version = "1", features = ["derive"] }  # serialization for certificate/replay
smallvec = "1"         # entity lists, typically < 8 elements
rustc-hash = "2"       # FxHashMap/FxHashSet for internal hot paths
```

### Workspace addition

Add `"crates/constraints"` to the `[workspace] members` list in the root
`Cargo.toml`.

### Entry point — `src/lib.rs`

```rust
pub mod id;
pub mod types;
pub mod compat;

pub mod technology;
pub mod facts;
pub mod intent;
pub mod reconcile;
pub mod project;
pub mod evidence;
pub mod quality;
pub mod solver;
pub mod certificate;
pub mod diagnostics;
```

Each sub-module starts as a file with a `// TODO: phase N` comment and no
public items, so the crate compiles from commit one.

---

## 1.2 Content-addressed identity — `src/id.rs`

The single most load-bearing primitive. Everything downstream keys on this.

### `CanonicalId`

```rust
/// A content-addressed obligation identity: `c_<hex64>`.
///
/// The first 8 bytes (64 bits) of a BLAKE3 hash of the canonical preimage.
/// Truncation is acceptable for obligation IDs (not security-critical);
/// collision probability is ~1 in 2^32 at 10^9 obligations.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CanonicalId([u8; 8]);
```

Display as `c_<hex64>` (16 hex chars).

### Hashing protocol

```rust
pub struct IdHasher {
    hasher: blake3::Hasher,
}

impl IdHasher {
    pub fn new(seed: u64) -> Self;

    /// Feed a &str field.
    pub fn field_str(&mut self, tag: &str, value: &str) -> &mut Self;

    /// Feed a u64 / i64 / bool field.
    pub fn field_u64(&mut self, tag: &str, value: u64) -> &mut Self;

    /// Feed a sorted slice of CanonicalIds (for entity lists, core members).
    pub fn field_ids(&mut self, tag: &str, ids: &[CanonicalId]) -> &mut Self;

    /// Finalize to a CanonicalId.
    pub fn finish(&self) -> CanonicalId;
}
```

The `tag` prefix before each value prevents ambiguity between fields that
happen to have the same byte representation. The protocol:

1. For each field, write `tag.len() as u32 LE`, then `tag` bytes, then
   `value.len() as u32 LE`, then value bytes.
2. Entity lists and evidence-domain lists are **sorted** before feeding.
3. The seed participates as the first field (`"seed"`, seed bytes).
4. `finish()` returns `CanonicalId(hasher.finalize().as_bytes()[..8])`.

### `CoreId`

Contradiction cores also get content-addressed IDs:

```rust
/// `core_<hex64>` — hash of sorted member IDs + reason tag.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CoreId([u8; 8]);
```

Same hashing protocol, different display prefix.

### Testing

- **Determinism:** hash the same preimage 1000 times, assert all IDs equal.
- **Position independence:** construct the same obligation with entities in
  different insertion order, assert same ID.
- **Collision-as-merge:** two "identical" obligations constructed independently
  produce the same ID.
- **Distinct obligations hash apart:** change one semantic field, ID changes.
- **Seed separation:** same obligation with different seed, different ID.

---

## 1.3 Canonical types — `src/types.rs`

### `ObligationClass` (the canonical 7-variant enum)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ObligationClass {
    Hard,
    Joint,
    Bounded,
    Soft,
    Unsupported,
    ExternalOnly,
    ManualReview,
}
```

Method: `is_gate(&self) -> bool` — true for `Hard` and `Joint`.

### `CoverageValue` (the canonical 9-valued lattice)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub enum CoverageValue {
    Contradictory = 0,
    Missing = 1,
    Unsupported = 2,
    ManualReview = 3,
    ExternalOnly = 4,
    RecommendedOnly = 5,
    Approximate = 6,
    Partial = 7,
    Coded = 8,
}
```

The `Ord` derivation matches the information ordering (Coded = most info).

Method: `meet(self, other: Self) -> Self` — returns `min(self, other)` by the
Ord, which is the conservative direction.

Method: `is_signoff_quality(self) -> bool` — true only for `Coded`.

### `Authority`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Authority {
    HeuristicRepair = 0,
    InferredSoft = 1,
    InferredHard = 2,
    GeneratedPrimitive = 3,
    UserBounded = 4,
    UserHard = 5,
    HostTemplate = 6,
    FoundryMandatory = 7,
}
```

### `Scope`

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Scope {
    Global,
    Hierarchical(Vec<String>),
    Block(String),
    Device(String),
    Net(String),
    Terminal(String),
    Region(String),
    HostTemplate(String),
}
```

### `ConsumerMask`

```rust
bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct ConsumerMask: u8 {
        const PLACEMENT = 0b0001;
        const ROUTING   = 0b0010;
        const ANALYSIS  = 0b0100;
        const EXPORT    = 0b1000;
    }
}
```

Requires `bitflags = "2"` dependency.

### `PredicateKind` — closed enum, not an expression language

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PredicateKind {
    // Placement predicates
    SymmetricPair { axis: Axis },
    SelfSymmetric { axis: Axis },
    OrderedPair { axis: Axis },
    Aligned { axis: Axis },
    CommonCentroid,
    Interdigitated,
    SameTemplate,
    SameOrientation,
    Proximity { max_distance: i64 },
    Spread { min_distance: i64 },
    FixedPosition { x: i64, y: i64 },
    GridSnapped { pitch: i64 },
    BoundaryPlaced { side: String },
    Grouped,
    GuardRingEnclosed,

    // Routing predicates
    MatchedLength { max_delta_nm: i64 },
    MatchedParasitic { metric: String, max_delta: i64 },
    Shielded,
    LowNoise,
    HighImpedance,
    DifferentialRouting,
    DoNotRoute,
    MultiWire { count: u8 },
    NetClassAssignment { class: String },
    PortLocationBound { position: f64 },

    // Analysis / coverage predicates
    DrcRule { rule_id: String },
    LvsEquivalence,
    PexBound { metric: String, bound: i64, corner: String },
    EmBound { max_current_density: i64 },
    AntennaBound { ratio_max: i64 },
    ErcCheck { check_id: String },

    // Power / domain
    PowerNet,
    GroundNet,
    SubstrateNet,
    ClockNet,
    DomainSeparation { domain_a: String, domain_b: String },

    // Meta
    ManualReviewRequired { note: String },
    ExternalDeckRequired { deck_id: String },
}
```

This is a **closed enum that grows additively**. Each variant carries only
the parameters specific to that predicate kind. The variant name + parameters
participate in the content-addressed hash.

### `RepairPolicy`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RepairPolicy {
    GroupRepair,
    SingleDevice,
    NetReroute,
    TerminalAccessRegenerate,
    PdkGap,
    TemplateConflict,
    NoAutoRepair,
}
```

### `ConstraintTuple` — the lean core

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConstraintTuple {
    pub id: CanonicalId,
    pub scope: Scope,
    pub entities: SmallVec<[EntityRef; 4]>,  // sorted
    pub predicate: PredicateKind,
    pub class: ObligationClass,
    pub bound: Option<BoundValue>,
    pub source: Source,
    pub confidence: FixedConfidence,
    pub coverage: CoverageValue,
    pub consumers: ConsumerMask,
    pub evidence_domains: SmallVec<[EvidenceDomain; 2]>,  // sorted
    pub repair: RepairPolicy,
    pub waiver: Option<Waiver>,
}
```

Where:
- `EntityRef` is `{ kind: EntityKind, name: String }` with `EntityKind` being
  `Device | Net | Terminal | Pin | Region | Layer | Rule | Analysis`.
- `BoundValue` is `{ metric: String, value: i64, unit: String }`.
- `Source` is `{ authority: Authority, source_id: String }`.
- `FixedConfidence` is a `u16` in `[0, 10000]` representing `0.0000..1.0000`.
- `EvidenceDomain` is `DRC | LVS | PEX | EM | ERC | Antenna | Density | Color | Manual`.
- `Waiver` is `{ authority: Authority, scope: Scope, reason: String }`.

The `id` field is computed by `ConstraintTuple::compute_id(seed: u64) -> CanonicalId`
which feeds all semantic fields (everything except `waiver` and metadata) into
the `IdHasher`.

### Testing

- Round-trip `ConstraintTuple` through serde_json, verify ID stability.
- `ObligationClass::is_gate` truth table.
- `CoverageValue::meet` lattice properties: commutativity, associativity,
  idempotence, `meet(Coded, x) = x`, `meet(Contradictory, x) = Contradictory`.
- `Authority` ordering matches the spec precedence.
- `ConsumerMask` bit operations.

---

## 1.4 Compatibility bridge — `src/compat.rs`

Bidirectional mapping between canonical types and the api's realized subsets.

```rust
use crate::types::{ObligationClass, CoverageValue};

/// Map the canonical 7-variant class to the api's 4-variant IntentClass.
///
/// Joint → Hard (with group-preserving repair scope noted separately).
/// ExternalOnly, ManualReview → carried by coverage/evidence, not by class.
pub fn to_intent_class(class: ObligationClass) -> ApiIntentClass { ... }

/// Map the api's IntentClass back to the canonical class.
/// Ambiguous: Hard could be Hard or Joint. The `is_group` flag disambiguates.
pub fn from_intent_class(ic: ApiIntentClass, is_group: bool) -> ObligationClass { ... }

/// Map the canonical CoverageValue to the api's Coverage enum.
/// The api enum has all 9 variants, so this is 1:1.
pub fn to_api_coverage(cv: CoverageValue) -> ApiCoverage { ... }

/// Map the api's Coverage back to CoverageValue.
pub fn from_api_coverage(c: ApiCoverage) -> CoverageValue { ... }
```

These don't depend on the api crate at compile time — they return the
*canonical* types and the api integration test will call them with the api
types. This keeps the dependency arrow one-directional.

In practice: the compat module defines a trait `IntoCanonical<T>` / `FromCanonical<T>`
that the api crate implements for its own types when it adds the
`philis_constraints` dependency.

---

## 1.5 Deliverables checklist

- [ ] `Cargo.toml` with dependencies, added to workspace
- [ ] `src/lib.rs` with all sub-module declarations (stubs for phases 2–7)
- [ ] `src/id.rs` — `CanonicalId`, `CoreId`, `IdHasher`, all tests passing
- [ ] `src/types.rs` — all canonical enums and `ConstraintTuple`, all tests passing
- [ ] `src/compat.rs` — mapping functions, tested against truth tables
- [ ] `cargo test -p philis_constraints` green
- [ ] `cargo clippy -p philis_constraints` clean
