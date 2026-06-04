# Integration — Wiring into crates/api, Certificate, GDS Output

> **Shared crate dependencies:** This crate depends on `crates/geom`
> (`philis_geom`) for geometry types and spatial indexing, `crates/tech`
> (`philis_tech`) for the compiled technology model and PDK state
> classification, and `crates/verify` (`philis_verify`) for DRC rule
> predicates and violation types. The integration layer wires these shared
> crates together with the placement-specific solver, constraint compiler,
> and certificate assembly.

## The `PlacementEngine` trait in `crates/api`

### Trait definition (added to `crates/api/src/core.rs`)

```
pub trait PlacementEngine {
    fn place(&self, input: &PlacementInput) -> PlacementOutput;
}
```

### `PlacementInput` (replaces the thin `PlacementRunInput`)

The existing `PlacementRunInput { device_count: usize }` stays as a
backwards-compatible alias. The real engine receives `PlacementInput`:

| Field | Type | Source |
|-------|------|--------|
| `netlist` | `SpiceNetlist` | Parsed SPICE |
| `constraints` | `Vec<Constraint>` | User constraints |
| `config` | `RunConfig` | Pitches, grid, mode flags |
| `hints` | `HintBuilder` | Seed, fixed positions, arrangements |
| `pdk` | `Pdk` | Loaded PDK |
| `outline` | `Option<(i64, i64)>` | Fixed outline (nm) |
| `fixed_pins` | `Vec<(String, i64, i64)>` | Immutable boundary pins |
| `obstacles` | `Vec<Rect>` | Forbidden regions |

### `PlacementOutput` (replaces the thin `PlacementResult`)

| Field | Type | Purpose |
|-------|------|---------|
| `placements` | `Vec<DevicePlacement>` | Per-device placement |
| `polygons` | `Vec<LayeredRect>` | GDS-ready geometry |
| `certificate` | `PlacementCertificate` | Filled certificate |
| `bbox` | `(i64, i64)` | Bounding box (nm) |

The existing `PlacementResult { placed_count, baseline_used, certificate }`
is derived from `PlacementOutput` for backwards compatibility.

### Registration pattern

No global state. The composition root (binary, Python extension, or test)
constructs the engine and passes it to the flow:

```
// In crates/api/src/core.rs:
pub fn run_placement_with(
    engine: &dyn PlacementEngine,
    input: &PlacementInput,
) -> PlacementOutput {
    engine.place(input)
}

// The existing stub becomes:
pub fn run_placement(input: &PlacementRunInput) -> PlacementResult {
    let stub_output = StubEngine.place(&PlacementInput::from_legacy(input));
    PlacementResult::from_output(stub_output)
}
```

### Wiring into the flow

The `Circuit::run` / staged flow in `flow.rs` currently calls
`core::run_placement`. It needs a way to receive an engine:

**Option A (recommended):** Add `engine: Option<Box<dyn PlacementEngine>>`
to `Circuit` or to a new `EngineConfig` struct. If None, use the stub.

**Option B:** Feature-flag `crates/place` as an optional dependency of
`crates/api`. When the feature is enabled, `run_placement` dispatches to the
real engine. When disabled, it uses the stub.

Option A is simpler and doesn't require feature flags.

### Wiring into the Python extension

The `python.rs` PyO3 module constructs the engine internally:

```
// in crates/api/src/python.rs, inside the place_and_route function:
let engine = philis_place::Sky130Placer::new();
let output = core::run_placement_with(&engine, &input);
```

The Python user never sees the engine — they just call
`design.place_and_route(placer="signoff")` and the engine is selected by
the placer name.

## Certificate filling

The certificate is assembled from the gate evaluation results. This is the
mapping from oracle outputs to `PlacementCertificate` fields:

### `problem_hash`

Hash the input tuple:
```
problem_hash = hash(
    netlist.to_spice(),
    constraints.iter().map(|c| c.kind()).collect(),
    config,
    pdk.tech_name(),
    outline,
    fixed_pins,
    obstacles,
)
```

Use a deterministic hash (e.g., `std::hash::DefaultHasher` with a fixed seed,
or FNV). The hash changes when any input changes, invalidating cached results.

