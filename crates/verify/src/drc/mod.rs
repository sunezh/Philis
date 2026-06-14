//! DRC violation types and the rule predicate library.
//!
//! See `crates/verify/PLAN.md` §3 for the full design.

mod influence;
mod predicates;

pub use predicates::{
    check_area, check_cut_spacing, check_enclosure, check_eol_spacing, check_grid_snap,
    check_implant_spacing, check_outline_exceed, check_overlap, check_prl_spacing,
    check_same_net_notch, check_spacing, check_well_enclosure, check_well_spacing, check_width,
};

use philis_geom::Rect;

/// Closed taxonomy of all DRC violation categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViolationKind {
    /// Two shapes on the same layer overlap when they should not.
    Overlap,
    /// Distance between two shapes is less than the required spacing.
    Spacing,
    /// A shape's width (or height) is below the layer's minimum width.
    Width,
    /// An enclosing shape does not extend far enough beyond the enclosed shape.
    Enclosure,
    /// A shape's area is below the layer's minimum area.
    Area,
    /// End-of-line spacing rule violated (spacing depends on run length at line end).
    EOL,
    /// Parallel run length (PRL) dependent spacing rule violated.
    PRL,
    /// Spacing between via cuts is below the minimum.
    CutSpacing,
    /// Same-net notch: a narrow channel between shapes on the same net.
    SameNetNotch,
    /// Shape edge does not land on the manufacturing grid.
    GridSnap,
    /// Shape extends beyond the outline / boundary of the cell.
    OutlineExceed,
    /// Well-to-well spacing violation (n-well to n-well or p-well to p-well).
    WellSpacing,
    /// Well enclosure of diffusion or implant is insufficient.
    WellEnclosure,
    /// Implant-to-implant spacing violation (e.g., NPlus to PPlus).
    ImplantSpacing,
}

/// Opaque reference to a shape in the caller's geometry store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShapeRef(pub u64);

/// Opaque rule identifier from the tech model (e.g., "M1.S.1" for M1 spacing rule 1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RuleId(pub String);

/// Opaque layer identifier.
///
/// Placeholder until `philis-tech::LayerId` exists; this newtype will be
/// replaced by (or made an alias of) that type once the tech crate is
/// implemented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LayerId(pub u32);

/// A single DRC violation with full metadata for diagnosis and repair.
#[derive(Debug, Clone, PartialEq)]
pub struct DrcViolation {
    /// Which rule predicate was violated.
    pub kind: ViolationKind,
    /// The rule identifier from the tech model.
    pub rule: RuleId,
    /// Layer on which the violation occurs.
    pub layer: LayerId,
    /// The two shapes (or shape + boundary) involved. Second is `None` for
    /// single-shape violations (Width, Area, GridSnap, OutlineExceed).
    pub shapes: (ShapeRef, Option<ShapeRef>),
    /// Bounding box of the violation region (for UI highlighting and repair scoping).
    pub region: Rect,
    /// The rule's required value (e.g., minimum spacing in nm).
    pub required: i64,
    /// The actual measured value.
    pub actual: i64,
    /// Whether the two shapes belong to the same net (affects some rule predicates).
    pub same_net: bool,
}

/// A dilated check region: the bounding box of a modified shape expanded by
/// `r_max`. Any shape whose bounding box intersects this region is a candidate
/// for narrow-phase DRC checking against the modified shape.
///
/// The consumer (place or route) constructs a `DrcRegion` from the modified
/// shape and queries its spatial index for overlapping shapes.
#[derive(Debug, Clone)]
pub struct DrcRegion {
    /// The original shape's bounding box before dilation.
    pub origin: Rect,
    /// The dilated bounding box (origin expanded by `r_max` on all sides).
    pub dilated: Rect,
    /// The influence radius used for dilation.
    pub r_max: i64,
}

impl DrcRegion {
    /// Create a new `DrcRegion` by dilating `origin` by `r_max` on all sides.
    ///
    /// # Panics
    ///
    /// Panics if `r_max` is negative enough to collapse `origin` (i.e.
    /// `origin`'s width or height plus `2 * r_max` is non-positive). `r_max`
    /// is expected to be non-negative in practice.
    pub fn new(origin: Rect, r_max: i64) -> Self {
        let dilated = origin
            .expand(r_max)
            .expect("dilating origin by r_max must produce a valid rect");
        DrcRegion {
            origin,
            dilated,
            r_max,
        }
    }
}

