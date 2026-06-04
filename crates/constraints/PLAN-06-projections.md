# Phase 6 — Projections

**Goal:** Project the reconciled IntentGraph into typed consumer obligations
for placement and routing (in parallel), with the soundness invariant
enforced per-projection.

**Depends on:** phase 5 (reconciled IntentGraph)  
**Unlocks:** phase 7 (evidence loop consumes projection reports), integration
with api crate's placement and routing paths

---

## 6.1 What projection does

A projection is a deterministic query that selects obligations relevant to
one consumer and reshapes them into that consumer's schema. It is a view,
not a translation — it adds no obligation absent from the graph and drops
none without recording a skip.

Three projections are built in parallel:
1. `PlacementConstraintProjection` — consumed by the placer
2. `RoutingConstraintProjection` — consumed by the router
3. `AnalysisObligationProjection` — consumed by verification / evidence loop

---

## 6.2 The projection engine — `src/project/mod.rs`

### Core abstraction

```
trait Projection {
    type Output;

    fn project(
        &self,
        graph: &IntentGraph,
        report: &ReconciliationReport,
        tech: &dyn CompiledTechnology,
    ) -> ProjectionResult<Self::Output>;
}

struct ProjectionResult<T> {
    expressed: T,
    skipped: Vec<ProjectedConstraintSkip>,
    downgraded: Vec<ProjectedConstraintDowngrade>,
    statistics: ProjectionStatistics,
}
```

### Projected skip and downgrade records

```
struct ProjectedConstraintSkip {
    constraint_id: CanonicalId,
    reason: SkipReason,
}

enum SkipReason {
    Inexpressible,       // consumer's model cannot represent this
    OutOfScope,          // not this consumer's responsibility
    CoverageGap,         // technology lacks the rule
    CapabilityMismatch,  // consumer version doesn't support this predicate
}

struct ProjectedConstraintDowngrade {
    constraint_id: CanonicalId,
    original_class: ObligationClass,
    downgraded_to: ObligationClass,
    reason: String,
    forces_degraded_confidence: bool,
}
```

### Soundness checker

Every projection result is verified before return:

```
fn check_soundness<T>(
    graph: &IntentGraph,
    consumer: ConsumerMask,
    result: &ProjectionResult<T>,
    expressed_ids: &HashSet<CanonicalId>,
) -> Result<(), SoundnessViolation> {
    let expected: HashSet<CanonicalId> = graph.obligations
        .values()
        .filter(|o| o.consumers.contains(consumer))
        .map(|o| o.id)
        .collect();

    let covered: HashSet<CanonicalId> = expressed_ids
        .union(&result.skipped.iter().map(|s| s.constraint_id).collect())
        .copied()
        .collect();

    // Invariant 1a: partition covers all consumer obligations
    let missing = expected.difference(&covered).collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(SoundnessViolation::MissingObligations(missing));
    }

    // Invariant 1b: partition is disjoint
    let overlap: Vec<_> = expressed_ids
        .intersection(&result.skipped.iter().map(|s| s.constraint_id).collect())
        .collect();
    if !overlap.is_empty() {
        return Err(SoundnessViolation::OverlapExpressedSkipped(overlap));
    }

    // Invariant 1c: every projected ID resolves back to the graph
    for id in expressed_ids.iter().chain(result.skipped.iter().map(|s| &s.constraint_id)) {
        if !graph.obligations.contains_key(id) {
            return Err(SoundnessViolation::InventedObligation(*id));
        }
    }

    Ok(())
}
```

This check runs on every projection, in every build (not just debug). It is
the translation-validation witness from the spec. A soundness violation is
a compiler bug, not a user error — it panics in debug and returns an error
in release.

---

## 6.3 Placement projection — `src/project/placement.rs`

### Output type

```
PlacementConstraintProjection {
    primitive_obligations: Vec<PrimitiveObligation>,
    geometry_obligations: Vec<GeometryObligation>,
    matching_obligations: Vec<MatchingObligation>,
    host_obligations: Vec<HostObligation>,
    terminal_access_requirements: Vec<TerminalAccessReq>,
    routability_requirements: Vec<RoutabilityReq>,
    pex_envelopes: Vec<PexEnvelope>,
    skipped: Vec<ProjectedConstraintSkip>,
}
```

### Predicate-to-placement-obligation mapping

