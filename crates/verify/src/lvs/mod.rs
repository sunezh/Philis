//! LVS (Layout vs. Schematic) connectivity types and union-find tracker.
//!
//! See `crates/verify/PLAN.md` §4 for the full design.

mod extract;
mod uf;

pub use extract::extract_connectivity;
pub use uf::{ConnectivityTracker, NetId, UnionResult};

/// Summary of LVS checking results.
#[derive(Debug, Clone, Default)]
pub struct LvsResult {
    /// Open circuits: net labels that span multiple disconnected components.
    pub opens: Vec<LvsOpen>,
    /// Short circuits: different net labels merged into one component.
    pub shorts: Vec<LvsShort>,
    /// Port mismatches: expected ports not found in the layout, or extra ports.
    pub mismatches: Vec<LvsMismatch>,
}

impl LvsResult {
    /// True if there are no opens, shorts, or mismatches.
    pub fn is_clean(&self) -> bool {
        self.opens.is_empty() && self.shorts.is_empty() && self.mismatches.is_empty()
    }

    /// Total number of errors.
    pub fn error_count(&self) -> usize {
        self.opens.len() + self.shorts.len() + self.mismatches.len()
    }
}

/// An open circuit: a net whose labeled elements span multiple components
/// that should have been connected.
#[derive(Debug, Clone)]
pub struct LvsOpen {
    /// The net that is split across components.
    pub net: NetId,
    /// The distinct components that should have been one.
    pub components: Vec<Vec<u32>>,
}

/// A short circuit: elements with different net labels ended up in the same
/// component.
#[derive(Debug, Clone)]
pub struct LvsShort {
    /// The nets that were incorrectly merged.
    pub nets: Vec<NetId>,
    /// The component representative where they merged.
    pub component: u32,
}

/// A port or device mismatch between the layout and the schematic.
#[derive(Debug, Clone)]
pub struct LvsMismatch {
    /// The kind of mismatch.
    pub kind: MismatchKind,
    /// The net the mismatch pertains to.
    pub net: NetId,
    /// Human-readable detail about the mismatch.
    pub detail: String,
}

/// The kind of LVS port/device mismatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MismatchKind {
    /// A port expected by the schematic was not found in the layout.
    MissingPort,
    /// The layout has a port not present in the schematic.
    ExtraPort,
    /// The number of devices on this net differs between layout and schematic.
    DeviceCountMismatch,
    /// A device's type differs between layout and schematic.
    DeviceTypeMismatch,
}

/// A connectivity invariant violation found by
/// [`ConnectivityTracker::invariant_check`].
#[derive(Debug, Clone)]
pub enum LvsError {
    /// A net's labeled elements span multiple disconnected components.
    Open(LvsOpen),
    /// Elements with different net labels ended up in the same component.
    Short(LvsShort),
}