### `pdk_oracle_hash`

Hash the compiled technology:
```
pdk_oracle_hash = hash(
    technology.layers,
    technology.spacing_rules,
    technology.width_rules,
    technology.enclosure_rules,
    technology.well_rules,
)
```

### `pdk_state`

Classified from the technology compilation coverage:
- If any `Coverage::Contradictory` → `ContradictoryOrMissing`
- If any `Coverage::Missing` for a required rule → `ContradictoryOrMissing`
- If any `Coverage::Unsupported` or `Coverage::ManualReview` → `SparseResearch`
- If all required rules are `Coded` or `Partial` → `AbstractComplete`
- If all rules are `Coded` → `FullSignoff`

For Sky130 V1: `SparseResearch` (many rules are `Missing`).

### `gates`

One `GateEvaluation` per gate in `ACCEPTANCE_GATE_ORDER`:

| Gate | Oracle | V1 status |
|------|--------|-----------|
| TechnologyCoverage | PDK compiler coverage report | `Degraded` (partial rules) |
| LocalDrc | DRC oracle | `Pass` or `Fail` (real check runs) |
| DeviceEquivalence | Primitive catalog equivalence check | `Degraded` (basic check only) |
| TerminalAccess | Terminal access map from primitives | `Degraded` (no via-stack checking) |
| Routability | Channel capacity estimate | `Degraded` (no congestion model) |
| IntentSatisfaction | Constraint evaluator | `Pass` or `Fail` (real check runs) |

For V1, only LocalDrc and IntentSatisfaction can reach `Pass`. The others
are `Degraded` because their oracles are partial.

### `coverage`

The `RuleCoverageReport` from the PDK compiler. For V1:
```
RuleCoverageReport {
    coded: 2,           // overlap, grid snap
    partially_coded: 4,  // spacing, well, enclosure, implant
    approximate: 0,
    lvs_only: 0,
    signoff_only: 0,
    recommended_only: 0,
    manual_review: 0,
    unsupported: 0,
    missing: 4,          // density, antenna, wide-wire, LDE
    contradictory: 0,
}
```

### `degraded_confidence`

`true` in V1. Will become `false` when all gates can reach `Pass` and
coverage is `Coded` for all required families.

### `degraded_causes`

One `DegradedConfidenceCause` per gate that's `Degraded`:
```
[
    DegradedConfidenceCause {
        gate: TechnologyCoverage,
        reason: "Sky130 PDK rules partially coded (15 of ~200 spacing rules)",
        coverage: Coverage::Partial,
    },
    DegradedConfidenceCause {
        gate: DeviceEquivalence,
        reason: "equivalence check covers model/W/L/nf only, not imported cells",
        coverage: Coverage::Partial,
    },
    // ...
]
```

### `handoff`

The `RouterHandoffWitness`:
```
RouterHandoffWitness {
    has_access_map: true,       // we have terminal pins from primitives
    reserved_channel_count: 0,  // no channel reservation yet
    terminal_count: N,          // total terminals across all placed devices
    host_pin_count: M,          // from fixed_pins input
    obstruction_count: K,       // from obstacles input
    note: "V1 placer — terminal access from primitive pins, no via-stack or congestion model",
}
```

### `infeasible_core`

Empty on a successful placement. On failure, populated with
`PlacementConflict` entries from the gate evaluators.

### `solver_path`

`"sa-sequence-pair->legalize"` for V1.

### `seed`

The seed from `HintBuilder.seed_value()`, or a default seed if none specified.

## GDS output

### From placements to polygons

For each `DevicePlacement`:
1. Look up the `Primitive` in the catalog by `primitive_id`
2. Apply the orientation transform to each polygon in the primitive
3. Translate by `(x, y)`
4. Collect all translated/transformed polygons

### Writing to `Gds`

Using the existing `io::gds::Gds` API:
```
let mut gds = Gds::new();
for polygon in &output.polygons {
    gds.add_rect(polygon.layer, polygon.datatype,
                 polygon.x0, polygon.y0, polygon.x1, polygon.y1);
}
gds.write("output.gds")?;
```

