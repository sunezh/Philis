# Phase 7 — Evidence, Solver Surface & Certificate

**Goal:** The evidence loop, the solver lowering surface, the quality
vectors, the structured diagnostics, and the final `ConstraintCertificate`.

**Depends on:** phase 6 (projections)  
**Unlocks:** full end-to-end constraint compilation, signoff-readiness claims

---

## 7.1 Evidence loop — `src/evidence/`

### The append-only ledger — `src/evidence/ledger.rs`

The ledger is the central data structure of the evidence loop. It records
every piece of signoff evidence ever ingested for the current compilation,
keyed by:

```
EvidenceKey {
    constraint_id: CanonicalId,
    domain: EvidenceDomain,        // DRC, LVS, PEX, EM, ERC, Antenna, Density, Color, Manual
    subject_key: String,           // the specific rule/metric/net/device
    corner: String,                // PVT corner
    mode: String,                  // operating mode
    deck_id: String,               // analysis deck
}
```

Entries:

```
EvidenceEntry {
    key: EvidenceKey,
    result: EvidenceResult,        // Pass, Fail { detail }, Degraded { reason }
    timestamp: u64,                // monotonic counter, not wall-clock
    source: EvidenceSource,        // Internal, External { hash }, Manual { signer }
}
```

The ledger is **append-only**: a later entry for the same key does not
overwrite the earlier one. Both are retained. This is the foundation of
Invariant 4 (monotonic evidence ingestion).

### Monotonicity enforcement

When a new entry arrives for an existing key:

1. Look up the latest entry for that key.
2. If the new entry is `Pass` and the latest was `Fail`:
   - This is a **repair transition**. Record it as such (both endpoints
     retained).
   - Check that the global hard vector has not worsened (the repair must
     be net-positive at the hard level).
   - If the hard vector worsened, **reject** the repair entry and emit a
     diagnostic: "repair reverted hard progress."
3. If the new entry is `Fail` and the latest was `Pass`:
   - This is a **regression**. Record it and update the hard vector.
4. The hard-then-bounded vector must be **non-increasing** across accepted
   repair steps (Invariant 4).

### Repair obligation generation — `src/evidence/repair.rs`

When evidence arrives as a failure:

1. Look up the `constraint_id` in the IntentGraph.
2. Determine the obligation's `RepairPolicy`.
3. Generate a repair obligation:
   - `GroupRepair`: re-run the solver for the entire obligation group.
   - `SingleDevice`: perturb the single device's position.
   - `NetReroute`: rip-up and reroute the affected net (respecting groups).
   - `NoAutoRepair`: emit a diagnostic only.
4. Check the repair obligation against the repair iteration budget.
   If budget exhausted, emit `Unknown` + best-seen state.

### Evidence ingestion — `src/evidence/ingest.rs`

Converts external evidence markers into ledger entries:

| Evidence marker | EvidenceDomain | Obligation kind |
|---|---|---|
| DRC spacing violation | `DRC` | routing repair, group-preserving if matched |
| LVS terminal mismatch | `LVS` | primitive/terminal-map obligation |
| PEX coupling excess | `PEX` | bounded shield/spacing obligation |
| EM current density | `EM` | bounded width/via obligation |
| Antenna ratio violation | `Antenna` | bounded or degraded-confidence |
| Missing PDK rule | domain-specific | CoverageGap singleton core |
| Manual review sign-off | `Manual` | clears ManualReview evidence requirement |

---

## 7.2 Quality vectors — `src/quality/mod.rs`

The lexicographic hard / bounded / soft vectors, lifted from the spec.

### Hard vector

Already defined in `core.rs` as `HardVector`. The constraints crate computes
it from the IntentGraph + evidence ledger:

```
fn compute_hard_vector(
    graph: &IntentGraph,
    report: &ReconciliationReport,
    ledger: &EvidenceLedger,
    tech: &dyn CompiledTechnology,
) -> HardVector {
    HardVector {
        invalid_pdk: tech.pdk_state() == PdkState::ContradictoryOrMissing,
        missing_access: count_unfulfilled(graph, EvidenceDomain::LVS, "terminal_access", ledger),
        drc_violations: count_unfulfilled(graph, EvidenceDomain::DRC, "*", ledger),
        lvs_mismatches: count_unfulfilled(graph, EvidenceDomain::LVS, "*", ledger),
        unrouted_required_nets: count_unrouted(graph, ledger),
        unresolved_em_antenna_hard_errors: count_unfulfilled_hard(graph, &[EvidenceDomain::EM, EvidenceDomain::Antenna], ledger),
    }
}
```

