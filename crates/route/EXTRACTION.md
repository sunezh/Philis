# Subsystem: In-Loop Extraction and Electrical Checking

> **Shared crate:** The PEX surrogate model, EM/IR checking types, and
> antenna checking types come from `crates/verify` (`philis_verify`). The
> route crate calls the shared PEX surrogate API with route-specific
> geometry and queries the shared EM/antenna models for violation checking.
> The incremental update loop (on commit/ripup) stays in the route crate.

**Module:** `crates/route/src/verify/extract.rs`, `verify/em.rs`

**Purpose:** Estimate parasitics (R, C, L), check electromigration limits, and
score antenna risk incrementally during routing, so the repair loop can fix
electrical violations before they accumulate into unfixable topology problems.

**Phase:** 3 (PEX surrogate), 3 (EM/IR/antenna)

---

## 1. What "in-loop extraction" means

Traditional flow: route everything, then extract parasitics, then discover that
a critical net has too much resistance or a power trunk will fail EM. By then
the topology is committed and the fix is disruptive.

Signoff-first flow: estimate parasitics after each commit. When an estimate
exceeds a threshold, the net enters the repair scope *immediately*. The repair
loop can reroute on a wider wire, add vias, reduce coupling, or change layers
before the rest of the layout freezes.

The in-loop extractor is a *calibrated surrogate*, not a field solver. It uses
analytical formulas with parameters from the technology model. The certificate
records the surrogate's accuracy class and, where a signoff extractor is
available, the delta between the two.

---

## 2. PEX surrogate (`verify/extract.rs`)

### What it computes per net

```
NetParasitics {
    net: NetId,
    total_r_ohm: f64,              // total wire resistance
    total_cg_ff: f64,              // total ground capacitance
    total_cc_ff: f64,              // total coupling capacitance to neighbors
    total_rvia_ohm: f64,           // total via resistance
    per_segment: Vec<SegmentParasitics>,
}

SegmentParasitics {
    segment: SegmentId,
    layer: LayerId,
    length_nm: i64,
    width_nm: i64,
    r_ohm: f64,
    cg_ff: f64,
    cc_ff: f64,                    // coupling to adjacent shapes
    coupling_neighbors: Vec<(NetId, f64)>,  // who and how much
}
```

### Resistance

For a wire segment:
```
R = (length / width) * sheet_resistance
```

`sheet_resistance` comes from the tech model (ohm/square per layer). For a
via, `R = via_resistance` from the via definition.

Total net resistance: sum of all segment R plus all via R.

### Ground capacitance

For a wire segment:
```
Cg = length * width * area_cap + 2 * length * fringe_cap
```

`area_cap` and `fringe_cap` come from the tech model (per layer). The first
term is the parallel-plate capacitance to the substrate/ground planes; the
second is the fringe field at the wire edges.

### Coupling capacitance

For two parallel wire segments on the same layer, separated by distance s,
with parallel run length prl:
```
Cc = prl * coupling_cap_per_um(layer, s)
```

The `coupling_cap_per_um` is interpolated from the tech model's coupling
capacitance table (a function of spacing, width, and layer).

For segments on adjacent layers (vertical coupling):
```
Cc = overlap_area * interlayer_cap_per_um2(layer1, layer2)
```

### Incremental update

**On commit(net):** compute `NetParasitics` for the new segments. For coupling,
query the spatial index for nearby shapes on the same and adjacent layers,
compute prl, and estimate Cc. Update the neighboring nets' coupling too (their
Cc to the new net changes).

**On ripup(net):** remove the net's parasitic contribution. Update neighbors'
coupling (their Cc decreases).

**Cost:** O(segments_committed * neighbors_per_segment). For a 10-segment net
with ~5 neighbors per segment: ~50 coupling computations. Each is O(1)
(arithmetic on precomputed parameters). Negligible.

### Accuracy

The surrogate is a lumped-element model with simple analytical formulas. Its
accuracy depends on:
- Technology model quality (are the extraction parameters calibrated?)
- Geometry simplification (wires are treated as ideal rectangles; corner
  effects, proximity effects, and non-uniform current distribution are
  ignored)
- Coupling window truncation (only shapes within `coupling_window_nm` are
  considered)

For routing-level decisions (is this net too resistive? is coupling too high?),
the surrogate is typically within 20-30% of a signoff extractor. This is good
enough to catch gross problems and guide optimization.

The certificate records: "parasitic estimates are from the routing surrogate
at corner X with accuracy class Approximate. Delta to signoff extractor: not
available (or: 15% on net Y)."

---

## 3. Feeding PEX into the repair loop

### Soft PEX damage objective

The formulation's `Phi_pex`:
```
Phi_pex = sum_n alpha_n * R_n + sum_n beta_n * Cg_n + sum_(n,m) gamma_nm * Cc_nm + ...
```

In practice, this is evaluated per net. Weights come from the net class:
- `Critical`: high alpha (resistance matters), moderate beta
- `Sensitive`: high gamma (coupling matters), moderate beta
- `DifferentialPair`: high gamma for asymmetry, moderate alpha
- `Ordinary`: low weights (wirelength proxy is sufficient)
- `RfMmwave`: all weights high, plus inductive terms

### PEX outlier detection

After computing parasitics for a committed net, check:
1. Is R above the threshold for this net class?
2. Is Cc above the threshold?
3. For a matched group, is the parasitic mismatch above the threshold?

If yes, emit a `RouteDiagnostic` with `DiagnosticClass::PexBound` and
`DiagnosticScope::LocalRepairable`. The violated net enters the repair scope.

### Repair strategies

The repair loop can:
- **Widen the wire:** reduce R at the cost of more area and potentially more
  coupling. Effective when R is the problem.
