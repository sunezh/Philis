# Philis `crates/api` — Stub Ledger

This crate implements the **full public API surface** described in
[`docs/Developer/api/`](../../docs/Developer/api/) — the gradually-tiered,
low-coupling, immediate-mode, caller-driven façade. The *interface* is complete
and exercised by tests; several *implementations* are stubs awaiting the real
engine.

This file is the honesty contract. The single defect the design docs single out
is an API that **accepts input it silently discards**. To avoid that, every stub
is listed here, and unsupported inputs surface as warnings/errors at runtime
(e.g. `BuildWarning::UnsupportedConstraint`) rather than being dropped quietly.

Legend: ✅ real · 🟡 partial · 🔲 stub (shape real, behavior placeholder).

---

## Foundation tier — `io/` (zero dependencies)

| Item | Status | Notes |
| --- | --- | --- |
| `io::spice` — `SpiceNetlist::parse` / `to_spice` | ✅ | Real reader/writer: comments, `+` continuations, `.subckt`/`.ends`, device cards (M/Q/D/R/C/L/X + generic), `key=value` params. |
| `io::spice` — expression params (`{...}` / `'...'`) | 🟡 | Stored as opaque strings; not evaluated. |
| `io::spice` — `.include` / `.lib` | 🟡 | Recorded, not pulled from disk (caller owns the filesystem). |
| `io::spice` — **bipartite hypergraph view** | 🔲 | The device/net hypergraph the engine consumes will be **exposed from `io::spice`** (deferred per design). `core::run_flow` notes where it plugs in. |
| `io::gds` — `Gds::read` / `write` / `add_rect` | ✅ | Real GDSII stream: record framing, INT2/INT4, 8-byte excess-64 REAL, BOUNDARY polygons; round-trips. |
| `io::gds` — `PATH` / `SREF` geometry | 🟡 | Counted in `element_count`, geometry not retained on read. |
| `io::pdk` — `Pdk::from_json/toml_*` | 🔲 | Tolerant std-only key scanner (extracts `tech`, `db_unit_nm`, `layers`); **not** a full JSON/TOML parser or schema validator. Warns on unrecognized shape. |

## Engine tier — `core.rs`

| Item | Status | Notes |
| --- | --- | --- |
| `run_constraints` | 🔲 | Returns `honored = device_count`; no real constraint compilation. |
| `run_placement` | 🔲 | Returns `placed_count = device_count`, `baseline_used = true`, plus the honest `PlacementCertificate::stub()` (every acceptance gate `Degraded`, `degraded_confidence = true`); no real placement or gate evaluation. The signoff-first contract it fills is the placement contract in `core.rs` (see below). |
| `run_routing` | 🔲 | Returns `routed_segments = 0` plus the honest `RouteCertificate::stub()` (feasible-shaped, `degraded_confidence = true`) and a degraded `RouteQuality`; no real routing/extraction. The signoff-first contract it fills is `core.rs` (see below). |
| `run_flow` | 🔲 | Composes the three stub oracles. |
| `Objective::apply_to_config` | 🔲 | Placeholder pitch presets, not characterized values. |
| `HintBuilder` (`seed`/`fix_position`/`arrange`) | 🔲 | Hints are **recorded, not acted on** (intentionally stored, never silently dropped). |
| `Strategy` | 🔲 | Consumed only by `HintBuilder::arrange`; not yet wired to placement. |
| `Constraint` — 10 "supported" variants | 🔲 | Classified as engine-forwarded; the engine that would honor them is a stub. |
| `Constraint` — 12 "unsupported" variants | 🔲 | Surface honest: `build()` emits `BuildWarning::UnsupportedConstraint(kind)`. |

## Placement contract — `core.rs`

The signoff-first placement vocabulary (documented in
[`docs/Developer/place/`](../../docs/Developer/place/index.html)) lives in
`core.rs` alongside the stage oracles. The placer returns geometry **plus** a
machine-checkable certificate, never geometry alone, and acceptance is a
lexicographic hierarchy of hard *gates* — technology coverage → local DRC →
device equivalence → terminal access → routability → intent satisfaction —
gated before any soft objective (area, HPWL, parasitics). It is the placement
counterpart of the routing contract in `core.rs` and shares its canonical
`Coverage`/`PdkState`/`RuleCoverageReport` vocabulary. The *types* are real and
unit-tested; the gate *evaluator* that proves their contents is the stub
`run_placement` above.