### Bounded vector

```
BoundedVector {
    pex_resistance_excess: f64,
    pex_ground_cap_excess: f64,
    pex_coupling_excess: f64,
    matched_group_delta_excess: f64,
    ir_drop_excess: f64,
    em_margin_deficit: f64,
    crosstalk_noise_excess: f64,
    antenna_margin_deficit: f64,
    lde_thermal_excess: f64,
}
```

Each component is the *sum of violations* for that metric across all
obligations. Zero means all bounded obligations are within budget.

### Soft vector

```
SoftVector {
    residual_pex_damage: f64,
    residual_matching_slack: f64,
    area: f64,
    hpwl_or_route_length: f64,
    via_count: usize,
    density_stress: f64,
    fill_dfm_cost: f64,
    runtime_seconds: f64,
    repair_iterations: usize,
}
```

### Lexicographic comparator

```
fn lex_compare(a: &QualityVector, b: &QualityVector) -> Ordering {
    // Compare hard vectors first
    match a.hard.as_priority().cmp(&b.hard.as_priority()) {
        Ordering::Equal => {},
        ord => return ord,
    }
    // If both hard-feasible, compare bounded
    match a.bounded.as_priority().partial_cmp(&b.bounded.as_priority()) {
        Some(Ordering::Equal) | None => {},
        Some(ord) => return ord,
    }
    // If both bounded-feasible, compare soft
    a.soft.as_priority().partial_cmp(&b.soft.as_priority())
        .unwrap_or(Ordering::Equal)
}
```

---

## 7.3 Solver lowering surface — `src/solver/mod.rs`

> **Shared crate:** The solver lowering surface uses `crates/solver`
> (`philis_solver`) for SAT/ILP solver traits and MUS/MCS extraction.
> The `SolverBackend` trait defined below delegates to
> `philis_solver::SatSolver` and `philis_solver::IlpSolver` for the
> underlying satisfiability and optimization calls. The convenience
> builders (at-most-one, at-least-one) from `philis_solver` are used to
> encode constraint structure.

The solver module defines the interface between the constraint compiler and
the backend solvers. It uses `philis_solver` for the underlying solver
abstractions; the placement and routing crates provide problem-specific
formulations through this interface.

### Lowered objective

```
LexicographicObjective {
    hard: Vec<HardGate>,
    bounded: Vec<BoundedMetric>,
    soft: Vec<SoftMetric>,
}

struct HardGate {
    id: CanonicalId,
    predicate: PredicateKind,
    entities: Vec<EntityRef>,
}

struct BoundedMetric {
    id: CanonicalId,
    metric: String,
    bound: i64,
    unit: String,
}

struct SoftMetric {
    id: CanonicalId,
    metric: String,
    weight: f64,         // relative weight within the soft tier
}
```

### Backend artifact types

```
enum BackendArtifact {
    ProofLog(String),
    UnsatCore(Vec<CanonicalId>),
    CorrectionSet(Vec<CanonicalId>),
    FeasibleModel(String),
    ModelInvalid(String),
    Unknown { partial_core: Vec<CanonicalId>, reason: String },
    HeuristicTrace(String),
}
```

### Lowering trait

```
trait SolverBackend {
    fn solve(
        &self,
        objective: &LexicographicObjective,
        budget: &SolverBudget,
    ) -> SolverResult;
}

struct SolverBudget {
    max_time: Duration,
    max_iterations: usize,
    max_memory_bytes: usize,
}

struct SolverResult {
    status: SolverStatus,       // Optimal, Feasible, Infeasible, Unknown, Timeout
    artifact: BackendArtifact,
    hard_vector: HardVector,
    bounded_vector: Option<BoundedVector>,
    soft_vector: Option<SoftVector>,
    runtime: Duration,
}
```

The constraint compiler produces the `LexicographicObjective`; the consumer
(placer, router) provides a `SolverBackend` implementation. This is the
only coupling point between the constraint compiler and the engine.

---

## 7.4 Structured diagnostics — `src/diagnostics/mod.rs`

Every diagnostic carries enough context for a human-readable rendering:

```
struct ConstraintDiagnostic {
    severity: DiagnosticSeverity,   // Error, Warning, Info
    scope: DiagnosticScope,         // from core.rs
    code: DiagnosticCode,           // stable, machine-parseable
    message: String,                // human-readable
    obligation_ids: Vec<CanonicalId>,
    entities: Vec<EntityRef>,
    suggestion: Option<String>,     // actionable repair suggestion
}

enum DiagnosticCode {
    CoverageGapHard,
    CoverageGapBounded,
    ContradictionCoreSingleton,
    ContradictionCorePairwise,
    ContradictionCoreGroupwise,
    WaiverRejected,
    InferenceConfidenceLow,
    ProjectionSkipHard,
    ProjectionDowngrade,
    RepairBudgetExhausted,
    EvidenceRegression,
    MonotonicityViolation,
    UnknownDeviceModel,
    // ...extensible
}
```