| PredicateKind | Placement obligation type | Notes |
|---|---|---|
| `SymmetricPair` | `GeometryObligation::Symmetry` | axis + device pair |
| `SelfSymmetric` | `GeometryObligation::SelfSymmetry` | axis + single device |
| `CommonCentroid` | `GeometryObligation::CommonCentroid` | group of devices |
| `Interdigitated` | `GeometryObligation::Interdigitate` | group + unit pattern |
| `OrderedPair` | `GeometryObligation::Order` | axis + device pair |
| `Aligned` | `GeometryObligation::Align` | axis + device list |
| `SameTemplate` | `PrimitiveObligation::SameTemplate` | device pair |
| `SameOrientation` | `PrimitiveObligation::SameOrientation` | device pair |
| `Proximity` | `GeometryObligation::MaxDistance` | device pair + bound |
| `FixedPosition` | `GeometryObligation::Fix` | device + coordinates |
| `GridSnapped` | `GeometryObligation::Grid` | device + pitch |
| `BoundaryPlaced` | `GeometryObligation::Boundary` | device + side |
| `Grouped` | `GeometryObligation::Group` | device list |
| `GuardRingEnclosed` | `GeometryObligation::GuardRing` | device list + ring |
| `MatchedParasitic` | `MatchingObligation` | group + metric + bound |
| `LvsEquivalence` | `PrimitiveObligation::LvsEquiv` | per device |
| `DrcRule` (placement-owned) | linked to AcceptanceGate::LocalDrc | |
| routing-only predicates | **skipped** (SkipReason::OutOfScope) | |
| analysis-only predicates | **skipped** (SkipReason::OutOfScope) | |

### Terminal access requirements

For each device terminal, placement must provide an access candidate. The
projection gathers terminal access requirements from the fact graph:

```
TerminalAccessReq {
    device: String,
    terminal: String,
    net: String,
    required_layers: Vec<String>,  // from technology
    min_access_candidates: usize,  // default 1
}
```

### Routability requirements

Placement must leave enough channel capacity for routing. The projection
computes a rough demand estimate from the obligation set:

```
RoutabilityReq {
    region: String,
    estimated_net_demand: usize,
    reserved_channels: usize,  // for matched/shielded nets
}
```

---

## 6.4 Routing projection — `src/project/routing.rs`

### Output type

```
RoutingConstraintProjection {
    net_classes: Vec<NetClassAssignment>,
    terminal_access: Vec<TerminalAccessObligation>,
    connectivity: Vec<ConnectivityObligation>,
    shields_and_guards: Vec<ShieldObligation>,
    matching: Vec<RouteMatchingObligation>,
    power_and_domain: Vec<PowerDomainObligation>,
    pex_ir_em_noise_bounds: Vec<BoundedMetricObligation>,
    repair_groups: Vec<RouteRepairGroup>,
    skipped: Vec<ProjectedConstraintSkip>,
}
```

### Predicate-to-routing-obligation mapping

| PredicateKind | Routing obligation type | Notes |
|---|---|---|
| `DifferentialRouting` | `RouteMatchingObligation::DiffPair` | net pair + skew bound |
| `MatchedLength` | `RouteMatchingObligation::LengthMatch` | net pair + delta |
| `MatchedParasitic` | `RouteMatchingObligation::ParasiticMatch` | group + metric |
| `Shielded` | `ShieldObligation` | net + shield net |
| `LowNoise` | `BoundedMetricObligation` | net + noise bound |
| `HighImpedance` | `ShieldObligation` (spacing) | net + min spacing |
| `DoNotRoute` | `ConnectivityObligation::Exclude` | net excluded from routing |
| `MultiWire` | `ConnectivityObligation::MultiWire` | net + wire count |
| `NetClassAssignment` | `NetClassAssignment` | net + class |
| `PowerNet` / `GroundNet` | `PowerDomainObligation` | wide trunk, EM/IR |
| `DomainSeparation` | `PowerDomainObligation::Separation` | domain pair + spacing |
| `PexBound` | `BoundedMetricObligation` | net + metric + bound + corner |
| `EmBound` | `BoundedMetricObligation` | net + current density bound |
| `AntennaBound` | `BoundedMetricObligation` | net + ratio bound |
| `PortLocationBound` | `TerminalAccessObligation` | terminal + position hint |
| placement-only predicates | **skipped** (SkipReason::OutOfScope) | |

