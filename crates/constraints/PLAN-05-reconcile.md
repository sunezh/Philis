# Phase 5 — Reconciliation

**Goal:** Detect and resolve conflicts in the IntentGraph. Produce waivers,
contradiction cores, and a conflict-free (or explicitly-conflicted) graph
ready for projection.

**Depends on:** phase 4 (IntentGraph)  
**Unlocks:** phase 6 (projections read the reconciled graph)

---

## 5.1 What reconciliation does

The IntentGraph after detection may contain:
- **Duplicates** — already handled by content-addressed merge in phase 4.
- **Pairwise conflicts** — two obligations that cannot both be satisfied
  (a fixed position + a symmetry constraint on the same device).
- **Groupwise conflicts** — a joint obligation's members conflict with
  an external constraint.
- **Authority conflicts** — a user constraint contradicts a foundry rule.
- **Coverage gaps** — a required obligation has no technology support.
- **Model gaps** — a bounded obligation references a metric with no model.

Reconciliation classifies each conflict, resolves those with clear authority
ordering, and packages the rest as contradiction cores.

---

## 5.2 Conflict detection — `src/reconcile/conflict.rs`

### Structural conflict detection (cheap, O(n^2) worst case on entity overlap)

The fast pass that catches most real conflicts:

1. **Build an entity-to-obligation index:** for each entity (device, net,
   terminal), collect all obligations that reference it.

2. **Pairwise conflict check:** for each entity with 2+ obligations,
   check every pair for predicate incompatibility:
   - `FixedPosition(x1,y1)` vs `FixedPosition(x2,y2)` where `(x1,y1) != (x2,y2)` → conflict
   - `SymmetricPair(axis=X)` vs `OrderedPair(axis=X)` on overlapping entities → conflict
   - `DoNotRoute` vs any routing obligation on the same net → conflict
   - `Hard` obligation with `CoverageValue::Missing` → singleton conflict (self-contradictory)

3. **Group conflict check:** for each `ObligationGroup`, verify that no
   member obligation is individually in conflict with a higher-authority
   non-group obligation.

### Conflict classification

```
ConflictKind {
    Singleton,          // a single self-contradictory obligation
    Pairwise,           // two obligations contradict
    GroupVsExternal,    // a group member contradicts an external obligation
    AuthorityClash,     // same entity, same predicate kind, different authorities
    CoverageGap,        // required coverage is Missing/Contradictory
    ModelGap,           // bounded metric references unavailable model
}
```

Each detected conflict is wrapped as:

```
DetectedConflict {
    kind: ConflictKind,
    obligations: SmallVec<[CanonicalId; 2]>,  // the conflicting obligations
    authority_winner: Option<CanonicalId>,      // if authority resolves it
    reason: String,
}
```

---

## 5.3 Authority-driven resolution

For conflicts where the authority ordering gives a clear winner:

1. Compare `authority(a)` vs `authority(b)` using the
   `Authority` `Ord` from `types.rs`.
2. If one strictly outranks the other:
   - The lower-authority obligation gets its class downgraded to `Soft`
     or receives a system-generated waiver.
   - A `ResolutionRecord` is emitted for the certificate.
3. If authorities are equal:
   - The conflict is unresolvable by authority alone.
   - Package as a contradiction core for the user.

```
ResolutionRecord {
    conflict: DetectedConflict,
    resolution: Resolution,  // AuthorityWin, WaiverApplied, EscalatedToCore
    winner: Option<CanonicalId>,
    loser_action: LoserAction,  // Downgraded, Waived, Removed (never silently)
}
```

`LoserAction::Removed` means the obligation is moved to a `deferred` set —
it stays in the graph with a waiver annotation, never deleted.

---

## 5.4 Waivers — `src/reconcile/waiver.rs`

### Waiver admissibility

```
fn is_admissible(waiver: &Waiver, obligation: &ConstraintTuple) -> bool {
    waiver.authority >= obligation.source.authority
}
```

### Waiver application

```
fn apply_waiver(obligation: &mut ConstraintTuple, waiver: Waiver) -> WaiverResult {
    if is_admissible(&waiver, obligation) {
        obligation.waiver = Some(waiver);
        WaiverResult::Applied
    } else {
        WaiverResult::Rejected { reason: "insufficient authority" }
    }
}
```

Properties that must hold (tested):
- **Idempotence:** `apply(apply(c, w), w) = apply(c, w)`.
- **Order independence:** `apply(apply(c, w1), w2) = apply(apply(c, w2), w1)`
  for admissible waivers w1, w2 (they both set the waiver field; last-writer
  wins is acceptable since both are admissible).
- **Non-deletion:** the waived obligation stays in the graph. Its `waiver`
  field is `Some(...)`, but it is still queryable and reportable.

### User waivers

User-provided waivers from the problem input (`U` in the input bundle) are
applied during reconciliation. Each user waiver specifies:
- The constraint kind or entity it targets (matched by predicate kind + entities)
- An authority level (defaults to `UserHard`)
- An optional scope restriction
- An optional expiration condition

---

## 5.5 Contradiction core extraction — `src/reconcile/core_extract.rs`

> **Shared crate:** Solver calls for MUS/MCS extraction use `crates/solver`
> (`philis_solver`). The QuickXplain and MARCO algorithms call
> `philis_solver::SatSolver` as the satisfiability oracle, and MUS/MCS
> extraction utilities from `philis_solver` handle the hitting-set duality
> enumeration.

### Structural cores (cheap)

From the conflict detection pass:

