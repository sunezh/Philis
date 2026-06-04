# Architecture — Crate Boundary, Trait, Data Flow

## The `PlacementEngine` trait

Defined in `crates/api/src/core.rs`. This is the seam between the contract
types and the engine implementation.

```
trait PlacementEngine {
    fn place(&self, input: &PlacementInput) -> PlacementOutput;
}
```

### `PlacementInput` (widened from `PlacementRunInput`)

The current `PlacementRunInput { device_count: usize }` is too thin. The real
input must carry everything the solver needs:

| Field | Type | Source |
|-------|------|--------|
| `netlist` | `&SpiceNetlist` | The parsed SPICE netlist (devices, nets, terminals) |
| `constraints` | `&[Constraint]` | The user's constraint list |
| `config` | `RunConfig` | Pitch/grid/mode tuning knobs |
| `hints` | `&HintBuilder` | Seed, fixed positions, arrangements |
| `pdk` | `&Pdk` | The loaded PDK data |
| `outline` | `Option<(i64, i64)>` | Host outline (width, height) in nm, if fixed |
| `fixed_pins` | `&[(String, i64, i64)]` | Immutable boundary pins (name, x, y) |
| `obstacles` | `&[Rect]` | Placement-forbidden regions |

### `PlacementOutput` (widened from `PlacementResult`)

The output must carry actual geometry alongside the certificate:

| Field | Type | Purpose |
|-------|------|---------|
| `placements` | `Vec<DevicePlacement>` | Per-device (x, y, orientation, primitive_id) |
| `geometry` | `Vec<Polygon>` | The GDS-ready polygon set |
| `certificate` | `PlacementCertificate` | The signoff-first proof (filled, not stub) |
| `area` | (i64, i64) | Bounding box (width, height) in nm |

Where `DevicePlacement` is:

| Field | Type |
|-------|------|
| `instance_name` | `String` |
| `primitive_id` | `String` (the PDK device model) |
| `x` | `i64` (nm) |
| `y` | `i64` (nm) |
| `orientation` | `Orientation` (R0, R90, R180, R270, MX, MY, MXR90, MYR90) |
| `width_nm` | `i64` |
| `height_nm` | `i64` |

### Registration

`crates/api` provides a default engine (the stub) and a way to register a real
engine. The simplest mechanism:

```
// in crates/api/src/core.rs
pub fn run_placement_with(engine: &dyn PlacementEngine, input: &PlacementInput)
    -> PlacementOutput;

// the existing run_placement becomes:
pub fn run_placement(input: &PlacementRunInput) -> PlacementResult {
    // delegate to the stub or to the registered engine
}
```

The binary in `crates/api/src/main.rs` or the Python extension constructs a
`philis_place::Sky130Placer` and passes it in. No feature flags, no global
state — the caller owns the engine instance.

## Data flow

```
User code (builder / flow / Python)
  |
  | builds SpiceNetlist + Vec<Constraint> + RunConfig + Pdk
  |
  v
crates/api: run_placement_with(engine, input)
  |
  | calls engine.place(input)
  |
  v
crates/place: PlacementEngine::place
  |
  +-- 1. PDK compile    (pdk/ module, via philis_tech)
  |      Sky130 rules -> CompiledTech (from crates/tech)
  |
  +-- 2. Constraint compile  (constraint/ module)
  |      Vec<Constraint> -> IntentGraph
  |
  +-- 3. Primitive catalog   (primitive/ module)
  |      netlist devices -> legal variants with geometry
  |
  +-- 4. Solver              (solver/ module)
  |      SA on sequence-pair -> candidate (x, y, orient) per device
  |
  +-- 5. Legalization        (legalize/ module)
  |      snap to grid, fix overlaps, group-preserve
  |
  +-- 6. Gate evaluation     (oracle/ module)
  |      run DRC, equiv, access, routability, intent checks
  |      -> fill GateEvaluation verdicts
  |
  +-- 7. Certificate         (lib.rs)
  |      assemble PlacementCertificate from gate results
  |
  +-- 8. Geometry emit       (geometry/ module)
  |      placed devices -> GDS polygons
  |
  v
PlacementOutput { placements, geometry, certificate, area }
```

## Module responsibilities

### `pdk/` — Technology compilation

> **Shared crate:** PDK compilation now lives in `crates/tech`
> (`philis_tech`). This module is a thin consumer of
> `philis_tech::CompiledTech` — it calls the tech compiler and queries the
> resulting model, rather than implementing compilation from scratch.

