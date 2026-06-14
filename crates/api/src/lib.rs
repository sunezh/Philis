//! Philis — the public façade for the analog place & route engine.
//!
//! A thin, **gradually-tiered** surface over Philis's stages, designed against
//! Casey Muratori's reusable-component criteria
//! (`docs/Developer/api/Philis-Api-Considerations.html`).
//!
//! ## The tiers (each a strict superset of the one below)
//! * **Foundation — [`io`]:** dependency-free readers/writers — SPICE
//!   ([`io::spice`]), GDSII ([`io::gds`], the gdstk-equivalent surface), and the
//!   PDK reader ([`io::pdk`], JSON/TOML).
//! * **Engine — [`core`]:** everything constraint/placement/routing-dependent —
//!   the [`Constraint`] taxonomy, [`Objective`]/[`Strategy`]/[`HintBuilder`], and
//!   the raw stage oracles. Wired in behind whichever tier drives it.
//! * **Flow — [`flow`]:** the one-shot [`Circuit::run`] and the staged
//!   [`Circuit::analyze`] → [`Constraints::place`] → [`Placed::route`].
//! * **Builder — [`builder`]:** construct a circuit in Rust and [`BuiltCircuit::solve`].
//! * **Verification — [`verify`]:** DRC/LVS/PEX/ERC, callable on any manual tier
//!   and on the automation result.
//!
//! ## Design properties
//! Low coupling (the façade owns its [`Axis`] and result types), immediate mode
//! (nothing retained across calls), caller-driven (configuration closures, no
//! callbacks), and honest — anything unimplemented is a documented STUB
//! (`crates/api/STUBS.md`), never a silently ignored input.

pub mod builder;
pub mod core;
pub mod flow;
pub mod io;
pub mod layout;
pub mod port;
pub mod units;
pub mod verify;

// Flat re-exports — the names a caller actually reaches for.
pub use builder::{
    ApiBuildError, BuildError, BuildResult, BuildWarning, BuiltCircuit, CircuitDef, DeviceBuilder,
    NetBuilder,
};
pub use core::{
    constraint_input, evaluate_gate_acceptance, run_constraints, run_placement, run_routing,
    AcceptanceGate, ConflictClass, Constraint, ConstraintResult, ConstraintRunInput,
    DegradedConfidenceCause, FlowResult, GateEvaluation, GateStatus, HintBuilder, IntentClass,
    Objective, PlacementCertificate, PlacementConflict, PlacementResult, PlacementRunInput,
    RouterHandoffWitness, RoutingResult, RoutingRunInput, RunConfig, Strategy,
    ACCEPTANCE_GATE_ORDER,
};
pub use core::{
    AccessConfidence, Coverage, DiagnosticClass, DiagnosticScope, Evidence, HardVector, NetClass,
    PdkState, RouteCertificate, RouteDiagnostic, RouteQuality, RuleCoverageReport,
};
pub use flow::{Circuit, Constraints, Placed};
pub use io::gds::Gds;
pub use io::pdk::Pdk;
pub use io::spice::SpiceNetlist;
pub use layout::{Layout, PlacedInstance, Violation};
pub use port::{
    port_ref_from_name, valid_port_names, BjtPort, DeviceKind, MosfetPort, PassivePort, PortRef,
};
pub use units::{parse_axis, Axis, Coord, Distance, Length};
pub use verify::{CheckKind, CheckReport};

// Oracle vocabulary, flat and under `next::` (mirrors the eventual split into
// separate engine crates).
pub use core::{constraints, placer, router};
pub mod next {
    pub use crate::core::{constraints, placer, router};
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// The fallible flow's error type.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ApiError {
    /// SPICE parse failure.
    Spice(String),
    /// PDK load/parse failure.
    Pdk(String),
    /// I/O failure (path in the message).
    Io(String),
    /// `select_subckt` named a subcircuit that does not exist.
    SubcktNotFound(String),
    /// The netlist has no devices to place.
    EmptyNetlist,
    /// `Pdk::from_env` was given an env var that is unset.
    MissingPdkPathEnv(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Spice(m) => write!(f, "spice parse error: {m}"),
            ApiError::Pdk(m) => write!(f, "pdk error: {m}"),
            ApiError::Io(m) => write!(f, "io error: {m}"),
            ApiError::SubcktNotFound(n) => write!(f, "subckt not found: {n}"),
            ApiError::EmptyNetlist => write!(f, "empty netlist: nothing to place"),
            ApiError::MissingPdkPathEnv(v) => write!(f, "env var not set: {v}"),
        }
    }
}
impl std::error::Error for ApiError {}

// ---------------------------------------------------------------------------
// Tier 0 quickstart — the simplest possible real path.
// ---------------------------------------------------------------------------

/// A placed cell: a name and its integer grid coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub name: String,
    pub x: i64,
    pub y: i64,
}

/// Parse a trivial netlist: one whitespace-separated `name x y` triple per line.
/// Blank lines and lines beginning with `#` are ignored. This is the genuinely
/// working quickstart path; the SPICE/builder paths are the real surface.
pub fn parse(input: &str) -> Result<Vec<Cell>, String> {
    let mut cells = Vec::new();
    for (lineno, raw) in input.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut it = line.split_whitespace();
        let name = it
            .next()
            .ok_or_else(|| format!("line {}: missing cell name", lineno + 1))?;
        let x = it
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| format!("line {}: missing or invalid x coordinate", lineno + 1))?;
        let y = it
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| format!("line {}: missing or invalid y coordinate", lineno + 1))?;
        cells.push(Cell {
            name: name.to_string(),
            x,
            y,
        });
    }
    Ok(cells)
}

/// Run the quickstart pass: bounding box + half-perimeter wirelength (HPWL).
pub fn place_and_route(input: &str) -> Result<String, String> {
    let cells = parse(input)?;
    if cells.is_empty() {
        return Ok("placed 0 cells".to_string());
    }
    let (mut min_x, mut max_x) = (cells[0].x, cells[0].x);
    let (mut min_y, mut max_y) = (cells[0].y, cells[0].y);
    for c in &cells[1..] {
        min_x = min_x.min(c.x);
        max_x = max_x.max(c.x);
        min_y = min_y.min(c.y);
        max_y = max_y.max(c.y);
    }
    let (w, h) = (max_x - min_x, max_y - min_y);
    Ok(format!(
        "placed {} cells, bbox {}x{}, hpwl {}",
        cells.len(),
        w,
        h,
        w + h
    ))
}

#[cfg(feature = "python")]
mod python;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_skips_comments_and_blanks() {
        let cells = parse("# header\na 0 0\n\nb 3 4\n").unwrap();
        assert_eq!(cells.len(), 2);
        assert_eq!(
            cells[1],
            Cell {
                name: "b".into(),
                x: 3,
                y: 4
            }
        );
    }

    #[test]
    fn computes_hpwl() {
        let out = place_and_route("a 0 0\nb 3 4").unwrap();
        assert_eq!(out, "placed 2 cells, bbox 3x4, hpwl 7");
    }

    #[test]
    fn empty_input_places_nothing() {
        assert_eq!(place_and_route("\n# nothing\n").unwrap(), "placed 0 cells");
    }

    #[test]
    fn rejects_malformed_lines() {
        assert!(place_and_route("a 1").is_err());
        assert!(place_and_route("a 1 two").is_err());
    }
}