- **Singleton:** a single obligation with `CoverageValue::Missing` or
  `Contradictory` on a required rule. Trivially minimal.
  Core tag: `singleton`.

- **Pairwise:** two obligations that directly conflict. Removing either
  resolves the contradiction. Core tag: `pairwise`.

- **Groupwise:** a Joint group has an unresolvable conflict with a non-group
  obligation. Core tag: `groupwise`.

These are produced in O(n) time from the conflict detection results.

### Exact cores (expensive, budget-gated)

For complex conflicts involving 3+ obligations, structural detection may
miss minimal cores. The exact pass uses:

1. **QuickXplain** — for finding a single preferred MUS under authority
   ordering. Divides the obligation set and recursively identifies the
   minimal set that, combined with the "background" (higher-authority
   obligations), is unsatisfiable.
   
   Complexity: O(k * log(n/k)) satisfiability checks, where k is the
   MUS size and n is the total obligation count.
   
   Core tag: `set-minimal-preferred`.

2. **MARCO** — for enumerating multiple independent MUSes. Uses the
   hitting-set duality: each MUS is a minimal hitting set of all MCSes.
   Enumeration alternates between finding a new MCS (a correction set)
   and checking if the remaining map contains another MUS.
   
   Complexity: exponential in the worst case, but budget-gated.
   
   Core tag: `set-minimal-enumerated`.

### Budget gating

The exact explanation pass is the only super-polynomial component. It is
bounded by the budget vector's `explanation_budget` field:

- **Time budget:** default 1 second per reconciliation round.
- **Iteration budget:** default 100 satisfiability checks.
- **Result budget:** default 10 cores.

If the budget is exhausted, the solver returns `Unknown` + the partial
cores found so far. The certificate records the budget and the partial status.

### Core identity

Each core gets a `CoreId` — hash of sorted member `CanonicalId`s + reason tag.
This makes "the same infeasible case always produces the same conflict core"
a checkable invariant.

---

## 5.6 Reconciliation output

The reconciled IntentGraph contains:
- All obligations (including waived ones, with waiver annotations)
- Resolution records for authority-resolved conflicts
- Contradiction cores for unresolvable conflicts
- Diagnostics for coverage/model gaps

```
ReconciliationReport {
    resolved: Vec<ResolutionRecord>,
    cores: Vec<ContradictionCore>,
    waivers_applied: Vec<(CanonicalId, Waiver)>,
    waivers_rejected: Vec<(CanonicalId, Waiver, String)>,
    coverage_gaps: Vec<CanonicalId>,
    model_gaps: Vec<CanonicalId>,
}

ContradictionCore {
    id: CoreId,
    members: Vec<CanonicalId>,      // sorted
    minimality: MinimalityTag,       // Singleton, Pairwise, Groupwise, SetMinimalPreferred, SetMinimalEnumerated
    algorithm: String,               // "structural", "quickxplain", "marco"
    reason: String,
    suggested_relaxation: Option<Vec<CanonicalId>>,  // the MCS dual
}
```

---

## 5.7 Testing

- **Pairwise conflict:** two `FixedPosition` on the same device at different
  coordinates. Verify a pairwise core is emitted.
- **Authority resolution:** a user Hard constraint vs a foundry Mandatory
  rule on the same entity. Verify the user constraint is downgraded and a
  resolution record is emitted.
- **Waiver admissibility:** attempt to waive a foundry rule with UserHard
  authority. Verify rejection.
- **Waiver idempotence:** apply the same waiver twice, verify same result.
- **Waiver non-deletion:** apply a waiver, verify the obligation is still
  in the graph with `waiver = Some(...)`.
- **Singleton core:** a Hard obligation with `CoverageValue::Missing`.
  Verify singleton core emitted.
- **Core identity stability:** same conflict in two runs, same `CoreId`.
- **Budget gating:** set a 0-iteration budget for exact explanation. Verify
  the solver returns `Unknown` + partial cores, not a hang.
- **No false conflicts:** a clean diff-pair circuit with no contradictions.
  Verify zero cores and zero resolutions.

---

## 5.8 Open questions

1. **Satisfiability oracle for QuickXplain/MARCO.** These algorithms need
   a satisfiability check for a subset of obligations. What is the oracle?
   
   For structural conflicts (geometric incompatibility), the check can be
   a simple predicate-compatibility test (does FixedPosition(x1) conflict
   with FixedPosition(x2)?). For more complex conflicts (can this symmetry
   + ordering + boundary placement all be satisfied simultaneously?), the
   check uses `philis_solver::SatSolver` from `crates/solver`.
   
   **Phasing answer:** v1 uses structural-only conflict detection (the
   cheap pass). QuickXplain/MARCO use `philis_solver` as the
   satisfiability oracle when available, falling back to structural checks
   when the solver is not yet integrated.

2. **How to present cores to users.** A `ContradictionCore` with
   `members = [c_abc123, c_def456]` is useless to a human. The diagnostics
   module (phase 7) needs to render cores as circuit-domain explanations:
   "Device M1 cannot be fixed at (100, 200) and symmetric with M2 about
   the Y axis simultaneously."

3. **Conflict between inferred and user-declared intent.** If the diff-pair
   detector infers `SymmetricPair(M1, M2)` and the user declares
   `Order(M1, M2, X)`, the authority ordering puts `UserHard > InferredHard`,
   so the inferred symmetry is downgraded. But the user might *want* both.
   Should there be a "user confirms inferred" mechanism? Probably yes, as
   a future enhancement.
