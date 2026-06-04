# Phase 3 — Fact Extraction

**Goal:** Build a `CanonicalFactGraph` from a SPICE netlist — the normalized,
sorted, stable-name representation of the circuit that every detector and
projector reads.

**Depends on:** phase 1 (types, EntityRef)  
**Unlocks:** phase 4 (intent compilation queries the fact graph)

---

## 3.1 What fact extraction does

Before any intent is inferred, the raw schematic must be canonicalized into a
single graph with stable, deterministic names. This is the precondition for
content-addressed identity: if device names shift between runs, every
obligation ID shifts with them.

Inputs:
- The parsed `SpiceNetlist` from `crates/api/src/io/spice.rs`
- The `CompiledTechnology` for device model resolution

Outputs:
- A `CanonicalFactGraph` with interned, sorted entities

---

## 3.2 The `CanonicalFactGraph` — `src/facts/mod.rs`

```
CanonicalFactGraph {
    devices: Vec<DeviceFact>,      // sorted by canonical name
    nets: Vec<NetFact>,            // sorted by canonical name
    terminals: Vec<TerminalFact>,  // sorted by (device, terminal_role)
    hierarchy: Vec<HierarchyFact>, // sorted by path
    adjacency: AdjacencyIndex,     // device↔net bipartite edges
}
```

### `DeviceFact`

```
DeviceFact {
    canonical_name: String,     // normalized, stable across re-parse
    model: String,              // resolved against CompiledTechnology
    kind: DeviceKind,           // Nmos, Pmos, Resistor, ...
    parameters: Vec<(String, f64)>,  // W, L, M, NF, ...
    terminals: Vec<TerminalRef>,     // ordered per technology terminal order
    hierarchy_path: Vec<String>,     // path from root to this instance
}
```

### `NetFact`

```
NetFact {
    canonical_name: String,
    connected_terminals: Vec<TerminalRef>,
    is_global: bool,            // VDD, VSS, etc.
    estimated_fanout: usize,
}
```

### `TerminalFact`

```
TerminalFact {
    device: String,             // canonical device name
    role: String,               // D, G, S, B (from technology terminal order)
    net: String,                // canonical net name
    index: usize,               // position in device's terminal list
}
```

### `AdjacencyIndex`

A bipartite graph (devices on one side, nets on the other), stored as two
adjacency lists:

```
AdjacencyIndex {
    device_to_nets: HashMap<String, SmallVec<[String; 4]>>,
    net_to_devices: HashMap<String, SmallVec<[(String, String); 4]>>,  // (device, terminal_role)
}
```

This is the primary query surface for detectors. "Which devices share a gate
net?" is `net_to_devices[gate_net].iter().filter(|(_, role)| role == "G")`.

---

## 3.3 Name canonicalization — `src/facts/normalize.rs`

The canonicalization rules:

1. **Case folding:** SPICE is case-insensitive. All names are lowercased.
2. **Hierarchy separator:** normalize `/`, `.`, `:` to `/`.
3. **Instance prefix stripping:** `X`, `M`, `R`, `C` prefixes on subcircuit
   instances are preserved (they carry semantic information in SPICE), but
   leading/trailing whitespace is stripped.
4. **Net name normalization:** global nets (`VDD`, `VSS`, `GND`, `VSSA`, etc.)
   are recognized and canonicalized to a fixed vocabulary. Non-global nets
   get their hierarchy path prepended: `block_a/net_12`.
5. **Sorting:** all entity lists are lexicographically sorted by canonical name
   after normalization.

The normalizer is a pure function: `normalize(raw_netlist, technology) -> CanonicalFactGraph`.
It is deterministic — same input always produces same output.

### Hierarchy handling

SPICE `.subckt` / `X` instantiation defines the hierarchy. The fact graph
flattens it but preserves the path:

```
.subckt ota
  M1 out+ in+ tail vss nfet W=2u L=0.18u
  M2 out- in- tail vss nfet W=2u L=0.18u
.ends

Xinst ota ...
```

Produces:
- `DeviceFact { canonical_name: "xinst/m1", hierarchy_path: ["xinst"] }`
- `DeviceFact { canonical_name: "xinst/m2", hierarchy_path: ["xinst"] }`

Repeated subcircuit instantiations produce distinct devices with distinct
hierarchy paths. The hierarchy path is what scoped constraints bind to.

---

## 3.4 Device model resolution

The `DeviceFact.model` field must be resolved against the `CompiledTechnology`
to determine:
- Terminal order (so terminal facts are consistently ordered)
- Device kind (NMOS vs PMOS matters for every detector)
- Available parameters (W, L, multiplier, fingers)

If a device model is not found in the technology, the fact is still created
but flagged with `kind: DeviceKind::Unknown`. This flows into a `CoverageGap`
conflict in reconciliation.

---

## 3.5 Derived query methods

The fact graph should expose queries that detectors need:

- `devices_on_net(net: &str) -> &[(String, String)]` — all (device, terminal_role) pairs
- `nets_of_device(device: &str) -> &[String]` — all nets connected to device
- `common_nets(dev_a: &str, dev_b: &str) -> Vec<String>` — shared nets
- `devices_by_model(model: &str) -> &[String]` — all devices of a given model
- `devices_by_kind(kind: DeviceKind) -> Vec<&DeviceFact>` — filter by kind
- `siblings(device: &str) -> Vec<&DeviceFact>` — same hierarchy parent
- `gate_net(device: &str) -> Option<&str>` — the net on terminal role "G"
- `drain_net(device: &str) -> Option<&str>` — the net on terminal role "D"
- `source_net(device: &str) -> Option<&str>` — the net on terminal role "S"
- `bulk_net(device: &str) -> Option<&str>` — the net on terminal role "B"
- `device_parameters(device: &str) -> &[(String, f64)]`
- `hierarchy_depth(device: &str) -> usize`

These are convenience wrappers over the adjacency index and device facts.

---

## 3.6 Testing

- **Determinism:** parse the same SPICE twice, assert `CanonicalFactGraph` is
  identical (all names, all orderings).
- **Sorting:** verify every entity list in the output is lexicographically sorted.
- **Hierarchy flattening:** a 2-level hierarchy produces correct paths.
- **Global net recognition:** VDD, VSS are marked `is_global`.
- **Model resolution:** known SKY130 models resolve to correct `DeviceKind`;
  unknown models get `Unknown`.
- **Adjacency correctness:** for a simple diff-pair SPICE, verify that
  `common_nets("m1", "m2")` returns the source net.
- **Reference circuits:** parse each test circuit (bandgap, diff pair + mirror,
  LDO) and snapshot the fact graph. Regression test against snapshot.

---

## 3.7 Open questions

1. **Should the fact graph own the raw SPICE parse tree?** Probably not —
   the fact graph is the *canonical* view. The raw parse tree is owned by
   `io::spice` in the api crate and is consumed during construction.

2. **How to handle parameterized subcircuits?** A subcircuit with parameter
   overrides (`Xinst ota W=4u`) should propagate the override to the inner
   devices. This requires parameter substitution during flattening.

3. **How deep to flatten?** For the constraint compiler, full flattening is
   correct (constraints operate on individual devices). But the hierarchy
   facts must be preserved so that hierarchy-scoped constraints can reference
   subcircuit boundaries.

4. **Incremental readiness:** the fact graph is rebuilt from scratch in batch
   mode. For future incremental support, the graph should use a stable
   interning scheme (device names → integer indices) so that a re-parse that
   adds one device produces a minimal delta. Arena allocation with stable
   indices achieves this.