### Repair groups

Joint obligations project to routing as repair groups — sets of nets that
must be rerouted together if any member violates a bounded metric:

```
RouteRepairGroup {
    group_id: CanonicalId,
    member_nets: Vec<String>,
    kind: GroupKind,
    repair_policy: RepairPolicy,
}
```

The group-preserving invariant: a DRC violation on one net of a differential
pair triggers reroute of both, not independent rip-up.

---

## 6.5 Analysis projection — `src/project/analysis.rs`

### Output type

```
AnalysisObligationProjection {
    drc_rules: Vec<DrcRuleObligation>,
    lvs_expectations: Vec<LvsExpectation>,
    pex_corners: Vec<PexCornerObligation>,
    em_ir_noise: Vec<BoundedAnalysisObligation>,
    external_decks: Vec<ExternalDeckRequirement>,
    manual_review: Vec<ManualReviewObligation>,
    unsupported: Vec<ProjectedConstraintSkip>,
}
```

This projection is consumed by the evidence loop. Each obligation maps to
an evidence domain (DRC, LVS, PEX, EM, ERC, Antenna, etc.) and a required
evidence form (internal proof, external report, manual sign-off).

---

## 6.6 Inexpressibility contract enforcement

When a projection encounters an obligation it cannot represent:

1. Check the obligation's class.
2. If `Hard` or `Joint`:
   - Emit a skip with `SkipReason::Inexpressible`.
   - The skip itself is a hard-gate input — it flows into the hard count
     and fails the lexicographic gate.
   - This ensures a hard obligation is never silently approximated as soft.
3. If `Bounded`:
   - May be downgraded to a heuristic with `DegradedConfidence` evidence.
   - The downgrade record forces the final certificate to `signoff_ready`,
     not `signoff_quality`.
4. If `Soft`:
   - Skip with reason. No gate impact.

---

## 6.7 Integration with api crate

The projections feed into the existing api types:

- `PlacementConstraintProjection` → consumed by `run_placement` (replacing
  the current stub that ignores constraints)
- `RoutingConstraintProjection` → consumed by `run_routing` (replacing the
  current stub)
- Skips and downgrades flow into the certificate types already defined in
  `core.rs` (`PlacementCertificate`, `RouteCertificate`)

The compat module from phase 1 maps canonical types to api types at the
boundary. The projection output types are defined in the constraints crate;
the api crate depends on the constraints crate and consumes them.

---

## 6.8 Testing

- **Soundness check on every test:** the `check_soundness` function runs on
  every projection in every test. A test that produces an unsound projection
  is a failure.
- **Partition completeness:** for a diff-pair circuit, verify that every
  obligation with `ConsumerMask::PLACEMENT` appears in either `expressed`
  or `skipped` of the placement projection. Same for routing.
- **No invented obligations:** verify that every ID in the projection output
  exists in the IntentGraph.
- **Skip classification:** an `ExternalDeckRequired` obligation with
  `ConsumerMask::ANALYSIS` should be in the analysis projection, not
  skipped in placement (it's OutOfScope for placement).
- **Hard-skip gate impact:** a Hard obligation that is Inexpressible in
  placement should fail Gate 0 or IntentSatisfaction.
- **Downgrade tracking:** a Bounded obligation downgraded to heuristic
  should produce a `DegradedConfidence` cause in the placement certificate.
- **Reference circuits:** project each reference circuit's IntentGraph and
  verify placement + routing obligation counts match expected values.

---

## 6.9 Open questions

1. **Should projections be lazy or eager?** Eager (compute the full
   projection once) is simpler and correct for batch mode. For future
   incremental support, the projection should be re-computable from a
   delta (changed obligation IDs). Structuring the projection as a pure
   function of the IntentGraph makes this straightforward.

2. **Cross-projection obligations.** Some obligations have consumers in
   both placement and routing (e.g., `MatchedParasitic` with
   `ConsumerMask::PLACEMENT | ConsumerMask::ROUTING`). These appear in
   both projections, which is correct — each consumer gets its own view.
   The canonical ID is the shared key.

3. **Projection versioning.** If the projection logic changes between
   compiler versions, the certificate must record the projection version.
   This is how replay detects a tool-version mismatch rather than silently
   comparing projections from incompatible schemes.
