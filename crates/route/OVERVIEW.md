# Route Crate — Implementation Overview

This directory contains the implementation plan for the Philis routing engine.
The design docs in `docs/Developer/route/` define the *what* — the signoff-first
formulation, the lexicographic objective, the certificate contract. These files
define the *how* — concrete architecture, data structures, algorithms, module
layout, and build order.

## Relationship to the formulation

The formulation docs are the north star. Nothing here contradicts them. But the
formulation is a specification of the *finished* router; these docs are a plan
for building it incrementally so that every intermediate state is useful,
testable, and honest (emits `degraded_confidence` for anything unproven).

The typed contract — `HardVector`, `RouteCertificate`, `RouteQuality`,
`RouteDiagnostic`, `Coverage`, `PdkState`, `NetClass`, `AccessConfidence`,
`Evidence` — already exists in `crates/api/src/core.rs` and is unit-tested.
The route crate's job is to fill those types with *proven contents* instead of
stub defaults.

## Shared crate dependencies

The route crate depends on four shared crates rather than implementing
these subsystems internally:

| Shared crate | What it provides to `crates/route` |
|---|---|
| `crates/geom` (`philis-geom`) | `Rect`, `RectiPoly`, spatial index (R-tree), boolean ops, `GeometryStore` |
| `crates/tech` (`philis-tech`) | `CompiledTech`, `LayerStack`, `ViaTable`, `GridTable`, `RuleSet`, `ExtractModel`, `PdkState` |
| `crates/verify` (`philis-verify`) | DRC rule predicates, `DrcViolation` types, LVS connectivity (union-find `ConnectivityTracker`), PEX surrogate, EM/IR/antenna checking |
| `crates/solver` (`philis-solver`) | `SatSolver` trait, `IlpSolver` trait, MUS/MCS extraction, convenience builders (at-most-one, etc.) |

## Module layout

```
crates/route/src/
  lib.rs              -- public API: re-exports, the top-level `route()` entry point
  graph/
    mod.rs            -- resource graph G_R = (Q, E_R)
    grid.rs           -- 3D grid graph construction from tech + placement
    edge.rs           -- edge attributes: attr(e)
    gcell.rs          -- global routing cells and capacity
    builder.rs        -- graph construction from tech + placement + obstacles
  access/
    mod.rs            -- terminal access oracle
    candidate.rs      -- access point generation and ranking
    conflict.rs       -- access conflict graph
    select.rs         -- compatible subset selection
  engine/
    mod.rs            -- engine dispatcher (picks engine per problem shape)
    maze.rs           -- A* / maze router (engine 3)
    negotiate.rs      -- negotiated congestion / PathFinder (engine 2)
    exact.rs          -- ILP/SMT exact-local engine (engine 1)
    alt.rs            -- ALT landmark heuristic for A*
  group/
    mod.rs            -- group routing coordinator
    diffpair.rs       -- differential pair routing
    symmetric.rs      -- self-symmetric and cross-symmetric nets
    matched.rs        -- matched arrays, common-centroid escapes
    shield.rs         -- shield reservation and routing
  verify/
    mod.rs            -- in-loop verification coordinator
    drc.rs            -- incremental DRC delta
    lvs.rs            -- connectivity extraction (union-find)
    extract.rs        -- in-loop PEX surrogate
    em.rs             -- EM/IR/antenna checking
  state/
    mod.rs            -- route state: the mutable world during search
    commit.rs         -- transactional commit/ripup
    history.rs        -- negotiated congestion history costs
    ownership.rs      -- net-to-edge ownership map
    uf.rs             -- union-find for connectivity invariant
  certificate/
    mod.rs            -- certificate assembly from verified route state
    evidence.rs       -- evidence collection (internal proof, external hash)
    diagnosis.rs      -- failure classification and infeasible core minimization
```

## Phase structure

The route crate is built in four phases. Each phase produces a working (if
limited) router that populates the existing `core.rs` contract honestly.

