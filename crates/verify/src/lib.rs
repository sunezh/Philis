//! DRC, LVS, and PEX verification engine for the Philis analog layout engine.
//!
//! Provides rule predicates (spacing, width, enclosure, area, EOL, PRL),
//! violation types, incremental DRC checking infrastructure, connectivity
//! extraction (LVS via union-find), and parasitic extraction surrogates (PEX).
//!
//! Currently implemented: the basic DRC predicates and violation types in
//! [`drc`]. The rest (advanced DRC predicates, incremental infra, LVS, PEX,
//! EM/IR/Antenna) is not yet implemented.
//!
//! See `crates/verify/PLAN.md` and `docs/Developer/verify/` for the full design.

pub mod coverage;
pub mod drc;

pub use coverage::RuleFamilyCoverage;
pub use drc::{
    check_area, check_cut_spacing, check_enclosure, check_eol_spacing, check_grid_snap,
    check_implant_spacing, check_outline_exceed, check_overlap, check_prl_spacing,
    check_same_net_notch, check_spacing, check_well_enclosure, check_well_spacing, check_width,
    DrcDelta, DrcRegion, DrcViolation, LayerId, RuleId, ShapeRef, ViolationKind,
};
