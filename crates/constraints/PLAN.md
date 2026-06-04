# Constraint Compiler — Implementation Plan

**Status:** plan  
**Target crate:** `crates/constraints` (crate name `philis_constraints`)  
**First concrete PDK:** SKY130  
**Execution model:** batch, structured for incremental later  
**Consumer targets:** placement and routing projections in parallel  
**Inference scope:** full (all detector families from the IntentGraph spec)

---

## Phase map

| # | Phase | Files | Depends on | Deliverable |
|---|-------|-------|------------|-------------|
| 1 | [Scaffold & core types](PLAN-01-scaffold.md) | `lib.rs`, `id.rs`, `types.rs`, `compat.rs` | nothing | crate compiles, canonical types exist, ID scheme works |
| 2 | [Technology compilation](PLAN-02-technology.md) | `technology/` | phase 1, `crates/tech` | Consumer of `philis_tech::CompiledTech`, coverage inventory |
| 3 | [Fact extraction](PLAN-03-facts.md) | `facts/` | phase 1 | `CanonicalFactGraph` built from SPICE netlist |
| 4 | [Intent compilation](PLAN-04-intent.md) | `intent/` | phases 1–3 | `IntentGraph` from user constraints + all inference detectors |
| 5 | [Reconciliation](PLAN-05-reconcile.md) | `reconcile/` | phase 4, `crates/solver` | conflict detection, waivers, contradiction cores (MUS/MCS via `philis_solver`) |
| 6 | [Projections](PLAN-06-projections.md) | `project/` | phase 5 | `PlacementConstraintProjection` + `RoutingConstraintProjection` |
| 7 | [Evidence, solver & certificate](PLAN-07-evidence-certificate.md) | `evidence/`, `solver/`, `certificate/`, `quality/`, `diagnostics/` | phase 6, `crates/solver` | evidence loop, `ConstraintCertificate`, solver lowering surface (via `philis_solver`) |

Phases 1–3 can be developed concurrently (they share only the type vocabulary).
Phase 4 depends on 1–3 being complete.
Phases 5–7 are sequential.

## Crate dependency direction

```
crates/tech         (shared: CompiledTech, LayerStack, RuleSet, Coverage, PdkState)
crates/solver       (shared: SatSolver, IlpSolver, MUS/MCS extraction)
       ↑                ↑
crates/constraints  (owns canonical types; depends on tech + solver)
       ↑
crates/api          (depends on constraints, re-exports via compat)
```

The constraints crate depends on two shared crates:
- **`crates/tech` (`philis-tech`)** — for `CompiledTech`, technology
  compilation, coverage inventory, and `PdkState` classification. The
  `technology/` module in constraints becomes a consumer of `philis_tech`
  rather than implementing compilation from scratch.
- **`crates/solver` (`philis-solver`)** — for `SatSolver`/`IlpSolver`
  traits and MUS/MCS extraction, used by reconciliation (contradiction core
  extraction) and the solver lowering surface.

The api crate currently owns `Coverage`, `IntentClass`, `AcceptanceGate`, etc.
in `core.rs`. When constraints lands, api gains a `philis_constraints`
dependency and the `compat` module in constraints provides bidirectional
mapping. The types in `core.rs` stay as the *realized* API-level vocabulary;
constraints owns the *canonical* superset. No breaking change to the public
API surface.

## Module map (target)

