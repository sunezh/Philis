# Phase 4 — Intent Compilation & Inference Detectors

**Goal:** Build the `IntentGraph` from user-declared constraints and
topology-inferred intent. All detector families from the spec are in scope.

**Depends on:** phases 1–3 (types, technology, fact graph)  
**Unlocks:** phase 5 (reconciliation operates on the IntentGraph)

---

## 4.1 The `IntentGraph` — `src/intent/mod.rs`

The single canonical graph that all consumers read projections of.

```
IntentGraph {
    obligations: HashMap<CanonicalId, ConstraintTuple>,
    provenance: HashMap<CanonicalId, Vec<ProvenanceEdge>>,
    groups: Vec<ObligationGroup>,
    statistics: IntentStatistics,
}
```

### `ProvenanceEdge`

Every obligation records how it was discovered — user declaration, which
detector, what confidence. Multiple edges on the same ID means multiple
independent sources discovered the same obligation (collision-as-merge).

```
ProvenanceEdge {
    source: ProvenanceSource,    // UserDeclared, Detector(name), PdkDerived, HostDerived
    confidence: FixedConfidence,
    detector_version: Option<String>,
    evidence: Option<String>,    // human-readable reasoning
}
```

### `ObligationGroup`

Joint obligations (common centroid, interdigitated banks, matched groups)
that must be solved or repaired together:

```
ObligationGroup {
    id: CanonicalId,            // content-addressed from sorted member IDs
    members: Vec<CanonicalId>,  // sorted
    kind: GroupKind,            // CommonCentroid, Interdigitated, MatchedSet, DifferentialPair
    repair_policy: RepairPolicy,
}
```

### `IntentStatistics`

```
IntentStatistics {
    total_obligations: usize,
    by_class: HashMap<ObligationClass, usize>,
    by_source: HashMap<ProvenanceSource, usize>,
    detector_runtimes: HashMap<String, Duration>,
    groups: usize,
}
```

---

## 4.2 User intent compilation — `src/intent/compile.rs`

Takes `Vec<Constraint>` from the api crate and compiles each into one or more
`ConstraintTuple`s in the IntentGraph.

The mapping is defined in the doc's taxonomy table
(`docs/Developer/constraints/model.html`). Each `Constraint` variant maps to:
- A `PredicateKind`
- An `ObligationClass` (the canonical one, not the realized IntentClass)
- A `ConsumerMask`
- Entity references extracted from the variant's fields
- A `Source` with `Authority::UserHard` or `Authority::UserBounded`

### Compilation rules

| Constraint variant | PredicateKind | Class | Consumers |
|---|---|---|---|
| `Symmetric(a,b)` | `SymmetricPair { axis: inferred }` | Hard | place, route |
| `SelfSymmetric(a)` | `SelfSymmetric { axis: inferred }` | Hard | place |
| `SymmetricGroup(pairs)` | one `SymmetricPair` per pair + one group | Joint | place, route |
| `Order(a,b,axis)` | `OrderedPair { axis }` | Hard | place |
| `Align(devs, axis)` | `Aligned { axis }` | Hard | place |
| `GuardRing(devs, ring)` | `GuardRingEnclosed` | Hard | place, route, analysis |
| `Matching(a,b)` | `MatchedParasitic` + group | Joint | place, route, analysis |
| `DistanceConstraint(a,b,d)` | `Proximity { max_distance }` | Bounded | place |
| `NetShield(net)` | `Shielded` | Bounded | route, analysis |
| `SetNetClass(net, class)` | `NetClassAssignment { class }` | Hard | route |
| `SignalFlow(path)` | one ordered-pair per adjacent pair | Soft/Hard | place, route |
| `AspectRatio(w,h)` | (global bounding) | Bounded | place |
| `CellBoundary(w,h)` | (global bounding) | Hard | place |
| `FixPosition(a,x,y)` | `FixedPosition { x, y }` | Hard | place |
| `PlaceOnGrid(a,d)` | `GridSnapped { pitch }` | Hard | place |
| `PlaceOnBoundary(a,side)` | `BoundaryPlaced { side }` | Hard | place |
| `Group(devs, name)` | `Grouped` + group | Joint | place |
| `PortLocation(a,net,pos)` | `PortLocationBound { position }` | Bounded | place, route |
| `NetMatch(a,b,d)` | `MatchedLength { max_delta }` | Bounded | route, analysis |
| `MultiWire(net,n)` | `MultiWire { count }` | Bounded | route |
| `DoNotRoute(net)` | `DoNotRoute` | Hard | route |
| `ChargeFlow(net)` | (flow annotation) | Soft | route |