The diagnostics module also provides a **circuit-domain renderer** that
translates constraint-level diagnostics into analog-designer language:

```
fn render_for_designer(diag: &ConstraintDiagnostic, facts: &CanonicalFactGraph) -> String {
    // "Device M1 (NMOS, W=2u L=180n) at hierarchy path ota/
    //  cannot be fixed at (100, 200) and simultaneously symmetric
    //  with M2 about the Y axis — these constraints contradict."
}
```

---

## 7.5 The `ConstraintCertificate` — `src/certificate/mod.rs`

The top-level output of the constraint compiler.

```
struct ConstraintCertificate {
    // --- input hashes for replay ---
    schematic_hash: u64,
    hierarchy_hash: u64,
    pdk_hash: u64,
    host_template_hash: u64,
    user_constraints_hash: u64,
    fixed_geometry_hash: u64,

    // --- tool identity ---
    compiler_version: String,
    hash_family: String,            // "blake3-truncated-64"
    seed: u64,

    // --- technology ---
    technology_id: String,
    pdk_state: PdkState,
    coverage_inventory_hash: u64,
    coverage_report: RuleCoverageReport,

    // --- intent graph ---
    intent_graph_root_hash: u64,    // Merkle root of the obligation DAG
    obligation_count: usize,
    group_count: usize,
    by_class: HashMap<ObligationClass, usize>,
    detector_provenance: HashMap<String, usize>,

    // --- projections ---
    placement_projection: ProjectionSummary,
    routing_projection: ProjectionSummary,
    analysis_projection: ProjectionSummary,

    // --- reconciliation ---
    cores: Vec<ContradictionCore>,
    waivers: Vec<(CanonicalId, Waiver)>,
    resolutions: Vec<ResolutionRecord>,

    // --- quality ---
    hard_vector: HardVector,
    bounded_vector: BoundedVector,
    soft_vector: SoftVector,

    // --- evidence ---
    evidence_entries: usize,
    evidence_domains_covered: Vec<EvidenceDomain>,
    repair_iterations: usize,

    // --- solver ---
    solver_path: String,
    solver_artifacts: Vec<BackendArtifact>,

    // --- status ---
    degraded_confidence: bool,
    degraded_causes: Vec<DegradedConfidenceCause>,
    final_claim: FinalClaim,

    // --- diagnostics ---
    diagnostics: Vec<ConstraintDiagnostic>,
}

enum FinalClaim {
    SignoffQuality,              // all gates pass on coded evidence
    SignoffReadyInternal,        // gates pass but external decks required
    SignoffReadyDegraded,        // gates pass but confidence is degraded
    Incomplete,                  // hard gates fail
    Infeasible(Vec<CoreId>),     // proven infeasible with cores
    Unknown,                     // timeout, no proof either way
}

struct ProjectionSummary {
    expressed_count: usize,
    skipped_count: usize,
    downgraded_count: usize,
    hard_skips: usize,          // hard obligations that couldn't be expressed
}
```

### Merkle root computation

The certificate's `intent_graph_root_hash` is computed as:

1. Each obligation's `CanonicalId` is already a content hash.
2. Each group's ID is a hash of its sorted member IDs.
3. The root hash is a hash of the sorted list of all obligation IDs +
   all group IDs + the reconciliation report hash.

A replay that recomputes the root and gets a different value can localize
the divergence by walking the DAG (which obligation changed?).

### `is_signoff_quality`

```
fn is_signoff_quality(&self) -> bool {
    self.final_claim == FinalClaim::SignoffQuality
        && !self.degraded_confidence
        && self.hard_vector.is_feasible()
        && self.cores.is_empty()
}
```

### `stub()`

The honest placeholder, mirroring the existing `PlacementCertificate::stub()`
and `RouteCertificate::stub()` patterns:

```
fn stub() -> ConstraintCertificate {
    ConstraintCertificate {
        pdk_state: PdkState::SparseResearch,
        degraded_confidence: true,
        final_claim: FinalClaim::SignoffReadyDegraded,
        degraded_causes: vec![DegradedConfidenceCause {
            gate: AcceptanceGate::TechnologyCoverage,
            reason: "constraint compiler stub — no compilation performed".into(),
            coverage: CoverageValue::Missing,
        }],
        ..Default::default()
    }
}
```