/// Diff of DRC violations between two states. Used by incremental checking:
/// when a shape is added or removed, the consumer runs narrow-phase checks
/// in the affected `DrcRegion` and produces a `DrcDelta`.
#[derive(Debug, Clone, Default)]
pub struct DrcDelta {
    /// Violations that appeared (new violations not present before the edit).
    pub added: Vec<DrcViolation>,
    /// Violations that disappeared (violations present before but resolved by the edit).
    pub removed: Vec<DrcViolation>,
}

impl DrcDelta {
    /// Merge another delta into this one, appending its `added` and `removed`
    /// violations to this delta's.
    pub fn merge(&mut self, other: DrcDelta) {
        self.added.extend(other.added);
        self.removed.extend(other.removed);
    }

    /// Net change in violation count: `added.len() - removed.len()`.
    pub fn net_change(&self) -> i64 {
        self.added.len() as i64 - self.removed.len() as i64
    }

    /// True if this delta introduces no new violations.
    pub fn is_clean(&self) -> bool {
        self.added.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x_lo: i64, y_lo: i64, x_hi: i64, y_hi: i64) -> Rect {
        Rect::new(x_lo, y_lo, x_hi, y_hi).unwrap()
    }

    fn violation(kind: ViolationKind) -> DrcViolation {
        DrcViolation {
            kind,
            rule: RuleId("R.1".to_string()),
            layer: LayerId(1),
            shapes: (ShapeRef(0), Some(ShapeRef(1))),
            region: rect(0, 0, 1, 1),
            required: 0,
            actual: 0,
            same_net: false,
        }
    }

    #[test]
    fn drc_region_dilates_on_all_sides() {
        let origin = rect(10, 10, 20, 20);
        let region = DrcRegion::new(origin, 5);

        assert_eq!(region.origin, origin);
        assert_eq!(region.r_max, 5);
        assert_eq!(region.dilated, rect(5, 5, 25, 25));
    }

    #[test]
    fn drc_region_zero_r_max_is_identity() {
        let origin = rect(0, 0, 10, 10);
        let region = DrcRegion::new(origin, 0);

        assert_eq!(region.dilated, origin);
    }

    #[test]
    fn drc_delta_merge_appends_added_and_removed() {
        let mut a = DrcDelta {
            added: vec![violation(ViolationKind::Spacing)],
            removed: vec![violation(ViolationKind::Width)],
        };
        let b = DrcDelta {
            added: vec![violation(ViolationKind::Overlap)],
            removed: vec![],
        };

        a.merge(b);

        assert_eq!(a.added.len(), 2);
        assert_eq!(a.removed.len(), 1);
    }

    #[test]
    fn drc_delta_net_change_and_is_clean() {
        let clean = DrcDelta {
            added: vec![],
            removed: vec![violation(ViolationKind::Spacing)],
        };
        assert_eq!(clean.net_change(), -1);
        assert!(clean.is_clean());

        let dirty = DrcDelta {
            added: vec![
                violation(ViolationKind::Spacing),
                violation(ViolationKind::Width),
            ],
            removed: vec![violation(ViolationKind::Overlap)],
        };
        assert_eq!(dirty.net_change(), 1);
        assert!(!dirty.is_clean());
    }

    #[test]
    fn drc_delta_merge_is_associative() {
        let a = DrcDelta {
            added: vec![violation(ViolationKind::Spacing)],
            removed: vec![],
        };
        let b = DrcDelta {
            added: vec![violation(ViolationKind::Width)],
            removed: vec![violation(ViolationKind::Overlap)],
        };
        let c = DrcDelta {
            added: vec![],
            removed: vec![violation(ViolationKind::Area)],
        };

        let mut left = a.clone();
        left.merge(b.clone());
        left.merge(c.clone());

        let mut bc = b.clone();
        bc.merge(c.clone());
        let mut right = a.clone();
        right.merge(bc);

        assert_eq!(left.added, right.added);
        assert_eq!(left.removed, right.removed);
    }
}
