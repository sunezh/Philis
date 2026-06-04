# Subsystem: Technology Compiler

> **Shared crate:** This subsystem now lives in `crates/tech`
> (`philis_tech`). The technology compiler, `CompiledTech`, `LayerStack`,
> `ViaTable`, `GridTable`, `RuleSet`, `ExtractModel`, coverage
> classification, and `PdkState` derivation are all provided by the shared
> crate. Both the placement and routing engines depend on `philis_tech`.
> This document describes what the shared crate provides. The route crate
> imports and queries `philis_tech::CompiledTech` rather than implementing
> compilation internally.

**Crate:** `crates/tech` (`philis-tech`)

**Purpose:** Compile a PDK's process data into the formulation's typed
technology model `T = (L, G, V, R, E, X, M)` — the immutable, read-only
oracle that every downstream step queries. Classify rule coverage and derive
the overall `PdkState`.

---

## 1. The problem

A routing engine needs to know the process rules to generate legal geometry.
These rules come from the foundry's PDK, which is distributed in proprietary
formats (Calibre SVRF, Synopsys TechFile, LEF/DEF, Liberty, extraction tech
files). Philis cannot parse all of them — and shouldn't try.

Instead, the technology compiler defines a **Philis-native technology schema**
(JSON or TOML) that captures the routing-relevant subset of the PDK. Separate
importers translate from external formats into this schema. The compiler reads
the schema and builds the in-memory `Tech` structure.

This two-stage pipeline (import -> compile) means:
- The compiler never depends on external format details
- New PDK formats are supported by writing a new importer, not touching the
  compiler
- Users can hand-write or hand-edit the schema for unsupported PDKs
- The schema is version-controlled and diffable

---

## 2. The Philis technology schema

### Schema structure (JSON)

```json
{
  "name": "sky130",
  "node_family": "130nm planar CMOS",
  "db_unit_nm": 1,
  "layers": [
    {
      "name": "li1",
      "gds_layer": 67,
      "gds_datatype": 20,
      "class": "local_interconnect",
      "direction": "any",
      "pitch_nm": 460,
      "min_width_nm": 170,
      "offset_nm": 230,
      "sheet_resistance_ohm_sq": 12.8,
      "thickness_nm": 100
    },
    {
      "name": "met1",
      "gds_layer": 68,
      "gds_datatype": 20,
      "class": "routing",
      "direction": "horizontal",
      "pitch_nm": 340,
      "min_width_nm": 140,
      "offset_nm": 170,
      "sheet_resistance_ohm_sq": 0.125,
      "thickness_nm": 330
    }
  ],
  "vias": [
    {
      "name": "mcon",
      "bottom": "li1",
      "top": "met1",
      "cut_size": [170, 170],
      "cut_spacing": [190, 190],
      "enclosure_bottom": [60, 30],
      "enclosure_top": [30, 60],
      "resistance_ohm": 9.3
    }
  ],
  "spacing_rules": [
    {
      "name": "met1.S.1",
      "layer": "met1",
      "min_spacing_nm": 140,
      "width_threshold_nm": null,
      "prl_threshold_nm": null,
      "coverage": "coded"
    },
    {
      "name": "met1.S.2",
      "layer": "met1",
      "min_spacing_nm": 280,
      "width_threshold_nm": 3000,
      "prl_threshold_nm": null,
      "coverage": "coded",
      "note": "wide metal spacing"
    }
  ],
  "width_rules": [
    {
      "name": "met1.W.1",
      "layer": "met1",
      "min_width_nm": 140,
      "max_width_nm": null,
      "coverage": "coded"
    }
  ],
  "enclosure_rules": [
    {
      "name": "mcon.E.1",
      "via": "mcon",
      "side": "bottom",
      "min_enclosure_nm": [60, 30],
      "coverage": "coded"
    }
  ],
  "area_rules": [
    {
      "name": "met1.A.1",
      "layer": "met1",
      "min_area_nm2": 27200,
      "coverage": "coded"
    }
  ],
  "extraction": {
    "corners": ["tt", "ss", "ff"],
    "default_corner": "tt",
    "layer_caps": [
      {
        "layer": "met1",
        "area_cap_ff_per_um2": 0.038,
        "fringe_cap_ff_per_um": 0.045,
        "corner": "tt"
      }
    ],
    "coupling_caps": [
      {
        "layer1": "met1",
        "layer2": "met1",
        "coupling_cap_ff_per_um": 0.052,
        "coupling_window_nm": 1000,
        "corner": "tt"
      }
    ]
  },
  "antenna_rules": [],
  "density_rules": [],
  "em_rules": [
    {
      "layer": "met1",
      "j_max_ma_per_um": 1.0,
      "blech_jl_crit": 3000,
      "coverage": "approximate"
    }
  ],
  "multipatterning": null,
  "external_decks": {
    "drc": "sky130A/libs.tech/magic/sky130A.tech",
    "lvs": "sky130A/libs.tech/netgen/sky130A_setup.tcl"
  },
  "notes": [
    "EOL spacing rules: external-only (covered by Magic DRC, not coded here)",
    "Density rules: manual-review"
  ]
}
```

