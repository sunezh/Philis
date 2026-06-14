//! DRC, LVS, and PEX verification engine for the Philis analog layout engine.
//!
//! Provides rule predicates (spacing, width, enclosure, area, EOL, PRL),
//! violation types, incremental DRC checking infrastructure, connectivity
//! extraction (LVS via union-find), and parasitic extraction surrogates (PEX).
//!
//! Currently implemented: the DRC predicate library and incremental-checking
//! types in [`drc`], per-rule-family [`coverage`], LVS connectivity
//! extraction in [`lvs`], and parasitic extraction surrogates in [`pex`].
//! Electromigration checking, static IR drop analysis, and antenna checking
//! are implemented in [`em`].
//!
//! See `crates/verify/PLAN.md` and `docs/Developer/verify/` for the full design.

pub mod coverage;
pub mod drc;
pub mod em;
pub mod lvs;
pub mod pex;

pub use coverage::RuleFamilyCoverage;
pub use drc::{
    check_area, check_cut_spacing, check_enclosure, check_eol_spacing, check_grid_snap,
    check_implant_spacing, check_outline_exceed, check_overlap, check_prl_spacing,
    check_same_net_notch, check_spacing, check_well_enclosure, check_well_spacing, check_width,
    DrcDelta, DrcRegion, DrcViolation, LayerId, RuleId, ShapeRef, ViolationKind,
};
pub use em::{
    check_antenna, check_em, compute_ir_drop, current_density, AntennaViolation, EmViolation,
    IrDropResult,
};
pub use lvs::{
    extract_connectivity, ConnectivityTracker, LvsError, LvsMismatch, LvsOpen, LvsResult, LvsShort,
    MismatchKind, NetId, UnionResult,
};
pub use pex::{
    detect_outliers, ground_capacitance, interlayer_coupling, same_layer_coupling, via_resistance,
    wire_resistance, MatchedBudget, NetParasitics, OutlierKind, PexOutlier, SegmentParasitics,
};