Each compiled obligation gets a content-addressed ID from its semantic fields.

### Entity validation

During compilation, every device/net name in the Constraint is checked against
the CanonicalFactGraph. A reference to a non-existent device emits a
diagnostic (not a silent skip) and the obligation is still created with an
`Unsupported` class and a `CoverageGap` repair policy.

---

## 4.3 Inference detector architecture — `src/intent/detect/mod.rs`

### Detector trait

```
trait IntentDetector: Send + Sync {
    /// Unique name for provenance tracking.
    fn name(&self) -> &str;

    /// Priority tier (lower = runs first). Detectors in the same tier
    /// run independently; cross-tier ordering lets later detectors
    /// build on earlier ones' results.
    fn tier(&self) -> u8;

    /// Run detection over the fact graph and technology, emitting
    /// obligations into the builder.
    fn detect(
        &self,
        facts: &CanonicalFactGraph,
        tech: &dyn CompiledTechnology,
        existing: &IntentGraph,    // obligations from earlier tiers
        builder: &mut IntentBuilder,
    );
}
```

### Detector registry and orchestration

```
DetectorRegistry {
    detectors: Vec<Box<dyn IntentDetector>>,  // sorted by tier
}

impl DetectorRegistry {
    fn run_all(&self, facts, tech, seed) -> IntentGraph;
}
```

Orchestration:
1. Sort detectors by tier.
2. For each tier, run all detectors in that tier (order within tier is
   deterministic but detectors are independent).
3. After each tier, merge results into the accumulating IntentGraph.
   Content-addressed IDs handle deduplication automatically — if two
   detectors in the same tier infer the same obligation, collision-as-merge
   deduplicates and records both provenance edges.
4. Pass the accumulated graph to the next tier's detectors via `existing`.

### Priority tiers

| Tier | Detectors | Rationale |
|------|-----------|-----------|
| 0 | user-declared (compile.rs) | user intent is ground truth |
| 1 | diff pair, current mirror, cascode | high-value, high-confidence analog patterns |
| 2 | matching, interdigitation, common centroid | depends on mirror/pair detection |
| 3 | signal/current flow, power/ground/domain | net classification |
| 4 | guard ring, isolation, well, substrate | depends on device classification |
| 5 | sensitive/critical nets, shielding | depends on flow + net classification |
| 6 | structural symmetry (WL), ordering, alignment | catch-all for structures not caught by domain detectors |
| 7 | host-template, padframe, fixed pins | external constraints |
| 8 | RF, HV, ESD | specialized domains |
| 9 | DRC/LVS/PEX/EM/antenna obligation inference | coverage-driven, depends on technology |

---

## 4.4 Detector specifications

### Tier 1: Differential pair — `src/intent/detect/diffpair.rs`

**Detection heuristic:**
1. Find all pairs of MOSFETs `(M_a, M_b)` where:
   - Same model (same type, same W/L within tolerance)
   - Share a source net (common tail)
   - Gate nets are distinct
   - Drain nets are distinct
2. Confirm the shared source net connects to a current source
   (a device whose gate is driven by a bias net, or is diode-connected).

**Emitted obligations:**
- `SymmetricPair` (Hard) on the pair — consumer: place, route
- `MatchedParasitic` (Joint) on drain nets — consumer: route, analysis
- `DifferentialRouting` (Bounded) on gate nets — consumer: route
- `LowNoise` (Bounded) on gate nets if the tail current is small
- `ObligationGroup` wrapping all pair obligations — repair: GroupRepair

**Confidence:** 0.95 for standard diff pair topology. Drops to 0.80 if
parameter matching is inexact (W/L within 5% but not identical). Drops to
0.60 if the tail current source identification is ambiguous.

### Tier 1: Current mirror — `src/intent/detect/mirror.rs`

**Detection heuristic:**
1. Find all groups of MOSFETs where:
   - Same model (same type, same W or ratioed W)
   - Share a gate net
   - At least one device is diode-connected (drain = gate)
   - All share a source net (usually VDD or VSS)
2. The diode-connected device is the reference; others are copies.

**Emitted obligations:**
- `SymmetricPair` or `CommonCentroid` (Joint) depending on group size
  (2 devices → symmetric, 3+ → common centroid candidate)