| Phase | Deliverable | Unlocks |
|-------|-------------|---------|
| [Phase 0](PHASE-0-FOUNDATION.md) | `tech/` + `geom/` modules | PDK compilation, geometry queries, spatial indexing — the floor everything stands on |
| [Phase 1](PHASE-1-MVP-ROUTER.md) | `graph/` + `engine/maze.rs` + `state/` + basic `verify/` | A working single-net-at-a-time maze router that produces DRC-checked geometry and a non-stub certificate |
| [Phase 2](PHASE-2-ANALOG-QUALITY.md) | `engine/negotiate.rs` + `group/` + incremental `verify/drc.rs` | Negotiated congestion, diff-pair routing, rip-up-and-reroute, in-loop DRC — an analog-quality router |
| [Phase 3](PHASE-3-FULL-FORMULATION.md) | `access/` + `engine/exact.rs` + `verify/extract.rs` + `verify/em.rs` + `certificate/` | Pin access oracle, exact-local engine, PEX surrogate, EM/IR, full certificate — the complete formulation |

## Subsystem docs

Each subsystem has its own design doc:

| Doc | Covers | Shared crate |
|-----|--------|-------------|
| [Geometry Kernel](GEOMETRY-KERNEL.md) | Shapes, boolean ops, spatial index, corner stitching decision | Now in `crates/geom` |
| [Technology Compiler](TECHNOLOGY-COMPILER.md) | PDK ingestion, layer/track/via model, rule oracle, extraction params | Now in `crates/tech` |
| [Resource Graph](RESOURCE-GRAPH.md) | 3D grid graph, edge attributes, global cells, graph construction | Route-specific (uses `philis_geom` types) |
| [Access Oracle](ACCESS-ORACLE.md) | Pin access candidates, conflict graph, compatible subset selection | Route-specific (uses `philis_solver` for SAT) |
| [Search Engines](SEARCH-ENGINES.md) | A*/maze, negotiated congestion, exact-local ILP/SMT, ALT heuristic | Route-specific (exact engine uses `philis_solver`) |
| [Group Router](GROUP-ROUTER.md) | Diff pair, symmetric, matched array, shield, guard ring | Route-specific |
| [DRC Engine](DRC-ENGINE.md) | Incremental DRC delta, bounded influence, rule predicate dispatch | Predicates/types from `crates/verify` |
| [Extraction](EXTRACTION.md) | In-loop PEX surrogate, EM/IR/antenna, return-path reasoning | PEX/EM types from `crates/verify` |
| [Route State](ROUTE-STATE.md) | Commit/ripup, ownership, union-find, history costs, determinism | Union-find from `crates/verify` |

## Key architectural decisions

### 1. The route crate is a library, not a binary

It exports a `route(problem: &RouteProblem) -> RoutingResult` function that the
`crates/api` facade calls. It does not own the CLI, Python bindings, or flow
orchestration — those stay in `crates/api`.

### 2. The contract types stay in `crates/api/src/core.rs`

`HardVector`, `RouteCertificate`, `RouteQuality`, etc. are the public contract.
The route crate depends on `crates/api` (or on a shared `crates/types` crate if
the dependency direction becomes awkward) and fills those types. It does not
duplicate them.

### 3. The geometry kernel is `crates/geom` (done)

The geometry kernel has been extracted into the shared `crates/geom`
(`philis-geom`) crate. Both the placement and routing crates depend on it.
The `tech/` and `verify/` subsystems similarly live in `crates/tech` and
`crates/verify`, and the solver abstraction lives in `crates/solver`.

### 4. No premature optimization

Start with simple, correct data structures (R-tree, hash maps, vectors). Profile
before replacing with specialized structures (corner stitching, arena
allocation). The formulation's performance claims (bounded-influence O(m) DRC
delta, near-constant union-find) are targets, not starting points.

### 5. Determinism from day one

Every data structure uses sorted/deterministic iteration (BTreeMap over HashMap,
sorted edge lists, fixed net ordering). Randomized net-order perturbation
records its seed. This is non-negotiable — the certificate's replay guarantee
depends on it.
