# Primitives — Device Geometry Generation

> **Shared crate:** Geometry types (`Rect`, `RectiPoly`) and orientation
> transforms come from `crates/geom` (`philis_geom`). The `Rect` used in
> `Primitive.bbox` and `LayeredRect`, as well as the eight orientation
> transforms (R0 through MYR90), are imported from `philis_geom` rather
> than defined locally.

## Purpose

Given a SPICE device card and the compiled technology, produce the polygon set
and terminal map for one placed device at origin. The solver places devices by
translating and optionally mirroring/rotating these origin-centered shapes.

## What a primitive is

A `Primitive` is the geometry template for one device instance before placement.
It is generated once per unique `(model, W, L, nf, mult)` tuple and cached.

### `Primitive`

| Field | Type | Description |
|-------|------|-------------|
| `id` | `String` | Unique key: `"{model}_W{w}_L{l}_NF{nf}"` |
| `model` | `String` | SPICE model name (e.g., `sky130_fd_pr__nfet_01v8`) |
| `bbox` | `Rect` | Bounding box at origin (0,0) to (width, height) |
| `polygons` | `Vec<LayeredRect>` | All geometry polygons |
| `terminals` | `Vec<Terminal>` | Named pins with shapes and layers |
| `params` | `DeviceParams` | Electrical parameters (W, L, nf, mult) |

### `LayeredRect`

| Field | Type | Description |
|-------|------|-------------|
| `layer` | `u16` | GDS layer number |
| `datatype` | `u16` | GDS datatype |
| `x0` | `i64` | Left edge (nm) |
| `y0` | `i64` | Bottom edge (nm) |
| `x1` | `i64` | Right edge (nm) |
| `y1` | `i64` | Top edge (nm) |

### `Terminal`

| Field | Type | Description |
|-------|------|-------------|
| `name` | `String` | Port name: "G", "D", "S", "B" for MOSFET |
| `shapes` | `Vec<LayeredRect>` | Pin shapes (where the router can connect) |
| `access_layer` | `u16` | Preferred access layer (typically li1 or met1) |

### `DeviceParams`

| Field | Type | Description |
|-------|------|-------------|
| `w_nm` | `i64` | Channel width |
| `l_nm` | `i64` | Channel length |
| `nf` | `u32` | Number of fingers |
| `mult` | `u32` | Multiplier |

## MOSFET primitive generation (Sky130)

The first target is the basic `sky130_fd_pr__nfet_01v8` and
`sky130_fd_pr__pfet_01v8` devices. The generation follows the foundry's standard
cell geometry rules.

### Cross-section of a single-finger NFET

```
                        Gate (poly)
                           |
                     +-----+-----+
                     |     |     |
         +-----------+     |     +-----------+
         |  Source         |          Drain   |
         |  (diff/nsdm)   |    (diff/nsdm)   |
         |  +licon1       |    +licon1        |
         |  +li1 pin      |    +li1 pin       |
         +-----------+     |     +-----------+
                     |     |     |
                     +-----+-----+
                           |
                      li1 gate pin
```

### Layer stack (bottom-up)

1. **Well/Implant:** PFET gets nwell; NFET is in substrate (no explicit pwell
   in most Sky130 variants). Both get source/drain implant markers (nsdm/psdm).
2. **Diffusion (diff):** The active area containing source, channel, drain.
3. **Poly:** The gate crossing over diffusion. Extends beyond diff by the
   poly overhang rule.
4. **Contacts (licon1):** Diff-to-li1 contacts in source and drain regions.
   Poly-to-li1 contact for gate.
5. **Local interconnect (li1):** Runs over contacts, forms the terminal pins.
6. **Metal contacts (mcon):** li1-to-met1 contacts (generated but optional
   at placement time — the router decides met1 access).

### Single-finger MOSFET geometry algorithm

Input: `model`, `W` (width), `L` (length), technology rules.

All dimensions in nm. All coordinates snapped to `grid_nm` (5nm for Sky130).

```
// Constants from Sky130 rules
POLY_OVERHANG     = 130     // poly extends beyond diff on each side
DIFF_POLY_ENC     = 130     // diff extends beyond poly edge (S/D extension)
CONTACT_SIZE      = 170     // licon1 is 170x170
CONTACT_PITCH     = 340     // min contact pitch
CONTACT_DIFF_ENC  = 60      // diff encloses licon on one axis
CONTACT_LI_ENC   = 80       // li1 encloses licon
GATE_CONTACT_SEP  = 55      // poly-to-contact spacing
NSDM_ENC          = 125     // nsdm encloses diff
NWELL_ENC         = 180     // nwell encloses psdm (for PFET)
```