- `MatchedParasitic` (Joint) on drain nets — consumer: route, analysis
- `SameTemplate` and `SameOrientation` (Hard) — consumer: place
- `Proximity` (Bounded) between reference and copies — consumer: place
- `ObligationGroup` — repair: GroupRepair

**Confidence:** 0.95 for diode-connected reference. 0.85 for cascode
mirrors (see below). 0.70 for wide-swing or regulated mirrors where the
topology is more complex.

### Tier 1: Cascode — `src/intent/detect/mirror.rs` (same file)

**Detection heuristic:**
1. Find pairs of MOSFETs stacked in series (one's drain connects to the
   other's source).
2. If the lower device is part of a current mirror, the stack is a cascode
   mirror.
3. If connected to a diff pair output, it's a cascode load.

**Emitted obligations:**
- `OrderedPair` (Hard) — the cascode device must be above the mirror device
- `Proximity` (Bounded) — cascode pairs should be close
- Additional `SymmetricPair` if the cascode pair serves a differential output

### Tier 2: Matching groups — `src/intent/detect/matching.rs`

**Detection heuristic:**
1. Devices already identified as mirror members or diff pair halves are
   candidates for matching.
2. Groups of 4+ matched devices are candidates for interdigitation or
   common centroid placement.
3. Ratioed matches (W1:W2 = 2:1) may need unit-cell decomposition.

**Emitted obligations:**
- `Interdigitated` or `CommonCentroid` (Joint) — consumer: place
- `MatchedLength` (Bounded) on connected nets — consumer: route
- `ObligationGroup` with all members

**Dependency on tier 1:** uses the diff-pair and mirror detections to
identify which devices are matched. Does not re-scan topology.

### Tier 3: Signal and current flow — `src/intent/detect/flow.rs`

**Detection heuristic:**
1. Starting from input ports, trace signal flow through gate→drain paths.
2. Starting from supply rails, trace current flow through source→drain paths.
3. Identify parallel current paths (devices whose sources and drains
   connect to the same pair of nets).
4. Classify nets: signal-path, bias, supply, feedback.

**Algorithm:** BFS/DFS over the adjacency index, following directed
device edges (gate is input, drain is output for signal; source→drain for
current). Maintain a visited set to handle feedback loops.

**Emitted obligations:**
- `SignalFlow` ordering hints (Soft) — consumer: place
- Net classification annotations used by later detectors (not themselves
  obligations, but metadata edges in the IntentGraph)
- `PowerNet`, `GroundNet`, `SubstrateNet`, `ClockNet` (Hard) — consumer: route

### Tier 4: Guard ring / isolation / well — `src/intent/detect/guard.rs`

**Detection heuristic:**
1. Identify NMOS and PMOS clusters (groups of same-type devices that
   share wells).
2. If a technology requires guard rings around well boundaries (SKY130 does
   for latch-up compliance), emit `GuardRingEnclosed` obligations.
3. Identify substrate/well taps and their required proximity to active devices.

**Emitted obligations:**
- `GuardRingEnclosed` (Hard) — consumer: place, route
- `Proximity` (Hard) for tap-to-active distance rules
- `DomainSeparation` if multiple voltage domains are detected

### Tier 5: Sensitive nets — `src/intent/detect/sensitive.rs`

**Detection heuristic:**
1. Nets identified as high-impedance by flow analysis (gate nets of
   input transistors, drain nets of cascode outputs).
2. Nets identified as low-noise by adjacency to sensitive analog blocks.
3. Nets carrying matched signals (from the matching detector).

**Emitted obligations:**
- `Shielded` (Bounded) — consumer: route
- `LowNoise` (Bounded) — consumer: route, analysis
- `HighImpedance` (Bounded) — consumer: route

### Tier 6: Structural symmetry — `src/intent/detect/symmetry.rs`

The catch-all for symmetric structures not caught by domain-specific detectors.

**Algorithm:** Color refinement (1-WL Weisfeiler-Leman):
1. Initialize each device's color from `(model, DeviceKind, terminal_count)`.
2. Iterate: new color = hash(old color, sorted multiset of neighbor colors).
3. Converge when no color class splits.
4. Devices with the same color are structurally interchangeable candidates.

**Emitted obligations:**
- `SymmetricPair` (Hard, confidence depends on WL expressiveness) for pairs
  that WL identifies as interchangeable.
- Confidence: 0.80 for WL-only detection (the honest expressiveness bound).

**Important:** this detector runs *after* domain-specific detectors. It
should not re-detect structures already identified by earlier tiers. Check
the `existing` IntentGraph and skip entities already covered.

### Tier 7: Host-template — `src/intent/detect/host.rs`

**Detection heuristic:**
1. If the circuit has declared host-template bindings (fixed pins,
   padframe, rail map), emit hard constraints for those bindings.
2. Fixed pins → `FixedPosition` (Hard)
3. Rail map → `PowerNet` with routing obligations
4. Reserved layers → constraints on routing layer usage

This detector consumes external metadata, not SPICE topology. The
host-template data comes from the problem input bundle.

### Tier 8: RF and HV — `src/intent/detect/rf.rs`, `hv.rs`

**RF detection:**
- Inductors (SPICE `L` elements or specific subcircuit models)
- Transmission-line-like structures (long parallel routes)
- Keepout regions around inductors/transformers

**HV detection:**
- Devices with `g5v0d10v5` (or similar HV model names)
- Voltage-dependent spacing requirements
- ESD protection structures
- Thick-metal routing obligations

Both are technology-dependent — the detector queries `CompiledTechnology`
for HV/RF device models and associated rules.

### Tier 9: DRC/LVS/PEX/EM obligation inference — `src/intent/detect/drc_lvs.rs`

**Not a topology detector** — this tier queries the technology's rule coverage
inventory and emits obligations for every required rule class:

1. For each rule in `tech.rule_ids()`:
   - Lookup coverage via the MCMM key
   - If `Coded`: emit an analysis obligation with `InternalProof` evidence domain
   - If `ExternalOnly`: emit `ExternalDeckRequired`
   - If `Missing`/`Unsupported`: emit `CoverageGap` diagnostic
   - If `ManualReview`: emit `ManualReviewRequired`

2. For each device: emit `LvsEquivalence` obligation (analysis consumer).

3. For each bounded metric (PEX, EM, IR, noise): emit `PexBound` / `EmBound`
   obligations with the technology's default bounds.

---

## 4.5 Content-addressed deduplication

When multiple detectors infer the same obligation, the content-addressed ID
causes a collision. The `IntentBuilder` handles this:

1. Compute the candidate obligation's ID.
2. If the ID already exists in the graph:
   - Do NOT create a duplicate.
   - Append a new `ProvenanceEdge` recording this detector as an additional source.
   - If the new detection has higher confidence, update the obligation's confidence
     to the max.
3. If the ID is new, insert the obligation.

This is the "collision-as-merge" property. It is correct because two obligations
with the same ID have identical semantic content by construction.

---

## 4.6 Testing

- **Per-detector unit tests** with minimal circuits:
  - Diff pair: 2 NMOS + 1 tail → detects pair, emits symmetry + matching.
  - Mirror: 2 PMOS, shared gate, one diode → detects mirror, emits matching.
  - Cascode: 4 devices stacked → detects cascode, emits ordering.
- **Deduplication:** run diff-pair detector and symmetry detector on the same
  circuit. The pair should be detected by both; verify only one obligation
  exists with two provenance edges.
- **Tier ordering:** verify that tier 2 detectors can see tier 1 results.
- **Reference circuits:** run all detectors on each reference circuit, compare
  emitted obligations against golden set.
- **Unknown topology:** a circuit with no recognizable analog patterns should
  produce only user-declared obligations + coverage obligations. No false
  positives from WL on random topology.
- **ID stability:** run detection twice on the same circuit, verify all IDs
  are identical.

---

## 4.7 Open questions

1. **Detector configurability.** Should users be able to disable specific
   detectors? Yes — the registry should support a detector mask. A user who
   knows their circuit is purely digital should be able to skip all analog
   detectors.

2. **Confidence thresholds.** At what confidence does an inferred obligation
   get compiled into the graph? A very low confidence (0.30) is noise; a
   0.90 is useful. Suggest: compile everything, but obligations below a
   configurable threshold (default 0.50) are classified as `Soft` regardless
   of what the detector says. The confidence is preserved for the certificate.

3. **Incremental readiness.** Detectors are pure functions of
   `(facts, technology, existing)`. For incremental support, the orchestrator
   needs to know which detectors to re-run when the fact graph changes. The
   dependency is by entity: if a device changes, re-run detectors that
   touched that device. Content-addressed IDs make the delta computation
   cheap (diff the ID sets before and after).

4. **Performance target.** For a 50-device bandgap, all detectors should
   complete in < 10ms. For a 5000-device ADC, < 500ms. The WL iteration
   in tier 6 is the most expensive (O(n * k) where k is the number of
   iterations to convergence, typically 5–15). Profile early.
