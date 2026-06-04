//! Verification checks: DRC, LVS, PEX, ERC.
//!
//! The beauty of the tiered flow is that these checks can be run at **any point**
//! on a manual tier — they read the netlist (a bipartite hypergraph) together
//! with the *incrementally-built* GDS — and on the automation tier **after** the
//! run, against the in-memory result (so no `.gds` dump is required, though one
//! can always be emitted). The tier handles in [`crate::flow`] and [`crate::Layout`]
//! expose thin methods that call into here.
//!
//! STUB: every check currently returns "clean". The harness, report shape, and
//! call sites are real; the rule decks are tracked in `crates/api/STUBS.md`.

use crate::io::gds::Gds;
use crate::io::spice::SpiceNetlist;

/// Which physical/electrical check a report describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckKind {
    /// Design-rule check (geometry vs. the PDK rule deck).
    Drc,
    /// Layout-versus-schematic (does the layout match the netlist).
    Lvs,
    /// Parasitic extraction (RC of the routed geometry).
    Pex,
    /// Electrical-rule check (netlist-level connectivity sanity).
    Erc,
}

impl CheckKind {
    pub const fn name(self) -> &'static str {
        match self {
            CheckKind::Drc => "DRC",
            CheckKind::Lvs => "LVS",
            CheckKind::Pex => "PEX",
            CheckKind::Erc => "ERC",
        }
    }
}

/// The outcome of one check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReport {
    pub kind: CheckKind,
    findings: Vec<String>,
}

impl CheckReport {
    /// A clean (no-findings) report — the stub result.
    pub fn clean(kind: CheckKind) -> CheckReport {
        CheckReport {
            kind,
            findings: Vec::new(),
        }
    }
    /// Whether the check passed (no findings).
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }
    /// The individual findings (violations / mismatches).
    pub fn findings(&self) -> &[String] {
        &self.findings
    }
}

/// Run DRC against the netlist and the (optionally partial) GDS. STUB.
pub fn drc(_netlist: &SpiceNetlist, _gds: Option<&Gds>) -> CheckReport {
    CheckReport::clean(CheckKind::Drc)
}

/// Run LVS comparing the netlist to the layout. STUB.
pub fn lvs(_netlist: &SpiceNetlist, _gds: Option<&Gds>) -> CheckReport {
    CheckReport::clean(CheckKind::Lvs)
}

/// Run parasitic extraction over the routed geometry. STUB.
pub fn pex(_netlist: &SpiceNetlist, _gds: Option<&Gds>) -> CheckReport {
    CheckReport::clean(CheckKind::Pex)
}

/// Run ERC over the netlist's connectivity. STUB.
pub fn erc(_netlist: &SpiceNetlist, _gds: Option<&Gds>) -> CheckReport {
    CheckReport::clean(CheckKind::Erc)
}
