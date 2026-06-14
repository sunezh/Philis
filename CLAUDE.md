# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Philis is an analog IC layout engine (place & route) with a Rust core and a
Python API built via maturin/pyo3. The mascot/docs describe it as letting users
build circuits via an API with automated place-and-route, dumping both GDS and
the Python code used to produce it.

## Workspace layout

Cargo workspace members (see `Cargo.toml`):
- `crates/api` — the only crate with real, substantial code. This is the
  public façade (`philis` crate/lib + `philis` binary). Everything else is
  early scaffolding.
- `crates/geom` (`philis_geom`) — integer (i64 nanometer) geometry kernel:
  rects, polygons, R-tree spatial index. Currently just a doc-comment stub.
- `crates/tech` (`philis_tech`) — PDK technology compiler (LEF/Sky130/JSON →
  `CompiledTech`). Doc-comment stub; depends on `philis-geom`.
- `crates/verify` (`philis_verify`) — DRC/LVS/PEX verification engine. Doc-comment
  stub; depends on `philis-geom` and `philis-tech`.
- `crates/solver` (`philis_solver`) — SAT/ILP/SMT solver abstraction. Doc-comment stub.
- `bench` — Criterion benchmarks (`bench/benches/engine.rs`) plus shared
  helpers in `bench/src/lib.rs` that build the `philis` release binary and load
  fixtures from `bench/fixtures/`.

`crates/place`, `crates/route`, `crates/constraints` exist on disk (each with
their own `Cargo.toml`/`lib.rs` and a stack of design-plan markdown files) but
are **not** in the workspace `members` list and have empty `lib.rs`/`Cargo.toml`
content — they are design-doc placeholders for future crates, not buildable.

## `crates/api` — the real engine surface

This is the crate to focus on for almost all work. It implements a
**gradually-tiered** API (see `crates/api/src/lib.rs` module docs), in order
of increasing power, each tier a strict superset of the one below:

1. **Foundation — `io/`** (zero deps): `io::spice` (SPICE netlist reader/writer),
   `io::gds` (GDSII stream reader/writer), `io::pdk` (JSON/TOML PDK reader).
2. **Engine — `core.rs`**: `Constraint` taxonomy, `Objective`/`Strategy`/
   `HintBuilder`, and the raw stage oracles `run_constraints`, `run_placement`,
   `run_routing`, `run_flow`.
3. **Flow — `flow.rs`**: one-shot `Circuit::run` and staged
   `Circuit::analyze` → `Constraints::place` → `Placed::route`.
4. **Builder — `builder.rs`**: construct a circuit in Rust, `BuiltCircuit::solve`.
5. **Verification — `verify.rs`**: DRC/LVS/PEX/ERC, callable on any tier's output.

Other notable files: `units.rs` (Axis/Coord/Distance/Length), `port.rs`
(device port vocabulary), `layout.rs` (`Layout`, `PlacedInstance`, `Violation`
result types), `python.rs` (pyo3 bindings, gated by `python`/`extension-module`
features), `main.rs` (the `philis` binary used by benches).

### Stub status — read `crates/api/STUBS.md` before changing engine behavior

The API surface is **complete and tested**, but several engine implementations
are intentional stubs (e.g. `run_constraints`, `run_placement`, `run_routing`,
`Layout::area`/`wirelength`, `verify::drc`/`lvs`/`pex`/`erc`). The project's
honesty contract: unsupported/unimplemented behavior must surface via typed
warnings (e.g. `BuildWarning::UnsupportedConstraint`) or "degraded confidence"
certificates — **never silently dropped or faked as passing**. `STUBS.md` has a
legend (✅ real / 🟡 partial / 🔲 stub) and a de-stub roadmap; update it when you
implement a stubbed item.

## Build, test, lint

```bash
# Build everything in the workspace
cargo build

# Build just the api crate / philis binary
cargo build -p api --bin philis

# Run all Rust tests
cargo test

# Run a specific integration test file (see [[test]] entries in crates/api/Cargo.toml)
cargo test -p api --test integration
cargo test -p api --test api_smoke
cargo test -p api --test builder_api
cargo test -p api --test units_ports
cargo test -p api --test constraints_axis
cargo test -p api --test layout_results
cargo test -p api --test verify_io

# Run a single test by name
cargo test -p api <test_name>

# Lints/format
cargo clippy
cargo fmt
```

Integration tests for `crates/api` live in `crates/api/test/` (note: non-default
`test/`, not `tests/`), explicitly listed as `[[test]]` entries in
`crates/api/Cargo.toml`. Each file exercises one slice of the public surface.

### Python API (maturin/pyo3)

```bash
# Build the Python extension module (from pyproject.toml: manifest crates/api/Cargo.toml)
maturin develop --manifest-path crates/api/Cargo.toml --features extension-module

# Run Python tests (configured testpaths = crates/api/pytest)
pytest
```

The `python` feature enables pyo3 bindings (`crates/api/src/python.rs`);
`extension-module` additionally sets `pyo3/extension-module` for building a
loadable `.so`/`.pyd`.

### Benchmarks

```bash
cargo bench -p bench
```

`bench/src/lib.rs` builds the release `philis` binary on demand and enumerates
fixtures from `bench/fixtures/` (`*.netlist` for the quickstart `name x y`
format, `.sp` for SPICE).

## Nix dev shell

`flake.nix` provides a dev shell with the Rust toolchain (stable + rust-analyzer),
Python 3, maturin, and VLSI tooling (`magic`, `klayout`, `netgen`). It also sets
up a `.venv` and fetches PDKs (sky130, gf180mcu, ihp-sg13g2) via `ciel` into
`$PDK_ROOT` (default `~/.ciel`). Enter with `nix develop`.

## Documentation

`docs/Developer/` contains per-crate design docs (`api`, `geom`, `tech`,
`verify`, `solver`, `place`, `route`, `constraints`, `general`), served as
static HTML. View locally with `python -m http.server -d docs`, or online at
https://omarsiwy.github.io/Philis/. `crates/api/src/lib.rs` and `STUBS.md`
reference specific pages (e.g. `docs/Developer/place/index.html`,
`docs/Developer/route/Api-Mapping.html`) when describing de-stub roadmaps —
check these when implementing placement/routing internals.