Step-by-step:

1. **Diffusion:** Height = W. Width = L + 2 * DIFF_POLY_ENC +
   2 * (CONTACT_SIZE + CONTACT_DIFF_ENC). Centered at origin.

2. **Poly:** Height = W + 2 * POLY_OVERHANG. Width = L. Centered
   horizontally on the diffusion.

3. **Source contacts:** Column of licon1 squares in the source region
   (left of poly). Number of contacts = floor((W - 2 * CONTACT_DIFF_ENC) /
   CONTACT_PITCH) + 1, minimum 1. Spaced evenly.

4. **Drain contacts:** Same as source, on the right side of poly.

5. **Gate contact:** One licon1 square above or below the diffusion,
   on the poly extension. Placed GATE_CONTACT_SEP from the diff edge.

6. **li1 pins (terminals):**
   - Source pin: li1 rectangle covering all source contacts + CONTACT_LI_ENC
   - Drain pin: li1 rectangle covering all drain contacts + CONTACT_LI_ENC
   - Gate pin: li1 rectangle covering the gate contact + CONTACT_LI_ENC
   - Bulk pin: for NFET, a substrate tap (generated separately or as part of
     guard ring). For PFET, an nwell tap.

7. **Implant markers:**
   - NFET: nsdm rectangle enclosing the diffusion by NSDM_ENC
   - PFET: psdm rectangle enclosing the diffusion by NSDM_ENC

8. **Well (PFET only):** nwell rectangle enclosing the psdm region by NWELL_ENC.

9. **Snap all coordinates** to the manufacturing grid.

10. **Compute bounding box** as the union of all rectangles.

### Multi-finger MOSFET

For `nf > 1`, the device has alternating source/drain diffusion regions sharing
poly gates:

```
  S | G | D | G | S | G | D    (nf=3, odd fingers)
```

The diffusion is one continuous rectangle. Poly gates are spaced at
`poly_pitch = L + 2 * GATE_CONTACT_SEP + CONTACT_SIZE`. Source and drain
contacts are placed in alternating inter-gate slots. The terminal map
merges all source contacts into one "S" terminal and all drain contacts into
one "D" terminal.

For common-centroid and interdigitated layouts, the primitive generator
produces the individual finger geometry and the solver/constraint system
handles the interleaving pattern.

## Primitive catalog

The `PrimitiveCatalog` is built during the "primitive catalog" phase of the
pipeline. It maps `(model, W, L, nf)` tuples to `Primitive` instances.

### Construction

1. Walk the `SpiceNetlist` device list
2. For each device, extract `(model, W, L, nf, mult)` from the device card
3. If the tuple is not in the catalog, generate the primitive
4. Store the primitive in a `HashMap<String, Primitive>` keyed by `id`

### Equivalence

Two devices are **equivalent** for placement purposes if:
- Same model name
- Same W, L, nf, mult (after parameter evaluation)
- Same terminal order

The device-equivalence oracle later checks this formally, but the catalog
pre-groups equivalent devices to avoid redundant generation.

## Orientation transforms

When a device is placed with a non-identity orientation, every polygon in its
primitive is transformed:

| Orientation | Transform on (x, y) relative to bbox center |
|-------------|---------------------------------------------|
| R0 | (x, y) |
| R90 | (-y, x) |
| R180 | (-x, -y) |
| R270 | (y, -x) |
| MX | (x, -y) |
| MY | (-x, y) |
| MXR90 | (y, x) |
| MYR90 | (-y, -x) |

The transform is applied at emit time (when generating the final GDS), not
during SA — the solver works with bounding boxes and the orientation label.

## What is NOT generated at primitive time

- **Dummy devices:** Generated by the guard-ring constraint handler, not the
  primitive generator. They use the same geometry template but are marked as
  dummies in the placement output.
- **Substrate/well taps:** Generated as part of guard-ring enclosure or as
  standalone tap primitives. A tap primitive is a contact stack
  (diff + licon + li1) in a well region with opposite-type implant.
- **Routing geometry:** The router owns metal wiring. The primitive only provides
  terminal pin shapes on li1 (and optionally mcon + met1 stubs).
