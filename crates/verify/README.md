# Philis — Verify Module (DRC / LVS / PEX / EM)

Physical- and electrical-verification engine for [Philis](https://github.com/UW-ASIC/Philis),
UWASIC's automated analog place-and-route engine. This crate (`philis-verify`) checks
whether an IC layout is manufacturable and matches its intended circuit: geometry against
design rules, connectivity against the netlist, extracted parasitics, and long-term
reliability effects.

> **Scope & status.** Philis is a team project. This crate is my contribution — the entire
> verification module (~3,100 lines of Rust, plus runnable demos). The workspace scaffold
> and the place/route crates are other contributors' work. The module currently runs
> **standalone via example harnesses**; integration into the main P&R flow and validation
> against a full PDK rule deck are the next steps (see [Roadmap](#roadmap)). All rule values
> are **parameterized and injected by the caller** — no PDK-specific numbers are baked in.

---

## What it does

Four families of checks, each independently runnable:

| Family | What it answers | Key pieces |
|---|---|---|
| **DRC** | Does the geometry obey the manufacturing rules? | 14 rule predicates + incremental re-check infrastructure |
| **LVS** | Does the physical layout connect up like the schematic? | Connectivity extraction via union-find |
| **PEX** | What parasitic R/C does the routed metal add? | Wire/via resistance, ground and coupling capacitance |
| **EM / IR / Antenna** | Will it survive over time and during fabrication? | Current-density (Black's equation), resistive-mesh IR-drop solve, antenna-ratio check |

### DRC — 14 rule predicates
Spacing, width, area, overlap, grid-snap, and outline-exceed (basic), plus enclosure,
end-of-line spacing, parallel-run-length spacing, cut spacing, same-net notch, well spacing,
well enclosure, and implant spacing (advanced). Backed by an incremental layer
(`DrcRegion` / `DrcDelta` / `RuleFamilyCoverage`) so that after a small layout edit only the
affected region is re-checked rather than the whole design.

### LVS — connectivity extraction
A `ConnectivityTracker` built on a **union-find** data structure walks the layout shapes and
vias, merging everything that is electrically joined into components, then compares the result
against the expected nets to flag opens, shorts, and mismatches.

### PEX — parasitic extraction surrogates
Computes per-segment wire resistance, ground (area + fringe) capacitance, via resistance, and
same-layer / interlayer coupling capacitance; aggregates them per net (`NetParasitics`) and
flags statistical outliers.

### EM / IR / Antenna — reliability
Current density with an electromigration check via **Black's equation**, a static IR-drop
analysis that solves the power grid as a **resistive mesh**, and an antenna-ratio check that
guards gate oxide against charge accumulation during fabrication.

---

## Running it

Each family ships a self-contained demo; the last one runs the whole pipeline on a small
two-net layout (two M1 wires joined through a via to M2):

```bash
cargo run -p philis-verify --example drc_demo
cargo run -p philis-verify --example lvs_demo
cargo run -p philis-verify --example pex_demo
cargo run -p philis-verify --example em_ir_antenna_demo

# end-to-end: DRC + LVS + PEX + EM + IR + antenna on one layout
cargo run -p philis-verify --example verify_demo
```

`verify_demo` prints, for that layout, the spacing check result, extracted connectivity,
net-1 parasitics (R, ground cap, via R), the EM current-density verdict, per-node IR-drop
voltages, and the antenna ratio.

---

## Architecture

```
crates/verify/
├── src/
│   ├── lib.rs            # public API surface (re-exports)
│   ├── drc/
│   │   ├── predicates.rs # the 14 rule checks
│   │   ├── mod.rs        # violation types, incremental DRC infra
│   │   └── influence.rs
│   ├── lvs/
│   │   ├── uf.rs         # union-find core
│   │   ├── extract.rs    # connectivity extraction
│   │   └── mod.rs
│   ├── pex/
│   │   ├── wire.rs       # resistance, ground capacitance
│   │   ├── coupling.rs   # same-layer / interlayer coupling
│   │   ├── via.rs        # via resistance
│   │   └── mod.rs        # per-net aggregation, outliers
│   ├── em/
│   │   ├── black.rs      # current density, Black's equation
│   │   ├── ir.rs         # IR-drop resistive-mesh solve
│   │   ├── antenna.rs    # antenna-ratio check
│   │   └── mod.rs
│   └── coverage.rs       # per-rule-family coverage tracking
└── examples/             # the runnable demos above
```

Depends only on `philis-geom` (rectangle/geometry primitives) and `philis-tech` (technology
layer). Rule thresholds are passed in by the caller, which keeps the checkers PDK-agnostic.

---

## Roadmap

Not yet done — called out here so the boundary is clear:

- **Flow integration.** Wiring the checkers into Philis's main P&R flow (the integration point,
  `crates/api/src/verify.rs`, is currently a stub that returns "clean").
- **Full PDK rule deck.** Driving the predicates from a real technology rule set rather than
  demo-supplied values (`compute_r_max` is deferred pending `philis-tech::RuleSet`).
- **LVS schematic comparison** and **PEX coupling shielding** (noted as future work from the
  team's design discussions).

---

## Background

Built while learning analog layout verification largely from scratch. Primary references were
Razavi (*Design of Analog CMOS Integrated Circuits*, ch. 2.4 & 19) for the device- and
routing-level considerations that motivate each check, plus the Philis developer docs. A
running progress log for this module lives at [`docs/progress_log-ethan.md`](docs/progress_log-ethan.md).

**Author:** Ethan Sun · verify module · [github.com/sunezh](https://github.com/sunezh)