### Integration with `Layout`

The `Layout` struct in `crates/api/src/layout.rs` currently has stub fields.
After integration:

- `Layout::area()` → `output.bbox.0 * output.bbox.1` (in nm^2, converted to um^2)
- `Layout::width()` → `output.bbox.0` as `Length`
- `Layout::height()` → `output.bbox.1` as `Length`
- `Layout::instances()` → iterator over `output.placements` wrapped as `PlacedInstance`
- `Layout::placed_count()` → `output.placements.len()`
- `Layout::certificate()` → `&output.certificate`
- `Layout::router_handoff()` → `&output.certificate.handoff`
- `Layout::is_signoff_quality()` → `output.certificate.is_signoff_quality()`
- `Layout::gds()` → the `Gds` built from `output.polygons`
- `Layout::write_gds(path)` → write the GDS to disk

## Test plan

### Unit tests (in `crates/place`)

1. **PDK compiler:** Parse Sky130 layer table, verify rule extraction
2. **Primitive generator:** Generate nfet primitive, verify bbox, terminal
   positions, layer correctness
3. **Sequence-pair:** Encode/decode a 4-device example, verify non-overlap
4. **SA solver:** Run SA on a 4-device test case, verify convergence
5. **Constraints:** Verify symmetric pair encoding, ordering enforcement
6. **DRC oracle:** Check overlap detection, spacing measurement
7. **Legalization:** Verify grid snap, overlap removal, symmetry repair
8. **Certificate:** Verify gate evaluations produce correct certificate fields

### Integration tests (in `crates/api/test/`)

1. **End-to-end:** Parse a SPICE netlist, load Sky130 PDK, run placement,
   verify GDS output and certificate
2. **Constraint round-trip:** Set constraints via builder, verify they're
   honored in the placement
3. **Failure case:** Give an impossible constraint set, verify the infeasible
   core contains the right conflicts
4. **Determinism:** Run twice with the same seed, verify byte-identical output

### Benchmark (in `bench/`)

1. **SA scaling:** Measure placement time for 10, 50, 100, 200 devices
2. **DRC oracle:** Measure per-check time for varying device counts
3. **Sequence-pair decode:** Measure decode time for varying n

## Migration plan

### Step 1: Add the trait to `crates/api`

- Define `PlacementEngine` trait in `core.rs`
- Add `PlacementInput` / `PlacementOutput` types
- Keep `run_placement` unchanged (still returns the stub)
- Add `run_placement_with` that delegates to a `&dyn PlacementEngine`
- Add backwards-compatibility conversions between old and new types

### Step 2: Build `crates/place` with the shared crates

- Implement `Sky130Placer` struct implementing `PlacementEngine`
- Use `philis_tech::CompiledTech` for the compiled technology model
  (Sky130 rules come from `crates/tech`, not hardcoded locally)
- Use `philis_geom::Rect` and orientation transforms for primitives
- Implement `PrimitiveCatalog` for nfet/pfet
- Wire into `crates/api` via `run_placement_with`
- At this point: real geometry, stub solver (still greedy)

### Step 3: Implement the SA solver

- Sequence-pair encoding/decoding
- SA loop with basic moves
- HPWL computation from the netlist
- At this point: real placement, no constraints

### Step 4: Add constraint-aware placement

- IntentGraph compilation
- Constraint-aware moves (symmetric swap, group move)
- Constraint penalty in cost function
- Gate evaluation for IntentSatisfaction
- At this point: constraint-aware placement

### Step 5: Add DRC oracle

- R-tree spatial index (from `philis_geom`)
- Overlap and spacing checks (using `philis_verify` predicates and violation types)
- Gate evaluation for LocalDrc
- Legalization (grid snap, overlap removal, spacing repair)
- At this point: DRC-checked placement

### Step 6: Wire GDS output and certificate

- Polygon generation from placed primitives
- Certificate assembly
- Layout integration
- At this point: end-to-end working placer

Each step produces a testable, committable increment. The certificate is
honest at every step — missing capabilities are `Degraded`, not `Pass`.