### Required vs. optional fields

**Required (compilation fails without):**
- `name`, `db_unit_nm`
- At least one routing layer with `class: "routing"` or `"local_interconnect"`
- At least one via connecting two routing layers
- Basic spacing and width rules for each routing layer

**Optional (classified as missing/external if absent):**
- Extraction parameters (without them, PEX is `degraded_confidence`)
- Antenna rules (without them, antenna checking is skipped)
- Density rules (without them, density checking is skipped)
- EM rules (without them, EM is `degraded_confidence`)
- Multipatterning (without them, color assignment is skipped)
- External deck paths (informational; the router doesn't call them)

---

## 3. The compiled `Tech`

### Data structures

```
Tech
  name: String
  db_unit_nm: i64
  layers: LayerStack
  vias: ViaTable
  grids: GridTable
  rules: RuleSet
  extract: ExtractModel
  metadata: TechMetadata
  coverage_report: RuleCoverageReport
  pdk_state: PdkState
```

**LayerStack:** ordered from bottom to top, each layer carrying its properties.
Random access by `LayerId` (an index into the vector). Name-to-id lookup via
a BTreeMap.

**ViaTable:** all legal via transitions, indexed by `(bottom_layer, top_layer)`.
Multiple via definitions per layer pair are supported (different cut sizes,
enclosures, or array rules).

**GridTable:** per-layer routing grid. For each routing layer, the set of legal
track positions (pitch + offset), preferred direction, and min/max width.

**RuleSet:** all DRC predicates, organized for efficient lookup:
- By layer: "give me all spacing rules for M1"
- By kind: "give me all enclosure rules"
- By layer pair: "give me all inter-layer rules between M1 and VIA1"
Each predicate carries its `Coverage` classification and `influence_radius_nm`.

**ExtractModel:** per-corner extraction parameters. Sheet resistance per layer,
area and fringe capacitance per layer, coupling capacitance per layer pair
with its coupling window.

### Rule predicate dispatch

The `RuleSet` provides typed query methods:

```
fn min_spacing(&self, layer: LayerId, width: i64) -> i64
fn min_width(&self, layer: LayerId) -> i64
fn max_width(&self, layer: LayerId) -> Option<i64>
fn via_enclosure(&self, via: ViaId, side: ViaSide) -> (i64, i64)
fn min_area(&self, layer: LayerId) -> Option<i64>
fn spacing_with_prl(&self, layer: LayerId, width: i64, prl: i64) -> i64
fn eol_spacing(&self, layer: LayerId, eol_width: i64) -> Option<i64>
```

Each returns the strictest applicable value. Width-dependent spacing works by
finding the rule with the largest `width_threshold_nm` that is <= the query
width. If no rule matches, fall back to the base spacing.

---

## 4. Coverage classification

### Per-predicate classification

During compilation, each rule from the schema is classified into the canonical
`Coverage` vocabulary. The schema's `"coverage"` field maps directly:
- `"coded"` -> `Coverage::Coded`
- `"partial"` -> `Coverage::Partial`
- `"approximate"` -> `Coverage::Approximate`
- `"external_only"` -> `Coverage::ExternalOnly`
- `"manual_review"` -> `Coverage::ManualReview`

Rules known to exist for the node but absent from the schema are classified
`Coverage::Unsupported` or `Coverage::Missing` based on whether they affect
routing (spacing, width, via rules are required; density, antenna are optional).

### PdkState derivation

After classifying all predicates, the compiler derives the overall PdkState:

```
if any required rule is Missing or Contradictory:
    PdkState::ContradictoryOrMissing
else if any rule is Unsupported or ManualReview:
    if most required rules are at least Partial:
        PdkState::AbstractComplete
    else:
        PdkState::SparseResearch
else if all required rules are Coded:
    PdkState::FullSignoff
else:
    PdkState::AbstractComplete
```

"Required" means: basic spacing, basic width, via enclosure, and connectivity
extraction for each routing layer. These are the rules without which the router
cannot produce legal geometry.

### The RuleCoverageReport

Filled by counting predicates per Coverage bucket:

```
RuleCoverageReport {
    coded:           count of Coded predicates,
    partially_coded: count of Partial predicates,
    ...
}
```

This travels with the certificate so the consumer knows exactly what was proven
and what wasn't.

---

## 5. Importers

### LEF importer

Reads Technology LEF (not Library LEF) and extracts:
- Layer definitions: name, type, direction, pitch, width, spacing
- Via definitions: layers, cut geometry, enclosures
- Site definitions (for placement, not routing)

Does NOT extract: parasitic models, antenna rules, density rules (these come
from other files or are hand-specified in the schema).

The LEF importer writes the Philis schema as JSON. The user can then edit the
JSON to add extraction parameters, EM rules, and coverage annotations before
compilation.

### SKY130 importer

A targeted importer that reads SKY130's specific file structure:
- `sky130A/libs.tech/openlane/` for layer/via definitions
- `sky130A/libs.ref/sky130_fd_sc_hd/` for standard cell geometry
- Hand-coded extraction parameters from published data

This is more complete than the generic LEF importer because SKY130 is a
primary target and its structure is known.

### Hand-authored schemas

For PDKs without an importer, users write the JSON schema directly. The
compiler validates it and reports missing required fields. Coverage annotations
let the user honestly declare what they know and don't know about the process.

---

## 6. Validation and error reporting

The compiler validates structural invariants:
- Every via connects two layers that exist in the stack
- Via layers are adjacent in the stack (no skipping)
- Layer pitches are positive and consistent with min widths
- Spacing rules don't contradict (a wide-metal rule is not less than the base)
- Extraction parameters are non-negative
- At least one legal routing layer and one via exist

Validation errors are typed (not string messages) so the caller can
programmatically distinguish "missing layer" from "contradictory rule."

---

## 7. Hashing

The compiled `Tech` is hashable. The hash covers:
- All layer, via, grid, and rule data
- The coverage classification
- The PdkState

This hash goes into the `RouteCertificate.pdk_oracle_hash` field, so the
certificate can prove it was generated against a specific technology version.
Any change to the tech data produces a different hash, invalidating cached
routing results.

---

## 8. Open questions

### Should the schema support conditional rules?

Some PDK rules are conditional: "if metal width > X AND parallel run length >
Y, then spacing = Z." The current schema supports width-dependent and PRL-
dependent spacing as separate fields. More complex conditions (multi-variable,
context-dependent) would need an expression language in the schema.

**Recommendation:** don't add an expression language yet. Model the common cases
(width-dependent, PRL-dependent) as structured fields. Classify everything else
as `Partial` or `ExternalOnly` and let the external DRC deck handle it.

### Should extraction parameters be per-corner?

The schema supports multiple corners (tt, ss, ff). The extraction model should
be per-corner, with a default corner for in-loop estimates. Full multi-corner
analysis is a signoff feature, not an in-loop feature.

**Recommendation:** store per-corner parameters in the schema. The in-loop PEX
surrogate uses the default corner. The certificate records which corner was
used, so the consumer can re-evaluate at other corners if needed.

### How do we handle rule updates?

When a PDK revision changes a spacing rule, the tech schema changes and the
hash changes. Any cached routing results are invalidated. The user re-runs
the router. There is no incremental re-compilation or partial re-routing
based on rule deltas — the compiled Tech is immutable and atomic.