```
src/
  lib.rs                     — public facade, re-exports
  id.rs                      — CanonicalId, content-addressed hashing
  types.rs                   — ObligationClass, CoverageValue, ConstraintTuple, Authority
  compat.rs                  — bidirectional maps to/from api's IntentClass/Coverage
  technology/
    mod.rs                   — consumer of philis_tech::CompiledTech, coverage inventory types
    sky130.rs                — SKY130 setup (wraps philis_tech)
    coverage.rs              — CoverageValue lattice, meet, MCMM key
  facts/
    mod.rs                   — CanonicalFactGraph
    normalize.rs             — name canonicalization, sorting
    device.rs                — device/terminal/net fact types
  intent/
    mod.rs                   — IntentGraph, IntentEdge
    compile.rs               — user Constraint → intent edges
    detect/
      mod.rs                 — detector trait, registry, orchestrator
      diffpair.rs            — differential pair detector
      mirror.rs              — current mirror / cascode detector
      symmetry.rs            — structural symmetry (color refinement)
      matching.rs            — matching groups, interdigitation, common centroid
      flow.rs                — signal / current flow paths
      guard.rs               — guard ring, isolation, well, domain
      power.rs               — power / ground / substrate / clock nets
      sensitive.rs           — sensitive / critical / shielded nets
      host.rs                — host-template, padframe, fixed-pin bindings
      rf.rs                  — RF, inductor, transmission-line nets
      hv.rs                  — high-voltage, ESD, voltage-dependent spacing
      drc_lvs.rs             — DRC/LVS/PEX/EM/antenna obligation inference
  reconcile/
    mod.rs                   — reconciliation orchestrator
    conflict.rs              — pairwise, groupwise, authority-driven conflict detection
    waiver.rs                — waiver model, admissibility, ATMS labels
    core_extract.rs          — contradiction core extraction (structural + exact)
  project/
    mod.rs                   — projection engine, soundness checker
    placement.rs             — PlacementConstraintProjection
    routing.rs               — RoutingConstraintProjection
    analysis.rs              — AnalysisObligationProjection
  evidence/
    mod.rs                   — evidence ledger facade
    ledger.rs                — append-only keyed ledger
    repair.rs                — repair obligation generation, monotonicity check
    ingest.rs                — DRC/LVS/PEX/EM marker → obligation conversion
  quality/
    mod.rs                   — hard/bounded/soft vectors, lexicographic comparator
  solver/
    mod.rs                   — LexicographicObjective, BackendArtifact, lowering trait (uses philis_solver)
  certificate/
    mod.rs                   — ConstraintCertificate assembly, Merkle root
  diagnostics/
    mod.rs                   — structured diagnostic types, rendering
```

## Key architectural decisions

1. **Constraints owns canonical types; api owns realized types.** The seven-variant
   `ObligationClass` and nine-valued `CoverageValue` live here. Api's four-variant
   `IntentClass` and its `Coverage` enum are the realized subsets.

2. **Content-addressed IDs from day one.** Even with a lean tuple, IDs are computed
   by hashing semantic fields. BLAKE3 for production, with the hash family recorded
   in the certificate.

3. **Batch-first, incremental-ready.** Every phase produces an immutable snapshot.
   Data structures are arena-allocated where possible (stable indices, cheap cloning).
   The `IntentGraph` uses an adjacency-list representation with `CanonicalId` as the
   key, so a future incremental pass can diff by ID set difference.

4. **Full inference, but detectors are ordered by value.** The detector registry
   runs detectors in priority order. The first tier (diff pair, mirror, cascode,
   matching) covers 80% of real analog circuits. Later tiers (RF, HV, host-template)
   add coverage for specialized domains.

5. **SKY130 grounds the technology model.** The `CompiledTech` type is
   provided by `crates/tech` (`philis_tech`), with a concrete SKY130
   implementation. The constraints crate's `technology/` module consumes
   `philis_tech::CompiledTech` rather than defining its own compilation
   pipeline. This prevents duplication and ensures the same technology model
   is used by placement, routing, and constraint compilation.

6. **Both projections in parallel.** Placement and routing projections are independent
   queries over the same IntentGraph. They share the soundness checker but produce
   different output types.

## Testing strategy

- **Per-module unit tests** for every type, algorithm, and invariant.
- **Reference circuits** in `tests/circuits/`: bandgap, diff pair + mirror,
  SAR comparator, LDO, OTA. Each has a golden constraint set.
- **Property tests** (proptest): fuzz the constraint graph and verify the four
  formal invariants hold.
- **Round-trip tests**: compile → serialize → deserialize → re-check ID stability.
- **Projection soundness check**: `expressed ∪ skipped = Oblig_s` for every test case.
- **Integration tests** via the api crate: `Circuit::run()` now calls the real
  constraint compiler instead of the stub.

## Open risks

- **PDK parsing for real foundry PDKs** is out of scope for SKY130 but will be
  the dominant effort for any commercial node. The `CompiledTechnology` trait
  must be designed so that parser output can fill it without rewriting the trait.
- **Exact MUS/MCS enumeration** is NP-hard in the general case. The budget-gated
  approach (return `Unknown` + partial core on timeout) is correct but users may
  not accept "we don't know if this is infeasible" as an answer.
- **Inference accuracy** on unfamiliar topologies. Domain-specific detectors are
  accurate on known structures; the structural-symmetry fallback (WL) carries
  confidence, not certainty.
