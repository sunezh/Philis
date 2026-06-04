//! The solved layout: result inspectors, verification checks, and GDS export.
//!
//! The common result questions (`area`, `wirelength`, …) are answered in the
//! façade's own types; [`Layout::flow`] is the labeled escape hatch to the raw
//! engine outputs. Because a `Layout` carries both the netlist and the built
//! geometry, the verification checks ([`Layout::drc`] etc.) run against the
//! in-memory result with no `.gds` dump required — though [`Layout::write_gds`]
//! can dump one on demand.

use crate::core::{FlowResult, PlacementResult, RoutingResult};
use crate::io::gds::Gds;
use crate::io::spice::SpiceNetlist;
use crate::units::Coord;
use crate::verify::{self, CheckReport};
use crate::ApiError;

/// A single DRC finding, pre-rendered for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub message: String,
}

/// One placed device instance, in the façade's own coordinate type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacedInstance {
    pub name: String,
    pub x: Coord,
    pub y: Coord,
}

/// A finished layout.
#[derive(Debug, Clone)]
pub struct Layout {
    flow: FlowResult,
    netlist: SpiceNetlist,
    gds: Gds,
}

impl Layout {
    /// Assemble a layout from the flow result, the source netlist, and the built
    /// geometry. Internal seam used by the staged/one-shot/builder paths.
    pub(crate) fn new(flow: FlowResult, netlist: SpiceNetlist, gds: Gds) -> Layout {
        Layout { flow, netlist, gds }
    }

    // ---- geometric inspectors (STUB engine → zeroed) ----------------------

    /// Bounding-box area in nm² (0 if degenerate).
    pub fn area(&self) -> u64 {
        (self.width().max(0) as u64) * (self.height().max(0) as u64)
    }
    /// Bounding-box width in nm. STUB: 0.
    pub fn width(&self) -> i64 {
        0
    }
    /// Bounding-box height in nm. STUB: 0.
    pub fn height(&self) -> i64 {
        0
    }
    /// Total routed wirelength. STUB: 0.0.
    pub fn wirelength(&self) -> f64 {
        0.0
    }
    /// How many instances were placed.
    pub fn placed_count(&self) -> usize {
        self.placement().placed_count
    }
    /// The placement certificate — acceptance-gate verdicts, degraded-confidence
    /// causes, and the router handoff. STUB: degraded (every gate `Degraded`).
    pub fn certificate(&self) -> &crate::core::PlacementCertificate {
        &self.placement().certificate
    }
    /// The router-handoff witness extracted from the certificate.
    pub fn router_handoff(&self) -> &crate::core::RouterHandoffWitness {
        &self.placement().certificate.handoff
    }
    /// Whether the placement certificate is signoff-quality (every gate passed
    /// on coded evidence). STUB engine ⇒ always `false`.
    pub fn is_signoff_quality(&self) -> bool {
        self.certificate().is_signoff_quality()
    }
    /// How many routed segments the router emitted.
    pub fn routed_segment_count(&self) -> usize {
        self.routing().routed_segments
    }
    /// Iterate placed instances in façade-owned coordinates. STUB: empty.
    pub fn instances(&self) -> impl Iterator<Item = PlacedInstance> + '_ {
        std::iter::empty()
    }

    // ---- verification (runs against the in-memory netlist + GDS) -----------

    /// Run design-rule check against the built geometry.
    pub fn drc(&self) -> CheckReport {
        verify::drc(&self.netlist, Some(&self.gds))
    }
    /// Run layout-versus-schematic.
    pub fn lvs(&self) -> CheckReport {
        verify::lvs(&self.netlist, Some(&self.gds))
    }
    /// Run parasitic extraction.
    pub fn pex(&self) -> CheckReport {
        verify::pex(&self.netlist, Some(&self.gds))
    }
    /// Run electrical-rule check.
    pub fn erc(&self) -> CheckReport {
        verify::erc(&self.netlist, Some(&self.gds))
    }

    /// DRC findings, rendered as [`Violation`]s.
    pub fn violations(&self) -> Vec<Violation> {
        self.drc()
            .findings()
            .iter()
            .map(|m| Violation { message: m.clone() })
            .collect()
    }
    /// True iff LVS finds no mismatches.
    pub fn lvs_clean(&self) -> bool {
        self.lvs().is_clean()
    }

    // ---- escape hatches ---------------------------------------------------

    /// The raw engine stage outputs.
    pub fn flow(&self) -> &FlowResult {
        &self.flow
    }
    /// The built GDS geometry (the gdstk-equivalent handle).
    pub fn gds(&self) -> &Gds {
        &self.gds
    }
    /// Dump the layout to a GDSII file.
    pub fn write_gds(&self, path: impl AsRef<std::path::Path>) -> Result<(), ApiError> {
        std::fs::write(path.as_ref(), self.gds.write())
            .map_err(|e| ApiError::Io(format!("{}: {e}", path.as_ref().display())))
    }

    fn placement(&self) -> &PlacementResult {
        &self.flow.placement
    }
    fn routing(&self) -> &RoutingResult {
        &self.flow.routing
    }
}