Reads the `Pdk` struct from `crates/api/io/pdk` and obtains the compiled
technology model from `philis_tech`. For Sky130 this means:
- Layer numbering and roles
- Minimum spacing rules per layer
- Minimum width per layer
- Well/implant enclosure rules
- Legal orientations (all 8 for Sky130 planar)
- Manufacturing grid (5nm for Sky130)

See [02-pdk-compiler.md](02-pdk-compiler.md).

### `primitive/` — Device geometry

Given a SPICE device card (model name, W, L, nf, mult) and the compiled
technology, produce the polygon set and terminal map for one placed device.
For Sky130 nfet/pfet this means: diffusion, poly, contacts, metal1 pins, well,
implant layers.

See [03-primitives.md](03-primitives.md).

### `solver/` — SA + sequence-pair

The core placement algorithm. Encodes a placement as a sequence-pair, generates
constraint-aware moves (swap, rotate, mirror-pair), evaluates cost (area +
constraint violations + DRC penalty), and runs simulated annealing with
geometric cooling.

See [04-sequence-pair.md](04-sequence-pair.md).

### `constraint/` — Intent compilation

Transforms the flat `Vec<Constraint>` into an `IntentGraph` — a set of typed
edges between devices/groups. Each edge carries its `IntentClass` and the
geometric invariant it enforces. The solver queries the IntentGraph to validate
constraint-aware moves and to compute the intent component of the cost function.

See [05-constraints.md](05-constraints.md).

### `oracle/` — Gate evaluators

One module per acceptance gate. Each oracle takes the current placement snapshot
and returns a `GateEvaluation { gate, status, evidence }`. The oracle dispatch
runs them in `ACCEPTANCE_GATE_ORDER`.

See [06-drc-oracle.md](06-drc-oracle.md).

### `spatial/` — Spatial indexing

> **Shared crate:** Geometry types (`Rect`, `RectiPoly`) and the R-tree
> spatial index come from `crates/geom` (`philis_geom`). This module wraps
> the shared `GeometryStore` for placement-specific query patterns.

An R-tree from `philis_geom` for overlap and neighbor queries. Built once
per SA iteration on accepted moves. Used by the DRC oracle and the
legalization pass.

### `legalize/` — Group-preserving legalization

Post-SA cleanup: snap to manufacturing grid, resolve residual overlaps by
shifting, re-mirror symmetry pairs if disturbed. Operates group-by-group, never
moves one side of a matched pair alone.

See [07-legalization.md](07-legalization.md).

### `geometry/` — GDS polygon emission

Takes the finalized `Vec<DevicePlacement>` and the primitive catalog and emits
the polygon set suitable for `io::gds::Gds::add_rect`. Also computes the
bounding box.

See [08-integration.md](08-integration.md).

## Dependency graph

```
crates/place
  depends on:
    crates/api    — for contract types, SpiceNetlist, Pdk, Constraint, etc.
    crates/geom   — Rect, RectiPoly, orientation transforms, spatial index,
                    GeometryStore (shared geometry kernel)
    crates/tech   — CompiledTech, LayerStack, ViaTable, RuleSet, Coverage,
                    PdkState, PDK importers (shared technology model)
    crates/verify — DRC rule predicates, DrcViolation types, LVS
                    connectivity tracking (shared verification layer)
    rand          — seeded RNG for SA (reproducibility via seed in certificate)

crates/api
  depends on:
    crates/place  — NO. api calls place via the trait, but the dependency is
                    inverted: api defines the trait, place implements it,
                    and the binary/Python extension constructs the engine
                    and passes it in.
```

The binary (`crates/api/src/main.rs`) or integration test is the composition
root that depends on both `crates/api` and `crates/place`.

## Orientation encoding

Sky130 planar CMOS supports all 8 standard orientations. The engine uses:

| Name | Transform |
|------|-----------|
| R0   | identity |
| R90  | 90 degrees counterclockwise |
| R180 | 180 degrees |
| R270 | 270 degrees counterclockwise |
| MX   | mirror about X axis |
| MY   | mirror about Y axis |
| MXR90 | MX then R90 |
| MYR90 | MY then R90 |

For symmetric pairs, `MY` is the default mirror operation (reflects the device
about its vertical center axis). The constraint compiler determines which mirror
operation a `Symmetric` constraint implies based on the symmetry axis direction.
