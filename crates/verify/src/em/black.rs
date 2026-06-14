//! Electromigration checking via Black's equation.
//!
//! See `crates/verify/PLAN.md` §6.1 for the full design.

use crate::{LayerId, ShapeRef};

/// Compute current density for a wire segment.
///
/// Formula: `J = I / (width * thickness)`
///
/// Inputs:
///   - `current`: current through the segment, in amps
///   - `width`: wire width in nm
///   - `thickness`: wire thickness in nm (from the layer stack)
///
/// Output: current density in A/nm^2.
///
/// # Panics
///
/// In debug builds, panics if `width <= 0` or `thickness <= 0`.
pub fn current_density(current: f64, width: i64, thickness: i64) -> f64 {
    debug_assert!(width > 0, "current_density: width must be positive");
    debug_assert!(thickness > 0, "current_density: thickness must be positive");
    current / (width as f64 * thickness as f64)
}

/// Check current density against the layer's `j_max` limit, subject to the
/// Blech short-line effect.
///
/// Inputs:
///   - `j`: computed current density (from [`current_density`])
///   - `j_max`: maximum allowed current density for the layer (A/nm^2)
///   - `segment_length`: length of the wire segment in nm
///   - `blech_length`: minimum length below which short-line effects prevent
///     EM failure (nm)
///   - `shape`/`layer`: identify the offending segment
///
/// Output: `Some(EmViolation)` if `j > j_max` and `segment_length >
/// blech_length`, otherwise `None`.
pub fn check_em(
    j: f64,
    j_max: f64,
    segment_length: i64,
    blech_length: i64,
    shape: ShapeRef,
    layer: LayerId,
) -> Option<EmViolation> {
    if j > j_max && segment_length > blech_length {
        Some(EmViolation {
            shape,
            layer,
            j_actual: j,
            j_max,
            ratio: j / j_max,
            segment_length,
            blech_length,
        })
    } else {
        None
    }
}

/// An electromigration violation: a wire segment whose current density
/// exceeds the layer's limit and is long enough for the Blech effect not to
/// apply.
#[derive(Debug, Clone)]
pub struct EmViolation {
    /// The offending shape.
    pub shape: ShapeRef,
    /// Layer of the offending shape.
    pub layer: LayerId,
    /// Computed current density (A/nm^2).
    pub j_actual: f64,
    /// Maximum allowed current density (A/nm^2).
    pub j_max: f64,
    /// Ratio `j_actual / j_max` (> 1.0 means violation).
    pub ratio: f64,
    /// Segment length (nm).
    pub segment_length: i64,
    /// Blech length (nm). If `segment_length <= blech_length`, this
    /// violation is filtered by the Blech criterion and should not appear.
    pub blech_length: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_density_known_value() {
        // 1e-3 A / (100nm * 50nm) = 2e-7 A/nm^2
        assert_eq!(current_density(1e-3, 100, 50), 2e-7);
    }

    #[test]
    fn current_density_zero_current_is_zero() {
        assert_eq!(current_density(0.0, 100, 50), 0.0);
    }

    #[test]
    #[should_panic]
    fn current_density_panics_on_zero_width_in_debug() {
        current_density(1e-3, 0, 50);
    }

    #[test]
    #[should_panic]
    fn current_density_panics_on_zero_thickness_in_debug() {
        current_density(1e-3, 100, 0);
    }

    #[test]
    fn check_em_fires_when_over_limit_and_past_blech_length() {
        let v = check_em(2e-7, 1e-7, 5000, 1000, ShapeRef(0), LayerId(1)).unwrap();
        assert_eq!(v.j_actual, 2e-7);
        assert_eq!(v.j_max, 1e-7);
        assert_eq!(v.ratio, 2.0);
        assert_eq!(v.segment_length, 5000);
        assert_eq!(v.blech_length, 1000);
        assert_eq!(v.shape, ShapeRef(0));
        assert_eq!(v.layer, LayerId(1));
    }

    #[test]
    fn check_em_does_not_fire_when_under_limit() {
        assert!(check_em(5e-8, 1e-7, 5000, 1000, ShapeRef(0), LayerId(1)).is_none());
    }

    #[test]
    fn check_em_does_not_fire_for_short_segments() {
        // Over the J_max limit, but short enough for the Blech effect to apply.
        assert!(check_em(2e-7, 1e-7, 500, 1000, ShapeRef(0), LayerId(1)).is_none());
    }

    #[test]
    fn check_em_boundary_at_exactly_blech_length_does_not_fire() {
        assert!(check_em(2e-7, 1e-7, 1000, 1000, ShapeRef(0), LayerId(1)).is_none());
    }

    #[test]
    fn check_em_boundary_at_exactly_j_max_does_not_fire() {
        assert!(check_em(1e-7, 1e-7, 5000, 1000, ShapeRef(0), LayerId(1)).is_none());
    }
}
