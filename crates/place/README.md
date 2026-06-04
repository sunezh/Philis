# `crates/place` — Philis Placement Engine

The actual analog placement engine. This crate implements the solver, oracles,
and geometry pipeline that fill the signoff-first contract defined in
`crates/api/src/core.rs`.

## Relationship to `crates/api`

`crates/api` defines the contract types (`PlacementCertificate`, `AcceptanceGate`,
`GateEvaluation`, etc.) and a `trait PlacementEngine` that this crate implements.
`crates/api` calls into this crate via dynamic dispatch on that trait. This crate
depends on `crates/api` for the types; there is no circular dependency.

```
crates/api          crates/place
  |                    |
  | defines            | implements
  | PlacementEngine    | PlacementEngine
  | trait + types      | (SA solver, oracles, geometry)
  |                    |
  +--- calls via ----->+
       dyn PlacementEngine
```

## First target

- **PDK:** SkyWater 130nm (sky130A)
- **Solver:** Simulated annealing on a sequence-pair encoding
- **Constraints:** All `IntentClass::Hard` variants (Symmetric, SelfSymmetric,
  SymmetricGroup, Order, Align, GuardRing)
- **DRC oracle:** Non-overlap + minimum spacing from Sky130 rules
- **Output:** Placed coordinates + real GDS geometry + honest certificate

## Shared crate dependencies

This crate depends on three shared crates instead of building those
subsystems internally:

| Shared crate | What it provides to `crates/place` |
|---|---|
| `crates/geom` (`philis-geom`) | `Rect`, `RectiPoly`, orientation transforms, spatial index (R-tree), `GeometryStore` |
| `crates/tech` (`philis-tech`) | `CompiledTech`, `LayerStack`, `ViaTable`, `RuleSet`, `Coverage` classification, `PdkState` |
| `crates/verify` (`philis-verify`) | DRC rule predicates, `DrcViolation` types, LVS connectivity (`ConnectivityTracker`) |

```
crates/api          crates/geom    crates/tech    crates/verify
  |                    |              |              |
  | defines            | geometry     | PDK model    | DRC predicates
  | PlacementEngine    | types &      | & rules      | & violation
  | trait + types      | spatial idx  |              | types
  |                    |              |              |
  +--- calls via ----->+-- crates/place depends on --+
       dyn PlacementEngine
```

## Module map

```
src/
  lib.rs              — crate root, PlacementEngine impl
  pdk/
    mod.rs            — PDK compiler interface (wraps philis_tech::CompiledTech)
    sky130.rs         — Sky130-specific setup (consumes philis_tech)
  primitive/
    mod.rs            — device geometry generation (uses philis_geom types)
    mosfet.rs         — MOSFET primitive (nfet/pfet on sky130)
  solver/
    mod.rs            — solver dispatch
    sequence_pair.rs  — sequence-pair encoding + decode
    annealing.rs      — SA loop, cooling schedule, move generation
    moves.rs          — constraint-aware move operators
  constraint/
    mod.rs            — constraint compilation into the IntentGraph
    symmetric.rs      — Symmetric / SelfSymmetric / SymmetricGroup
    order.rs          — Order (above/below/left/right)
    align.rs          — Align (same-row / same-column)
    guard_ring.rs     — GuardRing enclosure
  oracle/
    mod.rs            — oracle dispatch
    drc.rs            — local DRC (uses philis_verify predicates & types)
    equiv.rs          — device equivalence checking
    access.rs         — terminal access map
    routability.rs    — channel capacity witness
  spatial/
    mod.rs            — spatial index dispatch (uses philis_geom R-tree)
    rtree.rs          — R-tree for query-heavy snapshots
  legalize/
    mod.rs            — group-preserving legalization + repair loop
  geometry/
    mod.rs            — GDS polygon generation from placed devices
```

## Implementation documents

These files describe the implementation in detail:

1. [Architecture](01-architecture.md) — crate boundary, trait, module structure
2. [PDK compiler](02-pdk-compiler.md) — Sky130 technology compilation
3. [Primitives](03-primitives.md) — device geometry generation
4. [Solver](04-sequence-pair.md) — sequence-pair SA
5. [Constraints](05-constraints.md) — hard intent constraint implementation
6. [DRC oracle](06-drc-oracle.md) — local DRC checking
7. [Legalization](07-legalization.md) — group-preserving legalization
8. [Integration](08-integration.md) — wiring into crates/api, certificate filling
