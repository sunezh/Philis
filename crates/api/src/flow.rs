//! The SPICE-driven flow: the one-shot path and the staged tiers.
//!
//! The flow is *gradually tiered*:
//! * **one-shot:** [`Circuit::run`] takes a parsed circuit to a [`FlowResult`].
//! * **staged:** [`Circuit::analyze`] → [`Constraints::place`] → [`Placed::route`],
//!   each returning a façade-owned handle. `run` is defined as their composition.
//!
//! Verification ([`crate::verify`]) is reachable on each *manual* handle at any
//! time — `Constraints` checks the netlist, `Placed` checks netlist + partial
//! geometry. The automation (one-shot) path is checked *after*, on its [`Layout`].

use crate::core::{
    self, run_constraints, run_placement, run_routing, ConstraintResult, ConstraintRunInput,
    FlowResult, PlacementResult, PlacementRunInput, RoutingRunInput, RunConfig,
};
use crate::io::gds::Gds;
use crate::io::pdk::Pdk;
use crate::io::spice::SpiceNetlist;
use crate::layout::Layout;
use crate::verify::{self, CheckReport};
use crate::ApiError;

/// A parsed circuit netlist.
#[derive(Debug, Clone)]
pub struct Circuit {
    netlist: SpiceNetlist,
}

impl Circuit {
    /// Parse SPICE text.
    pub fn from_spice_str(text: &str) -> Result<Circuit, ApiError> {
        let netlist = SpiceNetlist::parse(text).map_err(|e| ApiError::Spice(e.to_string()))?;
        Ok(Circuit { netlist })
    }

    /// Parse a SPICE file.
    pub fn from_spice_file(path: impl AsRef<std::path::Path>) -> Result<Circuit, ApiError> {
        let text = std::fs::read_to_string(path.as_ref())
            .map_err(|e| ApiError::Io(format!("{}: {e}", path.as_ref().display())))?;
        Circuit::from_spice_str(&text)
    }

    /// Promote a named subckt's devices to the top level; error if absent.
    pub fn select_subckt(mut self, name: &str) -> Result<Circuit, ApiError> {
        if self.netlist.select_subckt(name) {
            Ok(self)
        } else {
            Err(ApiError::SubcktNotFound(name.to_string()))
        }
    }

    /// If the top level is empty, promote the first subckt. Infallible.
    pub fn use_first_subckt_as_top(mut self) -> Circuit {
        self.netlist.promote_first_subckt();
        self
    }

    /// The parsed netlist.
    pub fn netlist(&self) -> &SpiceNetlist {
        &self.netlist
    }

    /// One-shot: run constraints → placement → routing in one call.
    pub fn run(&self, pdk: &Pdk, cfg: RunConfig) -> Result<FlowResult, ApiError> {
        // Defined as the composition of the staged path — the decomposition is
        // real, not just documented.
        let layout = self.analyze(pdk)?.place(cfg)?.route(cfg)?;
        Ok(layout.flow().clone())
    }

    /// Staged step 1: compile constraints into a façade-owned handle.
    pub fn analyze(&self, _pdk: &Pdk) -> Result<Constraints, ApiError> {
        if self.netlist.device_count() == 0 {
            return Err(ApiError::EmptyNetlist);
        }
        let result = run_constraints(&ConstraintRunInput {
            device_count: self.netlist.device_count(),
        });
        Ok(Constraints {
            netlist: self.netlist.clone(),
            result,
        })
    }
}

/// Staged handle — compiled constraints, ready to place. Checks read the netlist.
#[derive(Debug, Clone)]
pub struct Constraints {
    netlist: SpiceNetlist,
    result: ConstraintResult,
}

impl Constraints {
    /// The raw constraint-stage result.
    pub fn result(&self) -> &ConstraintResult {
        &self.result
    }

    /// Staged step 2: run placement.
    pub fn place(&self, _cfg: RunConfig) -> Result<Placed, ApiError> {
        let placement = run_placement(&PlacementRunInput {
            device_count: self.netlist.device_count(),
        });
        Ok(Placed {
            netlist: self.netlist.clone(),
            constraints: self.result.clone(),
            placement,
            gds: Gds::new("PHILIS"),
        })
    }

    /// Run any check at this tier (geometry is not built yet, so DRC/LVS/PEX are
    /// trivially clean; ERC reads the netlist).
    pub fn check(&self, kind: crate::verify::CheckKind) -> CheckReport {
        run_check(kind, &self.netlist, None)
    }
}

/// Staged handle — a placement, ready to route. Checks read netlist + partial GDS.
#[derive(Debug, Clone)]
pub struct Placed {
    netlist: SpiceNetlist,
    constraints: ConstraintResult,
    placement: PlacementResult,
    gds: Gds,
}

impl Placed {
    /// The raw placement-stage result.
    pub fn result(&self) -> &PlacementResult {
        &self.placement
    }

    /// Staged step 3: run routing and finish the [`Layout`].
    ///
    /// `baseline_used` is threaded from the placement result *for* the caller.
    pub fn route(&self, _cfg: RunConfig) -> Result<Layout, ApiError> {
        let routing = run_routing(&RoutingRunInput {
            net_count: self.netlist.device_count(),
            baseline_used: self.placement.baseline_used,
        });
        let flow = FlowResult {
            constraints: self.constraints.clone(),
            placement: self.placement.clone(),
            routing,
        };
        Ok(Layout::new(flow, self.netlist.clone(), self.gds.clone()))
    }

    /// Run any check at this tier, against the netlist and the partial geometry.
    pub fn check(&self, kind: crate::verify::CheckKind) -> CheckReport {
        run_check(kind, &self.netlist, Some(&self.gds))
    }
}

fn run_check(
    kind: crate::verify::CheckKind,
    netlist: &SpiceNetlist,
    gds: Option<&Gds>,
) -> CheckReport {
    use crate::verify::CheckKind::*;
    match kind {
        Drc => verify::drc(netlist, gds),
        Lvs => verify::lvs(netlist, gds),
        Pex => verify::pex(netlist, gds),
        Erc => verify::erc(netlist, gds),
    }
}

/// Internal: build a finished [`Layout`] from a netlist (used by the builder's
/// one-shot `solve`).
pub(crate) fn solve_netlist(
    netlist: &SpiceNetlist,
    cfg: RunConfig,
) -> Result<Layout, ApiError> {
    let flow = core::run_flow(netlist, cfg)?;
    Ok(Layout::new(flow, netlist.clone(), Gds::new("PHILIS")))
}
