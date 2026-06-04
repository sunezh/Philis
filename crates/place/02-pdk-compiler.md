# PDK Compiler — Sky130 Technology Compilation

> **Shared crate:** The PDK compilation pipeline now lives in `crates/tech`
> (`philis_tech`). The `CompiledTech` struct, `LayerStack`, `ViaTable`,
> `GridTable`, `RuleSet`, `ExtractModel`, `Coverage` classification, and
> `PdkState` are all provided by the shared crate. This module in
> `crates/place` becomes a thin consumer: it calls `philis_tech` to compile
> the PDK and queries the resulting `CompiledTech` for placement-relevant
> rules. The design below describes what the shared crate provides and how
> the placement engine uses it.

## Purpose

Turn the raw `Pdk` struct (loaded by `crates/api/io/pdk`) into a
`CompiledTech` (from `philis_tech`) that every downstream step queries.
This is the most critical boundary in the engine: everything that follows
— primitive generation, DRC, equivalence checking — depends on the
technology model being correct.

## What the compiler produces

### `CompiledTechnology`

| Field | Type | Description |
|-------|------|-------------|
| `grid_nm` | `i64` | Manufacturing grid. Sky130: 5nm. |
| `layers` | `Vec<LayerInfo>` | Layer number, name, role, datatype |
| `spacing_rules` | `Vec<SpacingRule>` | Min spacing between layer pairs |
| `width_rules` | `Vec<WidthRule>` | Min width per layer |
| `enclosure_rules` | `Vec<EnclosureRule>` | Enclosure of one layer by another |
| `well_rules` | `WellRules` | Well/tap spacing, enclosure, same-well distance |
| `orientations` | `Vec<Orientation>` | Legal orientations (all 8 for Sky130) |
| `row_pitch_nm` | `i64` | Default row pitch (from RunConfig or PDK) |
| `site_width_nm` | `i64` | Placement site width |
| `coverage` | `RuleCoverageReport` | What fraction of rules are coded vs. missing |
| `pdk_state` | `PdkState` | Classified readiness (will be `Partial` for Sky130 V1) |

### `LayerInfo`

| Field | Type | Description |
|-------|------|-------------|
| `number` | `u16` | GDS layer number |
| `datatype` | `u16` | GDS datatype |
| `name` | `String` | Human name (e.g., "li1", "met1") |
| `role` | `LayerRole` | Functional role |

### `LayerRole`

```
enum LayerRole {
    Diffusion,     // diff (65/20)
    Poly,          // poly (66/20)
    LocalInterconnect,  // li1 (67/20)
    Metal(u8),     // met1..met5
    Via(u8),       // mcon, via, via2, via3, via4
    NWell,         // nwell (64/20)
    PWell,         // (implicit in sky130 — no explicit pwell layer for most)
    NPlus,         // nsdm (93/44)
    PPlus,         // psdm (94/20)
    TapImplant,    // tap
    Marker,
    Keepout,
}
```

### `SpacingRule`

| Field | Type | Description |
|-------|------|-------------|
| `layer_a` | `u16` | First layer number |
| `layer_b` | `u16` | Second layer (same layer for same-layer spacing) |
| `min_spacing_nm` | `i64` | Minimum edge-to-edge spacing |
| `condition` | `SpacingCondition` | When this rule applies |

`SpacingCondition`:
- `Always` — unconditional
- `SameNet` — same-net spacing (usually tighter)
- `WideWire(threshold_nm)` — applies when one shape is wider than threshold
- `ParallelRunLength(threshold_nm)` — applies when parallel run exceeds threshold

### `EnclosureRule`

| Field | Type | Description |
|-------|------|-------------|
| `inner_layer` | `u16` | The enclosed layer |
| `outer_layer` | `u16` | The enclosing layer |
| `min_enclosure_nm` | `i64` | Minimum enclosure on each side |

## Sky130 concrete values

These are the placement-relevant rules extracted from the Sky130A PDK. Source:
`sky130A/libs.tech/openlane/sky130_fd_sc_hd/tracks.info` and the DRC rule deck.

### Manufacturing grid

5nm. All coordinates must be multiples of 5.

### Key layer mapping