| Item | Status | Notes |
| --- | --- | --- |
| `AcceptanceGate` / `ACCEPTANCE_GATE_ORDER` (`name`/`is_repairable`) | ✅ | The six gates in realized evaluation order; only `LocalDrc` and `IntentSatisfaction` are repairable before legalization. |
| `evaluate_gate_acceptance` | ✅ | The repairable-failure policy, executable (pre- vs post-legalization). |
| `GateStatus` / `GateEvaluation` | ✅ | Per-gate verdict (`Pass`/`Degraded`/`Fail`) and evidence string. |
| `IntentClass` + `Constraint::intent_class` | ✅ | Maps the `Constraint` taxonomy onto `Hard`/`Bounded`/`Soft`/`Unsupported`; unsupported variants are surfaced, never dropped. |
| `ConflictClass` / `PlacementConflict` | ✅ | Typed conflict-core classes, each blamed against a gate (`ConflictClass::gate`). |
| `RouterHandoffWitness` | ✅ (shape) 🔲 (contents) | The placement→router readiness witness; populated trivially by the stub. |
| `PlacementCertificate` (`is_signoff_quality`/`is_accepted`/`stub`) | ✅ (shape) 🔲 (contents) | Full certificate shape; the stub marks it `degraded_confidence` so it is signoff-ready *shaped*, never signoff-quality. |

## Routing contract — `core.rs`

The signoff-first routing vocabulary (documented in
[`docs/Developer/route/`](../../docs/Developer/route/index.html)). The router
returns geometry **plus** a machine-checkable certificate, never geometry alone,
and acceptance is a lexicographic hard vector (PDK/access validity → DRC=0 →
LVS=true → all required nets routed) followed by a soft Pareto tail. The
*types* are real and unit-tested; the *engine* that proves their contents is the
stub `run_routing` above.

| Item | Status | Notes |
| --- | --- | --- |
| `HardVector` (`is_feasible`/`as_priority`/`lex_le`) | ✅ | The lexicographic acceptance relation, executable. |
| `Coverage` / `RuleCoverageReport` / `PdkState` | ✅ | Canonical rule-coverage vocabulary + the four PDK readiness states. The *classifier* that fills them from a real PDK is future work. |
| `NetClass` / `AccessConfidence` | ✅ | Net intent classes (with `is_grouped()`) and access confidence levels. |
| `RouteDiagnostic` / `DiagnosticClass` / `DiagnosticScope` | ✅ | Typed failure/infeasibility classes and repair scope. |
| `Evidence` / `RouteCertificate` / `RouteQuality` | ✅ (shape) 🔲 (contents) | Full certificate + quality shape; populated trivially by the stub oracle, marked `degraded_confidence`. |

## Flow / builder tiers — `flow.rs`, `builder.rs`

| Item | Status | Notes |
| --- | --- | --- |
| `Circuit::run` / staged `analyze`→`place`→`route` | 🟡 | Real tiering & threading; drives the stub engine. |
| `BuiltCircuit::solve` / `solve_with` | 🟡 | Real build/validation; stub engine. |
| `DeviceBuilder` params (`w`/`l`/`nf`/`r`/`c`/`area`/`pj`) | 🔲 | Recorded for shape, not yet used by placement. |
| `Circuit::select_subckt` / `use_first_subckt_as_top` | 🟡 | Selection honored; subckt body promotion depends on the spice reader's body model. |

## Results & verification — `layout.rs`, `verify.rs`

| Item | Status | Notes |
| --- | --- | --- |
| `Layout::area`/`width`/`height`/`wirelength` | 🔲 | Zeroed — no real geometry yet. |
| `Layout::instances` | 🔲 | Empty iterator. |
| `Layout::placed_count`/`routed_segment_count`/`flow`/`gds`/`write_gds` | ✅ | Real plumbing over the stub results. |
| `Layout::certificate`/`router_handoff`/`is_signoff_quality` | ✅ (plumbing) 🔲 (contents) | Surface the `PlacementCertificate` and its handoff; `is_signoff_quality` is honestly `false` under the stub. |
| `verify::drc`/`lvs`/`pex`/`erc` | 🔲 | Return clean reports; no rule decks. Callable on every manual tier and post-automation, against the netlist + incremental GDS (the design's "check anytime" property). |

---

## Roadmap to de-stub

1. Expose the bipartite hypergraph from `io::spice` and feed it to `core`.
2. Replace the three `run_*` oracles with the real constraint/placement/routing
   engines; geometry then flows into `Layout` and the incremental `Gds`. For the
   placer, this fills the placement contract in `core.rs` with proven contents — the gate
   evaluator, the typed oracles, the hybrid solver, and group-preserving
   legalization — see the de-stub roadmap in
   [`docs/Developer/place/Api-Mapping.html`](../../docs/Developer/place/Api-Mapping.html).
   For the router, this fills the `core.rs` contract — see the eight-step
   router de-stub roadmap in
   [`docs/Developer/route/Api-Mapping.html`](../../docs/Developer/route/Api-Mapping.html).
3. Implement the 12 unsupported constraints, then delete their
   `UnsupportedConstraint` warnings.
4. Wire `Strategy`/`HintBuilder` into placement.
5. Implement the DRC/LVS/PEX/ERC rule decks in `verify`.
6. Swap `io::pdk`'s key scanner for a real JSON/TOML schema parser.

Each row above maps to a checklist item in the API design docs; closing one
should flip its status here and, where applicable, remove a runtime warning.