---

## 7.6 The full compilation entry point

The public API of the constraint compiler:

```
pub fn compile(input: CompilerInput) -> CompilerOutput {
    // Phase 2: technology compilation
    let tech = compile_technology(&input.pdk_bundle);

    // Phase 3: fact extraction
    let facts = extract_facts(&input.netlist, &tech);

    // Phase 4: intent compilation + inference
    let intent = compile_intent(&input.constraints, &facts, &tech, input.seed);

    // Phase 5: reconciliation
    let (reconciled, report) = reconcile(&intent, &input.waivers);

    // Phase 6: projections
    let place_proj = project_placement(&reconciled, &report, &tech);
    let route_proj = project_routing(&reconciled, &report, &tech);
    let analysis_proj = project_analysis(&reconciled, &report, &tech);

    // Phase 7: evidence (if prior evidence provided)
    let ledger = if let Some(prev) = input.previous_evidence {
        ingest_evidence(&reconciled, prev)
    } else {
        EvidenceLedger::empty()
    };

    // Quality vectors
    let quality = compute_quality(&reconciled, &report, &ledger, &tech);

    // Certificate
    let cert = assemble_certificate(
        &input, &tech, &facts, &reconciled, &report,
        &place_proj, &route_proj, &analysis_proj,
        &ledger, &quality,
    );

    CompilerOutput {
        technology: tech,
        facts,
        intent: reconciled,
        placement_projection: place_proj,
        routing_projection: route_proj,
        analysis_projection: analysis_proj,
        reconciliation: report,
        evidence: ledger,
        quality,
        certificate: cert,
    }
}
```

### `CompilerInput`

```
struct CompilerInput {
    pdk_bundle: PdkBundle,          // or a pre-compiled CompiledTechnology
    netlist: SpiceNetlist,
    constraints: Vec<Constraint>,
    waivers: Vec<Waiver>,
    host_template: Option<HostTemplate>,
    fixed_geometry: Option<FixedGeometry>,
    previous_evidence: Option<Vec<EvidenceEntry>>,
    budget: CompilerBudget,
    seed: u64,
}
```

---

## 7.7 Testing

- **Full pipeline test:** compile a reference diff-pair circuit end-to-end.
  Verify the certificate is `SignoffReadyDegraded` (no real engine) with
  correct obligation counts.
- **Evidence monotonicity:** ingest a DRC fail, then a DRC pass for the same
  key. Verify both entries in the ledger, repair transition recorded.
- **Monotonicity rejection:** inject a repair that worsens the hard vector.
  Verify rejection.
- **Quality vector ordering:** construct two candidates with different hard
  vectors. Verify the lexicographic comparator ranks the DRC-clean one higher.
- **Certificate determinism:** compile the same circuit twice with the same
  seed. Verify identical certificate hashes.
- **Certificate Merkle root:** modify one obligation's entity list, recompile.
  Verify the root hash changes and the diff localizes to that obligation.
- **Stub certificate:** verify it is accepted but not signoff-quality.
- **FinalClaim classification:** construct scenarios for each variant
  (SignoffQuality, Incomplete, Infeasible, Unknown) and verify correct
  classification.
- **Solver lowering:** verify that the `LexicographicObjective` produced from
  a reference circuit has the correct hard/bounded/soft partition.

---

## 7.8 Open questions

1. **Serialization format for the certificate.** JSON is human-readable but
   verbose. Bincode or MessagePack is compact but opaque. Suggest: JSON for
   the default (debuggability matters more than size for early development),
   with a binary format flag for production.

2. **Certificate signing.** The spec mentions the certificate is "replayable"
   but doesn't discuss tamper-evidence. Should the certificate include a
   MAC or signature? Not for v1 — the Merkle root provides integrity
   (any modification changes the root), and signing requires a key management
   story that's premature.

3. **Evidence loop iteration limit.** The budget vector includes a
   `repair_iterations` limit. What's the default? Suggest: 10 iterations
   for placement, 20 for routing. The evidence loop is not a continuous
   optimizer — it's a bounded repair loop. If 10 iterations don't converge,
   the problem is likely infeasible and the cores should say so.

4. **How the api crate consumes the certificate.** The api's existing
   `PlacementCertificate` and `RouteCertificate` are stage-scoped. The
   `ConstraintCertificate` is system-scoped (it covers all three stages).
   The api crate should expose the `ConstraintCertificate` alongside the
   stage certificates, not replace them — each certificate proves different
   facts.