| Layer name | GDS (layer/datatype) | Role |
|------------|---------------------|------|
| nwell | 64/20 | NWell |
| diff | 65/20 | Diffusion |
| poly | 66/20 | Poly |
| licon1 | 66/44 | Contact (diff/poly to li1) |
| li1 | 67/20 | Local interconnect |
| mcon | 67/44 | Contact (li1 to met1) |
| met1 | 68/20 | Metal 1 |
| via | 68/44 | Via (met1 to met2) |
| met2 | 69/20 | Metal 2 |
| via2 | 69/44 | Via (met2 to met3) |
| met3 | 70/20 | Metal 3 |
| via3 | 70/44 | Via (met3 to met4) |
| met4 | 71/20 | Metal 4 |
| via4 | 71/44 | Via (met4 to met5) |
| met5 | 72/20 | Metal 5 |
| nsdm | 93/44 | N+ source/drain implant marker |
| psdm | 94/20 | P+ source/drain implant marker |
| hvtp | 78/44 | High-Vt PMOS marker |
| lvtn | 125/44 | Low-Vt NMOS marker |

### Key spacing rules (placement-relevant subset)

| Rule | Layer(s) | Spacing (nm) | Condition |
|------|----------|-------------|-----------|
| diff.3 | diff-diff | 270 | Always |
| poly.2 | poly-poly | 210 | Always |
| poly.4 | poly-diff (non-gate) | 75 | Always |
| li.3 | li1-li1 | 170 | Always |
| m1.2 | met1-met1 | 140 | Always |
| m2.2 | met2-met2 | 140 | Always |
| nwell.1 | nwell-nwell | 1270 | Same-type |
| nwell.5 | nwell to diff (outside) | 340 | N+ diff to nwell edge |
| dnwell.2 | dnwell-dnwell | 2500 | Always |

### Key width rules

| Rule | Layer | Min width (nm) |
|------|-------|---------------|
| diff.1 | diff | 150 |
| poly.1a | poly | 150 |
| li.1 | li1 | 170 |
| m1.1 | met1 | 140 |
| m2.1 | met2 | 140 |

### Key enclosure rules

| Rule | Inner | Outer | Enclosure (nm) |
|------|-------|-------|----------------|
| licon.5a | licon1 | diff | 60 (one side), 40 (other) |
| licon.8a | licon1 | li1 | 80 |
| ct.2 | mcon | li1 | 0 (coincident allowed) |
| ct.4 | mcon | met1 | 30 (one side), 60 (other) |
| nwell.4 | psdm (in nwell) | nwell | 180 |

## Compilation strategy

### Phase 1: Layer extraction

Read the `Pdk` struct's `layers` field. Map each layer name to a `LayerRole`
using a hardcoded Sky130 name table. Unknown layers get `Marker` role and a
warning.

### Phase 2: Rule extraction

The current `Pdk` reader is a tolerant key scanner. For V1, the spacing,
width, and enclosure rules are **hardcoded in `sky130.rs`** with the values
above. This is explicitly `Coverage::Partial` — we're honest that the rules
are not parsed from the PDK but manually transcribed.

The coverage report for V1:
- Spacing rules: ~15 coded out of ~200 total -> `Coverage::Partial`
- Width rules: ~10 coded -> `Coverage::Partial`
- Enclosure rules: ~10 coded -> `Coverage::Partial`
- Well/tap rules: ~5 coded -> `Coverage::Partial`
- Antenna rules: 0 coded -> `Coverage::Missing`
- Density rules: 0 coded -> `Coverage::Missing`
- LDE/WPE: 0 coded -> `Coverage::Missing`

`pdk_state` will be `PdkState::SparseResearch` until the coverage widens.

### Phase 3: Validation

Check for internal consistency:
- Every referenced layer exists in the layer table
- No negative spacings
- Manufacturing grid divides all rule values
- Required layers (diff, poly, li1, met1, nwell) are present

Inconsistencies produce `Coverage::Contradictory` and promote `pdk_state` to
`ContradictoryOrMissing`.

## Future: parsing real rule decks

The hardcoded-rules approach is the bootstrapping path. The long-term path is:
1. Parse the Calibre/Klayout DRC rule deck (the `.lydrc` or `.drc` file)
2. Extract rule predicates symbolically
3. Map them onto `SpacingRule`/`WidthRule`/`EnclosureRule`

This is a large project (Calibre DRC files are procedural programs, not
declarative rule sets). The honest coverage model means we can ship with
partial rules and the certificate says exactly what's covered.

## `CompiledTech` as the oracle substrate

Every oracle module receives `&philis_tech::CompiledTech`. It is immutable
after compilation. If the PDK changes, the entire placement must re-run
(this is the "whole certificate invalidation" from the self-adjusting
computation model in the algorithms doc). The `CompiledTech` type, its
construction, and its coverage classification are all owned by
`crates/tech` — the placement crate is a read-only consumer.