- **Add vias:** reduce via-chain resistance. Effective when the path has many
  vias.
- **Reroute on a different layer:** some layers have lower sheet R or less
  coupling. Effective when the current layer is congested or resistive.
- **Increase spacing to aggressor:** reduce Cc. Effective when coupling is
  the problem.
- **Add shielding:** insert a shield track between the victim and aggressor.
  The nuclear option for coupling.

The repair doesn't need to know which strategy to use — it just rips up the
net and re-routes with an updated cost function where the problematic metric
has higher weight.

---

## 4. Electromigration checking (`verify/em.rs`)

### Black's equation

```
MTTF = (A / J^n) * exp(E_a / (k_B * T))
J = I / (width * thickness)
EM_margin = J_max / J - 1
```

For each segment carrying current I:
1. Compute J from the wire width and layer thickness.
2. Check against J_max (from the tech model's EM rules for this layer).
3. If J > J_max, the segment fails EM.

### Blech filter

Before reporting an EM failure, check the Blech criterion:
```
if J * length < (J*L)_crit:
    segment is EM-immortal, skip
```

Short wire segments below the Blech length are immune to EM regardless of
current density. This avoids over-sizing short connections.

### Current sources

EM checking requires knowing which nets carry current and how much. Sources:
- `Constraint::ChargeFlow(net)` — the user declares a net carries significant
  current
- Power/ground nets — assumed to carry the circuit's total current, distributed
  across the supply network
- Device drain/source terminals — current flows through transistor channels

In Phase 3, the current map is a simple declaration: the user specifies which
nets carry how much current. Automatic current estimation from the circuit
(e.g., static analysis of the bias point) is beyond the router's scope —
it requires a circuit simulator.

### EM violations and repair

EM failure on a segment seeds `scope(V)` for re-sizing:
- Widen the wire (reduce J)
- Add via arrays (reduce J at via transitions)
- Change the route topology (shorter path on the same layer, or route on a
  thicker metal layer)

If the constraint promotes EM to hard (`RouteQuality.em_margin_min < 0` and
the constraint says EM is a gate), the failure appears in
`HardVector.unresolved_em_antenna_hard_errors`.

### IR drop

For power/ground nets, static IR drop is the voltage loss along the supply
network:
```
IR_drop(load) = sum of (I_segment * R_segment) from source to load
```

The worst-case IR drop across all loads is
`RouteQuality.max_ir_drop_mv`.

Computing this requires a resistive network model of the power grid:
1. Build a resistor network from the committed power segments.
2. Set current sources at each load (device, standard cell).
3. Solve the linear system (sparse matrix solve) for node voltages.
4. The drop at each load is `V_source - V_load`.

For analog (small networks), a direct sparse solve is fast. For large digital
power grids, iterative methods (multigrid, preconditioned conjugate gradient)
are needed — but that's not Philis's target.

---

## 5. Antenna checking

### The rule

During fabrication, each metal layer is built sequentially. A floating gate
oxide connected to a long metal antenna can accumulate charge from the plasma
etch process, potentially damaging the oxide. The antenna rule limits the
ratio:
```
antenna_ratio = metal_area_connected_to_gate / gate_oxide_area
```

If the ratio exceeds the limit, a protection diode must be inserted, or the
routing must be changed to break the antenna (route down to a lower layer,
then back up).

### Not delta-safe

Antenna accumulates along the full net from the gate terminal to the highest
routed layer. Adding a segment on M3 changes the antenna ratio for a gate on
M1 that is connected through M1-M2-M3. This is not local — the entire net
must be checked.

### Implementation

After a net is fully committed (all segments placed):
1. For each gate terminal on the net, compute the total metal area on each
   layer between the gate layer and the highest layer the net uses.
2. Compute the antenna ratio.
3. If above the limit, report `DiagnosticClass::AntennaOrEm`.

Repair: reroute to break the antenna chain (add a via down to a lower layer
and back up, so the metal area accumulation resets) or insert a diode.

---

## 6. Inductive extraction (Phase 3+ / RF only)

For RF/mm-wave nets, the lumped RC model is insufficient. The RLGC model adds:
- Series inductance L per unit length
- Shunt conductance G per unit length
- Return-path reasoning (the loop inductance depends on the forward+return
  current path)

This is beyond the simple analytical surrogate. Options:
1. **Tabulated RLGC:** the tech model provides per-layer, per-width RLGC
   parameters. The surrogate uses them directly. Accuracy depends on the
   table's coverage.
2. **PEEC (Partial Element Equivalent Circuit):** discretize the conductor,
   compute partial inductances. More accurate but expensive.
3. **External field solver:** call a commercial extractor for RF nets.

For Phase 3, option 1 (tabulated RLGC) is the right balance. PEEC and
external solvers are future work for production RF routing.

---

## 7. Integration with the certificate

The extraction subsystem populates:
```
RouteQuality {
    total_r_ohm:             sum of all nets' resistance
    total_c_ff:              sum of all nets' ground capacitance
    total_coupling_ff:       sum of all coupling capacitance
    via_count:               total vias
    max_ir_drop_mv:          worst-case power grid IR drop
    em_margin_min:           worst-case EM margin across all declared-current segments
    max_crosstalk_noise_mv:  estimated peak coupling noise
    max_length_match_delta_nm: from matched group measurement
    max_diff_pair_skew_nm:   from diff pair measurement
}
```

The certificate records:
- Which extraction model was used (surrogate vs. signoff)
- Which corner was evaluated
- Whether EM/IR/antenna were checked or skipped
- The coverage class of the extraction parameters

If the tech model lacks extraction parameters, all electrical fields are zeroed
and `degraded_confidence = true`. The certificate honestly says: "no parasitic
information is available for this technology."
